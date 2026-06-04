# Per-wallet backend

Status: **designed, not yet implemented.** This captures the agreed design and
an implementation plan.

## Goal

Let each wallet talk to its own backend server, so no single server sees the
addresses of all your wallets. This is a privacy (compartmentalization) feature
that fits Corvin's privacy-first stance: keep a low-stakes "spending" wallet on a
public server and a "savings" wallet on your own node, with neither server able
to correlate the two.

**Non-goals / settled decisions:**
- **No fallback.** A fallback to a different server is the exact moment a second
  server learns about a wallet, which defeats the compartmentalization. Dropped.
- **Network stays global.** One network per instance, as today. Per-wallet
  applies to the *server*, never the network.
- **Fees / price / SOCKS5 proxy stay global.** The mempool/price queries carry no
  wallet info (date + currency only), so a shared price server can't correlate
  anything.

## Model

- There's a **default** backend (a built-in pool of public Electrum servers, one
  pre-selected). A wallet on "Default" uses that one server.
- A wallet may instead use **its own** server. The backend is chosen when the
  wallet is added, and is **editable later** on the wallet page.
- "Default = a list" means a **menu** of public servers with one selected. It is
  **not** rotation or fallback: one server per wallet at a time.
- Behind the picker is a small **saved set of backends** (your node, your private
  Electrum, your Frigate) so a second wallet can reuse one without re-entering
  it, and so any wallet's backend can be edited.
- A wallet **syncs and broadcasts through its own backend** (broadcasting through
  a shared server would leak the tx origin even if sync didn't).

### Per wallet kind

| Kind | Backend choice at add time |
|---|---|
| Single signature | Default (public Electrum) / Private Electrum / Bitcoin node |
| Multisig | same three (one backend for the whole wallet; signers are about keys, not chain data) |
| Vault / Policy | same three |
| Single-sig from a pasted **xpub** (watch-only HD) | same three — watching a whole xpub is exactly what a privacy-minded user wants on their own server |
| **Watch-only (single address)** | silent **Default**, no prompt; still changeable later on the wallet page |
| **Silent Payments** | **Public** (`frigate.2140.dev`, the one BIP-352 server for now) / **Private** (your own Frigate-capable server) |

Notes:
- SP's "Private" is **not** the HD "Bitcoin node" option. SP scanning needs a
  server that speaks BIP-352 (Frigate); a vanilla node or electrs/Fulcrum can't.
  So SP's picker is its own branch: Public Frigate vs your Frigate.
- "Default" is **network-specific**: on regtest there's no public pool, so it
  won't be offered (pick private/node). SP "Public" depends on `frigate.2140.dev`
  supporting the chosen network.
- **payjoin-receive** needs the **Bitcoin node** option, so it becomes an honest
  per-wallet capability (a wallet "knows" if it's on a node) instead of a global
  one.

## UI changes

- **Add-wallet flow** gains a backend step (skipped for single-address
  watch-only): the per-kind picker above, offering saved backends + "add new."
- **Connection status moves to the wallet page** (server in use, connected /
  disconnected, block height, last sync). A single sidebar "Connected" no longer
  makes sense once wallets differ.
- **Sidebar** keeps a compact per-wallet status **dot** (ok / syncing / error) so
  at-a-glance health survives; the detail lives on the wallet page.
- **Backend settings page** changes role: it keeps the **global** settings
  (network, mempool/price URL, SOCKS5 proxy, the public-server pool + which is
  default) and the **saved backends** registry. Per-wallet assignment happens at
  add time / on the wallet page, not here.

## Data / persistence

**Shape decision (as built in Phase 1):** the default backend stays as
`Config.backend` (`[backend]`, unchanged); **additional** named backends live in
`Config.backends` (`[[backends]]`, a `Vec<BackendEntry>`). A wallet references one
via `WalletEntry.backend: Option<String>` — `None` = default backend, otherwise
the `id` of a registry entry.

**Default-backend selection (later addition).** `Config.default_backend:
Option<String>` (`#[serde(default)]`, so no migration: `None` = the built-in
`Config.backend` connection, i.e. the selected public Electrum server; `Some(id)`
= a saved backend used as the default). `Config::effective_backend_id(wallet_backend)`
centralizes resolution: a pinned wallet uses its own id, an unpinned wallet falls
back to `default_backend`, and a dangling id degrades to the built-in connection.
`electrum_config_for` / `backend_kind_for` / `rpc_config_for` and the subscriber's
`resolve_group_key` all route through it (SP resolution does **not** — SP keeps its
own scanner config). This is why the **global Backend page's connection editor is
just a dropdown** of public servers + your saved backends: picking a public server
writes host/port/ssl into `Config.backend` and clears `default_backend`; picking a
saved backend sets `default_backend = id` (no secret-copying — the node's RPC
password stays server-side in the registry, which the pure-copy approach couldn't
do). Changing the default re-homes unpinned wallets immediately
(`put_settings` → `regroup_default_wallets` re-issues `AddWallet`). Deleting a
backend that was the default clears `default_backend`. `GET /status` and the
`/backends/status` `null` alias both report the *effective* default so the sidebar
stays accurate.

**Unify (later refinement).** The default is now *always a reference* — no embedded
ad-hoc "Current" connection in the dropdown. To get there: (1) the **factory
default** `Config.backend` is a public Electrum server (`electrum.blockstream.info:50002`,
in `PUBLIC_SERVERS`) so a fresh install preselects it and works out of the box;
(2) `POST /backends/adopt-default` moves a *custom* built-in default into the
registry and pins it (reuses a matching existing entry instead of duplicating —
so a user whose default == their saved "Start9" node just repoints, no dupe — and
runs server-side so a custom RPC default keeps its password). The ServerPage
`onMount` calls it once when it detects a custom default (`isCustomDefault`, i.e.
not a `PUBLIC_SERVERS` host), then reloads. So every default resolves to either a
public server or a saved backend. **Page layout (final):** five uniform
collapsible cards — **Network** (collapsed) → **Default backend** (expanded, the
primary control) → **Saved backends** → **Mempool** → **Payjoin**. No more
"Connection" umbrella and no SP-scanner card (SP server is picked at wallet
creation). The whole page **autosaves** — no Save button; each section flashes
"Autosaved ✓" in its header on change (toggles/dropdowns on change, text/number on
blur). Each page (Backend, Display) refetches the latest config and writes only
its own fields on save, so they can't clobber each other. **Payjoin** stays a
global opt-in (third-party directory + OHTTP relay); copy clarifies send = any
software single-sig wallet, receive = a wallet on a Bitcoin node (RPC), per wallet.
**Mempool** has its own per-service `mempool_danger_accept_invalid_certs` + a
dedicated `AppState.mempool_http` client (don't reuse the global TLS flag).

Why not migrate `[backend]` into the registry with a `default_backend` pointer
(the originally-proposed shape): `GET/PUT /settings` serialize `Config` directly,
so the frontend's `Settings` type mirrors `Config`. Keeping `[backend]` intact
keeps that wire shape compatible, so **no frontend change and no migration** were
needed. `put_settings` preserves `backends` across saves (the settings form
doesn't carry it). Same end capability, far less risk.

- `BackendEntry` (in `config.rs`) holds the connection subset: id, label, type
  (electrum/rpc), electrum host/port/ssl, validate_tls, ca_cert_path,
  danger_accept_invalid_certs, socks5_proxy, rpc_url/user/pass.
- `WalletEntry.backend` is ts-rs exported; regenerate bindings with
  `just gen-types` after changing it.
- SP reframe (Phase 5) still **replaces `sp_use_main_electrum` / `sp_electrum_*`**:
  an SP wallet will reference a frigate backend (Public or Private). The old flag
  is misleading because the main Electrum usually can't scan SP anyway. Untouched
  in Phase 1.

## Engine: subscriber refactor

Today `subscriber.rs` opens **one** Electrum connection for the whole instance and
subscribes every wallet's scripts through it. The change:

- Group wallets by their **effective backend**; spawn **one connection per
  distinct backend** (everyone on the default → still one connection). Each
  connection subscribes its group's scripts + headers and tracks its own tip.
- Reuse the existing reconnect/backoff loop per connection.
- **Broadcast** for a wallet goes through that wallet's connection.
- `sp_subscriber.rs` already runs a task per SP wallet against a scanner server;
  point each at the wallet's referenced frigate backend.

## Suggested phasing

1. **Data model + persistence** — backend registry, `WalletEntry` backend ref.
   **DONE** (behavior-preserving: `BackendEntry` + `Config.backends` +
   `WalletEntry.backend`, settings preserves the registry, nothing reads them
   yet so runtime is unchanged; ts bindings regenerated).
2. **Per-wallet sync + subscriber + broadcast** — the engine work, in slices:
   - **2a (DONE):** `sync_wallet` / `background_sync` resolve the wallet's backend
     via `Config::electrum_config_for(wallet.backend)` (+ `backend_entry` /
     `BackendEntry::electrum_url`). This is where addresses are actually queried.
     Behavior-preserving (all wallets are `backend: None` until the Phase 3 UI).
   - **2b (DONE):** subscriber group-by-backend. `subscriber.rs` is now a
     supervisor (async) that routes each wallet's `SubCommand`s to a per-backend
     worker (`spawn_backend_worker`, the original loop scoped to one backend and
     resolving config by key). The supervisor looks up the wallet's backend from
     `state` (`resolve_group_key`), so `SubCommand` and its ~10 send sites were
     left unchanged. `reconnect` now uses `wake_signal.notify_waiters()` so all
     workers get kicked. Behavior-preserving today (all wallets → the single
     default group → one worker). Backend *changes* (re-grouping) are only a
     best-effort placeholder until Phase 3 (which should send a full `AddWallet`
     on change, not `UpdateScripts`). Empty groups leave an idle worker — fine
     for now, a possible later cleanup.
   - **2c (DONE):** broadcast through the wallet's backend. Added
     `Config::rpc_config_for`. `/broadcast` resolves the backend from
     `body.wallet_id` (the generic paste-a-tx tool sends none → default).
     `broadcast_transaction(state, tx, backend)` takes a backend; payjoin
     send/abandon/fallback pass the wallet's. sp-spend resolves
     `managed.entry.backend`. Frontend passes `wallet_id` from Send (already),
     Consolidate, FeeBump, and Sweep. Behavior-preserving (all `backend: None`).

**Phase 2 (engine) complete.** Sync, subscriber, and broadcast all honor a
wallet's backend; everything resolves to the default until a wallet is pinned, so
runtime is unchanged. **Phase 3 (UI)** is what lets a user actually pin a backend
— at which point all of this goes live and wants real sync/broadcast testing.
3. **Backend registry API + add-wallet picker + reuse**:
   - **3a (DONE):** registry CRUD API — `GET/POST /backends`,
     `PUT/DELETE /backends/{id}` (`api/backends.rs`). Masks `rpc_pass` on read,
     preserves it on empty update, validates id/url/proxy, nudges the subscriber
     via `notify_waiters`. Server-only; no behavior change. Also
     `POST /backends/test` (`test_backend`): probes an arbitrary `BackendEntry`
     (reusing `electrum::probe_status` / `rpc::probe_status`) without saving;
     reuses the stored `rpc_pass` by id when the body's is blank (edits arrive
     masked). The `BackendsSection.svelte` form exposes the **full** Electrum
     option set (SSL, accept-invalid/self-signed certs, strict hostname check, CA
     cert path, SOCKS5/Tor) — matching the global Backend settings page, not just
     host/port/SSL — plus **inline edit** of a saved backend (`PUT`, blank
     password keeps the stored one) and a **Test connection** button.
   - **3b (DONE for single-sig + multisig):** frontend `BackendEntry` type +
     `api.backends` CRUD client + registry UI (`BackendsSection.svelte`, "Saved
     backends" card above Save in Backend settings). Server: `backend` threaded
     through all non-SP creation requests (`AddWalletRequest`, `SeedImportRequest`,
     `HwImportRequest`, `ImportDescriptorRequest`, `CreateMultisigRequest`,
     `CreateVault`/`CreateTimelockedRequest` + the `finalize_descriptor_wallet`
     helper), set on `WalletEntry.backend` at creation (entry stays immutable, so
     Phase 2's `entry.backend` reads are correct). Frontend: a **Backend** dropdown
     (Default + saved backends) in the add-wallet form for `kind === 'single' ||
     'multisig'`, passed to seedImport/multisigCreate/importDescriptor/hwImport.
     The dedicated watch-only `address` kind has no picker (silent default, per
     design). **End-to-end working** for single-sig + multisig.
     The **vault** picker is also done (`VaultCreatePanel.svelte` gets a
     `savedBackends` prop + its own dropdown, passing `backend` to
     createVault/createTimelocked). So **all non-SP kinds** have the picker.
     **SP** picker is Phase 5.
4. **Status relocation** — wallet-page status + sidebar dot; rework the Backend
   settings page into global settings + registry. (Frontend; needs runtime testing.)
   - **Change-backend on an existing wallet: DONE.** Added a mutable
     `ManagedWallet.backend: RwLock<Option<String>>` (mirrors `label`; seeded
     from `entry.backend` in `add()`, read via `ManagedWallet::backend()` — all
     ~10 read sites switched to it, persisted via `list_entries()`). Endpoint
     `PUT /wallets/{id}/backend` validates, sets+saves, then re-routes (non-SP →
     fresh `AddWallet` so the supervisor re-groups; SP → `respawn_one`) and
     re-syncs. **UI (later):** changing a wallet's backend is a **kebab action**
     ("Change backend" → `ChangeBackendModal`), not the details modal — the
     `WalletDetailsModal` Backend row is now **read-only**. The picker offers
     Default + public Electrum servers + saved backends (SP wallets: public Frigate
     + saved Frigate); picking a public server **materializes** it into the registry
     (deduped) since a wallet can only reference a saved id. Wallet **delete** is
     likewise its own `DeleteWalletModal` (type the wallet name to confirm, always).
   - **Per-wallet connection status: DONE.** `AppState.backend_status`
     (`HashMap<Option<backend id>, BackendStatus>`, a std `RwLock`) is written by
     the subscriber workers: Electrum workers mark connected + chain tip in
     `run_session` (on `block_headers_subscribe` and each `block_headers_pop`) and
     disconnected (with the error) on connect-fail / session-loss; the RPC branch
     probes `rpc::probe_status` each cycle. `GET /backends/status` exposes it as
     `[{ backend, connected, tip_height, error }]` (`backend: null` = default).
     Frontend: `backendStatuses` store, polled by the sidebar alongside `/status`;
     a compact per-wallet **dot** in the sidebar wallet list (green/red/grey by the
     wallet's backend), and a **status line** on the wallet page that groups
     connection + backend + sync as one unit, with block separate:
     `$388.24 · ● Start9 synced 2m ago · block 951,817` (dot color = the backend's
     connection state; the sync slot reads `syncing…` during a sync, `disconnected`
     when down; block shown only when connected). Segments render conditionally so
     there are no empty `· ·` gaps. Opening a wallet **triggers a sync**
     (`api.wallets.sync`, fire-and-forget) so data + sync time are current rather
     than a stale snapshot; `sync_started`/`sync_complete` drive the indicator. The global
     sidebar connection/block line was **removed** (it reflected the default
     backend, which is misleading once a wallet is on a different one), and the
     mempool **fee rate moved onto the wallet status line** too
     (`… · Block 951,817 · Fees: 1.0 sat/vB`), so the sidebar footer has no status
     strip — per-wallet connection state lives on the wallet page + the list dots.
     (SP-only backends have no Electrum worker, so their dot reads "unknown" unless
     an HD wallet shares the backend.) The sidebar still polls `/status` + fees to
     keep the shared `nodeStatus` / `feeRates` stores fresh (read by the wallet
     page, send flow, `MobileLayout`, `UtxoTable`, `BalanceChart`, add-wallet); it
     just doesn't render them itself. Moving fees to the wallet line also gives
     **mobile** a fee readout it previously lacked.
5. **SP per-wallet backend (DONE, additive).** `Config::sp_electrum_config_for`
   (None → the existing global SP config, unchanged; Some(id) → the saved
   backend's connection). `sp_subscriber.run_one` and the `sync_wallet` SP
   reconcile resolve the wallet's backend. `CreateSpWalletRequest` accepts
   `backend`; the add-wallet SP form shows the same Backend dropdown (with
   SP-specific hint). So an SP wallet can be pinned to its own Frigate server;
   existing SP wallets (`None`) are unaffected.

   **SP reframe (DONE later).** `sp_use_main_electrum` is **removed**;
   `sp_electrum_config()` (the `None` default scanner) now always uses the
   `sp_electrum_*` fields, which default to **frigate.2140.dev** — a regular
   Electrum server can't scan BIP-352, so the default never mirrors the main
   connection. No migration needed: existing custom `sp_electrum_*` values are
   preserved (they round-trip through settings; the `sp_electrum_*` fields stay in
   `Config`/`Settings`, just no longer edited via a section). Backends gained a
   **`frigate: bool`** capability flag (an Electrum server that also speaks
   BIP-352); `BackendsSection` exposes it as a third "Frigate (Silent Payments)"
   type. The **SP scanner settings section is removed** — SP wallet creation now
   picks "Public · frigate.2140.dev" (= `None`) or a saved **Frigate** backend;
   HD/default pickers exclude Frigate backends and the SP picker shows only them.
   The Connection card's **Advanced** is collapsed by default and holds the global
   **SOCKS5/Tor** toggle. Electrum TLS knobs (`validate_tls`, `ca_cert_path`,
   accept-invalid) are **per-backend** now (edited in Saved backends). The
   **mempool server** got its own per-service cert flag too:
   `backend.mempool_danger_accept_invalid_certs` + a dedicated `AppState.mempool_http`
   client (`prices.rs`/`proxy.rs` use it; `state.http` stays for payjoin/BIP-353),
   exposed as an "Accept invalid / self-signed certificate" checkbox in the
   **Mempool** section (only when the URL is https). This replaced the old global
   `danger_accept_invalid_certs` driving the mempool client — a self-hosted mempool
   is a separate service from the chain backend and shouldn't share a TLS flag.
   *(SP scanning itself is untested here — needs a live Frigate.)*
6. **Capability wiring** — payjoin-receive enabled per wallet (node-backed only).
   Payjoin stays a global opt-in, reframed: send = any software single-sig wallet,
   receive = a wallet on a node (RPC).

## Open / to verify during implementation

- Exact touch points: `config.rs`, `state.rs`, `subscriber.rs`,
  `sp_subscriber.rs`, `api/settings.rs`, `api/wallets/*`, core `types.rs`;
  frontend `add-wallet/+page.svelte`, `ServerPage.svelte`, `WalletSidebar.svelte`,
  `WalletDetail.svelte`, `lib/types.ts`.
- Whether to keep any aggregate indicator beyond per-wallet dots.
- Connection sharing when two wallets reference the same saved backend (group by
  backend id; don't over-optimize).
