<script lang="ts">
  import type { Balance, BackendStatusEntry } from '../lib/types'
  import { balancesHidden, displayUnit } from '../stores/settings'

  let {
    balance,
    balanceParts,
    fiatText,
    conn,
    serverLabel,
    syncing,
    syncElapsedSec,
    feeDisplay,
  }: {
    balance: Balance | null
    balanceParts: { amount: string; unit: string } | null
    fiatText: string | null
    conn: BackendStatusEntry | undefined
    serverLabel: string
    syncing: boolean
    syncElapsedSec: number
    feeDisplay: string | null
  } = $props()

  function formatSyncTime(ts: string): string {
    const diff = Math.floor((Date.now() - new Date(ts).getTime()) / 60_000)
    if (diff < 1) return 'synced just now'
    if (diff < 60) return `synced ${diff}m ago`
    const h = Math.floor(diff / 60)
    if (h < 24) return `synced ${h}h ago`
    return `synced ${new Date(ts).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}`
  }
</script>

<div class="balance-hero">
  <button
    class="balance-toggle"
    onclick={() => balancesHidden.update(h => !h)}
    aria-pressed={$balancesHidden}
    aria-label={$balancesHidden ? 'Balance hidden — click to show' : 'Balance visible — click to hide'}
  >
    {#if $balancesHidden}
      <span class="balance-amount muted">•••</span>
      <span class="balance-unit muted">{$displayUnit === 'btc' ? 'BTC' : 'sats'}</span>
    {:else if balanceParts}
      <span class="balance-amount">{balanceParts.amount}</span>
      <span class="balance-unit">{balanceParts.unit}</span>
    {:else}
      <span class="balance-amount muted">—</span>
    {/if}
  </button>

  <div class="balance-sub">
    {#if fiatText}
      <span class="balance-fiat">{fiatText}</span>
      <span class="sub-dot">·</span>
    {/if}
    <span
      class="conn-chip"
      title={conn
        ? (conn.connected
            ? `${serverLabel} · connected${conn.tip_height ? ' · Block ' + conn.tip_height.toLocaleString() : ''}`
            : `${serverLabel} · disconnected${conn.error ? ': ' + conn.error : ''}`)
        : `${serverLabel} · connection status unknown`}
    >
      <span class="conn-chip-label">{serverLabel}</span>
      {#if syncing}
        <span class="balance-sync">syncing…{#if syncElapsedSec >= 3} ({syncElapsedSec}s{#if syncElapsedSec >= 30}, first sync of a busy wallet can take a few minutes{/if}){/if}</span>
      {:else if balance?.last_synced}
        <span class="balance-sync">{formatSyncTime(balance.last_synced)}</span>
      {:else if conn && !conn.connected}
        <span class="balance-sync">disconnected</span>
      {/if}
    </span>
    {#if conn?.connected && conn.tip_height}
      <span class="sub-dot">·</span>
      <span class="balance-sync">Block {conn.tip_height.toLocaleString()}</span>
    {/if}
    {#if feeDisplay}
      <span class="sub-dot">·</span>
      <span class="balance-sync">Fees: {feeDisplay}</span>
    {/if}
  </div>

  {#if balance && balance.immature_sats > 0 && !$balancesHidden}
    <div class="balance-immature" title="Coinbase (mining reward) outputs are spendable only after 100 confirmations.">
      <span class="balance-immature-icon" aria-hidden="true">⛏</span>
      <span class="balance-immature-label">Immature mining rewards:</span>
      <span class="balance-immature-amount">
        {$displayUnit === 'btc'
          ? `${(balance.immature_sats / 1e8).toFixed(8)} BTC`
          : `${balance.immature_sats.toLocaleString()} sats`}
      </span>
      <span class="balance-immature-hint">not yet spendable</span>
    </div>
  {/if}
</div>

<style>
  .balance-hero { margin-bottom: 24px; }
  .balance-toggle {
    background: none; border: none; padding: 0; cursor: pointer;
    display: flex; align-items: baseline; gap: 8px;
  }
  .balance-toggle:hover .balance-amount,
  .balance-toggle:hover .balance-unit { opacity: 0.65; }
  .balance-amount {
    font-size: 2.4rem; font-weight: 800; letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums; color: var(--text);
    transition: opacity 0.15s;
  }
  .balance-amount.muted { color: var(--text-muted); letter-spacing: 0.05em; }
  .balance-unit {
    font-size: 1rem; font-weight: 500; color: var(--text-muted);
    transition: opacity 0.15s; text-transform: uppercase; letter-spacing: 0.06em;
  }
  .balance-unit.muted { color: var(--text-muted); }

  .balance-sub {
    margin-top: 4px; display: flex; align-items: center; gap: 10px;
    min-height: 18px;
  }
  .balance-immature {
    margin-top: 6px;
    display: inline-flex; align-items: center; gap: 6px;
    font-size: 0.78rem; color: var(--text-muted);
    background: color-mix(in srgb, #e09c52 12%, transparent);
    border: 1px solid color-mix(in srgb, #e09c52 30%, var(--border));
    border-radius: 4px; padding: 3px 8px;
    width: max-content;
  }
  .balance-immature-icon { color: #e09c52; font-size: 0.85rem; }
  .balance-immature-label { color: var(--text); font-weight: 500; }
  .balance-immature-amount { color: var(--text); font-variant-numeric: tabular-nums; }
  .balance-immature-hint { color: var(--text-muted); font-style: italic; font-size: 0.72rem; }
  .balance-sync { font-size: 0.75rem; color: var(--text-muted); }
  .balance-fiat { font-size: 0.82rem; color: var(--text-muted); font-variant-numeric: tabular-nums; }
  .sub-dot { font-size: 0.75rem; color: var(--text-muted); }
  .conn-chip {
    display: inline-flex; align-items: center; gap: 5px;
    font-size: 0.75rem; color: var(--text-muted); cursor: default;
  }
  .conn-chip-label { white-space: nowrap; }

  @media (max-width: 768px) {
    .balance-amount { font-size: 2rem; }
    /* Let the fee-rate / sync / price row wrap instead of overflow */
    .balance-sub { flex-wrap: wrap; row-gap: 4px; }
  }
</style>
