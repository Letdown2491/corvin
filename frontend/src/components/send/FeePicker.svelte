<script lang="ts">
  // Visual mempool-blocks fee picker (#36). One row of speed tiles — each tile is
  // both the selector and the visual: a calibrated speed preset (from feeRates)
  // decorated with the matching projected block's congestion color, fullness bar
  // and size. Controlled: only sets `preset` + `customFeeRate`; the parent derives
  // the effective rate.
  import type { FeeRates, MempoolBlock } from '../../lib/types'
  import HelpLink from '../HelpLink.svelte'

  type Preset = 'hour' | 'halfhour' | 'fastest' | 'custom'
  let {
    feeRates,
    mempoolBlocks,
    preset = $bindable(),
    customFeeRate = $bindable(),
    effectiveFeeRate,
  }: {
    feeRates: FeeRates | null
    mempoolBlocks: MempoolBlock[] | null
    preset: Preset
    customFeeRate: number
    effectiveFeeRate: number
  } = $props()

  // Color a fee rate (sat/vB) by congestion tier. Quiet networks read green —
  // correct, there's no backlog. Yellow/orange/red only on a busy mempool.
  function feeColor(rate: number): string {
    if (rate < 2) return '#3fb950'
    if (rate < 10) return '#46a35e'
    if (rate < 30) return '#caa83a'
    if (rate < 60) return '#e0883b'
    if (rate < 120) return '#e0623b'
    return '#e05252'
  }

  // The projected block each speed targets (same mapping the old tiles used):
  // priority = the next block, standard ≈ 3rd, economy ≈ 6th.
  function blockAt(i: number): MempoolBlock | null {
    const bs = mempoolBlocks ?? []
    if (bs.length === 0) return null
    return bs[Math.min(i, bs.length - 1)]
  }

  let tiles = $derived([
    { preset: 'fastest' as Preset, label: 'Priority', eta: 'next block', rate: feeRates?.fastestFee, block: blockAt(0) },
    { preset: 'halfhour' as Preset, label: 'Standard', eta: '~30 min', rate: feeRates?.halfHourFee, block: blockAt(2) },
    { preset: 'hour' as Preset, label: 'Economy', eta: '~1 hr', rate: feeRates?.hourFee, block: blockAt(5) },
  ])
</script>

<section class="config-section">
  <h3 class="section-label">Fee rate <HelpLink anchor="fees" /></h3>

  <div class="fee-tiles" role="group" aria-label="Fee rate">
    {#each tiles as t (t.preset)}
      {@const color = feeColor(t.rate ?? (t.block ? t.block.medianFee : 1))}
      <button
        type="button"
        class="fee-tile"
        class:active={preset === t.preset}
        style="--c: {color}"
        onclick={() => preset = t.preset}
      >
        <span class="ft-label">{t.label}</span>
        <span class="ft-eta">{t.eta}</span>
        <span class="ft-rate">{t.rate ?? '—'} <span class="ft-unit">sat/vB</span></span>
        {#if t.block}
          {@const fill = Math.min(100, t.block.blockVSize / 10_000)}
          <span class="ft-fillbar" title={`${fill.toFixed(0)}% full`}><span class="ft-fill" style="width: {fill}%"></span></span>
          <span class="ft-size">{(t.block.blockVSize / 1e6).toFixed(2)} MB</span>
        {/if}
      </button>
    {/each}
    <button
      type="button"
      class="fee-tile fee-tile-custom"
      class:active={preset === 'custom'}
      onclick={() => preset = 'custom'}
    >
      <span class="ft-label">Custom</span>
      {#if preset === 'custom'}
        <span class="ft-rate">{customFeeRate} <span class="ft-unit">sat/vB</span></span>
      {:else}
        <span class="ft-eta">set your own</span>
      {/if}
    </button>
  </div>

  {#if preset === 'custom'}
    <div class="custom-fee-row">
      <input
        class="custom-fee-input"
        type="number"
        min="1"
        step="1"
        bind:value={customFeeRate}
        aria-label="Custom fee rate"
      />
      <span class="custom-fee-unit">sat/vB</span>
    </div>
  {/if}

  <p class="fee-effective">Selected: <strong>{effectiveFeeRate} sat/vB</strong></p>
</section>

<style>
  .config-section { padding: 16px 20px; border-bottom: 1px solid var(--border); }
  .section-label {
    display: block; margin: 0 0 10px;
    font-size: 0.7rem; font-weight: 700; letter-spacing: 0.06em;
    text-transform: uppercase; color: var(--text-muted);
  }

  .fee-tiles { display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; }
  .fee-tile {
    display: flex; flex-direction: column; align-items: center; gap: 3px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-top: 3px solid var(--c, var(--border));
    border-radius: 6px; padding: 9px 8px 8px; cursor: pointer; color: var(--text-muted);
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }
  .fee-tile:hover { color: var(--text); background: color-mix(in srgb, var(--c) 8%, var(--surface-2)); }
  .fee-tile.active {
    color: var(--text);
    background: color-mix(in srgb, var(--c) 16%, var(--surface-2));
    box-shadow: inset 0 0 0 1px var(--c);
  }
  .ft-label { font-size: 0.84rem; font-weight: 700; color: var(--text); }
  .ft-eta { font-size: 0.68rem; color: var(--text-muted); }
  .ft-rate { font-size: 0.92rem; font-weight: 700; color: var(--text); font-variant-numeric: tabular-nums; }
  .ft-unit { font-size: 0.62rem; font-weight: 400; color: var(--text-muted); }
  .ft-fillbar {
    width: 100%; height: 4px; border-radius: 2px; margin-top: 3px;
    background: color-mix(in srgb, var(--c) 16%, var(--surface-1)); overflow: hidden;
  }
  .ft-fill { display: block; height: 100%; background: var(--c); }
  .ft-size { font-size: 0.62rem; color: var(--text-muted); font-variant-numeric: tabular-nums; }
  .fee-tile-custom { justify-content: center; }

  .custom-fee-row { display: flex; align-items: center; gap: 8px; margin-top: 10px; }
  .custom-fee-input {
    width: 110px; background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 5px; color: var(--text); padding: 7px 10px; font-size: 0.85rem;
    font-variant-numeric: tabular-nums; outline: none;
  }
  .custom-fee-input:focus { border-color: var(--accent); }
  .custom-fee-unit { font-size: 0.8rem; color: var(--text-muted); }

  .fee-effective { margin: 10px 0 0; font-size: 0.75rem; color: var(--text-muted); }
  .fee-effective strong { color: var(--text); font-variant-numeric: tabular-nums; }

  @media (max-width: 520px) {
    .fee-tiles { grid-template-columns: repeat(2, 1fr); }
  }
</style>
