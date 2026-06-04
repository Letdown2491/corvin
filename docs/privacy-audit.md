# Privacy audit (2026-06)

Companion to the code audit. The code audit asked "is this correct?"; this pass
asks **"who learns what about the user, and is that the minimum?"** Threat model
is unchanged: single user, self-hosted, trusted local machine — but privacy from
*remote counterparties* (servers you talk to) and *on-chain observers* is a core
product value, so it gets its own lens.

Findings are organized by the three planes a wallet leaks on: network, on-chain,
and at-rest.

## Verified strong (no change needed)

- **All network egress honors the SOCKS5 proxy** — Electrum (TLS + plain), the SP
  scanner socket, the shared reqwest client, the mempool client, and BIP-353 DoH.
- **The browser makes zero direct external calls** — everything goes through
  `/api/*` to the local backend, so the SPA never leaks the user's IP.
- **Off by default:** price/fee fetching (mempool.space), payjoin, fiat display.
- **On-chain hygiene:** fresh change address per tx, anti-fee-sniping locktime
  (BDK default), coin control + categories + freezing for compartmentalization,
  and the send-time warning suite (reuse / mixed-label / mixed-category /
  look-alike / round-amount). Payjoin breaks common-input-ownership.
- **Disclosure already present:** SP wallet creation carries a "Trust note" (the
  Frigate operator sees your receipts); onboarding states "whoever runs a backend
  can see the addresses you watch."
- **Debug export** deliberately excludes seeds/keys/addresses/labels/balances.

## Fixed in this pass

- **Whole-wallet consolidation on every send** (code audit, HIGH) — was the single
  largest on-chain leak; auto-select now coin-selects instead of spending every
  UTXO. See the send-path fix + regtest tests.
- **SP scanner survived wallet deletion** — kept the scan key in RAM, stayed
  subscribed to the third-party SP server, and resurrected `sp_outputs.json`.
  Now aborted in `delete_wallet`.
- **BIP-353 DoH resolver was hardcoded to `dns.google`** with no disclosure. The
  resolver sees the name you look up (= who you're about to pay); DNSSEC stops
  forgery, not observation, and a SOCKS5 proxy hides the IP but not the name. Now:
  - configurable via `backend.bip353_doh_url` (Backend settings, Name resolution),
    defaulting to Quad9 (`dns.quad9.net`) rather than `dns.google`,
  - disclosed in the send flow when a name resolves, and in the help.

## Shipped (follow-up pass)

### (d) Change-output script-type fingerprinting — done
Added `SendWarningCode::ChangeScriptMismatch` (Info severity): warns when the
change output's script type differs from the recipient's, so the user knows the
change is fingerprintable (it's the output matching the wallet's own input type).
Wired into `compute_send_warnings` + the help; regression-tested.

### (e) SP-send input-side warnings — done
Extracted the input-only heuristics (mixed labels + mixed categories) into a shared
`compute_input_warnings`, and wired it into the SP-send path — spending HD coins
together still links their labels/categories on-chain regardless of where they go.
Recipient-side heuristics (repeat/look-alike) stay omitted: the SP recipient key is
fresh per payment.

## Deferred — documented, not yet actioned

### (f) At-rest encryption — SHIPPED (separate track)
Was: a local attacker / seized disk sees the full transaction graph, labels, and
the **SP scan secret** (which lets them watch your incoming SP payments). Files are
0600 but unencrypted. **Now built** (opt-in, default off): turning on encryption in
Settings → Security seals every on-disk file (wallet data, JSON, labels, SP scan
secret) under an Argon2id-derived key with XChaCha20-Poly1305; the app boots locked
and unlocks on access. See `docs/at-rest-encryption.md`. This closes the biggest
on-disk privacy gap for anyone who opts in.

## Open product decisions (not bugs)

- **Default public Electrum + Tor-off** is the biggest default-privacy posture: a
  fresh user's whole wallet is visible to the default server until they switch.
  The per-wallet-backend feature is the mitigation, but it's opt-in. Question for a
  future pass: should onboarding push Tor / your-own-node harder, or auto-enable the
  proxy when a local Tor port is detected?
- **Address-set exposure to the Electrum server** (subscribing to scripthashes +
  the 20-address lookahead) is inherent to the Electrum protocol; mitigated only by
  running your own server / Tor. Not fixable in-protocol.

## 2026-06-03 pass (net-new, all fixed — full record in `docs/audit-2026-06-03.md`)

- **Webview `localStorage` leaked beyond the at-rest boundary.** Send drafts stored the
  recipient address; multisig setup stored signer xpubs — both in the unencrypted
  webview profile. Fixed: drafts persist only non-identifying fields (amount/fee/
  coin-control/threshold); addresses and xpubs are no longer written (and stripped from
  older drafts on load).
- **`/api/status` reachable while locked** → pre-unlock probe to the default public
  Electrum server, un-proxied. Fixed: removed from the lock-gate allow-list.
- **SP subscriber logged the receive address (INFO) + a txid (WARN)** — the only
  wallet-identifying data in default logs. Fixed: demoted to DEBUG.
- **SP scanner TLS hostname check** relaxed when `validate_tls` was off (inconsistent
  with Electrum). Fixed: only relaxed under the explicit danger flag.
- Re-confirmed: all egress proxied, per-wallet backend resolution correct, service
  worker never caches `/api/*`, prior fixes hold.
