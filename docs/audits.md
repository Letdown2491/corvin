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

## Still open, by nature rather than as defects

- Payjoin live negotiation needs a real relay, directory, and counterparty, so it
  stays a manual verification item.
- Longer scheduled fuzz campaigns beyond the clean smoke and five-minute runs.
- Reproducible-build certification (the Docker double-build), tracked in
  `reproducible-builds.md`.
