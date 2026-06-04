# Frontend testing (F-8) + amount-parsing hardening (F-5)

Status: **scoped, not started.** From the 2026-05-31 audit (`docs/audit-2026-05-31.md`),
F-8 (no automated frontend tests) and F-5 (JS BTC↔sats float precision) are
intertwined: the safe way to do F-5 is to extract the money math into a pure module
and cover it with tests *first*. This plan does F-8 (stand up a test layer + cover
pure logic) and then F-5 (extract + harden + fix amount parsing) on top of it.

## Current state

- Vite 6 + Svelte 5 + SvelteKit 2; **no test runner** (`package.json` has dev/build/
  preview/check/lint only). `check` = `svelte-check` (types only, no logic exercise).
- The money math is **inline in `SendModal.svelte`** (2,491 LOC): `amountToSats`,
  `parseBip21`, `addressFromScan`, `formatSats`, and several `Math.round(n * 1e8)`
  BTC→sats conversions that close over component state (`localUnit`). Not testable in
  place.
- `src/lib/*.ts` is largely **pure + DOM-free** (verified: search/bip39/import/
  derivation/qr have zero document/window/fetch/localStorage refs) → unit-testable as-is.

## Tooling choice

- **Vitest** — shares the existing `vite.config.ts` (no separate build config),
  first-class Svelte/TS support, fast. Add `vitest` + `@vitest/ui` (optional) +
  `jsdom` (only if/when component tests are added; pure-logic tests need no DOM).
- Add scripts: `"test": "vitest run"`, `"test:watch": "vitest"`.
- Component/E2E (Playwright) is **out of scope for v1** — start with pure-logic units,
  which is where the value/effort ratio is best.

## Phase 1 (F-8) — test layer + cover existing pure logic

Add Vitest, then unit-test the framework-free `lib/` functions, highest-risk first:
- **`search.ts`** — `parseQuery` / `filterTxs` (complex query grammar; easy to regress).
- **`import.ts`** — `parseImportFile` / `parseDescriptor` (wallet import correctness).
- **`derivation.ts`** — `parseOrigins` / `scriptTypeFor` / `parseDerivation`.
- **`bip39.ts`** — `normalizeMnemonic` / `checkWordlist`.
- **`qr.ts`** — `encodeCborByteString` / `decodeCborByteString` (round-trip).
- **`utils.ts`** — `base64ToBytes`/`bytesToBase64` round-trip, `utxoKey`, `kindLabel`.
- **`public-servers.ts`** — trivial sanity (hosts well-formed).

Outcome: a real test suite + CI hook (pairs with F-7 / `docs/reproducible-builds.md`),
and a safety net before touching the send path.

## Phase 2 (F-5) — extract + harden amount parsing

1. **Extract** a new pure `src/lib/amount.ts` with no component-state closure:
   - `btcToSats(s: string): number | null` — parse a BTC decimal string to integer
     sats **without `parseFloat * 1e8`**. Use string handling (split on '.', pad/clamp
     the fractional part to 8 digits, integer-combine) so values like `21.4`,
     `0.1 + 0.2`-style decimals, and 8-dp edge cases are exact. Reject >8 dp, NaN,
     negatives, and amounts over `MAX_MONEY` (21e6 BTC).
   - `satsToBtc(sats: number): string`, `formatSats(sats, unit)`, and a unit-aware
     `amountToSats(input, unit)` that the component calls.
   - Keep `parseBip21` / address-scan parsing here too (currently inline).
2. **Unit-test `amount.ts` hard** — the float-trap cases, rounding, dp limits, bounds,
   sats-vs-btc unit switching, BIP21 with/without amount. This is the coverage that
   makes the change safe.
3. **Rewire `SendModal.svelte`** to call `amount.ts` (delete the inline versions). The
   component keeps only the `localUnit` state and passes it in.

Note: this is **cosmetic-correctness** — the server (BDK) is authoritative on the real
sat amount, so today's worst case is a 1-sat *display* discrepancy, never a wrong
send. The value is consistency (what's shown == what's sent) + a tested money helper,
not a fund-safety fix. That's why it waited for the test net.

## Phase 3 (DONE) — extract + test send-flow address logic

The address/URI helpers landed in their own `src/lib/send.ts` rather than `amount.ts`
(`amount.ts` stayed money-only). Pure + DOM-free, 21 Vitest cases:
- `isSilentPaymentAddress` / `looksLikeAddressShape` / `addressShapeHint` — client-side
  shape hints (the backend is still authoritative).
- `addressFromScan` — extract address + optional amount from a scanned/pasted payload
  (bare address, `bitcoin:` prefix, or full BIP21 URI).
- `uriHasPayjoin`, `looksLikeHrn` (BIP-353), `chunkAddress`, `showAddressEcho`,
  `bip21AmountToField(btc, unit)`.

SendModal imports these and keeps thin wrappers (e.g. `recipientShapeHintAt(i)`), so
behavior is unchanged. A full component-tree split (RecipientRow / coin-control /
verify panels) was scoped but deferred as higher-risk and not browser-testable here.

## Open / to decide

- Whether to also add a couple of **component tests** for the send happy-path later
  (needs jsdom + a mocked `api`), or stay pure-logic-only for now. Lean pure-only v1.
- Where the suite runs in CI (with F-7): `vitest run` + `svelte-check` + `cargo test`
  + `cargo audit` on every change.
