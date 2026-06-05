# corvin

A self-hosted, privacy-first Bitcoin wallet.

- Full wallets: single-signature HD, **multisig**, **Silent Payments** (BIP-352),
  and watch-only, created from a seed, an xpub/ypub/zpub, a descriptor, or a
  hardware wallet
- Send & receive, RBF/CPFP fee bumping, coin control, consolidation, and
  privacy warnings on the send path
- **Payjoin** (BIP-77 v2, send + receive), **Silent Payments** send & receive,
  WIF sweep, BIP-322 message signing, BIP-329 labels, tax reports, full backup
- **Hardware wallets**: BitBox02, Ledger, Trezor (USB), plus QR / air-gapped
  PSBT signing
- Connects to **your own** Electrum server (Fulcrum, electrs, ElectrumX) or a
  Bitcoin node via RPC (Bitcoin Core, Knots). No third-party wallet servers
- **Per-wallet backends**: pin each wallet to its own server (a public Electrum,
  your node, your Frigate) so no single server sees all your wallets
- Runs as a single local binary in your browser, or as a **desktop app**
  (Tauri), or on **Start9**

**Threat model:** single user on a trusted local machine or trusted Start9 host.
Not a public-web wallet; there's no API auth or multi-tenant hardening by
design. Seeds are never written to disk; only xpub-based descriptors (and the
Silent Payments scan key) are persisted, and the mnemonic is re-supplied for
each operation that needs to sign.

## Stack

- **Rust** + [BDK 3](https://bitcoindevkit.org/) (`bdk_wallet 3`,
  `bdk_electrum 0.23`, `bitcoin 0.32`): Bitcoin logic and Electrum sync
- **Axum**: HTTP server; the built SPA is embedded into the binary
- **Svelte 5** + SvelteKit: frontend UI
- **SQLite** (via BDK's `rusqlite`): one DB per HD wallet
- **Tauri 2**: optional desktop shell (`crates/desktop`)

## Workspace

```
crates/core/      corvin-core: Bitcoin/BDK logic, descriptors, SP crypto
crates/server/    corvin: axum server + binary (also a lib: build_app/run)
crates/desktop/   corvin-desktop: Tauri desktop shell (see its README)
frontend/         Svelte 5 SPA, built and embedded into the server binary
packaging/        Start9 + desktop packaging artifacts
```

## Quick start

### Prerequisites

```sh
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Node.js (for the frontend build): https://nodejs.org
```

### Build & run (browser)

```sh
just build   # build the frontend, then the release binary
just run     # build and run
```

Then open http://127.0.0.1:5757. (Without `just`: build the frontend with
`cd frontend && npm install && npm run build`, then
`cargo build --release --package corvin` and run `./target/release/corvin`.)

### Desktop app

```sh
just dev-desktop      # run the Tauri app (dev)
just build-desktop    # release binary
just bundle-desktop   # installers (.deb/.rpm / .dmg / .msi)
```

The desktop app runs the same axum server in-process and shows it in a native
webview. See **[`crates/desktop/README.md`](crates/desktop/README.md)** for
architecture, build/bundle details, system dependencies, and platform notes.

### Development

```sh
just dev-backend    # backend only
just dev-frontend   # Svelte dev server with hot reload (proxies to the backend)
just dev-desktop    # desktop shell
```

## Configuration

First run creates `~/.config/corvin/config.toml`:

```toml
[server]
port = 5757
bind = "127.0.0.1"    # LAN exposure also requires the CORVIN_ALLOW_LAN env var

[backend]
type = "electrum"                     # or "rpc"; default backend for unpinned wallets
electrum_host = "electrum.blockstream.info"
electrum_port = 50002
electrum_ssl = true
validate_tls = true                   # false to skip strict hostname check

# [backend] for a Bitcoin node via RPC (Bitcoin Core, Knots)
# type = "rpc"
# rpc_url = "http://127.0.0.1:8332"
# rpc_user = "your_rpc_user"
# rpc_pass = "your_rpc_pass"

[network]
type = "bitcoin"    # or "testnet", "signet", "regtest"
```

Config and wallet data live under `~/.config/corvin/`, written with `0600`
permissions. **One network per instance:** switching networks hides wallets
from the others until you switch back (they stay in storage).

## Wallet types

Added via **Add wallet** (a four-way picker):

| Kind | Created from |
|---|---|
| **Single signature** | generated seed, imported seed, xpub/ypub/zpub (watch-only), or a hardware wallet |
| **Multisig** | N-of-M with per-signer hardware/seed/xpub |
| **Silent Payments** (BIP-352) | a seed (generate or import), or watch-only (scan secret + spend pubkey) |
| **Watch-only** | a single static address |

## Security

- Binds to `127.0.0.1` by default; LAN exposure requires setting
  `CORVIN_ALLOW_LAN=1` (the API has no authentication, by design, per the
  threat model)
- CORS restricted to localhost
- **Seeds are never persisted.** Only xpub-based descriptors and the Silent
  Payments scan key live on disk; the mnemonic is re-supplied per signing
  operation and zeroized from memory immediately after
- **Core dumps are disabled** at startup (`RLIMIT_CORE=0` + Linux
  `PR_SET_DUMPABLE=0`) so a crash during signing can't write a live seed to disk.
  Set `CORVIN_ALLOW_COREDUMP=1` to re-enable for debugging
- No third-party wallet servers: sync goes only to your configured Electrum/RPC
  backend. Optional features that reach the network (price lookups, payjoin
  directories/OHTTP relays, BIP-353 DNS, the Silent Payments scanner) are opt-in
  and honor a configurable SOCKS5 proxy
- **Optional at-rest encryption.** Off by default; turn it on in Settings →
  Security (or the onboarding wizard) to encrypt everything stored on disk
  (wallet data, labels, settings) under a password. Argon2id-derived key, never
  written to disk; the app boots locked and unlocks on access. Protects the
  privacy of a stolen or copied config folder. Funds are safe either way since
  the seed is never stored

## Verifying releases

Release artifacts are signed with [minisign](https://jedisct1.github.io/minisign/).
The public key is committed to this repo as `minisign.pub`:

```
RWR/xmEYuMlLUz1D6Hjh+f+VBFrP6GInyxSUEzdoqHWJDvW9g40s0pEj
```

Its key ID is **`534BC9B81861C67F`**. Cross-check this fingerprint against the
release notes; it should never change between releases.

Put the downloaded artifact, `SHA256SUMS`, `SHA256SUMS.minisig`, and `minisign.pub`
in one folder, then:

```sh
scripts/verify-release.sh <download-dir>
# or, directly:
minisign -Vm SHA256SUMS -p minisign.pub && sha256sum -c SHA256SUMS
```

A valid signature plus matching hashes confirm the build is the maintainer's and
was not tampered with. Full details, including the maintainer signing flow, are in
[docs/releases.md](docs/releases.md).

## Roadmap

- [x] First-run onboarding wizard
- [x] At-rest encryption of the config directory
- [x] Reproducible builds + signed releases (minisign)

## License & policies

- [MIT License](LICENSE)
- [Security policy](SECURITY.md): how to report a vulnerability privately
- [Changelog](CHANGELOG.md)
