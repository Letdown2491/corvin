# Architecture + refactoring conventions

Guidance for keeping the codebase maintainable as it grows. Written 2026-06-01 after a
discovery pass on file sizes and duplication. Not a rewrite mandate, but a set of rules
for *new* code and a prioritized list of the genuine debt.

## The principle

**Size alone is not debt. Mixed concerns times churn is.** A long file that is cohesive
and rarely changes (static help content, a self-contained parser with its tests) is fine.
A long file that mixes unrelated concerns *and* changes often is where bugs hide and
merges hurt. Target the second kind; leave the first alone.

## Conventions for new code

- **Folder-per-feature for big components.** When a component grows past about one screen
  of distinct concerns, split it into a folder: `send/` (RecipientList, CoinControl,
  FeeSelector, SignPanel, VerifyPanel, and so on) with a thin page/modal that composes
  them.
- **Pure logic lives in `lib/` and is unit-tested.** The proven pattern (`lib/amount.ts`,
  `lib/send.ts`): pull framework-free parsing/formatting/validation out of components into
  `lib/*.ts` with colocated `*.test.ts`. It's safe to refactor (type-checked + Vitest) and
  shrinks the component. Do this *before* splitting the component itself.
- **Shared UI primitives live in `components/ui/`.** Don't hand-roll a modal shell, busy
  button, copy button, QR, or amount input per component; compose the primitive (see the
  catalog below). New components should reach for these first.
- **Backend: one module is one concern.** Split handler files by concern (crud / reads /
  sync), keeping public handler signatures stable so the router and callers don't churn.
  Mechanical moves, low risk.
- **Refactor money code safest-first:** pure-logic extraction (tested), then primitives,
  then component decomposition (type-check plus manual test, never big-bang). There are no
  Svelte component tests, only pure `lib` tests, so logic extraction is the safety net.

## Shared UI primitives (`components/ui/`)

Catalog from the 2026-06-01 discovery pass (counts are files/occurrences touched). Already
shared (don't re-invent): toasts (`stores/toasts` + `ToastContainer`), `EmptyState`,
`HelpLink`.

### Tier 1: built
- **`Modal.svelte`:** a shell over the native `<dialog>` with `open` (bindable), `title`,
  `desc`, and body + `footer` snippets; it centralizes show/close, Esc, backdrop-click,
  focus, the close button, and the header/desc layout. Replaces scaffolding that was
  duplicated across 16 files (and fixes the Esc inconsistency, since only 6 of 16 had it).
- **`BusyButton.svelte`:** owns a `busy` flag around an async `onclick` (try/finally),
  shows `idle`/`busyLabel`, and disables while busy. Replaces the hand-rolled
  `busy = $state(false)` plus ternary-label pattern in 15 files / 32 buttons.
- **`CopyButton.svelte`:** copy text plus a "Copied ✓" flash plus a failure toast. `text`
  is a string or getter; optional `disabled`. Migrated: MessagesModal, FeeBumpModal,
  ConsolidateModal, WalletDetailsModal (x3), ReceiveModal (x4). Justified exceptions left
  as hand-rolled: AddressTable/UtxoTable per-row icon buttons (one component instance per
  row across many rows, and they want a `.copied` flash class the primitive doesn't
  expose); ServerPage "Copy debug info" (text is built async, so a getter is sync);
  SeedGeneratorPanel (copy-failure shows an inline "write the words down" message, not a
  toast, deliberately for the seed flow).

### Tier 2
- **`AmountInput.svelte`:** built (Send v2). A Bitcoin-amount input bound to a string
  `value` in a given `unit`; controlled by default (the parent owns the BTC/sats toggle,
  e.g. SendFlow's one shared toggle across recipient rows), with an opt-in `showToggle`
  standalone mode that owns and converts the unit via `amount.ts`. Used by SendFlow
  recipients (controlled) and ReceiveModal's amount (standalone toggle). Fee-rate inputs
  (FeeBump, FeePicker) are sat/vB, not BTC amounts, so AmountInput doesn't apply there.
- **`QrDisplay.svelte`:** planned. Render a string/PSBT as a QR (static and animated/BBQr).
  7 files duplicate QR rendering today (the scanner is already shared).
- **`Field.svelte`** (plus small form helpers): planned. A label/input/hint/error wrapper,
  composed not monolithic. About 109 `<label>` across the app.
- **`TxFlowDiagram.svelte`:** built (#31, in Send v2's review step). Inputs to outputs as
  one stacked proportional bar plus an amount legend; values and formatter injected. Still
  to drop into the transaction-detail view.
- **`send/FeePicker.svelte`:** built (#36). A visual mempool-blocks fee picker: one row of
  speed tiles (Priority/Standard/Economy/Custom), each colored by congestion and showing
  the matching projected block's fullness/size. Controlled (binds preset + customFeeRate).

### Tier 3: defer / build with the feature
- **`AddressDisplay.svelte`:** chunked echo plus address fingerprint. Build with the
  flow-diagram / fingerprint work (#31 / #3), not before.

### Not primitives (deliberately)
- Toasts and EmptyState are already shared.
- The data-load generation-guard is single-use (WalletDetail only); keep it local.
- Native `confirm()` is only 5 calls; not worth abstracting.

## Migration approach

Build a primitive, migrate one or two safe call sites to validate, then roll out
incrementally, never all 16 modals at once.

**Scoped-CSS constraint (learned 2026-06-01).** A parent's component-scoped style (e.g.
`.copy-btn` in MessagesModal) does not apply to a child component's internal element. So
`BusyButton`/`CopyButton` can only match a button's existing look when that look comes from
a **global** class (`btn-primary/secondary/danger/ghost`, defined in
`routes/+layout.svelte`). They take an optional `class` prop, but it must be a global class.
Consequences:
- Buttons already using `btn-*` are clean, visually-identical drop-ins (e.g. the
  Backend-page "Test connection" button becomes `BusyButton variant="secondary"`).
- Buttons with bespoke per-component classes (`.copy-btn`, `.copy-psbt-btn`) can't adopt
  the primitive without first making that style reach the child. Two ways: (a) promote it
  to a global class in `routes/+layout.svelte` (e.g. `.btn-copy`), or (b) keep the rule in
  the owning component but wrap the selector in `:global(...)` and give it a namespaced name
  to avoid collisions (`.rcv-copy-btn`, `.wd-copy-link`). Use (b) for one-modal styling, (a)
  when several components share it. Either way, don't silently swap and lose the styling.
  (There's no browser test here, so visual regressions are invisible.)

## Prioritized refactor targets (the genuine debt)

Frontend (mixed-concern plus high-churn):
1. `SendModal.svelte` (was 2,498): Send v2 (#32) rebuilt it as `send/SendFlow.svelte`, a
   two-step modal (Compose, then Review & Sign). Extracted `ui/AmountInput`,
   `send/FeePicker` (#36), and `TxFlowDiagram` (#31), and merged the numeric preview and
   flow into one Transaction panel. SendFlow is a modal, not a full-page route: its content
   is modal-sized, and a route would be the app's only navigate-away action. It keeps a
   bespoke `<dialog>` shell mirroring `ui/Modal` (its phase-driven footer, Reset chrome, and
   edge-to-edge sectioned body don't fit the shared panel), a deliberate, documented
   exception to the all-modals-use-`ui/Modal` rule. Further decomposed:
   `send/RecipientRow.svelte` and `send/CoinControlPanel.svelte`; SendFlow is now about
   2,050 LOC. Remaining: refit the Payjoin panel into the two-step (currently parked in
   Compose).
2. `add-wallet/+page.svelte` (was 1,886): done (#39), now 983 LOC. Self-contained per-kind
   panels (`VaultCreatePanel`, `SpCreatePanel`, `MultisigCreatePanel`), each owning its
   sub-state, backend pin, create call, and actions, emitting `onCreated`; plus leaf
   extractions `KindPicker`, `MsSignerCard`, and pure `lib/import` helpers. What stays in the
   page is the single-sig kind (seed/paste/file/hardware/descriptor) and the watch-only
   address kind, which share `input`/`method`/the BitBox HW flow and feed one
   `submit()`/`canSubmit`, so they're the cohesive core, not debt.
3. `WalletDetail.svelte` (was 1,199): done (#40), now 874 LOC. Extracted `BalanceHero`,
   `WalletMenu`, `TxSearchBar` (plus the earlier `WalletToolsTab`).

Backend (size):
4. `api/wallets/mod.rs` (was 1,038): done (#41), split into crud / reads / sync.

Backend (module boundaries, the other axis; 2026-06-03 review, #44): reusable logic
accreted inside API handler modules and gets reached into cross-concern (there's no
service/domain layer separate from the handler layer). About 14 cross-module reach-ins,
bounded rather than spaghetti, but it's the mixed-concerns debt the size-driven passes
never swept.
- `hwi::common`: done (the one real misplacement). General descriptor/PSBT/policy parsing
  was filed under the hardware-wallet module yet imported by `seed_signer`/`sp_send`
  (non-HW signing). Promoted to `api::descriptor_util` (and `ledger_hmac_store` moved out of
  `hwi/`); only the USB `hwi` module stays.
- The other reach-ins were evaluated and kept: on per-function review they're legitimate
  dependencies / orchestration, not misplacements, and refactoring them would be the
  premature abstraction this doc warns against. `broadcast_transaction` (payjoin
  legitimately uses the broadcast capability), `prices::fetch_price_cached` (tax genuinely
  needs prices), `crud`'s `forget_wallet` calls (a delete orchestrator coordinating each
  subsystem's own cleanup, with load-bearing ordering), and `subscriber` calling
  `background_sync` (a mild infra-to-api wrinkle, but the sync logic is deeply
  wallet-coupled). These are feature-A-uses-feature-B, which is healthy.

Leave alone (big but cohesive/low-churn): `HelpContent.svelte` (static content) and
`descriptor.rs` (cohesive parsing plus a large test module).

## Frontend testing

The frontend has a Vitest suite over the pure, framework-free logic in `src/lib/*.ts`
(scripts: `npm test` and `npm run test:watch`; `npm run check` is svelte-check, types
only). Component and end-to-end tests are out of scope: the value-to-effort ratio is best
on pure-logic units, and the money paths are authoritative on the server. Covered modules
include `search`, `import`, `bip39`, `qr`, `utils`, the BTC/sats money math in `amount.ts`
(string-based parsing rather than `parseFloat * 1e8`, so 8-decimal edge cases are exact),
and the send-flow address/URI helpers in `send.ts`. CI runs `vitest run` and `svelte-check`
alongside the Rust checks.
