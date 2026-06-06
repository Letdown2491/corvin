# Internal review passes

Corvin has had several internal review passes covering security, correctness,
privacy, and robustness. These are in-house reviews (reading the code and running
tooling), not third-party audits. Every finding below has been resolved. This file
is a summary; the full detail lives in the git history and the relevant feature docs.

Across all passes the tooling baseline stays green: clippy clean on both feature
configs, the Rust and frontend test suites pass, `cargo audit` reports no real
vulnerabilities (only the GTK3 "unmaintained" binding warnings from Tauri's desktop
stack, which the headless/Start9 build does not link), `npm audit` is clean, and the
dependency licenses are all permissive (no GPL/AGPL).

## 2026-05-31: first broad pass (security, bugs, UX, memory, resources)

Scope was the Rust crates and the Svelte frontend. The codebase was in good shape:
careful secret handling, a correct loopback/CORS boundary, strict input validation,
and no panics in request paths. The findings were low-severity hardening, all fixed
or consciously accepted:

- Pruned an unbounded in-memory fee cache, and bounded and validated fee rates.
- Extracted money math to a tested, string-based BTC/sats module (no float drift)
  and stood up the Vitest frontend suite.
- Accepted and documented the read-only DNS-rebinding gap: every state-changing
  endpoint needs a JSON content type, which forces a CORS preflight a rebound origin
  fails, so a rebind could at most read, and Start9 is reverse-proxied anyway.

## 2026-06: money-out core and the remaining surface

One high-severity bug, the rest low/medium hardening, all fixed and the fund-safety
ones locked by tests:

- HIGH: the default send path spent the entire wallet on every transaction, an
  on-chain privacy and fee problem. It now coin-selects normally and excludes
  frozen, immature, and timelocked UTXOs.
- Deleting a Silent Payments wallet now aborts its scanner task. Previously the scan
  secret lingered in memory and the scanner could resurrect deleted data.
- Tightened SP-spend fee and dust handling, zeroized the BIP-322 derived key, fixed
  a stale backup-version guard that blocked restoring a freshly-exported backup, and
  closed brief world-readable windows on the SP-key and Ledger-HMAC files.
- Added a hermetic regtest harness with tests for coin selection, SP-spend signing,
  the privacy warnings, and backup round-trips.

## 2026-06: privacy lens

Asked "who learns what about the user, and is that the minimum?" across the network,
on-chain, and at-rest planes.

- Verified: all network egress honors the SOCKS5 proxy, the browser makes no direct
  external calls, and price, payjoin, and fiat display are off by default.
- Fixed the whole-wallet consolidation leak (above) and the SP-scanner-survives-
  deletion leak, and made the BIP-353 DoH resolver configurable (defaulting to a public
  resolver) and disclosed at use.
- Added input-side privacy warnings to the SP-send path and a change-script-mismatch
  warning to the send path.
- At-rest encryption, shipped separately, closes the largest on-disk gap for anyone
  who opts in.

## 2026-06-03: at-rest encryption plus a broad sweep

Focused on the newly-shipped at-rest encryption, run as four lenses (code, security,
privacy, workflow). All findings fixed:

- HIGH: a concurrent enable/disable/change-password race could corrupt the vault. A
  process-global claim now makes migrations exclusive; a second request gets a clear
  400.
- Stopped persisting sensitive data (recipient address, multisig signer xpubs) to the
  unencrypted webview localStorage, and strips it from older drafts on load.
- Serialized unlock attempts so the Argon2 cost acts as the throttle, demoted
  wallet-identifying SP logs to DEBUG, tightened the SP-scanner TLS check to the
  explicit danger flag, and bound the encryption AAD to the full relative path.
- Verified the AEAD/KDF construction, the migration crash-safety, and the boot-locked
  gating are sound.

## 2026-06-03: robustness sweep (parsers, concurrency, durability, accessibility, supply chain)

The standout cluster was "a corrupt file leads to silent data loss." Fixed:

- Added a quarantine-on-corruption path so a damaged store is renamed aside instead of
  defaulted then overwritten, applied to the SP scan secret, SP outputs, the payjoin
  stores, and the Ledger HMAC store. `wallets.json` now parses entries individually,
  so one bad entry can no longer hide every wallet.
- Capped the SP scanner's per-message read (a hostile server could otherwise exhaust
  memory) and routed the sweep fee rate through the same validation as send.
- Aborted payjoin poll tasks in offline mode and reaped finished task handles.
- Added accessibility to the shared modal (focus management, ARIA) and a no-camera
  paste fallback to the QR scanner.
- Built and ran a fuzz harness on the descriptor and address parsers (about 30 million
  runs, zero crashes), added BIP-84/49/86 and BIP-352 test-vector conformance,
  quantified coverage, and expanded the money-path test suite (signing round-trip,
  multisig combine, sweep, BIP-322, broadcast decode, payjoin persistence and
  anti-replay).

## 2026-06-06: rc.3 delta + at-rest boundary re-sweep

A focused pass over the changes since rc.2 (Cloudflare DoH default + migration, per-wallet
SP scanner status, the Electrum subscriber skipping SP wallets, the BitBox at-rest pairing
fix, SP dust-attack detection, the SP idle-timeout handling), plus a systematic re-check of
the at-rest boundary prompted by the BitBox finding. One material finding (the annotation load-timing bug below).

- **At-rest load timing (found after the sweep, fixed).** The boundary sweep confirmed every
  store *reads and writes* through `read_private`/`write_private`, but missed a timing issue:
  `Annotations::load()` runs in `AppState::new`, i.e. while the vault is still **locked** at
  boot, so all annotations (tx/UTXO/address notes, categories, cost basis, frozen UTXOs) read
  empty, and a later edit could overwrite the sealed files. Fixed by reloading annotations in
  the post-unlock startup (`run_startup_after_unlock`), before any save. Lesson: at-rest
  correctness is not just "does it use the sealed reader" but "does it read *after* unlock."
  A follow-up enumeration of every eager sealed read found the **price cache** had the same
  flaw (also reloaded post-unlock now); everything else was verified correct: config uses
  `Config::default()` while locked and reloads post-unlock, and the deferred stores (wallets,
  SP outputs/keys, payjoin sessions, Ledger HMACs, BitBox pairing) only read inside
  `start_services` / on-demand handlers, which run after unlock. No write path runs while
  locked (the API is gated and background tasks are deferred).

- **At-rest boundary is clean.** Enumerated every raw filesystem read/write in the crates:
  each one is either the migration mechanism itself, the deliberately-plaintext `vault.json`
  sentinel, a user-provided file (a CA cert), a user export (the desktop save dialog), or a
  test. No Corvin store reaches disk outside `write_private`/`read_private` anymore. The
  BitBox `bitbox.json` (bitbox-api writing its Noise pairing as plaintext) was the only
  instance of that pattern and is now routed through the at-rest layer.
- **BitBox `CorvinNoiseConfig` reviewed.** Its "fall back to a fresh config (re-pair) on any
  read/unseal failure" path can't be abused for a silent MITM: re-pairing still requires the
  device's physical trust-on-first-use confirmation, so a forced re-pair surfaces to the user.
- **New SP status endpoint** (`/sp/status`) exposes only `wallet_id`/`connected`/`error`,
  same shape and localhost-only surface as the existing `/backends/status`; no new exposure.
- **Tooling baseline green:** `cargo audit` 0 vulnerabilities (only the known unmaintained
  GTK-binding warnings from the Tauri desktop stack, not linked headless), `npm audit` 0,
  clippy clean (headless config), Rust + Vitest suites pass, and the descriptor/address
  fuzz targets ran clean (~0.8M and ~1.6M execs, no crashes).
- **Noted, not a regression:** an SP wallet records every received output, so a flood of
  dust receipts grows `sp_outputs.json` unboundedly. The new dust detection surfaces and
  freezes such outputs but doesn't cap storage; this is inherent to scanning-based SP
  receive (Sparrow has the same shape) and predates rc.3.

## 2026-06-06: workflow + lifecycle review (the axes the delta pass missed)

Run after the rc.3 delta pass kept surfacing bugs it hadn't caught, the gap being that a
tooling + diff review checks *mechanism*, not *behaviour over time*. Two targeted passes:
frontend transient-state-vs-refresh, and backend load/save timing.

- **Manual Sync button had the same clobber as the SSE path (fixed).** The SSE
  sync-complete handler was fixed to stop clearing the UTXO/address/chart arrays to `[]`
  before refetching (which flashed the view and reset an in-progress note edit), but the
  *manual* `sync()` in `WalletDetail` had the identical clear-then-refetch. Now swaps in
  place too. Classic "fixed one instance, missed the sibling."
- **Verified clean (frontend):** label/category writes are optimistic (store update, no
  refetch, no flash); note editing survives an atomic data swap (keyed `{#each}` keeps the
  row, input, and focus); the send flow takes `utxos` as a prop and keeps coin-control
  selection in component state, so a background sync doesn't disturb a compose; modals are
  `{#if}`-gated (fresh per open); only `WalletDetail`/`+layout` react to sync; the 30s
  poll / visibility / online refreshes all go through the atomic `loadData`.
- **Verified clean (backend):** every annotation setter holds the write lock across the
  modify *and* the save, so there's no lost-update window; eager sealed reads are all
  handled (annotations + price cache reload post-unlock, config defaults-while-locked,
  deferred stores read after unlock); the post-unlock reload runs before `start_services`,
  so no background save can race it; no write path runs while locked.
- **Lost-update race on shared JSON stores (fixed).** Stores not backed by an in-memory
  locked map (SP outputs, payjoin index + seen-inputs, SP keys) did an unsynchronized
  read-modify-write: `load()` the whole file, edit, `save()` it back, with no lock. Atomic
  temp+rename prevents corruption but not last-writer-wins lost updates. Worst case was
  `sp_outputs.json`, written by independent per-wallet scanner tasks, where two wallets
  receiving at once could drop a discovered output and its `tweak_t_n` spend data. Each
  store's mutators now hold a per-store `Mutex` across the load+modify+save. (Tell: the
  payjoin module already had a `#[cfg(test)]` serializer for exactly this shared state, but
  production was never serialized.) Continuing the sweep found the same class in
  `put_settings` (settings.rs): it read `backends`/`default_backend`/`onboarding` via several
  separate `read().await`s, then saved a merged config, so a concurrent `/backends` change in
  the gap was clobbered. Restructured to build the HTTP clients first, then hold the config
  write lock across the whole merge+save+swap, with the network/scanner side effects after.
  (`set_onboarding` already held the lock and was the model.)
- **Fee-rate poll rebuilt a transaction mid-sign (fixed).** `feeRates` is a background-polled
  store, passed as a prop; a fee *preset* makes `effectiveFeeRate` reactive to it. The
  build/preview effects in send, fee-bump, and consolidate cleared and rebuilt the PSBT on any
  `effectiveFeeRate` change with no phase guard, so a poll landing during signing rebuilt the
  PSBT and silently invalidated the signature (worst during the multi-second HW confirm). All
  three share an identical `txPhase` derivation; the build effects now bail unless
  `txPhase === 'compose'`, freezing the fee once signing begins.
- **Tx-detail panel staleness (fixed).** An open transaction-detail panel showed the
  `TxRecord` captured when it opened, so confirmation status didn't update live during a
  background sync. (The whole panel is frozen at open, not just the header as first thought;
  but a tx's structure never changes after broadcast, so confirmation-derived fields are the
  only thing that drifts.) `selectedTx` is now re-resolved by txid after each refetch, so the
  panel updates live; a tx that drops out (RBF-replaced) keeps its last-known view.

## Still open, by nature rather than as defects

- Payjoin live negotiation needs a real relay, directory, and counterparty, so it
  stays a manual verification item.
- Longer scheduled fuzz campaigns beyond the clean smoke and five-minute runs.
- Reproducible-build certification (the Docker double-build), tracked in
  `reproducible-builds.md`.
