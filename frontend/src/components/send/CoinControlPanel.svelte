<script lang="ts">
  // Manual UTXO selection for the send flow. The parent owns `enabled` + `selected`
  // (the build effect reads them), so they're bindable; this panel renders the list
  // and the per-row / select-all toggles. Frozen + immature handling lives here.
  import type { UtxoRecord } from '../../lib/types'
  import { utxoKey } from '../../lib/utils'
  import { frozenUtxos } from '../../stores/utxo_freeze'
  import { utxoLabels } from '../../stores/utxo_labels'
  import { addToast } from '../../stores/toasts'
  import { utxoCategories, addressCategories } from '../../stores/categories'
  import HelpLink from '../HelpLink.svelte'
  import CategoryChip from '../CategoryChip.svelte'

  let {
    utxos,
    enabled = $bindable(),
    selected = $bindable(),
    addressLabels = {},
    format,
  }: {
    utxos: UtxoRecord[]
    enabled: boolean
    selected: Set<string>
    addressLabels?: Record<string, string>
    format: (sats: number) => string
  } = $props()

  function hasFrozen(outpoint: string): boolean { return $frozenUtxos.has(outpoint) }
  let frozenCount = $derived(utxos.filter(u => $frozenUtxos.has(utxoKey(u.txid, u.vout))).length)
  let selectedSats = $derived(
    utxos.filter(u => selected.has(utxoKey(u.txid, u.vout))).reduce((s, u) => s + u.amount_sats, 0)
  )

  // "All" only refers to non-frozen, mature UTXOs. Frozen entries can still be
  // added individually via per-row toggles; the bulk select never sweeps them in.
  // Immature coinbase outputs can't be selected at all — the backend rejects them.
  let nonFrozenOutpoints = $derived(
    utxos
      .filter(u => !$frozenUtxos.has(utxoKey(u.txid, u.vout)) && u.is_mature)
      .map(u => utxoKey(u.txid, u.vout))
  )
  let allSelected = $derived(
    nonFrozenOutpoints.length > 0 && nonFrozenOutpoints.every(op => selected.has(op))
  )

  function toggleAll() {
    selected = allSelected ? new Set() : new Set(nonFrozenOutpoints)
  }
  function toggleOne(outpoint: string) {
    const u = utxos.find(x => utxoKey(x.txid, x.vout) === outpoint)
    if (u && !u.is_mature) {
      addToast('Immature coinbase outputs can\'t be spent until they reach 100 confirmations.')
      return
    }
    const next = new Set(selected)
    next.has(outpoint) ? next.delete(outpoint) : next.add(outpoint)
    selected = next
  }
</script>

<section class="config-section">
  <div class="coin-control-header">
    <h3 class="section-label" style="margin: 0">Coin control <HelpLink anchor="coin-control" /></h3>
    <label class="cc-toggle">
      <input type="checkbox" bind:checked={enabled} />
      <span>Manual UTXO selection</span>
    </label>
  </div>

  {#if enabled}
    {#if utxos.length === 0}
      <p class="cc-empty">No UTXOs — sync your wallet first.</p>
    {:else}
      <div class="cc-utxo-list">
        <div class="cc-list-header">
          <label class="cc-check-label">
            <input type="checkbox" checked={allSelected} onchange={toggleAll} aria-label="Select all" />
            <span>All</span>
          </label>
          {#if selected.size > 0}
            <span class="cc-selection-info">{selected.size} selected · {format(selectedSats)}</span>
          {/if}
        </div>
        {#each utxos as utxo (utxo.txid + ':' + utxo.vout)}
          {@const outpoint = utxoKey(utxo.txid, utxo.vout)}
          {@const frozen = hasFrozen(outpoint)}
          {@const isSelected = selected.has(outpoint)}
          {@const label = $utxoLabels[outpoint] ?? addressLabels[utxo.address ?? ''] ?? ''}
          {@const age = utxo.confirmations === 0 ? 'unconfirmed' : utxo.confirmations < 6 ? `${utxo.confirmations} conf` : utxo.block_height ? `#${utxo.block_height.toLocaleString()}` : null}
          {@const immature = utxo.is_coinbase && !utxo.is_mature}
          {@const matureIn = immature ? Math.max(0, 100 - utxo.confirmations) : 0}
          {@const catId = $utxoCategories[outpoint] ?? (utxo.address ? $addressCategories[utxo.address] : null) ?? null}
          <label
            class="cc-utxo-row"
            class:is-selected={isSelected}
            class:is-frozen={frozen}
            class:is-immature={immature}
          >
            <input
              type="checkbox"
              checked={isSelected}
              disabled={immature}
              onchange={() => toggleOne(outpoint)}
              aria-label={`Select UTXO of ${format(utxo.amount_sats)}${utxo.address ? ' at ' + utxo.address : ''}${frozen ? ' (frozen)' : ''}${immature ? ' (immature coinbase)' : ''}`}
              title={immature ? `Immature coinbase — matures in ${matureIn} more block${matureIn === 1 ? '' : 's'}` : undefined}
            />
            <div class="cc-utxo-info">
              <span class="cc-addr">{utxo.address ?? '—'}</span>
              <div class="cc-meta">
                {#if immature}
                  <span class="cc-immature-badge" title={`Mining reward — needs ${matureIn} more confirmation${matureIn === 1 ? '' : 's'} to be spendable`}>⛏ matures in {matureIn}b</span>
                {:else if utxo.is_coinbase}
                  <span class="cc-coinbase-badge" title="Mining reward (fully matured)">⛏ mined</span>
                {/if}
                {#if frozen}<span class="cc-frozen-badge">❄ frozen</span>{/if}
                {#if catId}<CategoryChip categoryId={catId} />{/if}
                {#if label}<span class="cc-label">{label}</span>{/if}
                {#if age}<span class="cc-age">{age}</span>{/if}
              </div>
            </div>
            <span class="cc-amount">{format(utxo.amount_sats)}</span>
          </label>
        {/each}
      </div>
    {/if}
  {:else if frozenCount > 0}
    <p class="cc-frozen-hint">
      {frozenCount} frozen UTXO{frozenCount !== 1 ? 's' : ''} will be excluded automatically.
      Enable coin control to override.
    </p>
  {/if}
</section>

<style>
  .config-section { padding: 16px 20px; border-bottom: 1px solid var(--border); }
  .section-label { font-size: 0.7rem; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; color: var(--text-muted); }

  .coin-control-header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 10px; }
  .cc-toggle { display: flex; align-items: center; gap: 6px; cursor: pointer; font-size: 0.8rem; color: var(--text-muted); }
  .cc-toggle input { accent-color: var(--accent); cursor: pointer; }
  .cc-empty { margin: 8px 0 0; font-size: 0.78rem; color: var(--text-muted); }
  .cc-frozen-hint { margin: 0; font-size: 0.75rem; color: var(--text-muted); }

  .cc-utxo-list {
    margin-top: 2px;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
    max-height: 200px;
    overflow-y: auto;
  }
  .cc-list-header {
    display: flex; align-items: center; gap: 12px;
    padding: 7px 10px;
    background: var(--surface-2); border-bottom: 1px solid var(--border);
    font-size: 0.75rem; color: var(--text-muted);
    position: sticky; top: 0;
  }
  .cc-check-label { display: flex; align-items: center; gap: 6px; cursor: pointer; }
  .cc-check-label input { accent-color: var(--accent); cursor: pointer; }
  .cc-selection-info { font-size: 0.72rem; color: var(--accent); margin-left: auto; }

  .cc-utxo-row {
    display: flex; align-items: center; gap: 8px;
    padding: 7px 10px;
    border-bottom: 1px solid var(--border);
    cursor: pointer; transition: background 0.1s;
    border-left: 2px solid transparent;
  }
  .cc-utxo-row:last-child { border-bottom: none; }
  .cc-utxo-row:hover { background: var(--surface-hover); }
  .cc-utxo-row.is-selected {
    border-left-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 5%, transparent);
  }
  .cc-utxo-row.is-frozen { opacity: 0.7; }
  .cc-utxo-row.is-immature {
    opacity: 0.65;
    border-left-color: #e09c52;
    background: color-mix(in srgb, #e09c52 4%, transparent);
  }
  .cc-utxo-row.is-immature input[type="checkbox"] { cursor: not-allowed; }
  .cc-immature-badge {
    font-size: 0.65rem; font-weight: 700;
    color: #e09c52;
    background: color-mix(in srgb, #e09c52 14%, transparent);
    padding: 1px 5px; border-radius: 3px;
  }
  .cc-coinbase-badge {
    font-size: 0.65rem; font-weight: 700;
    color: #a87fd4;
    background: color-mix(in srgb, #a87fd4 12%, transparent);
    padding: 1px 5px; border-radius: 3px;
  }
  .cc-utxo-row input { accent-color: var(--accent); flex-shrink: 0; cursor: pointer; }
  .cc-utxo-info { display: flex; flex-direction: column; gap: 2px; flex: 1; min-width: 0; }
  .cc-addr { font-family: monospace; font-size: 0.75rem; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cc-meta { display: flex; align-items: center; gap: 5px; flex-wrap: wrap; }
  .cc-frozen-badge { font-size: 0.65rem; color: #52a8d4; }
  .cc-label { font-size: 0.68rem; color: var(--accent); background: color-mix(in srgb, var(--accent) 12%, transparent); padding: 1px 5px; border-radius: 3px; }
  .cc-age { font-size: 0.68rem; color: var(--text-muted); }
  .cc-amount { font-size: 0.78rem; font-weight: 600; color: var(--text); font-variant-numeric: tabular-nums; flex-shrink: 0; }
</style>
