# Corvin — runtime verification plan

These are the code paths that are **built + typecheck/clippy-clean** but can't be
proven without a live chain, a real hardware wallet, or the packaged desktop app.
Work top to bottom; tick each box. None of this can be run by the dev tooling — it's
a hands-on pass.

Tracks A–E are the original wallet/signing paths. **Tracks F (per-wallet backends +
settings) and G (desktop shell)** cover the 2026-05-31 round and are entirely
unverified live.

**Already verified:** ✅ BIP-86 single-sig taproot send→sign→broadcast on BitBox (#89).

Legend: **Gate** = Corvin's build-time logic (no signing needed). **Sign** = needs an
external signer (hardware wallet, or Core/Sparrow with the seed). **Chain** = needs a
node.

---

## Track A — Regtest: the timelock GATE (fast, no signer, no device)

This verifies the per-path / per-UTXO timelock gating I built, by attempting the
**build** in Corvin and reading the result — Corvin rejects a locked spend at build
time, so you don't need to sign anything.

**Stack:** `bitcoind -regtest` + an electrum server (electrs/Fulcrum). In Corvin,
set Network = Regtest, then add your regtest Electrum under Backend → Saved backends
and select it as the Default server (the public list doesn't apply on regtest). Mine
with `bitcoin-cli -regtest -generate N`.

1. **Vault, primary path is never gated**
   - Create a Vault (single-key primary + a distinct recovery key), fund the primary
     receive address (`sendtoaddress` + mine 1), sync.
   - Send → **Spend via: Primary** → build.
   - ✅ Expect: builds a PSBT immediately (primary is never locked).

2. **Vault, recovery path locked → unlocked boundary** (relative `older(N)`, use small N e.g. 5)
   - With coins funded at height *h*, immediately: Send → **Spend via: Recovery**.
   - ✅ Expect: the **Recovery option is disabled** with "available in ~N blocks", and a
     build attempt errors *"…earliest unlocks in ~N more block(s)"*.
   - Mine `N-1` blocks, sync. ✅ Expect: still locked (boundary not yet reached).
   - Mine 1 more (now `N` confs), sync. ✅ Expect: Recovery option **enabled**, build
     succeeds. ← this is the CSV off-by-one check.

3. **Timelocked savings, locked → unlocked**
   - Create a Timelocked savings wallet (relative `older(N)`), fund it, sync.
   - ✅ Expect: lock banner "🔒 Locked savings — earliest unlocks in ~N blocks"; Send
     build is blocked.
   - Mine to N confs, sync. ✅ Expect: banner flips to unlocked; build succeeds.

4. **Absolute-height timelock** (repeat #2 or #3 with "At block height" = current tip + 5)
   - ✅ Expect: locked while `tip < H`; unlocks once `bitcoin-cli -generate` pushes
     `tip ≥ H`.

> If a build that should be locked instead succeeds (or vice-versa at the exact
> boundary), that's the bug to report — note the configured N/H, the tip, and the
> UTXO's confirmations.

---

## Track B — Signet + hardware wallet: the real signing paths

Signet (not regtest) because BitBox/Ledger/Coldcard speak signet natively. Get coins
from a signet faucet. This track proves the **actual signatures** and the **HW
wallet-policy registration**.

### B1 — Vault spend on hardware (BitBox / Ledger)
1. Create a **Taproot Vault**, single-key primary = **Connect device** (its `m/86'`
   key), recovery = a second key you control. Fund the receive address on signet.
2. **Primary (key-path) spend:** Send → Primary → **⚿ Hardware wallet**.
   - ✅ First time: the device prompts to **register the wallet policy** — verify the
     keys/timelock on-screen, accept.
   - ✅ Sign the key-path spend on the device → broadcast → confirms.
3. **Recovery (script-path) spend:** once the timelock has elapsed (signet ~10-min
   blocks, so use a short relative N), Send → **Recovery** → Hardware wallet.
   - ✅ Expect: device signs the tapleaf; broadcast finalizes via `/broadcast` (BDK
     assembles the script-path witness) → confirms.
   - ⚠️ If the device can't sign the tapleaf (Trezor, or older firmware), fall back to
     software signing — note which device/firmware.
4. **wsh Vault:** repeat 1-3 with the **SegWit** toggle. ✅ Same outcomes; recovery
   PSBT comes back partial and `/broadcast` finalizes it.

### B2 — Coldcard (airgap, EDGE firmware)
1. Wallet details → **Download descriptor** → import the multipath descriptor onto the
   Coldcard (requires **EDGE** firmware for miniscript/tapscript).
2. Fund → Send → **Export** the unsigned PSBT (file/QR) → sign on Coldcard → **import
   signed PSBT** → broadcast.
   - ✅ Expect: signs + confirms for primary; recovery after timelock.

---

## Track C — Silent Payments (signet, public Frigate)

Uses Corvin for both sides — an HD wallet funds the SP address, then the SP wallet
spends. SP scanner = the public Frigate `frigate.2140.dev:50002`, chosen at SP wallet
creation (the "Public" scanner option; or pin a saved Frigate backend).

1. **Receive:** create an SP wallet (from seed). From a funded HD wallet, Send to the
   `tsp1…` address (HD→SP send). ✅ Expect: the SP wallet **discovers the payment
   without a restart** (the new dynamic scanner) — balance appears within a scan cycle.
2. **Labeled address:** add a label, Send to the labeled `tsp1…`. ✅ Expect: discovered
   without restart (scanner re-subscribes on label add).
3. **SP spend:** from the SP wallet, Send to a regular signet address with your seed.
   - ✅ Expect: signs (no-tweak Schnorr) → broadcasts → confirms; **change returns as
     discovered SP change** (m=0). ← verifies the no-taproot-tweak signing + change
     derivation.
4. **High account index:** create a from-seed SP wallet at **account 25**, fund it, and
   confirm you can **spend** it. ← regression for the account-index fix (was unspendable
   at ≥20).
5. **Reconciliation:** with an SP wallet holding ≥1 unspent output, spend that coin from
   *another* wallet/device (or note one spent outside Corvin), then **Sync** the SP
   wallet. ✅ Expect: the spent output drops out and the balance corrects.

---

## Track D — BIP-353 (any network, DNS only)

1. Send modal → recipient field → enter a **real** on-chain `₿user@domain` (a name with
   a DNSSEC `bitcoin:` TXT record — not a Lightning Address / Strike / Proton, which
   won't resolve). ✅ Expect: "🔒 Resolved … via DNSSEC" and the on-chain/SP address
   fills in.
2. Enter a Lightning-address-format name with no BIP-353 record. ✅ Expect: the graceful
   "no record / Corvin can't pay this" message.

---

## Track E — Payjoin send (BIP-77 / v2)

Software single-sig wallets only. Easiest receiver = `payjoin-cli` (from the
rust-payjoin repo) or a BTCPay instance. Settings → Backend → **Payjoin → Enable**,
then set the directory + OHTTP relay (public defaults, or a local
`payjoin-directory` for regtest).

1. **Happy path (signet or regtest).** Have the receiver produce a `bitcoin:…?pj=…`
   v2 URI. Paste it into Send from a funded software wallet → enter seed → start.
   - ✅ Expect: status goes **negotiating → proposal ready**; the diff shows the
     receiver **added ≥1 input** and the payjoin fee. Confirm (re-enter seed) →
     broadcasts → confirms. The on-chain tx contains **both parties' inputs**
     (the common-input-ownership heuristic is broken). ← the core privacy win.
2. **Timeout → fallback.** Paste a `pj=` URI whose receiver is **offline**; wait
   `payjoin_fallback_secs`. ✅ Expect: the **original** (non-payjoin) tx broadcasts
   and status flips to **fell back** (SSE toast). Funds still arrive.
3. **Restart resilience.** Start a send, kill Corvin mid-negotiation, restart.
   ✅ Expect: the poll task resumes (`payjoin_subscriber::start`) — proposal still
   lands, or it falls back if the deadline already passed while down.
4. **v1-only URI.** Paste a v1 (no-OHTTP) `pj=` URI. ✅ Expect: Corvin reports it's
   v1-unsupported and offers a **normal send** to the on-chain address (never the
   v2 builder, which would panic).
5. **Wrong seed at confirm.** Re-enter a wrong seed at the confirm step. ✅ Expect:
   "didn't finalize after signing" error, no broadcast; original still available.

> The async session lives in `~/.config/corvin/payjoin/<sid>.json`; the index maps
> session→wallet+status. All payjoin HTTP honors `socks5_proxy`.

### Receive (phase 2 — needs a wallet pinned to an RPC/node backend + native-segwit/taproot)

6. **Receive happy path.** Use a zpub/taproot wallet **pinned to a Bitcoin node (RPC)
   backend** (add it under Backend → Saved backends, then pin the wallet to it). Receive
   modal → **Create payjoin invoice** → share the `pj=` URI/QR. Pay it from another
   wallet (or Corvin's payjoin send).
   - ✅ Expect: the receiver task runs the checks, contributes one of your UTXOs,
     and the modal flips to **payment received** (SSE). Enter seed → **Confirm
     payjoin** → the proposal posts back; the payer broadcasts a tx with **both
     parties' inputs**.
7. **No-RPC guard.** On a wallet whose backend is Electrum (not a node), the Receive
   modal should **not** offer payjoin (no `testmempoolaccept`). ✅ Expect: no payjoin block.
8. **Replay protection.** After one receive, confirm the seen-inputs store
   (`~/.config/corvin/payjoin/seen_inputs.json`) grew; a re-sent original reusing
   those inputs is rejected at `check_no_inputs_seen_before`.
9. **Frozen UTXOs.** Freeze all but one UTXO; confirm the contributed input is
   never a frozen one. Freeze *all* → expect "no spendable UTXOs to contribute".

10. **Resume a parked proposal.** Provision an invoice, get it paid (status flips to
    proposal_ready via SSE), then **close** the Receive modal without confirming.
    Reopen Receive on the same wallet. ✅ Expect: it resumes straight to the
    **confirm** step (the modal lists receive sessions on open and jumps to any
    `proposal_ready` one). Enter seed → confirm → posts.
11. **SOCKS5 key fetch.** With a `socks5_proxy` set, provision a receive invoice. ✅
    Expect: the OHTTP-gateway key fetch goes through the proxy (Tor), not the
    crate's direct relay client — check no clearnet DNS/connection for the
    directory during provision.

12. **Resume a waiting invoice.** Create a payjoin invoice, **close** the Receive
    modal before any payment, reopen it. ✅ Expect: the same invoice (QR + `pj=`)
    is re-displayed (one active invoice per wallet) — not cancelled, not duplicated.
    It persists until paid, **Cancel**led, or the crate's 24h expiry.

---

## Track F — Per-wallet backends + settings (any network; needs ≥1 real Electrum + a node)

Covers the per-wallet-backend feature and the autosaving Backend/Display pages
shipped this round. Most of this is **Gate**-style (read the UI result); the
sync/broadcast steps need live servers.

### F1 — Saved-backends registry
1. Backend → **Saved backends** → add an **Electrum** backend (your own server),
   **Test connection** → ✅ shows connected + tip. Save.
2. **Edit** it (change a field), Save → ✅ persists; **delete** it → ✅ confirms first,
   and any wallet using it falls back to the default.
3. Add a backend with a **self-signed cert**: tick "Accept invalid / self-signed
   certificates" in its Advanced options → ✅ Test connection succeeds (fails without).
4. Add a **Bitcoin node (RPC)** backend with creds → ✅ Test connection succeeds; the
   `rpc_pass` is blanked on reload but preserved (don't have to retype to re-test).

### F2 — Default backend
5. Backend → **Default backend** → pick a public Electrum server → ✅ the global status
   reflects it; wallets with no pin use it.
6. **adopt-default migration:** on an instance whose default was a *custom* server (not
   in the public list), open Backend → ✅ that server now appears in Saved backends and
   is selected as default, with **no duplicate** entry (re-open to confirm still one).

### F3 — Per-wallet pinning
7. Add a wallet and **pin it** to a saved backend at creation → ✅ it syncs through that
   backend (balance/txs populate); send a tx → ✅ broadcasts through that backend.
8. **Change backend** (wallet ⋯ menu → Change backend) to a different saved backend →
   ✅ re-homes and re-syncs without restart; the wallet-page chip + sidebar dot update.
9. Pick a **public server** in the change-backend picker → ✅ it materializes into Saved
   backends (deduped) and the wallet pins to it.

### F4 — Per-wallet connection status
10. With two wallets on two different backends, take one backend offline → ✅ that
    wallet's sidebar **dot turns red** and its page chip shows disconnected, while the
    other stays green. (Confirms status is per-backend, not global.)

### F5 — Mempool + autosave
11. Point **Mempool** at a self-hosted instance over `https://` with a self-signed cert;
    tick its **Accept invalid / self-signed certificate** → ✅ fees + BTC price return
    (this was the regression: the mempool client has its own TLS flag).
12. **Autosave:** change a toggle on Backend and on Display → ✅ "Autosaved ✓" flashes in
    that section header, no Save button, and the change survives a reload.
13. **No cross-page clobber:** change a Display price toggle, then change a Backend
    field, reload → ✅ both stuck (each page refetches + writes only its own fields).

### F6 — SP per-wallet Frigate
14. Create an SP wallet pinned to a **saved Frigate backend** (not the public default) →
    ✅ it scans against that server (see Track C for the receive/spend checks).

---

## Track G — Desktop shell (Tauri / WebKitGTK; Linux especially)

Regression + crash-fix checks specific to the desktop build. Rebuild
`corvin-desktop` first (release embeds `dist/` at compile time).

1. **New-window links don't crash** (the WebKitGTK `WindowFeatures` abort fix):
   click a tx's **View on mempool**, the **Whitepaper** link, a **Help** "?" link, and
   the BIP-329 link in Import/Export. ✅ Expect: off-origin/PDF open in the system
   browser, same-origin (Help) opens a second Corvin window — **no crash**.
2. **Onboarding wizard** (first run): start with a fresh config dir
   (`CORVIN_CONFIG_DIR=/tmp/corvin-x`) → ✅ wizard shows; **Skip** dismisses and doesn't
   return; finishing hands off to Add wallet. Then Display → **Onboarding** → uncheck →
   ✅ wizard reappears.
3. **Camera / QR** (regression): Receive/Broadcast → Scan QR → ✅ camera opens and scans
   (getUserMedia still granted after the new-window changes).
4. **Print + save dialogs:** seed backup **Print**, and any **download** (CSV/PSBT) →
   ✅ native dialogs appear (WebKitGTK print signal + save_file IPC).

---

## Report back
For each box: pass / fail + (on fail) the network, heights/confs, device + firmware,
and the exact Corvin error or on-chain txid. Failures most likely cluster at: the CSV
boundary (A2), HW tapscript signing (B1.3), SP change discovery (C3), per-wallet
backend re-homing (F3/F8), or the desktop new-window fix (G1).
