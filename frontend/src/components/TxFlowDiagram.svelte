<script lang="ts">
  // Inputs→outputs flow as one stacked proportional bar + an amount legend (#31).
  // The bar segments the total input into its outputs (recipient / change / fee),
  // so where the money goes reads at a glance — including the common "almost all
  // of this is change coming back" case. Values + formatter are injected, so it's
  // reusable in the send preview and the transaction-detail view.
  type FlowOutput = { label: string; sats: number; kind: 'recipient' | 'change' | 'fee' | 'received' }

  let {
    inputSats,
    inputCount = null,
    outputs,
    format = (s: number) => `${s.toLocaleString()} sats`,
  }: {
    inputSats: number
    inputCount?: number | null
    outputs: FlowOutput[]
    format?: (sats: number) => string
  } = $props()

  const KIND_COLOR: Record<FlowOutput['kind'], string> = {
    recipient: 'var(--accent)',
    change: '#5b9bd5',
    fee: '#d9684a',
    received: '#52a875',
  }
</script>

<div class="flow">
  <div class="flow-head">
    <span class="flow-head-label">Input{inputCount === 1 ? '' : 's'}</span>
    {#if inputCount != null}<span class="flow-head-count">{inputCount} UTXO{inputCount === 1 ? '' : 's'}</span>{/if}
    <span class="flow-head-amount">{format(inputSats)}</span>
  </div>

  <div class="flow-bar" role="img" aria-label="Transaction output breakdown">
    {#each outputs as o, i (i)}
      <span
        class="flow-seg"
        style="flex-grow: {Math.max(o.sats, 1)}; background: {KIND_COLOR[o.kind]}"
        title={`${o.label}: ${format(o.sats)}`}
      ></span>
    {/each}
  </div>

  <div class="flow-legend">
    {#each outputs as o, i (i)}
      <div class="flow-leg-row">
        <span class="flow-dot" style="background: {KIND_COLOR[o.kind]}"></span>
        <span class="flow-leg-label">{o.label}</span>
        <span class="flow-leg-amount">{format(o.sats)}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .flow { display: flex; flex-direction: column; gap: 8px; }

  .flow-head { display: flex; align-items: baseline; gap: 8px; }
  .flow-head-label { font-size: 0.7rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-muted); }
  .flow-head-count { font-size: 0.68rem; color: var(--text-muted); }
  .flow-head-amount { margin-left: auto; font-size: 0.92rem; font-weight: 700; color: var(--text); font-variant-numeric: tabular-nums; }

  /* One bar; segments grow proportionally to their sats. min-width keeps a tiny
     recipient/fee visible as a sliver against a large change output. */
  .flow-bar {
    display: flex; width: 100%; height: 11px; border-radius: 4px; overflow: hidden;
    background: var(--surface-2);
  }
  .flow-seg { min-width: 3px; }
  .flow-seg + .flow-seg { border-left: 1px solid var(--surface-1); }

  .flow-legend { display: flex; flex-direction: column; gap: 6px; }
  .flow-leg-row { display: flex; align-items: center; gap: 7px; }
  .flow-dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; }
  .flow-leg-label { font-size: 0.78rem; color: var(--text); }
  .flow-leg-amount { margin-left: auto; font-size: 0.78rem; color: var(--text); font-variant-numeric: tabular-nums; }
</style>
