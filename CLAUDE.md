# Corvin — Project Notes for LLMs

This file is for AI assistants picking up work on this repo. Skim sections as
needed; don't echo it back to the user.

## What this is

**Corvin** is a self-hosted Bitcoin wallet. Rust backend (axum) + Svelte 5
frontend, packaged as a single binary that serves the SPA at `127.0.0.1:5757`.
Deploys as a desktop app or on Start9.

**Threat model:** single user, trusted local machine or trusted Start9 host.
**Not** a public-web wallet. No API auth, no DoS hardening, no multi-tenant.
This informs many decisions you'll see — when something looks like a missing
hardening pass, check whether it's a deliberate scope exclusion before
"fixing" it.

**Network/runtime stance:** runs on whatever Bitcoin network the user configures
(mainnet, testnet, signet, regtest). One network per instance — switching
networks hides wallets from other networks.

## Workspace

```
Cargo.toml                       workspace root
crates/core/                     corvin-core lib
crates/server/                   corvin lib + binary (lib.rs: build_app/run, used
                                 headless and by the desktop shell)
crates/desktop/                  corvin-desktop — Tauri 2 shell (runs the axum
                                 server in-process). See crates/desktop/README.md
frontend/                        Svelte 5 + SvelteKit SPA, served from binary
packaging/                       Start9 + desktop packaging
justfile                         common dev tasks
```

The desktop app (`crates/desktop`) wraps the server in a Tauri webview rather
than reimplementing it — `crates/desktop/README.md` is the reference for its
architecture, build/bundle process, per-platform notes, and gotchas.

Workspace deps live in root `Cargo.toml`. Key pins:
- `bdk_wallet = "3"`, `bdk_electrum = "0.23"`
- `bitcoin = "0.32"` (BDK's reexport)
- `serde = { features = ["derive", "rc"] }` — `rc` lets serde transparently
  serialize `Arc<T>` (used by wallet snapshots)

## Backend stack (Rust)

- **axum 0.8** with tokio
- **bdk_wallet 3** with built-in `rusqlite` feature (one SQLite DB per HD
  wallet under `~/.config/corvin/wallets/<uuid>.db`). We deliberately skipped
  `bdk_sqlite` — see memory `[[bdk-versions]]` for the reasoning.
- **electrum-client** for the main subscriber; **custom TCP/TLS client** for
  the Silent Payments scanner (see "Silent Payments" below for why).
- **silentpayments 0.5** (encode + receiving + sending features) for BIP-352
  crypto.
- **bip39, bip322, bitcoincore_rpc** as needed.
- HW wallets: `bitbox-api` (native), `ledger_bitcoin_client` (native), Trezor
  (via subprocess) — native USB. Plus air-gapped **QR/PSBT** signing (`QrSignFlow` +
  `lib/qr.ts`: BBQr + UR). **Start9 decision (#18):** Start9 signs via QR/PSBT only and
  builds with the native-USB `hw` feature OFF (StartOS has no USB passthrough); desktop
  keeps `hw` ON. See `docs/start9-hardware-signing.md`.

### Server-side layout (`crates/server/src/`)

```
main.rs                          axum router, startup, SPA embed
config.rs                        Config struct + persistence (~/.config/corvin/config.toml)
state.rs                         AppState, WalletManager, ManagedWallet, WalletInner,
                                 SilentPaymentsCache, AddressCache, SseEvent
subscriber.rs                    Main Electrum subscriber loop (script subs + background sync)
sp_subscriber.rs                 SP scanner per-wallet tokio tasks (one per SP-enabled wallet)
sp_outputs.rs                    sp_outputs.json store (per-wallet SP UTXO records)
payjoin_subscriber.rs            Payjoin (BIP-77) async poll tasks — send + receive,
                                 one per session; pj_post() socks5-aware HTTP helper
payjoin_sessions.rs              Payjoin session store: per-session event log
                                 (FileSessionPersister) + index.json + seen_inputs.json
api/                             HTTP handlers
  mod.rs                         module decls + ApiError
  wallets/                       wallet CRUD + reads + send/multisig/tax/sp_send/payjoin/etc.
    mod.rs                       list/add/delete/rename + balance/txs/utxos/addresses/sync
    send.rs                      regular send PSBT + RBF/CPFP fee bumping
    sp_send.rs                   BIP-352 send (HD → SP recipients)
    payjoin_send.rs              BIP-77 v2 send (build/confirm/status/abandon)
    payjoin_receive.rs           BIP-77 v2 receive (provision/confirm/status/cancel/list)
    seed_signer.rs               shared transient-xpriv signing (sp_send + payjoin_send)
    multisig.rs                  multisig create + combine + Coldcard export
    tax.rs                       tax report (FIFO/LIFO/HIFO)
  silent_payments.rs             SP wallet enable/create + key persistence
  sweep.rs                       sweep WIF (scan + drain a private key)
  broadcast.rs                   /broadcast (accepts PSBT or raw hex)
  messages.rs                    BIP-322 sign/verify
  hwi/                           HW wallet integration (bitbox/ledger/trezor)
  prices.rs                      mempool price proxy
  proxy.rs                       mempool tx/block/fees proxy
  settings.rs                    GET/PUT /settings + /status + reconnect
  events.rs                      SSE channel
  bip329.rs                      BIP-329 label import/export
  backup.rs                      full backup export/restore
  backup_test.rs                 verify backup matches wallet
  labels.rs                      tx labels
  address_labels.rs              address labels
  utxo_labels.rs                 UTXO labels
  utxo_freeze.rs                 UTXO freezing
  categories.rs                  coin categories: defs + address/UTXO assignments (#33)
  cost_basis.rs                  user-set cost basis overrides
```

### Core layout (`crates/core/src/`)

```
types.rs                         InputKind, WalletEntry, Balance, TxRecord, UtxoRecord,
                                 AddressInfo, SyncResult, NodeStatus, SpOutputRecord
descriptor.rs                    descriptor parsing/building (parse_input, descriptor_from_multisig,
                                 silent payments descriptors)
seed.rs                          mnemonic → xpub-based descriptors (derive_descriptors)
                                 + generate_mnemonic, default_derivation_path
wallet.rs                        BDK wallet lifecycle (open_or_create_wallet, sync, get_balance, etc.)
sp_scanner.rs                    SP scanner client — TCP/TLS to Frigate-compatible Electrum
silent_payments.rs               BIP-352 scan/spend key derivation from mnemonic
backends/
  electrum.rs                    ElectrumConfig + native_tls/socks5 plumbing
  rpc.rs                         RpcConfig + bitcoincore_rpc wrapper
```

## Frontend stack

- **Svelte 5** (runes: `$state`, `$derived`, `$effect`, `$props`, `$bindable`)
- **SvelteKit** routing
- Built static SPA embedded into the Rust binary via `rust_embed`
- CSS tokens + global rules live in the `:global(...)` block of
  `routes/+layout.svelte` (`:root` vars, focus ring, shared buttons); everything
  else is component-scoped.
- **Buttons: use the global classes** `.btn-primary` (accent CTA), `.btn-secondary`
  (neutral bordered / Cancel), `.btn-ghost` (borderless), `.btn-danger`
  (destructive confirm), `.close-btn` (modal ✕). Don't define per-component button
  CSS — add a small scoped rule only for a genuine layout tweak (e.g. `width:100%`
  on mobile, `align-self`). Confirm dialogs order actions **Cancel (left) ·
  primary/destructive (right)**.
- **Async actions show busy state:** any button triggering an `await api.*` (or other
  slow work, e.g. crypto/KDF) uses the `{busy ? 'Verbing…' : 'Verb'}` label +
  `disabled={busy}` pattern (a `$state(false)` flag set in try/finally) so it can't be
  double-clicked and the user sees progress. Background polling (status/fees) doesn't
  need this.

### Layout

```
frontend/src/
  routes/                        SvelteKit routes
    +layout.svelte               app shell, SSE subscription, global stores
    +page.svelte                 home (delegates to active wallet)
    add-wallet/+page.svelte      add-wallet flow with 4-tile kind picker
    wallet/[id]/+page.svelte     wallet detail
    wallet/[id]/tax-report/      tax report route
  components/
    WalletSidebar.svelte         desktop sidebar (wallet list + backend status)
    WalletDetail.svelte          main wallet detail page
    WalletToolsTab.svelte        Tools tab (extracted from WalletDetail)
    SendModal.svelte             unified send (regular + SP recipients)
    ReceiveModal.svelte          receive (with SP address panel when enabled)
    SeedGeneratorPanel.svelte    shared generate-and-verify flow (single-sig + SP creation)
    SweepWifModal.svelte         WIF sweep
    PsbtInspectorModal.svelte    PSBT inspector
    AddressLookupModal.svelte    address lookup
    WalletDetailsModal.svelte    wallet info (read-only; branches for SP)
    ChangeBackendModal.svelte    per-wallet backend picker (kebab → "Change backend")
    DeleteWalletModal.svelte     delete confirm (type wallet name; modal, not inline)
    BalanceChart.svelte          charts
    TxRow.svelte / TxDetailPanel.svelte  tx list + detail
    UtxoTable.svelte / AddressTable.svelte
    ConsolidateModal.svelte      consolidation (kebab)
    BroadcastModal.svelte        broadcast (kebab; paste PSBT/hex → decode → broadcast)
    MessagesModal.svelte         BIP-322
    ServerPage.svelte            backend settings (autosaving; Network / Default
                                 backend / Saved backends / Mempool / Payjoin cards)
    BackendsSection.svelte       saved-backends registry (in ServerPage): add/edit/
                                 test/delete, Frigate type, Default pill
    QrSignFlow.svelte            QR PSBT signing
    HwSign*, Ledger*, Trezor*    HW wallet flows
    MobileLayout.svelte / MobileWalletList.svelte   mobile chrome
  lib/
    api.ts                       HTTP client + SSE subscription helper
    types.ts                     mirrors Rust types (InputKind, WalletEntry, Balance, etc.)
    bip39.ts / bip39_words.ts    BIP-39 validation client-side
    derivation.ts                descriptor origin parsing for the details modal
    search.ts                    tx search
    amount.ts                    BTC↔sats parsing/formatting + BIP21 (pure, tested)
    send.ts                      send-flow address/URI helpers: shape detection, QR/scan
                                 extraction, BIP-353 HRN detection, addr chunking (pure, tested)
    import.ts                    wallet-import file/descriptor parsing (pure, tested)
    public-servers.ts            curated default public Electrum servers
    utils.ts                     kindLabel, downloadBlob, psbtBlob, etc.
  stores/
    wallets.ts                   wallets, syncing, lastSyncComplete, walletBalances
    settings.ts                  nodeStatus, feeRates, displayUnit, balancesHidden,
                                 mempoolUrl, currentBtcPrice, notificationsEnabled
    labels.ts / cost_basis.ts / utxo_labels.ts / utxo_freeze.ts / address_labels.ts
    toasts.ts
```

## State + lifecycle (server)

### AppState (one Arc, cloned into handlers)

```rust
pub struct AppState {
    manager: Arc<RwLock<WalletManager>>,       // wallets by uuid
    config: Arc<RwLock<Config>>,                // persisted config
    event_tx: broadcast::Sender<SseEvent>,      // → /events
    sub_tx: mpsc::UnboundedSender<SubCommand>,  // → main subscriber
    labels / price_cache / cost_basis / utxo_labels / address_labels / frozen_utxos
    http: Arc<RwLock<reqwest::Client>>,         // honors socks5_proxy
    current_price_cache, hwi_lock, hwi_sign_jobs
    wake_signal: Arc<tokio::sync::Notify>,      // kicks subscriber out of backoff
}
```

### WalletInner

```rust
pub enum WalletInner {
    Hd(Mutex<HdWalletState>),                       // BDK Wallet + SQLite
    Address(Mutex<Option<AddressCache>>),           // single watched address
    SilentPayments(Mutex<SilentPaymentsCache>),     // SP-discovered outputs
}
```

When extending: **match exhaustively** on `WalletInner` everywhere. The
compiler will tell you which sites need an arm. Also update `InputKind`
enum exhaustiveness sites: `state.rs::load_wallets`, `backup.rs`,
`backup_test.rs`.

### Snapshot pattern

HD wallet reads are lock-free via `Mutex<Arc<Vec<T>>>`. On sync completion
the snapshot Arcs are swapped atomically. Endpoints clone the Arc and serve
it without re-locking the BDK wallet. The `serde "rc"` feature lets serde
serialize Arc transparently.

`ManagedWallet` fields:
- `txs_snapshot: Mutex<Arc<Vec<TxRecord>>>`
- `utxos_snapshot: Mutex<Arc<Vec<UtxoRecord>>>`
- `balance_snapshot: Mutex<Option<Balance>>`
- `addresses_snapshot: Mutex<Arc<Vec<AddressInfo>>>`
- `fee_cache: Mutex<HashMap<txid, fee_sats>>`
- `last_synced: Mutex<Option<DateTime<Utc>>>`

For SP wallets, `SilentPaymentsCache.outputs: Vec<SpOutputRecord>` is the
source of truth and balance/utxos/txs are derived via methods on the cache.

## SSE event channel

`GET /api/events` is a broadcast SSE stream. Events:

| event                    | payload                                                   | emitted from                |
|--------------------------|-----------------------------------------------------------|-----------------------------|
| `sync_started`           | `{ wallet_id }`                                           | sync_wallet, background_sync|
| `sync_complete`          | `{ wallet_id, new_txs }`                                  | sync_wallet, background_sync|
| `sp_output_discovered`   | `{ wallet_id, txid, height, count }`                      | sp_subscriber               |
| `error`                  | `{ wallet_id?, message? }`                                | various                     |

Frontend subscribes once in `+layout.svelte`. On `sync_complete` or
`sp_output_discovered`, `lastSyncComplete` store fires which retriggers data
fetches in `WalletDetail`.

## Conventions

### Secrets

- **Seeds are never persisted.** Only xpub-based descriptors + the SP
  `scan_secret_hex` live on disk. Spend secrets re-derive from mnemonic at
  use time.
- **Mnemonic is re-supplied on every operation that needs spend keys:** SP
  enable, BIP-322 signing, SP send, backup test.
- Mnemonic/passphrase wrapped in `Zeroizing<String>` immediately on the
  server, zeroized on every return path.
- SP scan secret IS persisted because the background scanner needs it.
- **Core dumps are suppressed at startup** (`server/src/harden.rs::suppress_core_dumps`,
  called first in `build_app`): `RLIMIT_CORE=0` (Unix) + `PR_SET_DUMPABLE=0` (Linux) so a
  crash during signing can't dump a live seed. Opt out with `CORVIN_ALLOW_COREDUMP=1`
  (debugging). We deliberately do **not** `mlock` secrets: the spend-critical ones are
  transient (re-derived per op, then zeroized) so they're unlikely to be swapped, and
  doing it properly needs a secure allocator for marginal gain — see the at-rest doc.

### Errors

- `ApiError` is the unified error type. A plain `?`-converted error → HTTP 500,
  body `{ error }` (unchanged default — `From<anyhow::Error>` still works at every
  call site). For cases the frontend should distinguish, use the constructors:
  `ApiError::not_found` (404), `bad_request` (400), `wrong_secret` (401),
  `insufficient_funds` (409) — each adds a stable machine `code` and a real status,
  body `{ error, code }`.
- Frontend `api.ts` throws **`ApiErr`** (extends `Error`) carrying `.code` + `.status`.
  Branch on `e.code` (e.g. `'wrong_secret'`) — never string-match `.message`. Most
  UIs just show `e.message` as toast/inline.

### IDs

- Wallet IDs are `Uuid` — generated server-side on add.

### File layout for storage

```
~/.config/corvin/
  config.toml                    backend config + settings
  wallets.json                   list of WalletEntry (xpub-based)
  wallets/<uuid>.db              per-HD-wallet BDK SQLite
  silent_payments.json           per-wallet SP keys (scan_secret + spend_pubkey + address)
  sp_outputs.json                per-wallet discovered SP UTXO records
  payjoin/<sid>.json             per-session payjoin event log (+ index.json, seen_inputs.json)
  labels.json / cost_basis.json / utxo_labels.json / address_labels.json / frozen_utxos.json
  categories.json                coin categories: definitions + address/UTXO assignments (#33)
  price_cache.json               daily BTC/USD price cache
  ledger_hmac_store.json         Ledger multisig registration HMACs
```

Atomic writes everywhere: temp file + rename. `restrict_to_owner(&path)`
sets 0600 on the file after write.

### Comments

- Default: write no comment.
- Add one only when the **why** is non-obvious (hidden constraint, workaround,
  surprising invariant). Don't describe what the code already says.
- Don't write multi-paragraph docstrings. One short line max.
- Don't write narrator-style ("we now do X, then we do Y") — explain
  reasoning, not steps.

## Major features — where they live

### Sending (regular)

- Endpoint: `POST /wallets/{id}/send-psbt`
- Code: `crates/server/src/api/wallets/send.rs`
- Output: unsigned PSBT (BDK signs nothing unless descriptor has secrets;
  HW or external signing expected)
- Privacy warnings: `compute_send_warnings` flags mixed-labels,
  mixed-categories (#33), repeat recipient, round-amount reveal, and look-alike
  address (#34, address-poisoning: a recipient sharing first-6 + last-6 chars
  with a different address in history). Frontend `SendWarningCode` mirrors these.
- Tx review (Send v2, #32): two-step `send/SendFlow.svelte` (Compose → Review &
  Sign); review step renders `TxFlowDiagram` (#31) and the warnings. Tx detail
  fetches `GET /wallets/{id}/tx/{txid}` (`reads.rs::get_tx_breakdown`) for the
  same diagram on historical txs.
- Anti-fee-sniping: BDK 3 default — nLockTime = chain tip, sequence
  0xFFFFFFFD. We don't override either.

### Sending to a Silent Payment recipient (BIP-352 send-side, shipped)

- Endpoint: `POST /wallets/{id}/sp-send`
- Code: `crates/server/src/api/wallets/sp_send.rs`
- Constraints: software-wallet only (single-sig HD with mnemonic-derivable
  descriptor). Multisig and SP-source wallets refuse.
- Flow: persisted xpub wallet builds PSBT with placeholder P2TR outputs →
  derive input privkeys from mnemonic (via PSBT `bip32_derivation` /
  `tap_key_origins`) → `silentpayments::utils::sending::calculate_partial_secret`
  → `silentpayments::sending::generate_recipient_pubkeys` → patch placeholder
  outputs with derived x-only keys → transient xpriv-backed BDK wallet signs.
- Mnemonic + passphrase in `Zeroizing`.

### Payjoin (BIP-77 / v2 — send + receive, shipped #70)

- **Crate:** `payjoin` v0.25 (PDK). Uses `bitcoin 0.32` (same as BDK 3 → PSBTs
  interop directly). Typestate + **event-sourced persistence**: each transition
  emits a `SessionEvent` we persist via `FileSessionPersister`; `replay_event_log`
  rebuilds on restart. BYO HTTP — the crate returns a `Request{url,content_type,body}`
  we POST through `payjoin_subscriber::pj_post` (socks5-aware `state.http`).
- **Software single-sig only** (shared `seed_signer`); off by default
  (`backend.payjoin_enabled`); public directory + OHTTP relay (configurable).
- **Send** (`payjoin_send.rs`, software xpub/ypub/zpub/taproot): build + sign the
  original (= broadcastable fallback), POST, then a poll task negotiates; on the
  receiver's proposal the user re-signs (`/confirm`) + broadcasts, or it falls
  back to the original after `payjoin_fallback_secs`. **v2 only** — a v1 URI would
  panic the v2 builder, so we match `pj_param()` and return `v1_unsupported`.
- **Receive** (`payjoin_receive.rs`, **RPC backend** + zpub/taproot only): provision
  returns a `pj=` URI; a poll task runs the BIP-77 checks, contributes one input
  (frozen-aware, `try_preserving_privacy`), and PARKS at `ProvisionalProposal` (no
  seed); the user `/confirm`s with their seed (signs our input, posts back).
  testmempoolaccept = `rpc::test_mempool_accept`; `seen_inputs.json` = anti-replay.
- **Both** restart-survive (poll tasks respawn in `payjoin_subscriber::start`) and
  resume in the UI (waiting invoice or parked proposal). Sessions live under
  `~/.config/corvin/payjoin/`. SSE: `payjoin_proposal_ready`/`_sent`/`_fell_back`,
  `payjoin_receive_proposal`/`_sent`.

### Sending FROM an SP wallet (BIP-352 spend-side, shipped #100)

- Endpoint: `POST /wallets/{id}/sp-spend`
- Code: `crates/server/src/api/wallets/sp_spend.rs`
- BDK can't help — inputs are SP-discovered P2TR outputs keyed by `spend_secret
  + t_n`, not described by any descriptor — so the tx is hand-built + signed:
  - Re-derive the spend secret from the mnemonic (account recovered by matching
    the stored spend pubkey; `account_index` is persisted to derive directly).
  - Source UTXOs from the wallet's `SilentPaymentsCache` (unspent, unfrozen);
    coin-select largest-first, estimate fee.
  - Per input the spending key is `d_n = spend_secret + t_n` (stored as
    `tweak_t_n_hex`); sanity-check its x-only pubkey == the recorded output key.
  - Change = a BIP-352 self-payment to our own **m=0** change address (the
    scanner registers m=0, so change is re-discovered).
  - Sign each input with a **key-path BIP-340 Schnorr** sig — the SP output key
    is used **directly, no taproot tweak**, so the signing key is `d_n`
    (`sign_schnorr_no_aux_rand`). Broadcast, then mark consumed outputs spent.
- v1 limit: recipients must be plain addresses (no SP→SP yet).

### Silent Payments scanner (BIP-352 receive-side)

- Architecture: Sparrow-style "SP is a wallet kind" (`InputKind::SilentPayments`).
- Backend pieces:
  - `core/sp_scanner.rs` — dedicated TCP/TLS client for Frigate's three SP
    methods (`silentpayments.subscribe`/`unsubscribe`/notification stream).
    Built standalone because `electrum-client 0.23` silently drops
    notifications for any method other than `headers.subscribe` and
    `scripthash.subscribe`.
  - `server/sp_subscriber.rs` — one tokio task per SP-enabled wallet, runs
    blocking scanner in `spawn_blocking` with exponential backoff. Bails
    out on JSON-RPC error -32601 (server doesn't speak BIP-352).
  - `server/sp_outputs.rs` — `sp_outputs.json` persistence.
  - `api/silent_payments.rs` — wallet creation endpoint
    (`POST /wallets/silent-payments`), status/export/labels handlers.
- Discovery flow: `history` notification → fetch tx via
  `blockchain.transaction.get` → run
  `silentpayments::receiving::Receiver::scan_transaction` → persist matches
  → update `SilentPaymentsCache` → emit `sp_output_discovered` SSE.
- Frigate protocol: `blockchain.silentpayments.subscribe(scan_priv, spend_pub,
  start?, labels?)` returns a stream of unified `{ subscription, progress,
  history }` notifications. Server holds keys in RAM only. **Trust note**:
  whoever runs the SP server sees what you receive (not spend) during the
  session — a real privacy consideration.
- SP scanner server resolution: per SP wallet via `Config::sp_electrum_config_for(wallet.backend)`.
  `None` → the default SP scanner `backend.sp_electrum_*` (default
  `frigate.2140.dev:50002` SSL); `Some(id)` → a saved backend's connection. SP
  wallets pick "Public · frigate.2140.dev" (None) or a saved **Frigate** backend
  at creation. (The old `backend.sp_use_main_electrum` flag was **removed** in the
  per-wallet-backend reframe — a plain Electrum server can't scan BIP-352, so the
  scanner never mirrors the main connection. A backend is Frigate-capable when
  `BackendEntry.frigate == true`; it connects like Electrum.) There is no longer a
  dedicated "SP scanner" settings section.
- **SP is its own wallet kind, period.** There is no "enable SP on an
  existing HD wallet" flow — it was removed because the scanner only
  populates a `WalletInner::SilentPayments` cache, so SP-on-HD silently
  produced invisible/unspendable funds. SP wallets are created only via
  Add wallet → Silent Payments.

### HD wallet sync

- `subscriber.rs` opens one Electrum connection for the whole instance,
  subscribes to all wallets' scripts + block headers, triggers
  `background_sync` on activity.
- `background_sync` (in `api/wallets/mod.rs`) is BDK's incremental sync. On
  success it writes new snapshots and emits `sync_complete`.
- `sync_wallet` (the explicit endpoint) is similar but synchronous.
- Sleep-resilient: `wake_signal: Arc<Notify>` lets the frontend kick the
  subscriber out of its backoff sleep on `visibilitychange`/`online` events.

### Add wallet flow (frontend)

`/add-wallet` opens with a 4-tile kind picker:
1. **Single Signature** — generate seed (via `SeedGeneratorPanel`) or
   import seed / paste xpub / HW wallet
2. **Multisig** — N-signer setup with per-signer HW/seed/xpub
3. **Silent Payments** — From seed (generate or import) / Watch-only
   (paste scan_secret + spend_pubkey)
4. **Watch-only** — single static address

`SeedGeneratorPanel.svelte` is the shared generate-and-verify component
used by both Single-Sig HD and SP from-seed flows. Owns word-count toggle,
reveal-on-click, copy/download/print, 3-word verification challenge.

### At-rest encryption (config dir, shipped)

- Opt-in, default off. Encrypts everything under `~/.config/corvin/` (all JSON +
  the wallet data) under a user password. **No SQLCipher / no `.db`** — the BDK
  SQLite store was replaced by a sealed CBOR `ChangeSet` blob, so the whole product
  uses **one cipher and no C dependency**.
- Crypto: `core/at_rest.rs` — Argon2id (auto-calibrated) → HKDF-SHA256 subkeys →
  XChaCha20-Poly1305 AEAD, per-file AAD = config-root-relative path (via
  `set_config_root`; binds a blob to its location, not just its name). `VaultState = Plaintext | Locked
  | Unlocked(SubKey)` in a process-global `RwLock`. Wallet persistence:
  `core/wallet_store.rs::EncryptedChangeSetStore` (impl BDK `WalletPersister`).
- Unlock-on-access (Alby-Hub style): when the **plaintext sentinel `vault.json`**
  exists, `build_app` boots **locked** — bind the listener, defer wallet/subscriber
  load, gate the API via `lock_gate` middleware until `/security/unlock`. Background
  sync + notifications pause while locked.
- Control surface: `server/api/security.rs` (`status`/`unlock`/`enable`/`disable`/
  `change-password`), with crash-safe migration = staging-dir + atomic two-rename swap
  + boot-time `recover_interrupted_migration`. Private writes route through
  `core::at_rest::write_sealed` / `state.rs::write_private`. **disable + change-password
  re-verify the password against the sentinel** (disable rewrites to plaintext, a
  session-outliving downgrade; change-password re-keys in one pass so plaintext never
  hits disk). Mid-session lock / idle auto-lock intentionally not built.
- **All persisted writes funnel through `at_rest::write_sealed`** (`write_private`/
  `save_json` and the BDK ChangeSet `persist` both call it). It holds a process-global
  `MIGRATION` lock shared; enable/disable/change-password hold it exclusively
  (`begin_migration()`) across snapshot→swap→vault-flip so a concurrent background write
  can't be lost to the atomic swap. **Any new persisted store must write via
  `write_private`/`write_sealed` and read via `read_private`** — a raw read/write
  bypasses both encryption and the migration barrier (this bit `load_wallets` once).
  Concurrent migrations are refused by a process-global claim
  (`at_rest::try_claim_migration`); `read_sealed` also holds `MIGRATION` shared. AAD is
  the config-root-relative path (`set_config_root` is called once in `build_app` + in
  `test_isolate_config_dir`). Browser `localStorage` is **outside** this boundary — keep
  identifying data (addresses, xpubs) out of persisted drafts.
- Frontend: `UnlockGate.svelte` (full-screen boot lock), `SecurityPage.svelte`
  (enable; disable is a danger confirm modal), onboarding "Protect your data" step in
  `OnboardingWizard.svelte`, store `stores/security.ts`, help article `encryption`.
- **Forgot password = privacy/annotation loss, never fund loss** (no keys on disk;
  re-add wallets from seed/xpub). Full design + final notes in
  `docs/at-rest-encryption.md`.

## Pending work (high-level — see `~/.claude/projects/.../memory/` for full state)

- Reproducible builds + signed releases — **Phase 1 (signed releases) shipped; Phase 3
  mechanism in place** (`Dockerfile.repro` + `scripts/repro-build.sh` + `just repro`,
  pinned images, `SOURCE_DATE_EPOCH`, `npm ci`, dist-mtime normalization for `rust_embed`)
  — wired into CI (`release.yml` builds via the container on tag) and `just release` now
  builds from the same container, so the signed binary is the reproducible one.
  **Not yet certified** (needs a first real run + `diffoscope` + base-image digest
  pinning). Phase 2 (desktop notarization, needs paid certs) + Phase 4 (Start9 image) planned.
  Phase 1: `rust-toolchain.toml` pins the compiler; `just release` + `just release-sign`
  (`scripts/sign-release.sh`) produce `SHA256SUMS` + a minisign signature;
  `scripts/verify-release.sh` + `docs/releases.md` cover user verification; CI
  (`.github/workflows/release.yml`) builds + hashes independently (signing stays
  offline). One-time maintainer TODO: generate the minisign key + commit `minisign.pub`.
  Full plan in `docs/reproducible-builds.md`.
- Strategy / competitive positioning vs Sparrow + candidate directions (privacy-score
  leak surface, SP-as-default-receive, etc.): `docs/positioning.md`.
- Native mobile (iOS/Android via Tauri 2) — **post-1.0, Android-first.** Feasible (same
  frontend + Rust-as-lib), but HW wallets must be gated out (reuses the `hw` feature flag
  from the Start9 work; mobile signs via QR/PSBT + watch-only), needs rustls + Tauri
  storage paths + iOS ATS/background-exec handling. The PWA already covers mobile for v1.
  Full assessment in memory `[[mobile-tauri-apps]]`.
- Verification passes: #89 Taproot HW roundtrip, #90 Ledger multisig HMAC, plus
  the full hands-on pass in `docs/verification-plan.md` (timelock gate, HW policy
  signing, SP spend, BIP-353, payjoin Track E)
- Privacy audit (2026-06): `docs/privacy-audit.md`. Fixes shipped (consolidation,
  SP-scanner lifecycle, configurable BIP-353 DoH resolver `backend.bip353_doh_url`);
  All findings now shipped: (d) change-script-mismatch warning + (e) SP-send input-side
  warnings (`send.rs`), and (f) at-rest encryption (see the feature section above). A
  follow-up audit (2026-06-03, `docs/audit-2026-06-03.md`) covered the at-rest code +
  a broad sweep; all findings fixed.

## Gotchas + non-obvious decisions

- **No API auth.** Bound to 127.0.0.1 only unless `CORVIN_ALLOW_LAN=1` env var.
- **Env overrides (for containers/Start9 — env wins over config.toml):**
  `CORVIN_BIND` (bind address), `CORVIN_PORT` (port), `CORVIN_CONFIG_DIR` (data
  dir). Applied at startup in `lib.rs` via `config::resolve_bind`/`resolve_port`
  (`config_dir()` reads `CORVIN_CONFIG_DIR`). A non-loopback `CORVIN_BIND` still
  needs `CORVIN_ALLOW_LAN=1` or it falls back to loopback. Lets a fresh container
  configure bind/port/data with no pre-seeded config file.
- **One network per instance.** Switching network in Settings hides wallets
  from the other network; restart required to fully apply. Wallets stay in
  storage and reappear on switch back.
- **Per-wallet backend (built).** Each wallet can sync/broadcast through its own
  server (privacy compartmentalization). `Config.backend` = the default connection;
  `Config.backends` = a registry of saved `BackendEntry`; `Config.default_backend:
  Option<String>` picks which backend unpinned wallets use (None = `Config.backend`,
  the selected public server); `WalletEntry.backend: Option<String>` pins a wallet.
  Resolve via `Config::effective_backend_id` / `electrum_config_for` / `rpc_config_for`
  / `backend_kind_for` (SP uses `sp_electrum_config_for`). The subscriber is a
  supervisor with one worker per distinct backend. Backend settings page = global
  Network + Default-server picker + Saved-backends registry (no Public/Private/RPC
  type selector). Full design + state: `docs/per-wallet-backend.md`.
  - **Known limitation (not a bug):** the incremental sync path is **Electrum-only**.
    An RPC/node backend gets periodic polling via the subscriber worker, not BDK's
    push-driven Electrum sync. Pre-existing, not introduced by the per-wallet work.
    Pinning a wallet to an RPC backend works, but its sync cadence is the poll interval.
- **BDK 3's `wallet.sign()` needs the descriptor to have xpriv keys.** For
  signing operations we re-derive xpriv-based descriptors from the mnemonic
  at use time and use a **transient** BDK wallet (no persistence) — see
  `wallets/seed_signer.rs` (`build_xpriv_descriptors` / `sign_with_seed`,
  shared by sp_send + payjoin_send).
- **`generate_recipient_pubkeys` returns `HashMap<addr, Vec<XOnlyPublicKey>>`.**
  Recipients sharing a scan key get distinct outputs indexed n=0,1,2…
  Walk in the user-supplied order to preserve intent.
- **SP scanner can't piggyback on the shared electrum-client** — the
  client's notification dispatcher only handles `blockchain.headers.subscribe`
  and `blockchain.scripthash.subscribe`; everything else gets silently
  dropped. That's why `sp_scanner.rs` is a dedicated socket.
- **Electrum protocol requires `server.version` as the first message** —
  Frigate/ElectrumX/Fulcrum all reject anything else. `SpScanner::connect`
  sends the handshake before subscribing.
- **Frigate keepalives are null-payload notifications every ~5s.**
  `next_notification` skips them silently rather than treating them as errors.
- **Anti-fee-sniping is automatic via BDK 3 defaults.** Don't add manual
  `nlocktime` calls or `set_exact_sequence` — they'd override BDK's
  tip-height-based locktime and `0xFFFFFFFD` sequence.
- **The user has memory in `~/.claude/projects/-var-home-martin-Documents-sentinelle/memory/`** —
  contains active priorities, SP architecture decisions, SeedGeneratorPanel
  conventions, BDK version matrix, etc. Read `MEMORY.md` index first.

## Build + test

- `just dev` / `just build` for common tasks (check `justfile`).
- Backend: `cargo build --bin corvin` from repo root. Tests: `cargo test --workspace`;
  lint with `cargo clippy --workspace --all-targets` (kept warning-clean).
- **`hw` cargo feature (default on)** = native-USB hardware wallets (BitBox/Ledger/Trezor).
  The headless/Start9 build uses `cargo build --no-default-features` to drop the USB stack
  (signs via QR/PSBT instead); `GET /version` reports `hw_enabled`. **Keep both configs
  green** — build/clippy `--no-default-features` too when touching `api/hwi`, the router,
  or AppState. The non-USB helpers `api::hwi_common` + `api::ledger_hmac_store` are always
  compiled (used by non-HW signing + wallet delete); only `api::hwi` (USB) is gated.
- **`clippy::unwrap_used` is a workspace lint (warn).** No bare `unwrap()` in
  production code — use `.expect("why")` to assert a genuine invariant (it
  documents the reason, like a comment) or real error handling; for poisoned
  `std` locks use `.unwrap_or_else(|e| e.into_inner())`. Tests are exempt
  (`clippy.toml`: `allow-unwrap-in-tests`). `expect_used` is intentionally NOT
  ratcheted. Config: `[workspace.lints.clippy]` in root `Cargo.toml`, opted in
  per crate via `[lints] workspace = true`.
- **Server handler integration tests** live in `crates/server/src/api_tests.rs`
  (a `#[cfg(test)]` module). They build an in-memory `AppState` and drive the
  real router via `build_router(state)` + `tower::oneshot` — no disk config, no
  background tasks, no network. Good place to regression-lock status/code
  contracts (the `ApiError` taxonomy).
  - **Regtest send harness:** `funded_regtest_wallet(n, sats)` funds a real BDK
    wallet in memory (bdk_wallet `test-utils` dev-dep) and drops it in the manager,
    so the live send/warning handlers run end-to-end *without* a backend (the build
    path never hits the network). Covers coin selection (no-consolidate / send-max /
    coin-control), all the privacy-warning codes, and SP-spend signing
    (`build_sp_spend_tx`, extracted from the handler so a test can verify the
    key-path Schnorr sigs validate). Pattern to copy for new money-path tests.
- **Audit records:** `docs/audit-2026-05-31.md`, `docs/audit-2026-06.md` (bug/
  security/perf), and `docs/privacy-audit.md` (privacy lens) — what was checked,
  found, and fixed. Update them when you do another pass.
- Frontend: `cd frontend && npm run dev` for hot reload; `npx svelte-check` to
  type-check; `npm test` runs the **Vitest** suite.
- **Frontend tests (Vitest):** pure-logic units for `src/lib/*.ts`, colocated as
  `*.test.ts`. Config is a standalone `vitest.config.ts` (node env, no SvelteKit
  plugin). Toolchain is kept on latest: **vite 8**, **@sveltejs/vite-plugin-svelte 7**,
  **TypeScript 6**, **svelte 5.56**, **vitest 4** — all deduped on a single vite 8, so
  svelte-check stays happy. Stay off vitest 2 (nested vite 5, broke svelte-check);
  vitest 3 carried a critical UI-server advisory (GHSA-5xrq-8626-4rwp), cleared by v4.
  Money math lives in the tested `src/lib/amount.ts`
  (string-based BTC↔sats, no `parseFloat * 1e8`) — extend its tests when touching it.
- Type-check the frontend after Svelte edits. Type-check Rust after edits.
- For UI changes, manual browser testing is expected when feasible — but
  you (LLM) often won't be able to. Say so explicitly rather than claiming
  success.

## Architecture + refactoring

See `docs/architecture-conventions.md` for the full conventions + the prioritized
refactor target list. Short version:
- **Size alone isn't debt; mixed-concerns × churn is.** Leave big-but-cohesive files
  (HelpContent static, descriptor.rs parsing+tests); target mixed-concern, high-churn
  ones (SendModal, add-wallet, WalletDetail, api/wallets/mod.rs).
- **Reach for shared UI primitives in `components/ui/`** — `Modal`, `BusyButton`,
  `CopyButton` exist (Tier 1); `QrDisplay`/`AmountInput`/`Field` planned. Don't
  hand-roll a `<dialog>` shell, busy-flag button, or copy-flash per component.
- **Pure logic → `lib/*.ts` with `*.test.ts`** (the `amount.ts`/`send.ts` pattern):
  extract framework-free logic out of big components first (testable, safe), then
  split the component (folder-per-feature). Money code: logic-extract → primitives →
  decompose, never big-bang (no Svelte component tests exist, only pure lib tests).

## Style — short version

- Don't add error handling for impossible scenarios.
- Don't add backwards-compat shims when the persisted shape can change.
- Three similar lines beats a premature abstraction.
- No emojis unless the user explicitly asks.
- One sentence per code comment, only when the why is non-obvious.
- Match real names: see `kindLabel` in `frontend/src/lib/utils.ts` for the
  user-facing names for each `InputKind`.
