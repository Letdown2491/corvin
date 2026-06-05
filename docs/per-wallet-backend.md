# Per-wallet backend

Each wallet can talk to its own backend server, so no single server sees the addresses
of all your wallets. This is a privacy (compartmentalization) feature: keep a low-stakes
"spending" wallet on a public server and a "savings" wallet on your own node, with
neither server able to correlate the two.

## Non-goals

- **No fallback.** A fallback to a different server is the exact moment a second server
  learns about a wallet, which defeats compartmentalization.
- **Network stays global.** One network per instance. Per-wallet applies to the server,
  never the network.
- **Fees, price, and the SOCKS5 proxy stay global.** The mempool/price queries carry no
  wallet info (date and currency only), so a shared price server can't correlate
  anything.

## Model

- There's a **default** backend (a built-in pool of public Electrum servers, one
  pre-selected). A wallet on "Default" uses that one server.
- A wallet may instead use **its own** server, chosen when the wallet is added and
  editable later on the wallet page.
- "Default = a list" means a menu of public servers with one selected. It is not rotation
  or fallback: one server per wallet at a time.
- Behind the picker is a small **saved set of backends** (your node, your private
  Electrum, your Frigate) so a second wallet can reuse one without re-entering it.
- A wallet **syncs and broadcasts through its own backend**; broadcasting through a shared
  server would leak the tx origin even if sync didn't.

### Per wallet kind

| Kind | Backend choice at add time |
|---|---|
| Single signature | Default (public Electrum) / private Electrum / Bitcoin node |
| Multisig | the same three (one backend for the whole wallet; signers are about keys, not chain data) |
| Vault / Policy | the same three |
| Single-sig from a pasted **xpub** (watch-only HD) | the same three |
| **Watch-only (single address)** | silent **Default**, no prompt; still changeable later |
| **Silent Payments** | **Public** (`frigate.2140.dev`) or **Private** (your own Frigate-capable server) |

Notes:
- SP's "Private" is not the HD "Bitcoin node" option. SP scanning needs a server that
  speaks BIP-352 (Frigate); a vanilla node or electrs/Fulcrum can't. So SP's picker is
  its own branch: public Frigate vs your Frigate.
- "Default" is network-specific: on regtest there's no public pool, so it isn't offered
  (pick private or node). SP "Public" depends on `frigate.2140.dev` supporting the chosen
  network.
- payjoin-receive needs the Bitcoin node option, so it's an honest per-wallet capability
  (a wallet "knows" if it's on a node) rather than a global one.

## Config & resolution

- `Config.backend` (`[backend]`) is the default connection.
- `Config.backends` (`[[backends]]`, a `Vec<BackendEntry>`) is the registry of saved
  named backends.
- `Config.default_backend: Option<String>` picks which backend unpinned wallets use:
  `None` means the built-in `Config.backend` connection (the selected public Electrum
  server), `Some(id)` means a saved backend.
- `WalletEntry.backend: Option<String>` pins a wallet: `None` means the default, otherwise
  the `id` of a registry entry.

`Config::effective_backend_id(wallet_backend)` centralizes resolution: a pinned wallet
uses its own id, an unpinned wallet falls back to `default_backend`, and a dangling id
degrades to the built-in connection. `electrum_config_for` / `backend_kind_for` /
`rpc_config_for` and the subscriber's `resolve_group_key` all route through it. SP
resolution is separate (`sp_electrum_config_for`); SP keeps its own scanner config.

The factory-default `Config.backend` is a public Electrum server
(`electrum.blockstream.info:50002`, in `PUBLIC_SERVERS`), so a fresh install preselects it
and works out of the box. `POST /backends/adopt-default` moves a custom built-in default
into the registry and pins it, reusing a matching entry instead of duplicating (so a user
whose default is their saved node just repoints), and runs server-side so a custom RPC
default keeps its password. The ServerPage calls it once when it detects a custom default,
so every default resolves to either a public server or a saved backend.

`BackendEntry` (in `config.rs`) holds the connection subset: id, label, type
(electrum/rpc/frigate), electrum host/port/ssl, validate_tls, ca_cert_path,
danger_accept_invalid_certs, socks5_proxy, and rpc_url/user/pass. `WalletEntry.backend`
is ts-rs exported; regenerate bindings with `just gen-types` after changing it.

## Backend settings page

The page holds the global settings and the saved-backends registry, as five uniform
collapsible cards: **Network**, **Default backend** (the primary control), **Saved
backends**, **Mempool**, and **Payjoin**. Per-wallet assignment happens at add time or on
the wallet page, not here.

The page autosaves: no Save button, and each section flashes "Autosaved ✓" in its header
on change (toggles and dropdowns on change, text and numbers on blur). Each settings page
(Backend, Display) refetches the latest config and writes only its own fields, so they
can't clobber each other.

- **Default backend** is a dropdown of public servers plus saved backends. Picking a
  public server writes host/port/ssl into `Config.backend` and clears `default_backend`;
  picking a saved backend sets `default_backend = id` (no secret-copying, so a node's RPC
  password stays server-side in the registry). Changing the default re-homes unpinned
  wallets immediately (`put_settings` calls `regroup_default_wallets`). Deleting the
  backend that was the default clears `default_backend`. `GET /status` reports the
  effective default so the sidebar stays accurate.
- **Saved backends** (`BackendsSection.svelte`) exposes the full Electrum option set (SSL,
  accept invalid/self-signed certs, strict hostname check, CA cert path, SOCKS5/Tor), plus
  inline edit (blank password keeps the stored one) and a Test-connection button, backed
  by the registry CRUD API (`GET/POST /backends`, `PUT/DELETE /backends/{id}`,
  `POST /backends/test` in `api/backends.rs`; `rpc_pass` masked on read, preserved on
  empty update).
- **Mempool** has its own per-service `mempool_danger_accept_invalid_certs` and a dedicated
  `AppState.mempool_http` client (`prices.rs`/`proxy.rs` use it; `state.http` stays for
  payjoin/BIP-353), since a self-hosted mempool is a separate service from the chain
  backend and shouldn't share a TLS flag.
- **Payjoin** is a global opt-in (third-party directory + OHTTP relay); send works from any
  software single-sig wallet, receive needs a wallet on a Bitcoin node (RPC).

## Engine: subscriber

`subscriber.rs` is a supervisor that groups wallets by their effective backend and spawns
one worker connection per distinct backend (everyone on the default shares one). Each
worker subscribes its group's scripts and headers, tracks its own tip, and reuses the
reconnect/backoff loop. The supervisor looks up a wallet's backend via `resolve_group_key`
and routes its `SubCommand`s to the right worker, so the command shape and its send sites
were left unchanged; `reconnect` uses `wake_signal.notify_waiters()` to kick every worker.
Broadcast for a wallet goes through that wallet's connection
(`broadcast_transaction(state, tx, backend)`; `/broadcast` resolves the backend from
`wallet_id`, and a generic paste-a-tx with no wallet uses the default). `sp_subscriber.rs`
runs one task per SP wallet against its referenced Frigate backend.

## Per-wallet UI

- The **add-wallet flow** has a Backend dropdown (Default plus saved backends) for every
  kind except the single-address watch-only (which uses a silent default). `backend` is
  threaded through all creation requests and set on `WalletEntry.backend` at creation.
- **Change backend** on an existing wallet is a kebab action (`ChangeBackendModal`), not the
  details modal (whose Backend row is read-only). The picker offers Default, public Electrum
  servers, and saved backends (SP wallets: public Frigate plus saved Frigate); picking a
  public server materializes it into the registry (deduped), since a wallet can only
  reference a saved id. `PUT /wallets/{id}/backend` validates, saves, re-routes (non-SP
  re-issues `AddWallet` so the supervisor re-groups; SP respawns its task), and re-syncs.
- **Connection status is per-wallet.** `AppState.backend_status` is written by the workers
  (Electrum workers mark connected + tip in `run_session`, disconnected with the error on
  loss; the RPC branch probes each cycle) and exposed at `GET /backends/status`. The sidebar
  shows a compact per-wallet dot (green/red/grey by the wallet's backend), and the wallet
  page shows a status line that groups connection, backend, sync, block, and fee rate, for
  example `$388.24 · ● Start9 synced 2m ago · block 951,817 · Fees: 1.0 sat/vB`. Opening a
  wallet triggers a sync so the data and sync time are current. The old global sidebar
  connection/block line was removed (it reflected only the default backend, misleading once
  a wallet is on a different one); the sidebar still polls `/status` and fees to keep the
  shared `nodeStatus`/`feeRates` stores fresh for the wallet page, send flow, and mobile
  layout. SP-only backends have no Electrum worker, so their dot reads "unknown" unless an
  HD wallet shares the backend.

## Silent Payments

An SP wallet is created with "Public · frigate.2140.dev" (`backend: None`) or a saved
**Frigate** backend, resolved via `Config::sp_electrum_config_for`. Backends carry a
`frigate: bool` capability flag (an Electrum server that also speaks BIP-352);
`BackendsSection` exposes it as a "Frigate (Silent Payments)" type, the HD/default pickers
exclude Frigate backends, and the SP picker shows only them. There is no separate "SP
scanner" settings section: the `None` default scanner uses the `sp_electrum_*` fields,
which default to `frigate.2140.dev`, because a regular Electrum server can't scan BIP-352
and so the default never mirrors the main connection. SP scanning against a live Frigate
is a manual verification item.

## Open / to verify

- Whether to keep any aggregate indicator beyond the per-wallet dots.
- Connection sharing when two wallets reference the same saved backend (group by backend
  id; don't over-optimize).
- SP scanning end to end needs a live Frigate server.
