<script lang="ts">
  import { untrack } from 'svelte'
  import { goto } from '$app/navigation'
  import { page } from '$app/stores'
  import { wallets } from '../../../../stores/wallets'
  import { api } from '../../../../lib/api'
  import { downloadBlob } from '../../../../lib/utils'
  import type { TaxRecord } from '../../../../lib/types'

  let walletId = $derived($page.params.id ?? '')
  let selectedWalletId = $state<string>(untrack(() => walletId))
  $effect(() => { if (walletId) selectedWalletId = walletId })
  let selectedWallet = $derived($wallets.find(w => w.id === selectedWalletId) ?? null)

  type Method = 'hifo' | 'fifo' | 'lifo'
  type SortCol = 'date' | 'type' | 'btc' | 'usd_price' | 'usd_value' | 'cost_basis' | 'gain_loss'
  type SortDir = 'asc' | 'desc'

  const currentYear = new Date().getFullYear()
  let year = $state(currentYear)
  let method = $state<Method>('hifo')
  let records = $state<TaxRecord[]>([])
  let loading = $state(false)
  let error = $state('')

  let sortCol = $state<SortCol>('date')
  let sortDir = $state<SortDir>('asc')

  let reqVersion = 0
  $effect(() => {
    if (selectedWallet) {
      const v = ++reqVersion
      fetchReport(selectedWallet.id, year, method, v)
    }
  })

  async function fetchReport(id: string, y: number, m: Method, ver: number) {
    loading = true; error = ''
    try {
      const result = await api.wallets.taxReport(id, y, m)
      if (ver !== reqVersion) return
      records = result
    } catch (e) {
      if (ver !== reqVersion) return
      error = e instanceof Error ? e.message : 'Failed to load'
    } finally {
      if (ver === reqVersion) loading = false
    }
  }

  let sortedRecords = $derived.by(() => {
    const m = sortDir === 'asc' ? 1 : -1
    return [...records].sort((a, b) => {
      switch (sortCol) {
        case 'date':       return a.date.localeCompare(b.date) * m
        case 'type':       return a.type.localeCompare(b.type) * m
        case 'btc':        return (a.btc - b.btc) * m
        case 'usd_price':  return ((a.usd_price ?? 0) - (b.usd_price ?? 0)) * m
        case 'usd_value':  return ((a.usd_value ?? 0) - (b.usd_value ?? 0)) * m
        case 'cost_basis': return ((a.cost_basis ?? 0) - (b.cost_basis ?? 0)) * m
        case 'gain_loss':  return ((a.gain_loss ?? 0) - (b.gain_loss ?? 0)) * m
      }
    })
  })

  function toggleSort(col: SortCol) {
    if (sortCol === col) {
      sortDir = sortDir === 'asc' ? 'desc' : 'asc'
    } else {
      sortCol = col
      // Numerical columns default to desc.
      sortDir = (col === 'date' || col === 'type') ? 'asc' : 'desc'
    }
  }

  function sortArrow(col: SortCol): string {
    if (sortCol !== col) return '↕'
    return sortDir === 'asc' ? '↑' : '↓'
  }

  function downloadCsv() {
    const headers = ['Date', 'Type', 'BTC Amount', 'USD Price', 'USD Value', 'Cost Basis', 'Gain/Loss', 'TXID']
    const rows = sortedRecords.map(r => [
      r.date,
      r.type,
      r.btc.toFixed(8),
      r.usd_price?.toFixed(2) ?? '',
      r.usd_value?.toFixed(2) ?? '',
      r.cost_basis?.toFixed(2) ?? '',
      r.gain_loss?.toFixed(2) ?? '',
      r.txid,
    ])
    const csv = [headers, ...rows].map(row => row.map(cell => `"${cell}"`).join(',')).join('\n')
    const blob = new Blob([csv], { type: 'text/csv' })
    const name = (selectedWallet?.label ?? 'wallet').replace(/\s+/g, '-')
    downloadBlob(blob, `${name}-tax-${year}-${method.toUpperCase()}.csv`)
  }

  function fmtUsd(n: number | null | undefined): string {
    if (n == null) return '–'
    return '$' + n.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })
  }

  function fmtGain(n: number | null | undefined): string {
    if (n == null) return '–'
    const prefix = n > 0 ? '+$' : '$'
    return prefix + Math.abs(n).toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })
  }

  let totalGain = $derived(records.reduce((sum, r) => sum + (r.gain_loss ?? 0), 0))
  let hasGainData = $derived(records.some(r => r.gain_loss != null))

  let realizedGain = $derived(
    records.reduce((s, r) => s + Math.max(r.gain_loss ?? 0, 0), 0)
  )
  let realizedLoss = $derived(
    records.reduce((s, r) => s + Math.min(r.gain_loss ?? 0, 0), 0)
  )

  function goBack() {
    if (window.history.length > 1) window.history.back()
    else goto(`/wallet/${walletId}`)
  }
</script>

<svelte:head>
  <title>Tax Report — {selectedWallet?.label ?? 'Wallet'}</title>
</svelte:head>

<div class="page">
  <button class="back-btn" onclick={goBack} aria-label="Back to wallet">← Back</button>

  <div class="tax-header">
    <h1 class="tax-title">Tax Report <span class="tax-wallet">· {selectedWallet?.label ?? 'Wallet'}</span></h1>
    <button
      class="download-btn"
      onclick={downloadCsv}
      disabled={loading || records.length === 0}
    >Download CSV</button>
  </div>

  <div class="controls">
    {#if $wallets.length > 1}
      <label class="ctrl-label">
        Wallet
        <select bind:value={selectedWalletId} class="ctrl-select">
          {#each $wallets as w (w.id)}
            <option value={w.id}>{w.label}</option>
          {/each}
        </select>
      </label>
    {/if}
    <label class="ctrl-label">
      Year
      <select bind:value={year} class="ctrl-select">
        {#each Array.from({ length: Math.max(6, currentYear - 2008) }, (_, i) => currentYear - i) as y (y)}
          <option value={y}>{y}</option>
        {/each}
      </select>
    </label>
    <label class="ctrl-label">
      Method
      <select bind:value={method} class="ctrl-select">
        <option value="hifo">HIFO — highest cost first</option>
        <option value="fifo">FIFO — oldest first</option>
        <option value="lifo">LIFO — newest first</option>
      </select>
    </label>
  </div>

  {#if hasGainData && !loading && !error && records.length > 0}
    <div class="summary-grid">
      <div class="summary-card">
        <span class="summary-label">Realized gains</span>
        <span class="summary-value gain">{fmtGain(realizedGain)}</span>
      </div>
      <div class="summary-card">
        <span class="summary-label">Realized losses</span>
        <span class="summary-value loss">{fmtGain(realizedLoss)}</span>
      </div>
      <div class="summary-card summary-net">
        <span class="summary-label">Net gain / loss {year}</span>
        <span class="summary-value" class:gain={totalGain > 0} class:loss={totalGain < 0} class:neutral={totalGain === 0}>
          {fmtGain(totalGain)}
        </span>
      </div>
    </div>
  {/if}

  {#if loading}
    <p class="hint">Loading…</p>
  {:else if error}
    <p class="error">{error}</p>
  {:else if records.length === 0}
    <p class="hint">No confirmed transactions for {year}.</p>
  {:else}
    <div class="table-wrap">
      <table>
        <caption class="sr-only">{selectedWallet?.label ?? ''} tax records — {year} — {method.toUpperCase()}</caption>
        <thead>
          <tr>
            {#snippet sortTh(col: SortCol, label: string, cls: string = '')}
              <th class={cls} aria-sort={sortCol === col ? (sortDir === 'asc' ? 'ascending' : 'descending') : 'none'}>
                <button class="th-btn" class:sorted={sortCol === col} onclick={() => toggleSort(col)}>
                  {label}
                  <span class="sort-arrow" class:active={sortCol === col}>{sortArrow(col)}</span>
                </button>
              </th>
            {/snippet}
            {@render sortTh('date', 'Date')}
            {@render sortTh('type', 'Type')}
            {@render sortTh('btc', 'BTC', 'num')}
            {@render sortTh('usd_price', 'BTC Price', 'num')}
            {@render sortTh('usd_value', 'USD Value', 'num')}
            {@render sortTh('cost_basis', 'Cost Basis', 'num')}
            {@render sortTh('gain_loss', 'Gain / Loss', 'num')}
          </tr>
        </thead>
        <tbody>
          {#each sortedRecords as r (r.txid)}
            <tr>
              <td>{r.date}</td>
              <td><span class="type-badge type-{r.type}">{r.type}</span></td>
              <td class="num mono">{r.btc.toFixed(8)}</td>
              <td class="num">{fmtUsd(r.usd_price)}</td>
              <td class="num">{fmtUsd(r.usd_value)}</td>
              <td class="num">{fmtUsd(r.cost_basis)}</td>
              <td
                class="num"
                class:gain={r.gain_loss != null && r.gain_loss > 0}
                class:loss={r.gain_loss != null && r.gain_loss < 0}
              >{fmtGain(r.gain_loss)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    <p class="footer-note">
      Cost basis defaults to USD value at time of transaction. Edit per-transaction in the detail view.
    </p>
  {/if}
</div>

<style>
  .page {
    flex: 1; display: flex; flex-direction: column;
    padding: 16px 24px 24px;
    min-width: 0; min-height: 0;
    gap: 12px;
  }

  /* Mirror the wallet-detail header: prominent title on the left, primary
     action on the right. The back link sits above as an unobtrusive crumb. */
  .tax-header {
    display: flex; align-items: center; justify-content: space-between; gap: 12px;
  }
  .tax-title {
    margin: 0; font-size: 1.5rem; font-weight: 800; letter-spacing: -0.01em; color: var(--text);
  }
  .tax-wallet { color: var(--text-muted); font-weight: 600; font-size: 1.1rem; }
  .back-btn {
    align-self: flex-start;
    background: none; border: 1px solid var(--border); border-radius: 4px;
    color: var(--text-muted); cursor: pointer;
    font-size: 0.8rem; padding: 4px 10px;
  }
  .back-btn:hover { color: var(--text); border-color: var(--text-muted); }

  .controls {
    display: flex; align-items: center; gap: 14px; flex-wrap: wrap;
    padding: 10px 14px;
    background: var(--surface-1);
    border: 1px solid var(--border); border-radius: 6px;
  }
  .ctrl-label {
    display: flex; align-items: center; gap: 6px;
    font-size: 0.8rem; color: var(--text-muted);
  }
  .ctrl-select {
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 4px; color: var(--text);
    padding: 4px 8px; font-size: 0.85rem;
  }
  .download-btn {
    flex-shrink: 0;
    background: var(--accent); color: #000; border: none; border-radius: 5px;
    padding: 6px 14px; font-size: 0.82rem; font-weight: 600; cursor: pointer;
  }
  .download-btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .download-btn:not(:disabled):hover { filter: brightness(1.1); }

  .summary-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 10px;
  }
  @media (max-width: 700px) {
    .summary-grid { grid-template-columns: 1fr; }
  }
  .summary-card {
    display: flex; flex-direction: column; gap: 4px;
    background: var(--surface-1); border: 1px solid var(--border);
    border-radius: 6px; padding: 10px 14px;
  }
  .summary-card.summary-net {
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
  }
  .summary-label {
    font-size: 0.72rem; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.05em;
  }
  .summary-value { font-size: 1.05rem; font-weight: 700; font-variant-numeric: tabular-nums; }

  .table-wrap {
    overflow: auto; min-height: 0;
    max-height: calc(100vh - 280px);
    background: var(--surface-1);
    border: 1px solid var(--border); border-radius: 6px;
  }
  @media (max-width: 600px) {
    .page { padding: 12px 12px 16px; gap: 10px; }
    .tax-title { font-size: 1.25rem; }
    .controls { padding: 8px 10px; gap: 10px; }
    .table-wrap {
      max-height: calc(100dvh - 360px);
      -webkit-overflow-scrolling: touch;
    }
    table { font-size: 0.78rem; }
    .th-btn { padding: 6px 8px; font-size: 0.72rem; }
    tbody td { padding: 7px 8px; }
  }
  table { width: 100%; border-collapse: collapse; font-size: 0.84rem; }
  thead th {
    position: sticky; top: 0; z-index: 1;
    text-align: left; padding: 0;
    background: var(--surface-1);
    border-bottom: 1px solid var(--border);
  }
  .th-btn {
    width: 100%; display: flex; align-items: center; gap: 4px;
    background: none; border: none; cursor: pointer;
    padding: 8px 10px;
    color: var(--text-muted); font-size: 0.78rem; font-weight: 700;
    text-transform: uppercase; letter-spacing: 0.04em;
    transition: color 0.1s;
  }
  .th-btn:hover { color: var(--text); text-decoration: underline; text-underline-offset: 3px; }
  .th-btn.sorted { color: var(--text); }
  th.num .th-btn { justify-content: flex-end; }
  .sort-arrow { font-size: 0.7rem; opacity: 0.4; }
  .sort-arrow.active { opacity: 1; color: var(--accent); }
  tbody td {
    padding: 8px 10px; border-bottom: 1px solid var(--border);
    color: var(--text);
  }
  tbody tr:last-child td { border-bottom: none; }
  tbody tr:hover { background: var(--surface-hover); }
  .num { text-align: right; font-variant-numeric: tabular-nums; }
  .mono { font-family: monospace; }

  .type-badge {
    display: inline-block; padding: 2px 7px; border-radius: 3px;
    font-size: 0.74rem; font-weight: 600; text-transform: capitalize;
  }
  .type-received { background: color-mix(in srgb, #52a875 15%, transparent); color: #52a875; }
  .type-sent { background: color-mix(in srgb, #e09c52 15%, transparent); color: #e09c52; }

  .gain { color: #52a875; }
  .loss { color: #e05252; }
  .neutral { color: var(--text-muted); }

  .hint { color: var(--text-muted); font-size: 0.85rem; margin: 0; padding: 24px 0; text-align: center; }
  .error { color: var(--error); font-size: 0.85rem; margin: 0; padding: 24px 0; text-align: center; }
  .footer-note { font-size: 0.75rem; color: var(--text-muted); margin: 0; }

  .sr-only {
    position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
    overflow: hidden; clip: rect(0,0,0,0); white-space: nowrap; border: 0;
  }
</style>
