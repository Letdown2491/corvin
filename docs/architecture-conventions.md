# Architecture + refactoring conventions

Guidance for keeping the codebase maintainable as it grows. Written 2026-06-01
after a discovery pass on file sizes + duplication. Not a rewrite mandate —
a set of rules for *new* code and a prioritized list of the genuine debt.

## The principle

**Size alone is not debt. Mixed concerns × churn is.** A long file that is
cohesive and rarely changes (static help content, a self-contained parser with
its tests) is fine. A long file that mixes unrelated concerns *and* changes
often is where bugs hide and merges hurt. Target the second kind; leave the
first alone.

## Conventions for new code

- **Folder-per-feature for big components.** When a component grows past ~1 screen
  of distinct concerns, split it into a folder: `send/` (RecipientList, CoinControl,
  FeeSelector, SignPanel, VerifyPanel, …) with a thin page/modal that composes them.
- **Pure logic lives in `lib/`, and is unit-tested.** The proven pattern
  (`lib/amount.ts`, `lib/send.ts`): pull framework-free parsing/formatting/validation
  out of components into `lib/*.ts` with colocated `*.test.ts`. Safe to refactor
  (type-checked + Vitest), and shrinks the component. Do this *before* splitting the
  component itself.
- **Shared UI primitives live in `components/ui/`.** Don't hand-roll a modal shell,
  busy button, copy button, QR, or amount input per component — compose the primitive
  (see catalog below). New components should reach for these first.
- **Backend: one module = one concern.** Split handler files by concern
  (crud / reads / sync), keeping public handler signatures stable so the router and
  callers don't churn. Mechanical moves, low risk.
- **Refactor money code safest-first:** pure-logic extraction (tested) → primitives →
  component decomposition (type-check + manual test, never big-bang). We have no
  Svelte component tests, only pure `lib` tests — so logic extraction is the safety net.

## Shared UI primitives (`components/ui/`)

Catalog from the 2026-06-01 discovery pass (counts = files/occurrences touched).
Already shared (don't re-invent): toasts (`stores/toasts` + `ToastContainer`),
`EmptyState`, `HelpLink`.

### Tier 1 — built
- **`Modal.svelte`** — shell over native `<dialog>`: `open` (bindable), `title`,
  `desc`, body + `footer` snippets; centralizes show/close, **Esc**, backdrop-click,
  focus, the ✕ close button, header/desc layout. Replaces scaffolding duplicated
  across **16 files** (and fixes the Esc inconsistency — only 6/16 had it).
- **`BusyButton.svelte`** — owns a `busy` flag around an async `onclick` (try/finally),
  shows `idle`/`busyLabel`, disables while busy. Replaces the hand-rolled
  `busy = $state(false)` + ternary-label pattern in **15 files / 32 buttons**.
- **`CopyButton.svelte`** — copy text + "Copied ✓" flash + failure toast. `text` is a
  string or getter; optional `disabled`. Migrated: MessagesModal, FeeBumpModal,
  ConsolidateModal, WalletDetailsModal (×3), ReceiveModal (×4). **Justified exceptions
  (left as hand-rolled):** AddressTable/UtxoTable per-row icon buttons (one component
  instance per row × many rows, and they want a `.copied` flash *class* the primitive
  doesn't expose); ServerPage "Copy debug info" (text is built async — getter is sync);
  SeedGeneratorPanel (copy-failure shows an inline "write the words down" message, not a
  toast — deliberate for the seed flow). SendModal copy buttons migrate with Send v2 (#32).

### Tier 2
- **`AmountInput.svelte`** — **built** (Send v2). Bitcoin-amount input bound to a
  string `value` in a given `unit`; controlled by default (parent owns the BTC/sats
  toggle, e.g. SendFlow's one shared toggle across recipient rows), with an opt-in
  `showToggle` standalone mode that owns + converts the unit via `amount.ts`. Now used
  by SendFlow recipients (controlled) **and** ReceiveModal's amount (standalone toggle).
  Note: fee-rate inputs (FeeBump, FeePicker) are sat/vB, **not** BTC amounts — AmountInput
  doesn't apply there; there are no other standalone amount fields to migrate.
- **`QrDisplay.svelte`** — planned. Render a string/PSBT as QR (static + animated/BBQr).
  **7 files** duplicate QR rendering today (the *scanner* is already shared).
- **`Field.svelte`** (+ small form helpers) — planned. label/input/hint/error wrapper,
  composed not monolithic. **~109 `<label>`** across the app.
- **`TxFlowDiagram.svelte`** — **built** (#31, in Send v2's review step). Inputs→outputs
  as one stacked proportional bar + amount legend; values + formatter injected.
  Still to drop into the transaction-detail view (#31's other home).
- **`send/FeePicker.svelte`** — **built** (#36). Visual mempool-blocks fee picker:
  one row of speed tiles (Priority/Standard/Economy/Custom), each colored by congestion
  + showing the matching projected block's fullness/size. Controlled (binds preset +
  customFeeRate).

### Tier 3 — defer / build with the feature
- **`AddressDisplay.svelte`** — chunked echo + address fingerprint. Build with the
  flow-diagram / fingerprint work (tasks #31 / #3), not before.

### Not primitives (deliberately)
- Toasts, EmptyState — already shared.
- Data-load generation-guard — single use (WalletDetail only); keep it local.
- Native `confirm()` — only 5 calls; not worth abstracting.

## Migration approach

Build a primitive, migrate **1–2 safe call sites** to validate, then roll out
incrementally — never all 16 modals at once. **Money modals (SendModal) migrate as
part of Send v2 (#32)**, so they're restructured once into their final shape rather
than refactored twice.

**Scoped-CSS constraint (learned 2026-06-01).** A parent's component-scoped style
(e.g. `.copy-btn` in MessagesModal) does **not** apply to a child component's
internal element. So `BusyButton`/`CopyButton` can only match a button's existing
look when that look comes from a **global** class (`btn-primary/secondary/danger/
ghost`, defined in `routes/+layout.svelte`). They take an optional `class` prop, but
it must be a *global* class. Consequences:
- Buttons already using `btn-*` are clean, visually-identical drop-ins (e.g. the
  Backend-page "Test connection" button → `BusyButton variant="secondary"`).
- Buttons with **bespoke per-component classes** (`.copy-btn`, `.copy-psbt-btn`, …)
  can't adopt the primitive without first making that style reach the child. Two ways:
  (a) promote it to a global class in `routes/+layout.svelte` (e.g. `.btn-copy`), or
  (b) keep the rule in the owning component but wrap the selector in `:global(...)` and
  give it a **namespaced** name to avoid collisions (`.rcv-copy-btn`, `.wd-copy-link`,
  `.copy-psbt-btn`). Use (b) for one-modal styling, (a) when several components share it.
  Either way, don't silently swap and lose the styling. (No browser test here = visual
  regressions are invisible, so this matters.)

## Prioritized refactor targets (the genuine debt)

Frontend (mixed-concern + high-churn):
1. `SendModal.svelte` (2,498) — **Send v2 (#32) shipped: rebuilt as `send/SendFlow.svelte`,
   a two-step modal** (Compose → Review & Sign). Extracted `ui/AmountInput`, `send/FeePicker`
   (#36), `TxFlowDiagram` (#31); merged the numeric preview + flow into one Transaction panel.
   Tried a full-page route, reverted (modal-sized content; route was the app's only
   navigate-away action). SendFlow keeps a **bespoke `<dialog>` shell** mirroring `ui/Modal`
   (its phase-driven footer + Reset chrome + edge-to-edge sectioned body don't fit the shared
   panel) — a deliberate, documented exception to the all-modals-use-ui/Modal rule.
   Decomposed further: `send/RecipientRow.svelte` (one recipient row — address/scan/wallet-picker/
   HRN/SP-detect/echo + amount, bindable recipient + callbacks) and `send/CoinControlPanel.svelte`
   extracted; SendFlow now ~2,050 LOC. Remaining: refit the Payjoin panel into the two-step
   (currently parked in Compose).
2. ~~`add-wallet/+page.svelte` (1,886)~~ — **done (#39): 983 LOC.** Self-contained
   per-kind panels (`VaultCreatePanel`, `SpCreatePanel`, `MultisigCreatePanel`), each
   owning its sub-state + backend pin + create call + actions, emitting `onCreated`;
   plus leaf extractions `KindPicker`, `MsSignerCard` and pure `lib/import` helpers.
   What stays in the page: the single-sig kind (5 methods: seed/paste/file/hardware/
   descriptor) + the watch-only address kind — they share `input`/`method`/the BitBox
   HW flow and feed one `submit()`/`canSubmit`, so they're the cohesive core, not debt.
   Pattern for a self-contained kind panel: `label` prop (make it `$bindable()` only if
   the panel restores it, e.g. multisig's localStorage draft), own backend `<select>`,
   own validation + create, hide the page's shared footer for that kind.
3. ~~`WalletDetail.svelte` (1,199)~~ — **done (#40): 874 LOC.** Extracted `BalanceHero`,
   `WalletMenu`, `TxSearchBar` (+ earlier `WalletToolsTab`).

Backend (size):
4. ~~`api/wallets/mod.rs` (1,038)~~ — **done (#41):** split crud / reads / sync.

Backend (module boundaries — the *other* axis; 2026-06-03 audit, #44):
Reusable logic accreted inside API *handler* modules and gets reached into cross-concern
(there's no service/domain layer separate from the handler layer). ~14 cross-module
reach-ins total — bounded, not spaghetti, but it's the "mixed-concerns" debt the size-driven
passes never swept. Ranked:
- ~~`hwi::common`~~ — **done (the one real misplacement):** general descriptor/PSBT/policy
  parsing was filed under the *hardware-wallet* module yet imported by `seed_signer`/`sp_send`
  (non-HW signing). Promoted to `api::descriptor_util` (+ `ledger_hmac_store` moved out of
  `hwi/`); only the USB `hwi` module stays. Surfaced by the `hw`-feature work (#19).
- **The other reach-ins were evaluated and kept** — on per-function review they're legitimate
  dependencies / orchestration, not misplacements, and refactoring them would be the premature
  abstraction this doc warns against: `broadcast_transaction` (payjoin legitimately uses the
  broadcast capability), `prices::fetch_price_cached` (tax genuinely needs prices),
  `crud`'s `forget_wallet` calls (a delete *orchestrator* coordinating each subsystem's own
  cleanup, with load-bearing ordering), and `subscriber → background_sync` (a mild infra→api
  wrinkle, but the sync logic is deeply wallet-coupled; a service layer is more ceremony than a
  single-binary pre-1.0 wallet warrants). The hwi case was special because descriptor parsing
  has nothing to do with hardware; these are feature-A-uses-feature-B, which is healthy.

Leave alone (big but cohesive/low-churn): `HelpContent.svelte` (static content),
`descriptor.rs` (cohesive parsing + a large test module).
