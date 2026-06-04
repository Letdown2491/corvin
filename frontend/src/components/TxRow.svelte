<script lang="ts">
  import type { TxRecord } from '../lib/types'
  import { displayUnit, balancesHidden } from '../stores/settings'
  import { labels } from '../stores/labels'

  let {
    tx,
    replacedBy = null,
    onSelect,
  }: {
    tx: TxRecord
    replacedBy?: string | null
    onSelect?: (tx: TxRecord) => void
  } = $props()

  let note = $derived($labels[tx.txid] ?? '')

  function formatAmount(sats: number): string {
    if ($displayUnit === 'btc') return (sats / 1e8).toFixed(8) + ' BTC'
    return sats.toLocaleString() + ' sats'
  }

  function formatDate(ts: string): string {
    return new Date(ts).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
  }

  let isReceived = $derived(tx.amount_sats >= 0)
  // Sign on both sides so direction is readable without color.
  let sign = $derived(isReceived ? '+' : '−')
  let amountClass = $derived(isReceived ? 'received' : 'sent')
  let shortTxid = $derived(tx.txid.slice(0, 8) + '…' + tx.txid.slice(-8))

  let showBadge = $derived(tx.confirmations < 6)
  let badgeClass = $derived(tx.confirmations === 0 ? 'unconfirmed' : 'pending')
  let badgeLabel = $derived(tx.confirmations === 0 ? 'Unconfirmed' : `${tx.confirmations} of 6 confirmations`)
  let badgeTitle = $derived(tx.confirmations === 0
    ? 'This transaction has not been included in a block yet'
    : `${tx.confirmations} of 6 confirmations — transactions are considered final at 6`)

  // Coinbase outputs need 100 confirmations before they're spendable.
  let immature = $derived(tx.is_coinbase && tx.confirmations < 100)
  let matureIn = $derived(immature ? Math.max(0, 100 - tx.confirmations) : 0)
</script>

<button
  class="tx-row"
  class:unconfirmed-row={tx.confirmations === 0}
  onclick={() => onSelect?.(tx)}
  aria-label="{tx.amount_sats >= 0 ? 'Received' : 'Sent'} {Math.abs(tx.amount_sats).toLocaleString()} sats{tx.timestamp ? ', ' + new Date(tx.timestamp).toLocaleDateString() : ''}"
>
  <div class="tx-icon" class:icon-recv={isReceived} class:icon-sent={!isReceived} aria-hidden="true">
    {isReceived ? '↓' : '↑'}
  </div>

  <div class="tx-left">
    <div class="tx-id-row">
      {#if note}
        <span class="tx-note" title={tx.txid}>{note.length > 36 ? note.slice(0, 36) + '…' : note}</span>
      {:else}
        <span class="txid" title={tx.txid}>{shortTxid}</span>
      {/if}
    </div>
    {#if replacedBy}
      <span class="badge replaced" title="Replaced by {replacedBy}">Replaced</span>
    {:else if showBadge}
      <span class="badge {badgeClass}" title={badgeTitle}>{badgeLabel}</span>
    {/if}
    {#if tx.is_coinbase}
      {#if immature}
        <span class="badge coinbase-immature" title="Mining reward — coinbase outputs require 100 confirmations to be spendable">⛏ Matures in {matureIn}</span>
      {:else}
        <span class="badge coinbase" title="Mining reward (coinbase) — fully matured">⛏ Mined</span>
      {/if}
    {/if}
  </div>

  <div class="tx-right">
    {#if $balancesHidden}
      <span class="amount-masked">•••</span>
    {:else}
      <span class="amount {amountClass}">{sign}{formatAmount(Math.abs(tx.amount_sats))}</span>
    {/if}
    <div class="tx-meta">
      {#if tx.timestamp}
        <span class="meta-item">{formatDate(tx.timestamp)}</span>
      {:else if tx.block_height}
        <span class="meta-item">block {tx.block_height.toLocaleString()}</span>
      {/if}
      {#if !isReceived && tx.fee_sats != null}
        <span class="meta-sep">·</span>
        <span class="meta-item">{tx.fee_sats.toLocaleString()} sats fee</span>
      {/if}
    </div>
  </div>
</button>

<style>
  .tx-row {
    display: flex; width: 100%;
    align-items: center;
    justify-content: space-between;
    padding: 10px 0;
    border: none; border-bottom: 1px solid var(--border);
    background: none; cursor: pointer;
    gap: 10px; text-align: left;
  }
  .tx-row:hover { background: var(--surface-hover); margin: 0 -8px; padding: 10px 8px; width: calc(100% + 16px); }
  .unconfirmed-row { opacity: 0.6; }

  .tx-icon {
    width: 26px; height: 26px; border-radius: 50%;
    display: flex; align-items: center; justify-content: center;
    font-size: 0.85rem; font-weight: 700; flex-shrink: 0;
  }
  .icon-recv { background: color-mix(in srgb, #52a875 15%, transparent); color: #52a875; }
  .icon-sent { background: color-mix(in srgb, #e05252 15%, transparent); color: #e05252; }

  .tx-left { display: flex; flex-direction: column; gap: 3px; flex: 1; min-width: 0; }
  .tx-right { display: flex; flex-direction: column; align-items: flex-end; gap: 2px; flex-shrink: 0; }

  .tx-id-row { display: flex; align-items: center; gap: 5px; }
  .txid { font-family: monospace; font-size: 0.82rem; color: var(--text-muted); white-space: nowrap; }
  .tx-note { font-size: 0.84rem; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .badge {
    font-size: 0.68rem; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.05em; padding: 1px 5px; border-radius: 3px;
    display: inline-block; width: fit-content;
  }
  .badge.unconfirmed { background: color-mix(in srgb, #e05252 15%, transparent); color: #e05252; }
  .badge.pending { background: color-mix(in srgb, #e09c52 15%, transparent); color: #e09c52; }
  .badge.replaced { background: color-mix(in srgb, #9b59b6 20%, transparent); color: #c39bd3; }
  .badge.coinbase { background: color-mix(in srgb, #a87fd4 15%, transparent); color: #a87fd4; }
  .badge.coinbase-immature { background: color-mix(in srgb, #e09c52 18%, transparent); color: #e09c52; }

  .amount { font-size: 0.9rem; font-weight: 600; font-variant-numeric: tabular-nums; }
  .amount.received { color: #52a875; }
  .amount.sent { color: #e05252; }
  .amount-masked { font-size: 0.9rem; font-weight: 600; color: var(--text-muted); letter-spacing: 0.15em; }

  .tx-meta { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; justify-content: flex-end; }
  .meta-item { font-size: 0.76rem; color: var(--text-muted); }
  .meta-sep { font-size: 0.76rem; color: var(--text-muted); }
</style>
