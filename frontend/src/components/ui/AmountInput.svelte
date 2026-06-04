<script lang="ts">
  // A bitcoin-amount text input bound to a string `value` in the given `unit`.
  // Controlled by default (the parent owns the BTC/sats toggle — e.g. SendFlow's
  // one shared toggle across recipient rows). Pass `showToggle` for standalone
  // callers (FeeBump custom rate, Receive) and the component owns + converts the
  // unit itself, using the tested amount.ts helpers (no float drift).
  import { amountToSats, satsToBtcString } from '../../lib/amount'

  let {
    value = $bindable(),
    unit = $bindable('sats'),
    showToggle = false,
    disabled = false,
    placeholder,
    ariaLabel,
    id,
    onfocus,
  }: {
    value: string
    unit?: 'btc' | 'sats'
    showToggle?: boolean
    disabled?: boolean
    placeholder?: string
    ariaLabel?: string
    id?: string
    onfocus?: () => void
  } = $props()

  let ph = $derived(placeholder ?? (unit === 'btc' ? '0.001' : '10000'))

  function setUnit(u: 'btc' | 'sats') {
    if (u === unit) return
    // Translate the typed amount through sats so the number keeps its meaning.
    const sats = amountToSats(value, unit)
    if (sats > 0) value = u === 'btc' ? satsToBtcString(sats) : sats.toString()
    unit = u
  }
</script>

{#if showToggle}
  <div class="amount-input-wrap">
    <input
      class="amount-input"
      type="text"
      inputmode="decimal"
      placeholder={ph}
      bind:value
      {disabled}
      {id}
      onfocus={() => onfocus?.()}
      aria-label={ariaLabel}
    />
    <div class="unit-toggle" role="group" aria-label="Unit">
      <button type="button" class:active={unit === 'sats'} onclick={() => setUnit('sats')}>sats</button>
      <button type="button" class:active={unit === 'btc'} onclick={() => setUnit('btc')}>BTC</button>
    </div>
  </div>
{:else}
  <input
    class="amount-input"
    type="text"
    inputmode="decimal"
    placeholder={ph}
    bind:value
    {disabled}
    onfocus={() => onfocus?.()}
    aria-label={ariaLabel}
  />
{/if}

<style>
  .amount-input {
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 4px; color: var(--text); padding: 7px 10px;
    font-size: 0.88rem; font-variant-numeric: tabular-nums;
    width: 160px; outline: none;
  }
  .amount-input:focus { border-color: var(--accent); }
  .amount-input:disabled { opacity: 0.5; }
  .amount-input-wrap { display: flex; align-items: center; gap: 8px; }
  .unit-toggle {
    display: flex; border: 1px solid var(--border); border-radius: 4px;
    overflow: hidden; flex-shrink: 0;
  }
  .unit-toggle button {
    background: var(--surface-2); border: none; color: var(--text-muted);
    padding: 5px 8px; cursor: pointer; font-size: 0.75rem; font-weight: 600;
  }
  .unit-toggle button + button { border-left: 1px solid var(--border); }
  .unit-toggle button.active { background: var(--accent); color: #000; }
  .unit-toggle button:hover:not(.active) { color: var(--text); }

  @media (max-width: 768px) {
    .amount-input { width: 120px; }
  }
</style>
