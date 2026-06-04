//! At-rest encryption control surface: status + unlock (+ enable/disable, part B).
//!
//! The `vault.json` sentinel stays plaintext; its presence means "encryption is
//! on", which makes the app boot locked (no config/wallets/subscribers loaded
//! until unlock). See `docs/at-rest-encryption.md`.

use axum::{extract::State, Json};
use corvin_core::at_rest::{self, VaultSentinel};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::api::ApiError;
use crate::state::AppState;

/// True when the vault sentinel is on disk (encryption enabled).
pub fn sentinel_exists() -> bool {
    crate::config::vault_path().exists()
}

/// Claim the right to run a migration, or 400 if one is already underway. Held
/// across the whole handler so concurrent enable/disable/change-password can't
/// interleave and corrupt the vault.
fn claim_migration() -> Result<at_rest::MigrationClaim, ApiError> {
    at_rest::try_claim_migration()
        .ok_or_else(|| ApiError::bad_request("another encryption change is already in progress"))
}

pub fn load_sentinel() -> anyhow::Result<Option<VaultSentinel>> {
    let path = crate::config::vault_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read(&path)?;
    Ok(Some(serde_json::from_slice(&raw)?))
}

// ── Migration (enable / disable) ────────────────────────────────────────────
//
// Crash-safe by construction: we build a complete *converted* copy of the config
// dir in a staging sibling, write the sentinel into it (for enable), then atomic-
// swap it in via two renames. `recover_interrupted_migration` (run first thing at
// boot) reverts or completes an interrupted swap, so a crash never leaves a mix
// of plaintext and encrypted files in the live dir.

use std::path::{Path, PathBuf};
use std::time::Duration;

fn sibling(suffix: &str) -> PathBuf {
    let cd = crate::config::config_dir();
    let name = cd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "corvin".into());
    cd.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{name}.{suffix}"))
}
fn staging_dir() -> PathBuf {
    sibling("migrating")
}
fn backup_dir() -> PathBuf {
    sibling("old")
}

/// Files never migrated: the sentinel (must stay plaintext) and temp files.
fn skip_file(name: &std::ffi::OsStr) -> bool {
    let n = name.to_string_lossy();
    n == "vault.json" || n.ends_with(".tmp")
}

/// All regular files under the config dir (recursive), relative to it.
fn collect_files(base: &Path) -> std::io::Result<Vec<PathBuf>> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            let path = entry.path();
            if ft.is_dir() {
                walk(&path, base, out)?;
            } else if ft.is_file() && !skip_file(&entry.file_name()) {
                if let Ok(rel) = path.strip_prefix(base) {
                    out.push(rel.to_path_buf());
                }
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if base.exists() {
        walk(base, base, &mut out)?;
    }
    Ok(out)
}

fn build_staging(
    transform: impl Fn(&str, &[u8]) -> anyhow::Result<Vec<u8>>,
) -> anyhow::Result<()> {
    build_staging_in(&crate::config::config_dir(), &staging_dir(), transform)
}
fn swap_in_staging() -> anyhow::Result<()> {
    swap_in_staging_in(&crate::config::config_dir(), &staging_dir(), &backup_dir())
}

/// Run at the very start of boot: revert or finish an interrupted migration swap
/// so the live config dir is never a mix of plaintext + encrypted files.
pub fn recover_interrupted_migration() {
    recover_in(&crate::config::config_dir(), &staging_dir(), &backup_dir());
}

/// Build `staging` as a copy of `config` with each file's bytes run through
/// `transform(file_id, bytes)`. The sentinel + temp files are skipped.
fn build_staging_in(
    config: &Path,
    staging: &Path,
    transform: impl Fn(&str, &[u8]) -> anyhow::Result<Vec<u8>>,
) -> anyhow::Result<()> {
    if staging.exists() {
        std::fs::remove_dir_all(staging)?;
    }
    std::fs::create_dir_all(staging)?;
    for rel in collect_files(config)? {
        let bytes = std::fs::read(config.join(&rel))?;
        // AAD = root-relative path (matches `at_rest::write_sealed`'s `file_aad`).
        let file_id = at_rest::aad_for_relpath(&rel);
        let out = transform(&file_id, &bytes)?;
        let dst = staging.join(&rel);
        if let Some(p) = dst.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&dst, &out)?;
        crate::state::restrict_to_owner(&dst);
    }
    Ok(())
}

/// Atomically replace `config` with `staging` (two renames). An interrupted swap
/// is recovered by `recover_in`.
fn swap_in_staging_in(config: &Path, staging: &Path, backup: &Path) -> anyhow::Result<()> {
    if backup.exists() {
        std::fs::remove_dir_all(backup)?;
    }
    std::fs::rename(config, backup)?;
    std::fs::rename(staging, config)?;
    std::fs::remove_dir_all(backup)?;
    Ok(())
}

fn recover_in(config: &Path, staging: &Path, backup: &Path) {
    if backup.exists() {
        if config.exists() {
            // Swap finished; clean up leftovers.
            let _ = std::fs::remove_dir_all(backup);
            let _ = std::fs::remove_dir_all(staging);
        } else {
            // Crashed between the two renames — restore the original dir.
            tracing::warn!("recovering interrupted at-rest migration: restoring config dir");
            let _ = std::fs::rename(backup, config);
            let _ = std::fs::remove_dir_all(staging);
        }
    } else if staging.exists() {
        // Crashed building staging; the live dir is untouched — discard it.
        let _ = std::fs::remove_dir_all(staging);
    }
}

#[derive(Deserialize)]
pub struct EnableRequest {
    pub password: String,
}

/// `POST /security/enable` — turn on at-rest encryption: derive a key, seal every
/// file into staging (sentinel included), atomic-swap it in, and unlock.
pub async fn enable(
    State(_state): State<AppState>,
    Json(mut req): Json<EnableRequest>,
) -> Result<Json<SecurityStatus>, ApiError> {
    if at_rest::is_encryption_on() {
        return Err(ApiError::bad_request("encryption is already enabled"));
    }
    let _claim = claim_migration()?;
    let password = Zeroizing::new(std::mem::take(&mut req.password));
    if password.chars().count() < 8 {
        return Err(ApiError::bad_request("password must be at least 8 characters"));
    }

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let params = at_rest::calibrate(Duration::from_millis(750), 64 * 1024);
        let salt = at_rest::random_salt();
        let master = at_rest::derive_master_key(&password, &salt, &params)?;
        let sentinel = VaultSentinel::create(&master, &salt, params)?;
        let fk = at_rest::file_key(&master);
        // Block background writers for the snapshot + swap (after the slow KDF).
        let _mig = at_rest::begin_migration();
        build_staging(|file_id, plaintext| at_rest::encrypt(&fk, file_id.as_bytes(), plaintext))?;
        // Sentinel into staging *before* the swap, so the swap brings files +
        // sentinel together (a crash never leaves sealed files without a sentinel).
        let sp = staging_dir().join("vault.json");
        std::fs::write(&sp, serde_json::to_vec_pretty(&sentinel)?)?;
        crate::state::restrict_to_owner(&sp);
        swap_in_staging()?;
        // Flip the running vault immediately (the window vs background writes is
        // a few instructions).
        at_rest::unlock_with(&master);
        Ok(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("enable task panicked: {e}"))?
    .map_err(|e| anyhow::anyhow!("enabling encryption failed: {e}"))?;

    Ok(Json(current_status()))
}

#[derive(Deserialize)]
pub struct DisableRequest {
    pub password: String,
}

/// `POST /security/disable` — turn off at-rest encryption: decrypt every file into
/// plaintext staging (no sentinel), atomic-swap it in, and clear the vault.
///
/// Requires the password even though the vault is already unlocked: disabling
/// rewrites the whole config dir to plaintext on disk (a downgrade that outlives the
/// session), so we re-verify against the sentinel rather than trust an unattended
/// unlocked session.
pub async fn disable(
    State(_state): State<AppState>,
    Json(mut req): Json<DisableRequest>,
) -> Result<Json<SecurityStatus>, ApiError> {
    if !at_rest::is_encryption_on() {
        return Err(ApiError::bad_request("encryption is not enabled"));
    }
    let _claim = claim_migration()?;
    let password = Zeroizing::new(std::mem::take(&mut req.password));
    let sentinel = load_sentinel()?.ok_or_else(|| anyhow::anyhow!("no vault sentinel found"))?;
    let master = sentinel
        .unlock(&password)
        .map_err(|_| ApiError::wrong_secret("incorrect password"))?;
    let fk = at_rest::file_key(&master);
    drop(master);

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let _mig = at_rest::begin_migration();
        build_staging(|file_id, ct| {
            at_rest::decrypt(&fk, file_id.as_bytes(), ct).map(|z| z.to_vec())
        })?;
        // Staging has no sentinel, so the swapped-in dir boots plaintext.
        swap_in_staging()?;
        at_rest::set_vault(corvin_core::at_rest::VaultState::Plaintext);
        Ok(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("disable task panicked: {e}"))?
    .map_err(|e| anyhow::anyhow!("disabling encryption failed: {e}"))?;

    Ok(Json(current_status()))
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// `POST /security/change-password` — re-key the whole config dir under a new
/// password: verify the current one, derive a new master (fresh salt + params),
/// decrypt-then-re-encrypt every file in staging under the new key, write the new
/// sentinel, atomic-swap, and update the running vault. No disable round-trip, so
/// plaintext never touches the disk.
pub async fn change_password(
    State(_state): State<AppState>,
    Json(mut req): Json<ChangePasswordRequest>,
) -> Result<Json<SecurityStatus>, ApiError> {
    if !at_rest::is_encryption_on() {
        return Err(ApiError::bad_request("encryption is not enabled"));
    }
    let _claim = claim_migration()?;
    let current = Zeroizing::new(std::mem::take(&mut req.current_password));
    let new = Zeroizing::new(std::mem::take(&mut req.new_password));
    if new.chars().count() < 8 {
        return Err(ApiError::bad_request("new password must be at least 8 characters"));
    }
    let sentinel = load_sentinel()?.ok_or_else(|| anyhow::anyhow!("no vault sentinel found"))?;
    let old_master = sentinel
        .unlock(&current)
        .map_err(|_| ApiError::wrong_secret("incorrect current password"))?;
    let old_fk = at_rest::file_key(&old_master);
    drop(old_master);

    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let params = at_rest::calibrate(Duration::from_millis(750), 64 * 1024);
        let salt = at_rest::random_salt();
        let new_master = at_rest::derive_master_key(&new, &salt, &params)?;
        let new_sentinel = VaultSentinel::create(&new_master, &salt, params)?;
        let new_fk = at_rest::file_key(&new_master);
        // Block background writers for the snapshot + swap (after the slow KDF).
        let _mig = at_rest::begin_migration();
        build_staging(|file_id, ct| {
            let pt = at_rest::decrypt(&old_fk, file_id.as_bytes(), ct)?;
            at_rest::encrypt(&new_fk, file_id.as_bytes(), &pt)
        })?;
        let sp = staging_dir().join("vault.json");
        std::fs::write(&sp, serde_json::to_vec_pretty(&new_sentinel)?)?;
        crate::state::restrict_to_owner(&sp);
        swap_in_staging()?;
        at_rest::unlock_with(&new_master);
        Ok(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("change-password task panicked: {e}"))?
    .map_err(|e| anyhow::anyhow!("changing password failed: {e}"))?;

    Ok(Json(current_status()))
}

#[derive(Serialize)]
pub struct SecurityStatus {
    /// "off" (no encryption), "locked" (on, not unlocked), or "unlocked".
    pub state: &'static str,
}

fn current_status() -> SecurityStatus {
    let state = if !at_rest::is_encryption_on() {
        "off"
    } else if at_rest::is_locked() {
        "locked"
    } else {
        "unlocked"
    };
    SecurityStatus { state }
}

/// `GET /security/status`
pub async fn get_security_status(State(_state): State<AppState>) -> Json<SecurityStatus> {
    Json(current_status())
}

#[derive(Deserialize)]
pub struct UnlockRequest {
    pub password: String,
}

/// `POST /security/unlock` — verify the password, unlock the vault, and run the
/// deferred startup (load config + wallets, start subscribers).
pub async fn unlock(
    State(state): State<AppState>,
    Json(mut req): Json<UnlockRequest>,
) -> Result<Json<SecurityStatus>, ApiError> {
    let password = Zeroizing::new(std::mem::take(&mut req.password));
    if !at_rest::is_locked() {
        return Err(ApiError::bad_request("the vault is not locked"));
    }
    let sentinel = load_sentinel()?
        .ok_or_else(|| anyhow::anyhow!("no vault sentinel found"))?;
    // Serialize unlock attempts: it makes the ~750ms Argon2 cost a real rate limit
    // (parallel requests can't run the KDF concurrently to bypass it) and caps memory
    // at one 64 MiB hash rather than N. The legitimate single user never contends.
    static UNLOCK_GATE: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));
    let _gate = UNLOCK_GATE.lock().await;
    // Argon2 is deliberately slow (~750ms); keep it off the async runtime thread.
    let master = tokio::task::spawn_blocking(move || sentinel.unlock(&password))
        .await
        .map_err(|e| anyhow::anyhow!("unlock task panicked: {e}"))?
        .map_err(|_| ApiError::wrong_secret("incorrect password"))?;
    at_rest::unlock_with(&master);
    drop(master);

    // Files are now decryptable — bring the app up.
    crate::run_startup_after_unlock(&state)
        .await
        .map_err(|e| anyhow::anyhow!("startup after unlock failed: {e}"))?;

    Ok(Json(current_status()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvin_core::at_rest;

    // Exercises the migration mechanics directly with explicit keys + temp dirs,
    // so it never touches the shared config dir or the global vault state.
    #[test]
    fn migration_roundtrip_and_recovery() {
        let root = std::env::temp_dir().join(format!("corvin-mig-{}", uuid::Uuid::new_v4()));
        let config = root.join("cfg");
        let staging = root.join("cfg.migrating");
        let backup = root.join("cfg.old");
        std::fs::create_dir_all(config.join("wallets")).unwrap();
        std::fs::write(config.join("labels.json"), b"{\"a\":\"b\"}").unwrap();
        std::fs::write(config.join("wallets").join("w.db"), b"changeset-bytes").unwrap();
        std::fs::write(config.join("vault.json"), b"sentinel").unwrap(); // skipped
        std::fs::write(config.join("scratch.tmp"), b"temp").unwrap(); // skipped

        let key = [9u8; 32];

        // Encrypt: build staging, then swap it in.
        build_staging_in(&config, &staging, |fid, pt| {
            at_rest::encrypt(&key, fid.as_bytes(), pt)
        })
        .unwrap();
        assert!(!staging.join("vault.json").exists(), "sentinel is not migrated");
        assert!(!staging.join("scratch.tmp").exists(), "temp files are not migrated");
        swap_in_staging_in(&config, &staging, &backup).unwrap();

        // Files on disk are now ciphertext that decrypts back (AAD = file name,
        // even in a subdir).
        let enc = std::fs::read(config.join("labels.json")).unwrap();
        assert_ne!(enc, b"{\"a\":\"b\"}");
        assert_eq!(
            at_rest::decrypt(&key, b"labels.json", &enc).unwrap().as_slice(),
            b"{\"a\":\"b\"}"
        );
        let wenc = std::fs::read(config.join("wallets").join("w.db")).unwrap();
        assert_eq!(
            // AAD is the root-relative path now, not the basename.
            at_rest::decrypt(&key, b"wallets/w.db", &wenc).unwrap().as_slice(),
            b"changeset-bytes"
        );

        // Decrypt back: round-trips to the original plaintext.
        build_staging_in(&config, &staging, |fid, ct| {
            at_rest::decrypt(&key, fid.as_bytes(), ct).map(|z| z.to_vec())
        })
        .unwrap();
        swap_in_staging_in(&config, &staging, &backup).unwrap();
        assert_eq!(
            std::fs::read(config.join("labels.json")).unwrap(),
            b"{\"a\":\"b\"}"
        );

        // Recovery: simulate a crash mid-swap (config renamed away, not yet
        // replaced) — recovery restores the original.
        std::fs::rename(&config, &backup).unwrap();
        assert!(!config.exists());
        recover_in(&config, &staging, &backup);
        assert!(config.exists(), "recovery restored the config dir");
        assert_eq!(
            std::fs::read(config.join("labels.json")).unwrap(),
            b"{\"a\":\"b\"}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    // The change-password re-key: decrypt under the old file key and re-encrypt
    // under the new one, in a single staging pass. Verifies the swapped-in files
    // open with the new key and no longer with the old.
    #[test]
    fn rekey_reencrypts_under_new_key() {
        let root = std::env::temp_dir().join(format!("corvin-rekey-{}", uuid::Uuid::new_v4()));
        let config = root.join("cfg");
        let staging = root.join("cfg.migrating");
        let backup = root.join("cfg.old");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("labels.json"), b"{\"a\":\"b\"}").unwrap();

        let old_key = [1u8; 32];
        let new_key = [2u8; 32];

        // Start from ciphertext under the old key.
        build_staging_in(&config, &staging, |fid, pt| at_rest::encrypt(&old_key, fid.as_bytes(), pt))
            .unwrap();
        swap_in_staging_in(&config, &staging, &backup).unwrap();

        // Re-key: decrypt(old) → encrypt(new).
        build_staging_in(&config, &staging, |fid, ct| {
            let pt = at_rest::decrypt(&old_key, fid.as_bytes(), ct)?;
            at_rest::encrypt(&new_key, fid.as_bytes(), &pt)
        })
        .unwrap();
        swap_in_staging_in(&config, &staging, &backup).unwrap();

        let blob = std::fs::read(config.join("labels.json")).unwrap();
        assert!(at_rest::decrypt(&old_key, b"labels.json", &blob).is_err(), "old key no longer opens it");
        assert_eq!(
            at_rest::decrypt(&new_key, b"labels.json", &blob).unwrap().as_slice(),
            b"{\"a\":\"b\"}"
        );

        std::fs::remove_dir_all(&root).ok();
    }
}
