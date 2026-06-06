# Changelog

All notable changes to Corvin are documented here. This project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Consolidating UTXOs that span more than one coin category now shows a privacy warning
  before you sign (consolidation links those inputs on-chain), matching the mixed-category
  check the send flow already does. Only appears when there's an actual mix.
- Silent Payments dust-attack detection: tiny outputs sent to your reusable SP address
  (below a 5000-sat threshold, configurable via `sp_dust_threshold_sats`) are flagged in
  the UTXO table with a "dust attack?" badge and can be frozen in one click, so they can't
  be spent and used to probe or link your wallet.

### Fixed
- The UTXO, address, and charts views no longer flash empty (and reset what you were doing,
  like a half-typed note) when a background sync completes. The refreshed data now swaps in
  place instead of clearing the view to empty first.
- With at-rest encryption enabled, annotations (transaction / UTXO / address notes,
  categories, cost-basis overrides, and frozen-UTXO state) came up empty after a restart:
  they were read while the vault was still locked, and a later edit could then overwrite
  the sealed files on disk. They're now reloaded right after unlock, before any save can
  run, so they persist across restarts (data not yet overwritten by an edit is recovered).
- The UTXO table keeps its Category and Note columns visible (read-only) while
  consolidating, so you can see what you're combining and avoid mixing categories. Frozen
  coins are now excluded from the consolidation view entirely (you froze them to keep them
  out of transactions, so they're no longer selectable consolidation inputs), enforced
  server-side too. If too few unfrozen coins remain to consolidate, a clear message
  replaces the empty table.
- The category picker's starter suggestions vanished after you created your first
  category, making it tedious to categorize more coins (you'd have to hand-create each).
  It now keeps offering the common categories you haven't created yet until you've built
  up your own set.
- BIP-353 name resolution failed with "HTTP 505" over a SOCKS5/Tor proxy. The default
  DNS-over-HTTPS resolver (Quad9) is HTTP/2-only and rejects the HTTP/1.1 requests sent
  through a proxy. The default is now an HTTP/1.1-compatible resolver, and installs still
  on the old default are migrated automatically.
- Silent Payments wallets showed the default Electrum backend's name and connection state
  ("Default backend", and a false "can't reach" warning) instead of their Frigate
  scanner's. The scanner's connection is now tracked per wallet and shown correctly
  (e.g. "Public Frigate"), with no spurious connectivity warnings.
- The Electrum subscriber spawned a needless worker for Silent Payments wallets, connecting
  to (and retrying) a server they never use (e.g. the default backend in an SP-only-on-default
  setup). SP wallets are now skipped by the Electrum subscriber entirely; only their Frigate
  scanner connects.
- BitBox hardware-wallet unlock failed ("noise config error: stream did not contain valid
  UTF-8") when at-rest encryption was enabled: the device pairing file (`bitbox.json`) was
  sealed by the encryption migration, but bitbox-api read it as plaintext. The pairing now
  goes through Corvin's at-rest layer, so it stays sealed when encryption is on, plain when
  it's off, and an already-sealed `bitbox.json` is recovered automatically (no re-pairing).

## [1.0.0-rc.2] - 2026-06-05

> **This is a pre-release.** Test with small amounts first and keep a secure backup
> of your seed.

### Added
- macOS (`.dmg`) and Windows (`-setup.exe`) desktop installers, alongside the
  existing Linux builds. All desktop downloads are signed (minisign) and covered by
  the release `SHA256SUMS`.

### Changed
- Reproducible builds are now verified automatically in CI: the release binary is
  built twice and must produce an identical hash.

## [1.0.0-rc1] - 2026-06-04

First public release candidate. Corvin is a self-hosted Bitcoin wallet that runs
as a single binary, on your desktop or on a Start9 server, built around privacy
and self-custody.

> **This is a pre-release.** A release candidate is for testing and is not a final
> release. Expect rough edges, test with small amounts first, and always keep a
> secure backup of your seed.

### Wallets
- Create single-signature, multisig, watch-only, and Silent Payments wallets.
- Generate a new seed with a guided word-count choice and a verification step, or
  import an existing seed, xpub/ypub/zpub, or output descriptor.
- Run on mainnet, testnet, signet, or regtest.

### Sending
- Build transactions with coin control and send-max, then confirm on a two-step
  review that diagrams inputs, outputs, and change before you sign.
- Privacy warnings flag risky sends: address-poisoning look-alikes, address reuse,
  mixed labels or categories, and round-amount reveals.
- Bump stuck transactions with RBF, or speed up incoming ones with CPFP.
- Consolidate UTXOs and sweep funds from a private key (WIF).

### Privacy
- Give each wallet its own server connection to keep activity compartmentalized.
- Route traffic over Tor or a SOCKS5 proxy.
- Organize coins with labels, categories, and UTXO freezing, and pay contacts via
  human-readable BIP-353 addresses.

### Silent Payments (BIP-352): experimental
- Receive to a reusable Silent Payment address with a background scanner.
- Send to Silent Payment recipients from a regular wallet, and spend funds that a
  Silent Payments wallet has received.
- Silent Payments support is experimental; test carefully and report any issues.

### Payjoin (BIP-77)
- Send and receive Payjoin (v2) transactions for improved on-chain privacy.

### Hardware wallets & signing
- Sign with BitBox, Ledger, and Trezor over USB on desktop.
- Sign fully air-gapped by exchanging PSBTs as QR codes (BBQr / UR).
- Sign and verify messages (BIP-322).

### Security
- Your seed is never written to disk; it is re-entered only when a signature is
  needed, then wiped from memory immediately afterward.
- Optional at-rest encryption protects everything Corvin stores behind a password.

### Tools
- Export a tax report (FIFO / LIFO / HIFO).
- Import and export labels (BIP-329), inspect PSBTs, look up addresses, broadcast
  raw transactions or PSBTs, view balance-history charts, and back up and restore
  your entire set of wallets.

[1.0.0-rc.2]: https://github.com/Letdown2491/corvin/releases/tag/v1.0.0-rc.2
[1.0.0-rc1]: https://github.com/Letdown2491/corvin/releases/tag/v1.0.0-rc1
