# Corvin positioning — vs. Sparrow, and where to go

A strategic reference (not a spec). Compares Corvin to Sparrow (the closest
peer: desktop, descriptor-based, privacy-minded, open source) and captures
candidate directions. Revisit as both projects move.

> Accuracy caveat: Corvin claims here are from this codebase. Sparrow claims are
> from its public docs/features as of 2026-05 and may drift — re-check before
> leaning on any single point publicly, especially Sparrow's payjoin version and
> whether it has added Silent Payments.

## Where Corvin is ahead

- **Silent Payments (BIP-352), full send + receive + spend** as a first-class
  wallet kind with a background Frigate scanner. Sparrow has no SP. This is the
  standout — very few wallets ship full SP.
- **Payjoin v2 / BIP-77** (async, send + receive, survives restarts, directory +
  OHTTP relay). Sparrow's payjoin is the older online-only v1.
- **Per-wallet backends** — registry + default + per-wallet pinning so no single
  server correlates all your wallets (privacy compartmentalization).
- **Deployment model** — single binary serving a web UI: desktop app *and* Start9
  *and* headless, reachable from a browser/phone. Sparrow is desktop-only (JavaFX).
- **BIP-353** human-readable `₿user@domain` payments via DNSSEC.
- **Tax reports** (FIFO/LIFO/HIFO) built in; Sparrow only exports CSV.

## Where Sparrow is ahead

- **Maturity & trust** — years old, widely scrutinized, reproducible builds, signed
  releases, known maintainer. Corvin is new and **largely unverified live** (whole
  verification plan still pending). Biggest gap for money software.
- **Hardware wallet breadth + airgap UX** — essentially every device, USB +
  airgapped, with polished PSBT-over-QR/file flows and years of quirk handling.
- **Transaction-construction depth** — manual in/out selection, fee modeling,
  drag-to-build, deep PSBT inspection.
- **PayNym / BIP-47** reusable payment codes (SP arguably supersedes for the same
  goal, but Sparrow ships it today).
- **UTXO/tx visualization + privacy analysis** workflows.
- **Reproducible builds + signed releases** — table stakes Corvin hasn't reached.

## Where to lean (identity)

Corvin is ahead on **privacy-tech surface area** (SP, payjoin v2, per-wallet
backends, BIP-353) and **deployment** (headless / web / Start9). Sparrow is ahead on
everything **maturity** buys. Strategy: **don't out-Sparrow Sparrow on the desktop
power-user tx editor.** Lean into **"privacy by default, self-hosted anywhere"** —
where the codebase already invests and where a desktop-only wallet structurally
won't follow.

## Candidate directions (to discuss, not committed)

1. **Silent Payments as the default receive flow.** Treat a reusable SP address as
   the *normal* way to receive (the way wallets treat fresh addresses today).
   "One address forever, stays private" as the headline UX. Novel; builds on what
   we already have.
2. **Integrated privacy stack as the product.** SP receive + payjoin v2 send/receive
   + per-wallet backend isolation, presented as one coherent "privacy by default"
   experience. No one occupies this cleanly.
3. **Privacy score / leak-warning surface.** **User is interested; notated for later
   (2026-05-31), not committed.** Reference point: the "Am I Exposed?" tool the user
   already uses. Corvin's structural advantage over such a tool: it *builds the tx*,
   so it can catch leaks **before** they happen and coach during the action (Sparrow
   shows data but doesn't coach).

   **Design conclusion from discussion — one heuristics engine, two surfaces:**
   - Build the leak heuristics once: address reuse, change-type mismatch (change to a
     different script type than the payment), toxic/dust change, common-input-
     ownership (a spend links UTXOs from separately-labeled clusters), round-amount
     reveal, output to a reused address.
   - **Surface A — pre-send coaching (prevention; the high-value, uniquely-Corvin
     part).** Extend the existing `compute_send_warnings` (`crates/server/src/api/
     wallets/send.rs`, today flags mixed-labels / repeat-recipient / round-amount)
     with the per-tx checks above, shown before signing while leaks are still fixable.
   - **Surface B — per-UTXO/wallet flags (cheap bonus from the same engine).** Flag
     toxic/reused UTXOs in the existing UTXO table; the actionable response (freeze
     it) already exists via UTXO freezing.
   - **Optional later:** a standalone "privacy score: NN/100" dashboard — flashier but
     the lower-value half (diagnostic, about leaks already made).

   **Sequencing + caveat:** do the **per-tx, pre-send checks first** (tractable, low
   false-positive). Whole-tx-graph **clustering** heuristics (common-input-ownership
   *history*, "these got linked") are real work and noisy — defer, or skip if too
   false-positive-prone. Lead with prevention, not diagnosis.
4. **Reproducible builds + signed releases.** Trust table-stakes; arguably the
   single highest-leverage non-feature for a money wallet. **User is interested;
   researched + planned (2026-05-31) — see `docs/reproducible-builds.md`.** Phased:
   (1) signed releases + SHA256SUMS/minisign, (2) desktop notarization/signing,
   (3) bit-for-bit reproducible Linux binary, (4) Start9 package image.
5. **At-rest encryption tuned for headless** (the unlock-on-access model in
   `docs/at-rest-encryption.md`). **SHIPPED** (opt-in, default off): Argon2id +
   XChaCha20-Poly1305 over the whole config dir, boot-locked with a full-screen
   unlock gate, so it works the same on desktop and on a headless node reached from
   your phone via its URL. Nobody else has nailed "encrypted, headless, unlock from
   your phone via the node URL." Strong fit for the self-hosted crowd.
6. **The web-UI form factor itself** — full wallet on your own node, reachable from
   any browser/phone, no app store, no custodian. A deployment story desktop wallets
   can't match.

## Notes
- Items 3 and 4 flagged by the user as most interesting (2026-05-31); discuss scope.
- "Am I Exposed?" is a reference point for the privacy-score heuristics (item 3).
