# At-rest encryption

Status: **BUILT (Phases 0–3 done).** End-to-end functional: enable via Settings →
Security or the onboarding "Protect your data" step, restart boots locked, unlock
gate accepts the password. One architecture change from the original plan: we do
**not** use SQLCipher. Wallet data is persisted as an XChaCha20-Poly1305-sealed CBOR
`ChangeSet` blob (`core/wallet_store.rs`), so the whole product uses one cipher and
no C/SQLite dependency. Crypto core in `core/at_rest.rs`; control surface in
`server/api/security.rs`; frontend gate `UnlockGate.svelte` + `SecurityPage.svelte`.
Migration is staging-dir + atomic swap with boot-time recovery. The rest of this
doc is the original design (still accurate except the SQLCipher → ChangeSet-blob
change).

## Goal & threat model

Optionally encrypt Corvin's config directory at rest so that someone with raw
access to the disk (stolen/seized device, a copied or cloud-synced config dir, an
old drive, a backup) can't read it.

**This is a privacy feature, not anti-theft.** Corvin never writes a seed or any
spend key to disk — signing re-derives from the mnemonic supplied at use time. So
a stolen disk can already **never spend funds**. What it *can* read in plaintext
today, and what this feature protects:

- **xpubs / descriptors** (`wallets.json`, `wallets/<uuid>.db`) — the master public
  keys derive every past+future address, so they expose the wallet's **entire
  balance and transaction history** going forward.
- **SP scan secret** (`silent_payments.json`) — a real secret: lets a holder
  **discover all your Silent Payments receipts** (not spend them).
- **RPC password** (`config.toml`), labels, cost basis, UTXO data, payjoin
  sessions, Ledger HMACs.

Files are already `0600` (owner-only), which stops *other logged-in users* but not
someone reading the raw bytes. On desktop, OS full-disk encryption (LUKS/FileVault/
BitLocker) covers the raw-disk case; the marginal value of app-level encryption is
highest on **Start9** (StartOS has no FDE yet — open issue Start9Labs/start-os#735)
and for **backups / copied config dirs**.

## Settled decisions

- **Opt-in, default off.** A fresh install boots straight in, unencrypted, as today.
- **Coverage = everything** (all JSON + the BDK `.db`). No "secrets-only" half-measure;
  the locked-until-unlock model (below) removes the reason we'd have kept history
  plaintext.
- **Key source = user password.** Argon2id-derived. The password *is* the protection;
  the key never touches disk. (Not a key-on-box auto-unlock — that only protects
  copied dirs, not a stolen machine, and isn't worth the weaker guarantee here.)
- **Unlock-on-access (Alby-Hub style).** When encryption is on, the app boots
  **locked**: nothing is read, wallets aren't loaded, the Electrum subscriber and SP
  scanner don't start. The user visits the URL (localhost on desktop, the Start9 URL
  on a server), enters the password on a full-screen unlock gate, and the app
  decrypts into memory and starts. This makes the strong password model viable on
  headless Start9 too — **accepted consequence: after a reboot, background sync +
  new-tx notifications pause until someone unlocks.**
- **Decrypt-back-to-plaintext** is supported (disable encryption → migrate files back
  to plaintext).
- **No idle auto-lock in v1.** Unlock once per process start; revisit a timeout later.
- **Change-password — SHIPPED (v2).** `POST /security/change-password` (verify current
  → derive new master from a fresh salt → decrypt-then-re-encrypt every file under the
  new key in one staging pass → new sentinel → atomic swap → re-unlock in memory).
  Plaintext never hits disk during the change. Security page "Change password" card.
- **Disable requires the password (v2 hardening).** Even though disable is only
  reachable while unlocked, it rewrites the whole config dir to plaintext (a downgrade
  that outlives the session), so `POST /security/disable` re-verifies against the
  sentinel and the UI prompts for the password.
- **Forgot-password = privacy/annotation loss, never fund loss.** No keys on disk, so
  recovery is re-adding wallets from xpub/seed; you lose labels/cost-basis/history.
  Say this plainly on the enable screen.

## Crypto

- **KDF:** Argon2id, tuned to ~500ms on modern hardware (Sparrow's choice; OWASP
  guidance). Random salt stored in the vault sentinel.
- **JSON files:** XChaCha20-Poly1305 AEAD, random nonce per write. Pure-Rust
  RustCrypto crates (`argon2`, `chacha20poly1305`) — no OpenSSL/system deps, clean
  cross-compile for desktop + Start9.
- **BDK `.db`:** ⚠️ **Superseded — not how it shipped.** The `.db` was removed; wallet
  data is a sealed CBOR `ChangeSet` blob (`core/wallet_store.rs`), encrypted with the
  same XChaCha20-Poly1305 as the JSON. The SQLCipher path below is kept as the
  original design record only.
  SQLCipher via `PRAGMA key`. **Feasibility proven (Phase 0):** add a
  direct `rusqlite = "0.31"` (BDK 3's exact version) with feature
  `bundled-sqlcipher`; cargo feature-unification flips the *shared* `libsqlite3-sys`
  to compile SQLCipher, and BDK's re-exported `Connection` transparently accepts
  `PRAGMA key`. No BDK fork. (Spike test: create+persist a real `bdk_wallet::Wallet`
  on an encrypted conn, file has no `SQLite format 3` header, right key loads, wrong
  key fails.) Note: `bundled-sqlcipher` compiles SQLCipher from vendored C — needs a
  C compiler in the build env (already true here) and bumps build time; confirm it
  survives the desktop/Start9 packaging pipeline.
- **One master key in memory** after unlock, used for both the JSON AEAD and the
  SQLCipher key. Never written to disk.
- **Vault sentinel** (`vault.json` or similar, unencrypted): KDF salt + params + a
  small **verifier** (a known plaintext encrypted under the key) so we can
  distinguish "right password" from "wrong password" cheaply, and so its *presence*
  signals "encryption is on" → boot locked.

## Where the file I/O lives (hook points)

JSON persistence is mostly centralized in `crates/server/src/state.rs`:
`write_private` / `load_json` / `save_json`. Encrypt/decrypt wrap these. **Stragglers
that bypass them and must be routed through the same path:** `config.rs::save_config`,
`sp_outputs.rs`, `payjoin_sessions.rs`. (As shipped, these all route through
`core::at_rest::write_sealed`.) ~~The `.db` is opened in
`wallet.rs::open_or_create_wallet` — add `PRAGMA key`.~~ Superseded: there is no `.db`;
`open_or_create_wallet` loads/persists via `EncryptedChangeSetStore` instead.

## Phased plan

**Phase 0 — SQLCipher feasibility spike. ✅ DONE** (proven; see Crypto above).

**Phase 1 — Server crypto core (headless, API-testable).**
- Add deps: `argon2`, `chacha20poly1305`, `hkdf`, `sha2`, `ciborium` in `corvin-core`.
  (Original plan also added `rusqlite { bundled-sqlcipher }`; **dropped** when the `.db`
  was replaced by the sealed ChangeSet blob.)
- `Vault` state on `AppState`: `Locked` | `Unlocked { key }`.
- Argon2 + XChaCha helpers; the vault sentinel (salt/params/verifier).
- Encrypt/decrypt in the `state.rs` file-I/O helpers + the 3 stragglers; `PRAGMA key`
  in `open_or_create_wallet` when unlocked.
- Endpoints: `GET /security/status`, `POST /security/enable` (set password, migrate
  plaintext→encrypted), `POST /security/unlock`, `POST /security/disable`
  (decrypt→plaintext).
- **Gate startup:** if the vault sentinel exists, boot Locked — don't load wallets or
  start the subscriber / SP scanner until unlock. Wire unlock to kick off the normal
  startup path.

**Phase 2 — Frontend gate + Security page.**
- Full-screen **Unlock** screen shown (before the app shell) whenever status = locked.
- **Security** sidebar link → Security settings page: enable (set password + the
  forgot-password warning), disable (decrypt back), show status.

**Phase 3 — Onboarding step.**
- Optional "Secure your wallet" step in the wizard, placed **before** "Add your first
  wallet" (so a user who opts in is *born encrypted* — no plaintext wallet bytes ever
  written). Skippable; skipping leaves it off, changeable later in Security settings.

## Resolved during implementation

- ~~Confirm `bundled-sqlcipher` builds cleanly.~~ **Moot — SQLCipher was dropped.** The
  BDK `.db` is gone; wallet data persists as a sealed CBOR `ChangeSet` blob
  (`core/wallet_store.rs::EncryptedChangeSetStore`). One cipher, no C dependency.
- Migration correctness both directions: built as a **staging-dir + atomic two-rename
  swap** with boot-time `recover_interrupted_migration` (`server/api/security.rs`),
  unit-tested (`migration_roundtrip_and_recovery`). Covers every file (there is no
  longer a `.db`; the encrypted ChangeSet blob migrates with the JSON).
- Vault sentinel: `vault.json` (plaintext) holding `{ version, kdf, salt, params,
  verifier }`; its presence signals encryption-on → boot locked.
- `put_settings` and friends preserve the vault state across saves: all private writes
  route through `core::at_rest::write_sealed` / `state.rs::write_private`, which seal
  with the current file key whenever the vault is unlocked.

## Shipped surface (final)

- Crypto core: `core/at_rest.rs` (Argon2id → HKDF-SHA256 subkeys → XChaCha20-Poly1305,
  per-file AAD = config-root-relative path; `VaultState = Plaintext | Locked | Unlocked`).
- Wallet persistence: `core/wallet_store.rs` (sealed CBOR ChangeSet).
- Control surface: `server/api/security.rs` (`status`/`unlock`/`enable`/`disable`/
  `change-password` + migration/recovery; disable + change-password re-verify the
  password against the sentinel); startup gating + `lock_gate` middleware in
  `server/lib.rs`.
- Frontend: `UnlockGate.svelte` (full-screen boot lock), `SecurityPage.svelte`
  (enable + disable + change-password; disable is a danger confirm modal; a
  `lib/password.ts` strength nudge on the new-password fields), and the
  onboarding "Protect your data" step in `OnboardingWizard.svelte`. Help article:
  HelpContent `encryption`. `ImportExportPage` warns that an exported backup is a
  separate plaintext file unless a passphrase is set (at-rest covers the device, not
  the export).

## Concurrency + correctness (v2 hardening)

- **Migration write barrier.** Every persisted write funnels through
  `at_rest::write_sealed`; it holds a process-global `MIGRATION` lock *shared*, and
  enable/disable/change-password hold it *exclusively* (`at_rest::begin_migration()`)
  across build-staging → swap → vault-flip. So a background write (subscriber
  ChangeSet, price cache, a label edit) can't land in the directory that's about to be
  atomic-swapped (where it would be silently lost). The exclusive guard is taken after
  the slow KDF, so writers only block for the brief snapshot+swap.
- **`load_wallets` read path.** Was reading `wallets.json` raw while `save_wallets`
  writes it sealed — fixed to go through `read_private` (would have failed to load
  wallets whenever encryption was on). All other JSON stores already load via
  `load_json`/`read_private`; this was the only straggler.
- **`unlock` runs Argon2 in `spawn_blocking`** (like the other handlers) so the ~750ms
  KDF never blocks an async runtime worker, **and is serialized behind a global async
  gate** (added 2026-06-03) so parallel requests can't run the KDF concurrently to
  bypass the cost or pin N×64 MiB. A *hard lockout* is still not added (it would risk
  locking out the legitimate user); the serialized Argon2 cost is the throttle.
- Key material is `Zeroizing<[u8;32]>` throughout (`MasterKey`/`SubKey`); the
  `Unlocked(SubKey)` vault state zeroizes on drop, so disable wipes the key.

### Audit 2026-06-03 hardening (see `docs/audit-2026-06-03.md`)

- **Concurrent-migration guard.** A process-global claim (`try_claim_migration` /
  `MigrationClaim`) is taken at the entry of enable/disable/change-password; a second
  concurrent request fails fast (400) instead of running a second migration and
  double-sealing files. Closes a data-loss TOCTOU.
- **AAD = config-root-relative path**, not the basename — binds a sealed blob to its
  location so two same-named files in different subdirs can't be swapped
  (`at_rest::file_aad` + `set_config_root`; migration uses `aad_for_relpath`). Format
  change vs the first cut: an existing encrypted vault must be disabled before updating.
- **`read_sealed` holds `MIGRATION` shared**, closing the read-during-swap window.
- **`/api/status` removed from the locked allow-list** (it would probe the default
  public backend, un-proxied, pre-unlock).
- **Webview-profile data is outside the boundary.** `localStorage` now holds only
  non-sensitive prefs (send drafts no longer persist the recipient address; multisig
  drafts no longer persist signer xpubs). The WebKit/Tauri profile is not sealed.

## Verification

- Unit: migration roundtrip + recovery, re-key, all `at_rest` crypto tests.
- HTTP contract (`api_tests`): status=off; enable rejects short pw; disable /
  change-password / unlock return `bad_request` in the off/not-locked states (lock the
  `ApiError` taxonomy). Full live enable→restart→unlock→change→disable verified by hand
  (web).
- Still open: desktop (Tauri) + Start9/headless live passes; mid-session lock / idle
  auto-lock (deferred by choice).
