# Changelog

All notable changes to Corvin are documented here. This project follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
