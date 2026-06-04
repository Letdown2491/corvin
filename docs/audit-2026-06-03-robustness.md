# Audit — 2026-06-03 (robustness sweep: parser / concurrency / durability / a11y / perf / supply-chain)

A second-wave audit beyond the security/privacy/workflow pass (`audit-2026-06-03.md`),
run as parallel read-only lenses, then fixed. Headline: the codebase was already strong
(defensive parsers, disciplined concurrency, atomic writes, **zero dep vulnerabilities,
zero GPL/AGPL deps, no secrets in logs**). The real cluster was *corrupt-file → silent
data loss*. Everything below is **fixed** unless marked deferred.

## Fixed

### Durability (the standout finding)
Four stores did `read_private(...).unwrap_or_default()` then full-file read-modify-write-
back, turning a *readable* corruption into *silent loss on the next write*. Added
`state::load_or_quarantine` (renames a corrupt file to `.corrupt` and starts empty instead
of defaulting-then-overwriting) and applied it to **`silent_payments.json`** (the scan
secret), **`sp_outputs.json`** (spend tweaks), the payjoin index/seen/event-log, and the
Ledger HMAC store. **`wallets.json`** now parses entries individually, so one bad/future
entry no longer makes every wallet vanish.

### Parser robustness
- **SP scanner `read_line` was unbounded** → a hostile/compromised SP server could OOM the
  process. Now capped at 16 MB/message (`core/sp_scanner.rs`, `MAX_LINE_BYTES`).
- **sweep** fee rate now routes through the hardened `validated_fee_rate_sats` (NaN/upper
  bound), matching the send path.
- (Parser audit otherwise: very defensive — every untrusted parser uses `?`/`ok()?`, the
  only `unwrap`s on this surface were in `#[cfg(test)]`, the Json body limit is 2 MB.)

### Concurrency
- **Payjoin poll tasks kept hitting the network in offline mode** → now aborted on
  offline + respawned on back-online (matches the SP-scanner treatment).
- **`payjoin_tasks` leaked finished `JoinHandle`s** → reaped (`retain(!is_finished())`) on
  respawn.
- (Concurrency audit otherwise solid: consistent lock ordering, no std-guard-across-await,
  KDF/BDK/sockets in `spawn_blocking`. Deferred as not-worth-it: `background_sync` in-flight
  dedup and cross-field snapshot bundling — "wasted work / latent," not correctness.)

### Test coverage
- Added the highest-value missing test: **`seed_signer` derivation correctness** — the xpriv
  (signing) descriptors derive the *same addresses* as the xpub (watch-only) ones per script
  type, i.e. the signer signs for the wallet you're watching. The broader money-path suite is
  tracked (#45).

### Frontend a11y
- **`ui/Modal`** gained `aria-modal`, `aria-labelledby`/`describedby`, focus-on-open, and
  focus-return — fixes all ~13 modals at once.
- **QrScanModal** gained a no-camera "paste instead" fallback (validated, same result path).
- Offline banner `role=alert`/assertive → `status`/polite (less interrupting for a
  persistent banner).

## Verified
`cargo build` (hw on) + `--no-default-features` (Start9) both clean · clippy clean both
configs · **server 46 / core 42 tests** · frontend svelte-check 0 · **vitest 100** ·
`cargo audit` clean (only accepted GTK3-unmaintained warnings, desktop-only; server is
GTK-free) · `npm audit` 0 · license tally: all permissive, **no GPL/AGPL**.

## Deferred (tracked)
- **#46** frontend perf/a11y polish — **partly shipped:** dialog **focus-return**
  (SendFlow + TxDetailPanel, matching `ui/Modal`); **search debounce** (WalletDetail,
  180ms); **CBOR length hygiene** (`lib/qr.ts` `decodeCborByteString`: bounds-checked
  reads, unsigned 32-bit length, 4 MB cap, +16 tests); **lazy QR libs (complete)** — `jsQR`
  dynamic-imported; `QrCollector.init()` async-loads bbqr + bc-ur (cached so the
  per-frame `ingest()` stays synchronous) and is awaited before the camera loop;
  `QrSignFlow` lazy-loads its display encoders (splitQRs/UR/UREncoder/Buffer).
  manualChunks split into `qr-decode` (jsqr+qrcode) + `qr-libs` (bbqr+bc-ur+buffer).
  **Measured: the eager wallet-route closure dropped 767 KB → 411 KB (−356 KB); both
  chunks (149 KB + 361 KB, ~510 KB total) are now async-only**, loaded only when a QR
  flow opens. **Browser smoke-test passed** — camera scan (BBQr + UR) and QR display
  still work with the codecs loaded on demand via `QrCollector.init()` / `loadEncoders()`.
  **List virtualization: dropped** — no measured
  slowness at single-user scale (UTXO/address sets are small; only tx history grows),
  and the table + date-grouping structure is a poor fit for JS windowing;
  `content-visibility: auto` stays a cheap one-shot if a real wallet ever drags. #46 closed.
- **#45** money-path test suite (sign roundtrips, multisig combine, sweep, BIP-322, broadcast,
  payjoin).

## Deep-audit follow-ups (the lenses done at light depth, now run properly)

- **Crypto test-vector conformance — added.** `core/seed.rs` now asserts the full
  seed→xpub-descriptor→address pipeline reproduces the **official BIP-84/49/86 published
  first-address vectors** for the all-zeros mnemonic, to the byte. (BIP-322/340 conformance
  folded into #45.)
- **Coverage quantified** via `cargo llvm-cov`: server lib ≈ **31% region / 30% line**.
  Confirms the gap map exactly — `multisig.rs`, `payjoin_send/receive.rs`, `vault.rs` at
  **0%**; `send` 59%, `sp_spend` 70%, `seed_signer` 67%, `state` 62%; the background
  subscribers near 0% (hard to unit-test). Drives #45.
- **Coverage run caught a real test-isolation bug:** `at_rest` tests share the process-
  global `VAULT` and raced under llvm-cov's ordering (`seal_open_follows_vault_state`
  failed). Fixed with a crate-wide `VAULT_TEST_SERIAL` lock the vault-touching tests
  (at_rest + wallet_store changeset) acquire — `cargo test` had only passed by luck.
- **Fuzzing — harness built AND run clean, twice.** `crates/core/fuzz/` targets
  `parse_input` (descriptor/xpub/address) and `describe_policy`. A 45s smoke
  (875k / 2.5M runs) was followed by a **5-min/target campaign**: **14.7M runs**
  on `parse_input` and **15.3M runs** on `describe_policy` (≈30M total), **zero
  crashes/hangs**, no artifacts — the untrusted-descriptor parsers are fuzz-clean.
  (Harness is committed for re-running / multi-hour campaigns; excluded from the
  workspace.)
- **Signing money-path test added:** a full `send-psbt` → `seed_signer::sign_with_seed` →
  finalize → extract-tx roundtrip on a funded regtest wallet (`api_tests`), proving the
  API's unsigned PSBT is signable by the same seed and finalizes to a valid tx.
- **Duplicate-dependency check** (`cargo tree -d`): 99 duplicate versions, ~all transitive
  crypto-crate skew (aead/aes/base64/bitcoin_hashes/chacha20/getrandom…) from BDK / payjoin /
  silentpayments — upstream, not ours to dedup; no vulns, no copyleft. A `deny.toml` for CI
  gating is the only remaining `cargo-deny` piece.

## Money-path test suite (#45) — expanded

The signing roundtrip was the first slice; the suite now also locks (all in
`api_tests`, driven through the real router on the hermetic `funded_regtest_wallet`
/ in-memory-multisig harness):

- **BIP-322** — sign a message with the wallet's own address via the mnemonic, then
  verify it keylessly (full sign→verify roundtrip); a tampered message must not
  verify; a wrong mnemonic is refused before any signature is emitted.
- **Multisig combine** (`/combine-psbt`) — a real in-memory 2-of-2 P2WSH wallet,
  funded and drained, signed *separately* by each cosigner; combining the two
  single-sig PSBTs reaches the threshold, finalizes, and reports both signer
  fingerprints. Plus: one signer is **not** ready, and two different txs are
  refused (not silently merged).
- **Broadcast decode** (`/decode`) — the same tx decoded as an unsigned PSBT and as
  finalized raw hex agree on txid; the raw tx reports an exact (non-approximate)
  vsize; garbage hex and an empty body are 400s.
- **BIP-352 wire-format conformance** (`core/silent_payments.rs`) — the bech32m
  address our code emits decodes, through the canonical BIP-352 codec, back to
  exactly the scan + spend pubkeys we encoded (interoperable, not just
  self-consistent). Complements the existing spend-secret-recovery roundtrips.

- **Sweep — derivation AND drain+sign.** `preview_sweep` was split into a pure
  `sweep_candidates()` (WIF→candidate set) and `build_sweep_tx()` (the post-sync
  drain+sign). Both are tested with the no-node chain-injection harness: a
  compressed WIF yields exactly p2wpkh / p2sh-p2wpkh / p2pkh (each address actually
  of the labelled type, all distinct); an uncompressed WIF yields legacy-only; a
  funded wallet drains its full balance into one signed output with the fee
  deducted and a real witness; an empty wallet yields no candidate. Only the
  `full_scan` line in `sweep_one` (BDK's own Electrum call) is untested — that's
  third-party network plumbing, not our money logic.
- **Payjoin — our persistence + anti-replay layers.** The session **index
  lifecycle** (register / list-by-wallet / status update / forget / forget-wallet),
  the receiver **seen-inputs anti-replay** (set-union, no double-count, empty
  no-op), and the **event-log persister** (save → replay-in-order → survives a
  fresh persister, i.e. restart) are all tested in `payjoin_sessions`. These are
  the security- and durability-critical pieces we own.

**Still uncovered in #45 — and genuinely not unit-testable:** the payjoin protocol
**negotiation** itself (the `payjoin` crate's typestate transitions driven by the
HTTP exchange with the OHTTP relay + directory + a live counterparty). That's a
third-party state machine over the network, not our logic; it stays a live/manual
verification item (see `docs/verification-plan.md`, Track E). Everything in #45 that
is *our* money/durability/security logic is now covered hermetically.

## Still genuinely open
- **Longer fuzz campaigns** (the 45s smoke + a 5-min/target campaign are both clean;
  a multi-hour/CI-scheduled run is the next rung).
- **Payjoin live negotiation** — needs a real OHTTP relay + directory + counterparty;
  manual/live verification (Track E), not a unit test. All of our own payjoin logic
  (sessions, anti-replay, event-log replay) is now covered.
- **Reproducible-build certification** (Phase 3, needs the Docker double-build).

## GTK4 question
Can't move unilaterally — Tauri 2 is built on webkit2gtk-4.1 (GTK3) and has no GTK4 release
yet (upstream `tauri#12563`, work began ~Nov 2025). The advisories are "unmaintained
*bindings*" warnings (not vulnerabilities), desktop-only; the headless/Start9 binary is
GTK-free. Track upstream; get GTK4 free on a future Tauri bump. No action now.
