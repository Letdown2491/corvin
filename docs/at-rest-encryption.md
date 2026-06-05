# At-rest encryption

Opt-in, default off. When enabled, everything Corvin stores under its config
directory is encrypted under a user password, and the app boots locked behind a
full-screen unlock gate. Crypto core is in `core/at_rest.rs`, wallet persistence in
`core/wallet_store.rs`, the control surface in `server/api/security.rs`, and the
frontend gate in `UnlockGate.svelte` + `SecurityPage.svelte`.

## Goal & threat model

Optionally encrypt the config directory at rest so that someone with raw access to the
disk (a stolen or seized device, a copied or cloud-synced config dir, an old drive, a
backup) can't read it.

**This is a privacy feature, not anti-theft.** Corvin never writes a seed or spend key
to disk; signing re-derives from the mnemonic supplied at use time, so a stolen disk can
already never spend funds. What it can read in plaintext without this feature, and what
encryption protects:

- **xpubs / descriptors** (`wallets.json`, the wallet store): the master public keys
  derive every past and future address, so they expose the wallet's entire balance and
  transaction history.
- **SP scan secret** (`silent_payments.json`): a real secret that lets a holder discover
  all your Silent Payments receipts (not spend them).
- **RPC password** (`config.toml`), labels, cost basis, UTXO data, payjoin sessions,
  Ledger HMACs.

Files are already `0600` (owner-only), which stops other logged-in users but not someone
reading the raw bytes. On desktop, OS full-disk encryption (LUKS/FileVault/BitLocker)
covers the raw-disk case, so the marginal value of app-level encryption is highest on
Start9 (StartOS has no full-disk encryption yet, Start9Labs/start-os#735) and for
backups or copied config dirs.

## How it works

- **Opt-in, default off.** A fresh install boots straight in, unencrypted.
- **Coverage is everything** under the config dir: all JSON plus the wallet data. Wallet
  data persists as a sealed CBOR `ChangeSet` blob
  (`core/wallet_store.rs::EncryptedChangeSetStore`), not a SQLite `.db`, so the whole
  product uses one cipher and no C dependency.
- **Key source is the user password**, Argon2id-derived. The password is the protection;
  the key never touches disk.
- **Unlock-on-access (Alby-Hub style).** When encryption is on, the app boots locked:
  nothing is read, wallets aren't loaded, and the Electrum subscriber and SP scanner
  don't start. The user opens the URL (localhost on desktop, the Start9 URL on a server),
  enters the password on a full-screen unlock gate, and the app decrypts into memory and
  starts. This makes the strong password model viable on headless Start9. The accepted
  consequence: after a reboot, background sync and new-tx notifications pause until
  someone unlocks.
- **Disable** decrypts back to plaintext, and re-verifies the password against the
  sentinel first (it rewrites the whole config dir to plaintext, a downgrade that
  outlives the session).
- **Change-password** verifies the current password, derives a new key from a fresh
  salt, then decrypts and re-encrypts every file under the new key in one staging pass
  before an atomic swap, so plaintext never hits disk during the change.
- **No idle auto-lock in v1.** Unlock once per process start.
- **Forgot-password is privacy/annotation loss, never fund loss.** No keys are on disk,
  so recovery is re-adding wallets from xpub or seed; you lose labels, cost-basis, and
  history. The enable screen says this plainly.

## Crypto

- **KDF:** Argon2id, tuned to roughly 750ms on modern hardware (OWASP guidance). Random
  salt stored in the vault sentinel.
- **AEAD:** XChaCha20-Poly1305, random nonce per write. Pure-Rust RustCrypto crates
  (`argon2`, `chacha20poly1305`), so there's no OpenSSL or system dependency and a clean
  cross-compile for desktop and Start9.
- **Key hierarchy:** one master key in memory after unlock, fed through HKDF-SHA256 into
  domain-separated subkeys. Per-file AAD is the config-root-relative path, which binds a
  sealed blob to its location (set via `set_config_root`; migration uses
  `aad_for_relpath`).
- **Vault sentinel:** `vault.json` (plaintext) holds `{ version, kdf, salt, params,
  verifier }`. The verifier is a known plaintext encrypted under the key, so a wrong
  password is detected cheaply, and the sentinel's presence signals "encryption is on",
  which is what makes the app boot locked.
- **Zeroization:** key material is `Zeroizing<[u8;32]>` throughout (`MasterKey` /
  `SubKey`); the `Unlocked(SubKey)` vault state zeroizes on drop, so disable wipes the
  key.

## Deployment constraint

The migration renames the config dir to a sibling, so the config dir must be an ordinary
directory on a writable filesystem, **not** a mount point. Renaming a mount point fails
with `EBUSY`. On Start9 the volume mounts at `/data` and `CORVIN_CONFIG_DIR` points at
the `/data/corvin` subdirectory for exactly this reason; any container deployment should
do the same.

## Where the file I/O lives

Every persisted write funnels through `core::at_rest::write_sealed` (reached via
`state.rs::write_private` / `save_json`), and every read through `read_private`. When the
vault is unlocked these seal and unseal with the current file key; when it's plaintext
they pass through. The wallet store loads and persists via `EncryptedChangeSetStore` in
`open_or_create_wallet`. Any new persisted store must use `write_private` / `read_private`
so it inherits both the encryption and the migration barrier below.

## Concurrency & correctness

- **Migration write barrier.** `write_sealed` holds a process-global `MIGRATION` lock
  shared; enable/disable/change-password hold it exclusively (`begin_migration()`) across
  build-staging, swap, and vault-flip. So a background write (a subscriber ChangeSet, the
  price cache, a label edit) can't land in the directory that's about to be atomic-
  swapped and be silently lost. The exclusive guard is taken after the slow KDF, so
  writers only block for the brief snapshot and swap.
- **Concurrent-migration guard.** A process-global claim (`try_claim_migration` /
  `MigrationClaim`) is taken at the entry of enable/disable/change-password; a second
  concurrent request fails fast with a 400 instead of running a second migration and
  double-sealing files.
- **Crash-safe migration.** A staging dir plus an atomic two-rename swap, with boot-time
  `recover_interrupted_migration` reverting or completing an interrupted swap, so a crash
  never leaves a mix of plaintext and encrypted files. Unit-tested
  (`migration_roundtrip_and_recovery`).
- **`read_sealed` holds `MIGRATION` shared**, closing the read-during-swap window.
- **Unlock** runs Argon2 in `spawn_blocking` (so the KDF never blocks an async runtime
  worker) and is serialized behind a global async gate, so parallel requests can't run the
  KDF concurrently to bypass the cost or pin N times 64 MiB. A hard lockout is
  intentionally not added; it would risk locking out the legitimate user, and the
  serialized Argon2 cost is the throttle.

## Outside the boundary

Browser `localStorage` and the WebKit/Tauri webview profile are not sealed. Send drafts
persist only non-sensitive fields (no recipient address), and multisig drafts no longer
persist signer xpubs. Keep identifying data out of persisted drafts. An exported backup
is a separate plaintext file unless you set a passphrase on it; at-rest encryption covers
the device, not the export, and `ImportExportPage` says so.

## Shipped surface

- Crypto core: `core/at_rest.rs`.
- Wallet persistence: `core/wallet_store.rs` (sealed CBOR ChangeSet).
- Control surface: `server/api/security.rs` (`status` / `unlock` / `enable` / `disable` /
  `change-password`, plus migration and recovery), with startup gating and the
  `lock_gate` middleware in `server/lib.rs`.
- Frontend: `UnlockGate.svelte` (full-screen boot lock), `SecurityPage.svelte` (enable,
  disable as a danger confirm modal, change-password, with a `lib/password.ts` strength
  nudge), the onboarding "Protect your data" step in `OnboardingWizard.svelte`, and the
  `encryption` help article.

## Verification

- Unit: migration round-trip and recovery, re-key, and the `at_rest` crypto tests.
- HTTP contract (`api_tests`): status reports off; enable rejects a short password;
  disable, change-password, and unlock return `bad_request` in the off/not-locked states.
- The full live enable, restart, unlock, change, disable flow is verified by hand in the
  browser. Still open: the desktop (Tauri) and Start9/headless live passes. Mid-session
  lock and idle auto-lock are deferred by choice.
