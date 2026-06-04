//! At-rest encryption primitives.
//!
//! One cipher for the whole product: every encrypted file (the JSON stores and
//! the wallet `ChangeSet` blob) is sealed with XChaCha20-Poly1305 under a key
//! derived from the user's password via Argon2id. HKDF-SHA256 splits the master
//! into purpose-separated subkeys, and each ciphertext is bound to its file's
//! logical identity via the AEAD's associated data, so a blob can't be swapped
//! in for a different file.
//!
//! Layout of an encrypted blob: `nonce(24) || ciphertext+tag`.
//!
//! The [`VaultSentinel`] stays plaintext on disk. Its *presence* signals
//! "encryption is on, boot locked"; it carries the KDF salt + params and a
//! verifier (a known plaintext sealed under the key) so a wrong password fails
//! fast without touching any real data.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;

const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24; // XChaCha20 nonce

/// HKDF domain-separation contexts. Each key use gets its own subkey.
const FILE_SUBKEY_CONTEXT: &str = "corvin/at-rest/file/v1";
const VERIFIER_SUBKEY_CONTEXT: &str = "corvin/at-rest/verifier/v1";
/// AAD + known plaintext for the sentinel verifier.
const SENTINEL_AAD: &[u8] = b"corvin/at-rest/sentinel/v1";
const VERIFIER_PLAINTEXT: &[u8] = b"corvin-vault-ok";

/// 32-byte master key, zeroized on drop.
pub type MasterKey = Zeroizing<[u8; KEY_LEN]>;
/// A purpose-separated 32-byte subkey, zeroized on drop.
pub type SubKey = Zeroizing<[u8; KEY_LEN]>;

/// Argon2id cost parameters, persisted in the sentinel so any host re-derives
/// the same key.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory in KiB.
    pub mem_kib: u32,
    /// Number of passes.
    pub time_cost: u32,
    pub parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        // A sane floor; real installs overwrite this via `calibrate` at enable time.
        Self {
            mem_kib: 64 * 1024,
            time_cost: 3,
            parallelism: 1,
        }
    }
}

fn argon2_for(params: &KdfParams) -> Result<Argon2<'static>> {
    let p = Params::new(
        params.mem_kib,
        params.time_cost,
        params.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|e| anyhow!("invalid argon2 params: {e}"))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, p))
}

/// Derive the master key from a password + salt with Argon2id.
pub fn derive_master_key(password: &str, salt: &[u8], params: &KdfParams) -> Result<MasterKey> {
    let argon2 = argon2_for(params)?;
    let mut key: MasterKey = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(password.as_bytes(), salt, key.as_mut_slice())
        .map_err(|e| anyhow!("argon2 derive: {e}"))?;
    Ok(key)
}

/// A fresh random 16-byte salt.
pub fn random_salt() -> [u8; SALT_LEN] {
    let mut s = [0u8; SALT_LEN];
    getrandom::getrandom(&mut s).expect("OS RNG unavailable");
    s
}

/// HKDF-SHA256 purpose-separated subkey. The master is already high-entropy, so
/// no HKDF salt is needed; `context` is the info string that domain-separates uses.
pub fn subkey(master: &[u8; KEY_LEN], context: &str) -> SubKey {
    let hk = Hkdf::<Sha256>::new(None, master);
    let mut out: SubKey = Zeroizing::new([0u8; KEY_LEN]);
    hk.expand(context.as_bytes(), out.as_mut_slice())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    out
}

/// The file-encryption subkey used by every encrypted file/blob.
pub fn file_key(master: &[u8; KEY_LEN]) -> SubKey {
    subkey(master, FILE_SUBKEY_CONTEXT)
}

/// Seal `plaintext` under `key`, binding it to `aad` (the file's logical id, e.g.
/// its name) so ciphertexts can't be moved between files. Returns
/// `nonce || ciphertext+tag`.
pub fn encrypt(key: &[u8; KEY_LEN], aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| anyhow!("bad key length"))?;
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(&nonce, Payload { msg: plaintext, aad })
        .map_err(|_| anyhow!("encryption failed"))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Open a blob produced by [`encrypt`], verifying `aad`. Wrong key, a wrong aad,
/// truncation, or any tampering all fail.
pub fn decrypt(key: &[u8; KEY_LEN], aad: &[u8], blob: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if blob.len() < NONCE_LEN {
        bail!("ciphertext too short");
    }
    let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| anyhow!("bad key length"))?;
    let nonce = XNonce::from_slice(nonce_bytes);
    let pt = cipher
        .decrypt(nonce, Payload { msg: ct, aad })
        .map_err(|_| anyhow!("decryption failed (wrong password, or corrupt/tampered file)"))?;
    Ok(Zeroizing::new(pt))
}

/// Pick Argon2id params that take roughly `target` to derive on this host, at a
/// fixed memory cost. Scales `time_cost` up from 1 until a single derive reaches
/// the target (capped, so a slow box can't pick absurd params).
pub fn calibrate(target: Duration, mem_kib: u32) -> KdfParams {
    const MAX_TIME_COST: u32 = 20;
    let parallelism = 1;
    let salt = [0u8; SALT_LEN];
    let mut time_cost = 1u32;
    loop {
        let params = KdfParams {
            mem_kib,
            time_cost,
            parallelism,
        };
        let start = Instant::now();
        let _ = derive_master_key("calibration", &salt, &params);
        let elapsed = start.elapsed();
        if elapsed >= target || time_cost >= MAX_TIME_COST {
            return params;
        }
        // Jump toward the target, but always advance by at least one pass.
        let factor = (target.as_secs_f64() / elapsed.as_secs_f64().max(1e-6)).max(1.0);
        let next = ((time_cost as f64) * factor).ceil() as u32;
        time_cost = next.clamp(time_cost + 1, MAX_TIME_COST);
    }
}

/// The plaintext on-disk sentinel. Its presence means "encryption is on".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultSentinel {
    pub version: u32,
    /// KDF name, currently always "argon2id".
    pub kdf: String,
    /// Hex-encoded salt.
    pub salt: String,
    pub params: KdfParams,
    /// Hex-encoded `encrypt(verifier_subkey, SENTINEL_AAD, VERIFIER_PLAINTEXT)`.
    pub verifier: String,
}

impl VaultSentinel {
    /// Build a fresh sentinel for an already-derived `master` key.
    pub fn create(master: &[u8; KEY_LEN], salt: &[u8], params: KdfParams) -> Result<Self> {
        let vk = subkey(master, VERIFIER_SUBKEY_CONTEXT);
        let verifier = encrypt(&vk, SENTINEL_AAD, VERIFIER_PLAINTEXT)?;
        Ok(Self {
            version: 1,
            kdf: "argon2id".to_string(),
            salt: hex::encode(salt),
            params,
            verifier: hex::encode(verifier),
        })
    }

    /// Re-derive the master key from `password` and verify it against this
    /// sentinel. Returns the master key on success; a wrong password fails.
    pub fn unlock(&self, password: &str) -> Result<MasterKey> {
        let salt = hex::decode(&self.salt).map_err(|_| anyhow!("malformed sentinel salt"))?;
        let master = derive_master_key(password, &salt, &self.params)?;
        let vk = subkey(&master, VERIFIER_SUBKEY_CONTEXT);
        let blob = hex::decode(&self.verifier).map_err(|_| anyhow!("malformed verifier"))?;
        let pt = decrypt(&vk, SENTINEL_AAD, &blob).map_err(|_| anyhow!("incorrect password"))?;
        if pt.as_slice() != VERIFIER_PLAINTEXT {
            bail!("vault verifier mismatch");
        }
        Ok(master)
    }
}

// ── Process-wide vault state ────────────────────────────────────────────────
//
// Exactly one config dir per process, so a singleton models the encryption state
// correctly. The file-I/O helpers consult it instead of threading a key through
// every call site. Set at startup (from the sentinel), and on unlock/enable/
// disable/lock.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

pub enum VaultState {
    /// Encryption off: read/write plaintext (default, today's behaviour).
    Plaintext,
    /// Encryption on but not unlocked: real data must not be read or written.
    Locked,
    /// Encryption on and unlocked: the file subkey is held in memory.
    Unlocked(SubKey),
}

static VAULT: RwLock<VaultState> = RwLock::new(VaultState::Plaintext);

fn vault() -> std::sync::RwLockReadGuard<'static, VaultState> {
    VAULT.read().unwrap_or_else(|e| e.into_inner())
}

/// Serializes config-dir migrations (enable/disable/change-password) against
/// background writes. Every `write_sealed` holds this shared; a migration holds
/// it exclusively across its snapshot + atomic swap + vault-state flip, so a
/// concurrent write can't land in the directory that's about to be replaced
/// (where the swap would silently discard it).
static MIGRATION: RwLock<()> = RwLock::new(());

/// Exclusive migration hold. While alive, every `write_sealed` call blocks.
pub struct MigrationGuard(#[allow(dead_code)] std::sync::RwLockWriteGuard<'static, ()>);

/// Acquire the exclusive migration lock, waiting for in-flight writes to drain.
/// Keep the returned guard for the whole build-staging → swap → vault-flip.
pub fn begin_migration() -> MigrationGuard {
    MigrationGuard(MIGRATION.write().unwrap_or_else(|e| e.into_inner()))
}

/// Set once and only one migration may proceed: enable/disable/change-password
/// each claim this at handler entry (before the slow KDF), so a second concurrent
/// request fails fast instead of running a second migration against a directory
/// whose encryption state the first one is changing (which would double-seal and
/// lose data).
static MIGRATION_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Held for the full duration of one migration handler; clears the flag on drop.
pub struct MigrationClaim(());

impl Drop for MigrationClaim {
    fn drop(&mut self) {
        MIGRATION_IN_PROGRESS.store(false, Ordering::Release);
    }
}

/// Claim the right to run a migration. `None` if one is already in progress.
pub fn try_claim_migration() -> Option<MigrationClaim> {
    MIGRATION_IN_PROGRESS
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .ok()
        .map(|_| MigrationClaim(()))
}

/// The config-dir root, so a file's AEAD AAD can be its path *relative to the
/// root* (e.g. `wallets/<uuid>.changeset`) rather than just its basename — that
/// binds a sealed blob to its location, not merely its name, so two same-named
/// files in different subdirs can't be swapped.
static CONFIG_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Record the config-dir root for relative-path AAD. Call once at boot (and in
/// tests) before any sealed read/write.
pub fn set_config_root(root: PathBuf) {
    *CONFIG_ROOT.write().unwrap_or_else(|e| e.into_inner()) = Some(root);
}

/// AAD string for a relative path: components joined with `/` (stable across
/// platforms, unlike `to_string_lossy` on the whole path).
pub fn aad_for_relpath(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// The AEAD AAD for an absolute config-dir path: its root-relative path, or the
/// basename if the root isn't set or the path isn't under it.
fn file_aad(path: &Path) -> String {
    let root = CONFIG_ROOT.read().unwrap_or_else(|e| e.into_inner());
    match root.as_ref().and_then(|r| path.strip_prefix(r).ok()) {
        Some(rel) => aad_for_relpath(rel),
        None => path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
    }
}

/// Replace the process vault state.
pub fn set_vault(state: VaultState) {
    *VAULT.write().unwrap_or_else(|e| e.into_inner()) = state;
}

/// Move to the unlocked state from an already-derived master key. Stores only
/// the file subkey; the caller's master key can then be dropped.
pub fn unlock_with(master: &[u8; KEY_LEN]) {
    set_vault(VaultState::Unlocked(file_key(master)));
}

/// True when encryption is enabled (locked or unlocked).
pub fn is_encryption_on() -> bool {
    !matches!(&*vault(), VaultState::Plaintext)
}

/// True when encryption is on but not yet unlocked.
pub fn is_locked() -> bool {
    matches!(&*vault(), VaultState::Locked)
}

/// The current file-encryption subkey, when unlocked. Used by the disable
/// migration to decrypt files with an explicit key (rather than flipping the
/// global state mid-migration).
pub fn current_file_key() -> Option<SubKey> {
    match &*vault() {
        VaultState::Unlocked(fk) => Some(fk.clone()),
        _ => None,
    }
}

/// Transform plaintext bytes for writing to the file identified by `file_id`
/// (its name, bound as AAD): passthrough when off, AEAD-sealed when unlocked, an
/// error when locked (nothing should persist while locked).
pub fn seal_for_disk(file_id: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
    match &*vault() {
        VaultState::Plaintext => Ok(plaintext.to_vec()),
        VaultState::Unlocked(fk) => encrypt(fk, file_id.as_bytes(), plaintext),
        VaultState::Locked => bail!("vault is locked; refusing to write {file_id}"),
    }
}

/// Reverse of [`seal_for_disk`] when reading `file_id` from disk.
pub fn open_from_disk(file_id: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    match &*vault() {
        VaultState::Plaintext => Ok(bytes.to_vec()),
        VaultState::Unlocked(fk) => decrypt(fk, file_id.as_bytes(), bytes).map(|z| z.to_vec()),
        VaultState::Locked => bail!("vault is locked; refusing to read {file_id}"),
    }
}

#[cfg(unix)]
fn restrict_to_owner(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn restrict_to_owner(_path: &std::path::Path) {}

/// Atomic, owner-only (0600), at-rest-sealed write. Seal `plaintext` (bound to
/// the file name as AAD), write to a temp file, fsync the data, chmod, then
/// rename. The single shared writer for every persisted store, plaintext or not.
pub fn write_sealed(path: &std::path::Path, plaintext: &[u8]) -> Result<()> {
    use std::io::Write;
    // Held shared for the whole write so a migration's exclusive swap can't run
    // in the middle of it (and so our write can't be lost to the swap).
    let _writing = MIGRATION.read().unwrap_or_else(|e| e.into_inner());
    let file_id = file_aad(path);
    let bytes = seal_for_disk(&file_id, plaintext)?;
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(&bytes)
            .with_context(|| format!("writing {}", tmp.display()))?;
        f.sync_all()
            .with_context(|| format!("fsync {}", tmp.display()))?;
    }
    restrict_to_owner(&tmp);
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Read + un-seal a file written by [`write_sealed`]. `Ok(None)` if absent.
pub fn read_sealed(path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    // Held shared so a read can't observe the brief window where a migration has
    // swapped the on-disk format but not yet flipped the vault state.
    let _reading = MIGRATION.read().unwrap_or_else(|e| e.into_inner());
    if !path.exists() {
        return Ok(None);
    }
    let file_id = file_aad(path);
    let raw = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(open_from_disk(&file_id, &raw)?))
}

// Tests across this crate that touch the process-global VAULT (here and the
// wallet_store changeset test) must not run concurrently — they'd observe each other's
// lock/unlock state. Serialize them on this lock. (`cargo test` happened to pass;
// `cargo llvm-cov`'s ordering exposed the race.)
#[cfg(test)]
pub(crate) static VAULT_TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn vault_guard() -> std::sync::MutexGuard<'static, ()> {
        VAULT_TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    // Fast params so the memory-hard KDF doesn't slow the suite (min mem is
    // 8*parallelism KiB). Real installs calibrate to ~750ms.
    fn fast() -> KdfParams {
        KdfParams {
            mem_kib: 16,
            time_cost: 1,
            parallelism: 1,
        }
    }

    #[test]
    fn derive_is_deterministic_and_password_sensitive() {
        let salt = [7u8; SALT_LEN];
        let a = derive_master_key("hunter2", &salt, &fast()).unwrap();
        let b = derive_master_key("hunter2", &salt, &fast()).unwrap();
        let c = derive_master_key("hunter3", &salt, &fast()).unwrap();
        assert_eq!(*a, *b, "same inputs derive the same key");
        assert_ne!(*a, *c, "a different password derives a different key");
        // A different salt also diverges.
        let d = derive_master_key("hunter2", &[9u8; SALT_LEN], &fast()).unwrap();
        assert_ne!(*a, *d);
    }

    #[test]
    fn encrypt_decrypt_roundtrips() {
        let key = [3u8; KEY_LEN];
        let blob = encrypt(&key, b"wallets", b"secret state").unwrap();
        assert_ne!(&blob[NONCE_LEN..], b"secret state", "ciphertext isn't plaintext");
        let pt = decrypt(&key, b"wallets", &blob).unwrap();
        assert_eq!(pt.as_slice(), b"secret state");
    }

    #[test]
    fn wrong_key_aad_or_tamper_all_fail() {
        let key = [3u8; KEY_LEN];
        let other = [4u8; KEY_LEN];
        let blob = encrypt(&key, b"wallets", b"secret state").unwrap();
        assert!(decrypt(&other, b"wallets", &blob).is_err(), "wrong key fails");
        assert!(decrypt(&key, b"labels", &blob).is_err(), "wrong aad (file-swap) fails");
        let mut tampered = blob.clone();
        *tampered.last_mut().unwrap() ^= 0x01;
        assert!(decrypt(&key, b"wallets", &tampered).is_err(), "tampering fails");
        assert!(decrypt(&key, b"wallets", &blob[..NONCE_LEN - 1]).is_err(), "truncation fails");
    }

    #[test]
    fn nonce_is_random_per_write() {
        let key = [3u8; KEY_LEN];
        let a = encrypt(&key, b"f", b"x").unwrap();
        let b = encrypt(&key, b"f", b"x").unwrap();
        assert_ne!(a, b, "same plaintext encrypts to different ciphertext (random nonce)");
    }

    #[test]
    fn subkeys_are_domain_separated() {
        let master = [1u8; KEY_LEN];
        assert_eq!(*subkey(&master, "a"), *subkey(&master, "a"), "deterministic");
        assert_ne!(*subkey(&master, "a"), *subkey(&master, "b"), "context separates");
        assert_ne!(*file_key(&master), *subkey(&master, VERIFIER_SUBKEY_CONTEXT));
    }

    #[test]
    fn sentinel_unlocks_only_with_the_right_password() {
        let salt = random_salt();
        let master = derive_master_key("correct horse", &salt, &fast()).unwrap();
        // Build the sentinel by hand with the fast params so the test stays quick.
        let sentinel = VaultSentinel::create(&master, &salt, fast()).unwrap();
        let unlocked = sentinel.unlock("correct horse").unwrap();
        assert_eq!(*unlocked, *master, "right password recovers the same master key");
        assert!(sentinel.unlock("battery staple").is_err(), "wrong password rejected");
    }

    #[test]
    fn seal_open_follows_vault_state() {
        let _serial = vault_guard();
        // Plaintext (default): passthrough both ways.
        assert_eq!(seal_for_disk("wallets.json", b"hi").unwrap(), b"hi");
        assert_eq!(open_from_disk("wallets.json", b"hi").unwrap(), b"hi");

        // Unlocked: round-trips through real AEAD, and the AAD (file id) is bound.
        let master = [5u8; KEY_LEN];
        unlock_with(&master);
        let sealed = seal_for_disk("wallets.json", b"secret").unwrap();
        assert_ne!(sealed, b"secret");
        assert_eq!(open_from_disk("wallets.json", &sealed).unwrap(), b"secret");
        assert!(
            open_from_disk("labels.json", &sealed).is_err(),
            "a blob can't be opened as a different file"
        );

        // Locked: refuses I/O.
        set_vault(VaultState::Locked);
        assert!(seal_for_disk("x", b"y").is_err());
        assert!(open_from_disk("x", b"y").is_err());

        // Restore the default so other tests see Plaintext.
        set_vault(VaultState::Plaintext);
    }

    #[test]
    fn migration_claim_is_exclusive_and_frees_on_drop() {
        let c1 = try_claim_migration();
        assert!(c1.is_some(), "first claim succeeds");
        assert!(try_claim_migration().is_none(), "a second concurrent claim is refused");
        drop(c1);
        let c2 = try_claim_migration();
        assert!(c2.is_some(), "the claim frees on drop");
        drop(c2);
    }

    #[test]
    fn relpath_aad_binds_location_not_just_name() {
        let _serial = vault_guard();
        // Two files with the same basename in different subdirs get distinct AAD,
        // so a blob sealed for one can't be opened as the other.
        let a = aad_for_relpath(std::path::Path::new("payjoin/index.json"));
        let b = aad_for_relpath(std::path::Path::new("other/index.json"));
        assert_eq!(a, "payjoin/index.json");
        assert_ne!(a, b);

        let master = [7u8; KEY_LEN];
        unlock_with(&master);
        let sealed = seal_for_disk(&a, b"data").unwrap();
        assert!(open_from_disk(&b, &sealed).is_err(), "same-name different-dir can't be swapped");
        assert_eq!(open_from_disk(&a, &sealed).unwrap(), b"data");
        set_vault(VaultState::Plaintext);
    }

    #[test]
    fn calibrate_returns_usable_params() {
        // Tiny target + tiny memory so this is fast; just assert it produces a
        // valid, derivable param set.
        let p = calibrate(Duration::from_millis(1), 16);
        assert!(p.time_cost >= 1 && p.mem_kib == 16);
        assert!(derive_master_key("x", &[0u8; SALT_LEN], &p).is_ok());
    }
}
