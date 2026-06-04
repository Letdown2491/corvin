<script lang="ts">
  import { onDestroy } from 'svelte'
  import { browser } from '$app/environment'
  import { api } from '../lib/api'
  import type { AddressInfo, UtxoRecord, WalletEntry } from '../lib/types'
  import { displayUnit, balancesHidden } from '../stores/settings'
  import { addToast } from '../stores/toasts'
  import { addressCategories, assignAddressCategory, categoryById } from '../stores/categories'
  import { addressLabels, setAddressLabel } from '../stores/address_labels'
  import CategoryPicker from './CategoryPicker.svelte'

  let {
    addresses,
    utxos,
    wallet,
  }: {
    addresses: AddressInfo[]
    utxos: UtxoRecord[]
    wallet: WalletEntry
  } = $props()

  let walletId = $derived(wallet.id)

  type SortCol = 'index' | 'address' | 'txs' | 'balance' | 'label' | 'category'
  type SortDir = 'asc' | 'desc'
  type Filter = 'all' | 'used' | 'unused' | 'labelled'

  // Default to "used" to avoid drowning a few rows in hundreds of unused.
  let filterStorageKey = $derived(`corvin:addressFilter:${walletId}`)
  function loadInitialFilter(id: string): Filter {
    if (!browser) return 'used'
    try {
      const saved = localStorage.getItem(`corvin:addressFilter:${id}`)
      if (saved === 'all' || saved === 'used' || saved === 'unused' || saved === 'labelled') {
        return saved
      }
    } catch {}
    return 'used'
  }

  let tab = $state<'receive' | 'change'>('receive')
  // Address notes come straight from the shared store, so a save here is
  // reactively reflected wherever a note is inherited (UTXO tab, coin control…).
  let labels = $derived($addressLabels)
  let editingAddr = $state<string | null>(null)
  let editValue = $state('')
  let savingAddr = $state<string | null>(null)
  let copiedAddr = $state<string | null>(null)
  let copyTimer: ReturnType<typeof setTimeout> | null = null
  onDestroy(() => { if (copyTimer) clearTimeout(copyTimer) })
  let search = $state('')

  let sortStorageKey = $derived(`corvin:addressSort:${walletId}`)
  function loadInitialSort(id: string, f: Filter): { col: SortCol; dir: SortDir } {
    if (browser) {
      try {
        const raw = localStorage.getItem(`corvin:addressSort:${id}`)
        if (raw) {
          const parsed = JSON.parse(raw)
          const validCol = ['index', 'address', 'txs', 'balance', 'label', 'category'].includes(parsed?.col)
          const validDir = parsed?.dir === 'asc' || parsed?.dir === 'desc'
          if (validCol && validDir) return parsed
        }
      } catch {}
    }
    // "Used" → biggest balance first; others → derivation order.
    if (f === 'used') return { col: 'balance', dir: 'desc' }
    return { col: 'index', dir: 'asc' }
  }
  // svelte-ignore state_referenced_locally
  let filter = $state<Filter>(loadInitialFilter(walletId))
  // svelte-ignore state_referenced_locally
  const initialSort = loadInitialSort(walletId, filter)
  let sortCol = $state<SortCol>(initialSort.col)
  let sortDir = $state<SortDir>(initialSort.dir)

  $effect(() => {
    if (!browser) return
    try { localStorage.setItem(filterStorageKey, filter) } catch {}
  })
  $effect(() => {
    if (!browser) return
    try { localStorage.setItem(sortStorageKey, JSON.stringify({ col: sortCol, dir: sortDir })) } catch {}
  })

  function focusInput(node: HTMLElement) {
    node.focus()
  }


  function toggleSort(col: SortCol) {
    if (sortCol === col) {
      sortDir = sortDir === 'asc' ? 'desc' : 'asc'
    } else {
      sortCol = col
      sortDir = 'asc'
    }
  }

  let receive = $derived(
    addresses.filter(a => a.kind === 'external').sort((a, b) => a.index - b.index)
  )
  let change = $derived(
    addresses.filter(a => a.kind === 'internal').sort((a, b) => a.index - b.index)
  )

  let nextReceiveAddr = $derived(receive.find(a => !a.used)?.address ?? null)

  let shown = $derived.by(() => {
    const q = search.trim().toLowerCase()
    const base = (tab === 'receive' ? receive : change).filter(a => {
      if (filter === 'used' && !a.used) return false
      if (filter === 'unused' && a.used) return false
      if (filter === 'labelled' && !(labels[a.address]?.trim())) return false
      if (q) {
        const label = (labels[a.address] ?? '').toLowerCase()
        return a.address.toLowerCase().includes(q) || label.includes(q)
      }
      return true
    })
    const m = sortDir === 'asc' ? 1 : -1
    return [...base].sort((a, b) => {
      switch (sortCol) {
        case 'index':   return (a.index - b.index) * m
        case 'address': return a.address.localeCompare(b.address) * m
        case 'txs':     return (a.tx_count - b.tx_count) * m
        case 'balance': return (addrBalance(a.address) - addrBalance(b.address)) * m
        case 'label': {
          // Empty labels sort last regardless of direction.
          const la = labels[a.address] ?? ''
          const lb = labels[b.address] ?? ''
          if (!la && lb) return 1
          if (la && !lb) return -1
          return la.localeCompare(lb) * m
        }
        case 'category': {
          // Sort by category name so coins in the same compartment group;
          // uncategorized addresses sort last regardless of direction.
          const na = $categoryById[$addressCategories[a.address] ?? '']?.name ?? ''
          const nb = $categoryById[$addressCategories[b.address] ?? '']?.name ?? ''
          if (!na && nb) return 1
          if (na && !nb) return -1
          return na.localeCompare(nb) * m
        }
        default: return 0
      }
    })
  })

  // Per-address balance from UTXOs
  function addrBalance(address: string): number {
    return utxos.filter(u => u.address === address).reduce((s, u) => s + u.amount_sats, 0)
  }

  function formatBalance(sats: number): string {
    if ($balancesHidden) return sats === 0 ? '—' : '•••'
    if (sats === 0) return '—'
    if ($displayUnit === 'btc') return (sats / 1e8).toFixed(8) + ' BTC'
    return sats.toLocaleString() + ' sats'
  }

  async function copyAddress(addr: string) {
    try {
      await navigator.clipboard.writeText(addr)
      copiedAddr = addr
      if (copyTimer) clearTimeout(copyTimer)
      copyTimer = setTimeout(() => (copiedAddr = null), 1500)
    } catch { addToast('Copy failed — clipboard unavailable') }
  }

  function startEdit(addr: string) {
    editingAddr = addr
    editValue = labels[addr] ?? ''
  }

  function cancelEdit() {
    editingAddr = null
    editValue = ''
  }

  async function saveLabel(addr: string) {
    savingAddr = addr
    try {
      await setAddressLabel(addr, editValue)
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Failed to save label')
    }
    savingAddr = null
    editingAddr = null
    editValue = ''
  }

  function handleLabelKeydown(e: KeyboardEvent, addr: string) {
    if (e.key === 'Enter') { e.preventDefault(); saveLabel(addr) }
    if (e.key === 'Escape') cancelEdit()
  }
</script>

<div class="address-table">
  <div class="toolbar">
    <div class="tabs" role="tablist" aria-label="Address types">
      <button
        role="tab"
        aria-selected={tab === 'receive'}
        class:active={tab === 'receive'}
        onclick={() => tab = 'receive'}
      >
        Receive <span class="tab-count">{receive.length}</span>
      </button>
      {#if change.length > 0}
        <button
          role="tab"
          aria-selected={tab === 'change'}
          class:active={tab === 'change'}
          onclick={() => tab = 'change'}
        >
          Change <span class="tab-count">{change.length}</span>
        </button>
        <span
          class="change-help"
          title="Internal addresses Corvin creates to receive leftover funds after you send a transaction. You don't share these — the wallet uses them automatically."
          aria-label="What are change addresses?"
        >ⓘ</span>
      {/if}
    </div>
    <div class="filter-group" role="group" aria-label="Filter addresses">
      <button class:active={filter === 'all'}      onclick={() => filter = 'all'}>All</button>
      <button class:active={filter === 'used'}     onclick={() => filter = 'used'}>Used</button>
      <button class:active={filter === 'unused'}   onclick={() => filter = 'unused'}>Unused</button>
      <button class:active={filter === 'labelled'} onclick={() => filter = 'labelled'}>Labelled</button>
    </div>
  </div>

  <!-- Search lives on its own row so it doesn't compete with the bucket
       selectors above it. Filter pills choose a category; search refines
       within that category — different kinds of work that benefit from
       being visually separated. -->
  <label class="search">
    <span class="search-icon" aria-hidden="true">⌕</span>
    <span class="sr-only">Search addresses or labels</span>
    <input
      type="search"
      bind:value={search}
      placeholder="Search address or label"
      aria-label="Search addresses or labels"
    />
    {#if search}
      <button
        type="button"
        class="search-clear"
        onclick={() => search = ''}
        aria-label="Clear search"
      >✕</button>
    {/if}
  </label>

  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          {#snippet sortTh(col: SortCol, cls: string, label: string, tip?: string)}
            <th class={cls} title={tip} aria-sort={sortCol === col ? (sortDir === 'asc' ? 'ascending' : 'descending') : 'none'}>
              <button
                class="sort-btn"
                class:sorted={sortCol === col}
                onclick={() => toggleSort(col)}
              >
                {label}
                <span class="sort-arrow" aria-hidden="true">
                  {#if sortCol === col}{sortDir === 'asc' ? '↑' : '↓'}{:else}↕{/if}
                </span>
              </button>
            </th>
          {/snippet}
          <!-- Column order: identity (# + address) → user metadata (label) →
               numerical data (txs + balance), all right-aligned. Action cell
               always last. Matches accounting / spreadsheet convention so
               header alignment lines up with value alignment. -->
          {@render sortTh('index',   'col-index',   '#')}
          {@render sortTh('address',  'col-address',  'Address')}
          {@render sortTh('category', 'col-category', 'Category')}
          {@render sortTh('label',    'col-label',    'Note', 'A note for this address — becomes the default note for coins received here')}
          {@render sortTh('txs',     'col-txs',     'Txs')}
          {@render sortTh('balance', 'col-balance', 'Balance')}
        </tr>
      </thead>
      <tbody>
        {#each shown as addr (addr.address)}
          {@const bal = addrBalance(addr.address)}
          {@const isUnused = !addr.used}
          {@const isNext = tab === 'receive' && addr.address === nextReceiveAddr}
          {@const acatId = $addressCategories[addr.address] ?? null}
          <tr class:unused={isUnused} class:next-addr={isNext}>
            <td class="col-index">{addr.index}</td>
            <td class="col-address">
              <div class="addr-cell">
                <span class="addr-text" title={addr.address}>
                  <span class="addr-start">{addr.address.slice(0, -8)}</span><span class="addr-end">{addr.address.slice(-8)}</span>
                </span>
                {#if isNext}<span class="next-badge">next</span>{/if}
                <button
                  class="copy-btn"
                  class:copied={copiedAddr === addr.address}
                  title="Copy address"
                  aria-label="Copy address"
                  onclick={() => copyAddress(addr.address)}
                >
                  {copiedAddr === addr.address ? '✓' : '⎘'}
                </button>
              </div>
            </td>
            <td class="col-category">
              <CategoryPicker current={acatId} onSelect={(id) => assignAddressCategory(addr.address, id)} />
            </td>
            <td class="col-label">
              {#if editingAddr === addr.address}
                <input
                  class="label-input"
                  bind:value={editValue}
                  onblur={() => saveLabel(addr.address)}
                  onkeydown={(e) => handleLabelKeydown(e, addr.address)}
                  disabled={savingAddr === addr.address}
                  use:focusInput
                  placeholder="Add note…"
                />
              {:else}
                <button
                  class="label-btn"
                  class:has-label={!!labels[addr.address]}
                  onclick={() => startEdit(addr.address)}
                  title="Click to edit this address's note"
                >
                  {#if labels[addr.address]}
                    {labels[addr.address]}
                    <span class="label-edit-hint" aria-hidden="true">✎</span>
                  {:else}
                    <span class="label-empty">+ note</span>
                  {/if}
                </button>
              {/if}
            </td>
            <td class="col-txs">{addr.tx_count > 0 ? addr.tx_count : '—'}</td>
            <td class="col-balance">{formatBalance(bal)}</td>
          </tr>
        {/each}
        {#if shown.length === 0}
          <tr>
            <td colspan="6" class="empty">
              {#if search.trim()}
                No addresses match "{search.trim()}".
              {:else if filter !== 'all'}
                No {filter} addresses.
              {:else}
                No addresses yet. They appear as the wallet syncs and reveals them.
              {/if}
            </td>
          </tr>
        {/if}
      </tbody>
    </table>
  </div>
</div>

<style>
  .address-table { display: flex; flex-direction: column; gap: 12px; }


  .toolbar { display: flex; align-items: center; justify-content: space-between; gap: 8px; flex-wrap: wrap; }

  .tabs { display: flex; align-items: center; gap: 2px; }

  .change-help {
    color: var(--text-muted); font-size: 0.85rem; cursor: help;
    margin-left: 4px; user-select: none;
  }
  .change-help:hover { color: var(--text); }

  .search {
    display: flex; align-items: center; gap: 6px;
    background: var(--surface-2);
    border: 1px solid var(--border); border-radius: 4px;
    padding: 4px 8px;
    transition: border-color 0.12s;
  }
  .search:focus-within { border-color: var(--accent); }
  .search-icon {
    color: var(--text-muted); font-size: 0.95rem; line-height: 1;
    flex-shrink: 0;
  }
  .search input {
    flex: 1; min-width: 0;
    background: none; border: none; outline: none;
    color: var(--text); font-size: 0.82rem;
    padding: 2px 0;
  }
  .search input::placeholder { color: var(--text-muted); }
  /* Hide the browser's native search-clear button in favor of our own. */
  .search input::-webkit-search-cancel-button { display: none; }
  .search-clear {
    background: none; border: none; cursor: pointer;
    color: var(--text-muted); font-size: 0.75rem; padding: 2px 4px;
    line-height: 1; border-radius: 3px; flex-shrink: 0;
  }
  .search-clear:hover { color: var(--text); background: var(--border); }

  .sr-only {
    position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
    overflow: hidden; clip: rect(0,0,0,0); white-space: nowrap; border: 0;
  }

  .tabs button {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px;
    padding: 5px 12px; font-size: 0.8rem; cursor: pointer; color: var(--text-muted);
    display: flex; align-items: center; gap: 5px;
  }
  .tabs button.active { background: var(--accent); color: #000; border-color: var(--accent); font-weight: 600; }
  .tab-count {
    background: color-mix(in srgb, currentColor 15%, transparent);
    border-radius: 10px; padding: 1px 6px; font-size: 0.72rem; font-weight: 700;
  }

  .filter-group {
    display: flex; border: 1px solid var(--border); border-radius: 4px; overflow: hidden;
  }
  .filter-group button {
    background: none; border: none; padding: 4px 10px; font-size: 0.75rem;
    cursor: pointer; color: var(--text-muted); font-weight: 500;
  }
  .filter-group button + button { border-left: 1px solid var(--border); }
  .filter-group button.active { background: var(--surface-2); color: var(--text); font-weight: 600; }

  .table-wrap { overflow-x: auto; }

  table { width: 100%; border-collapse: collapse; font-size: 0.8rem; }

  thead th {
    text-align: left; padding: 0;
    font-size: 0.7rem; font-weight: 600;
    border-bottom: 1px solid var(--border);
  }

  .sort-btn {
    background: none; border: none; cursor: pointer;
    display: flex; align-items: center; gap: 3px;
    padding: 5px 8px 7px; width: 100%;
    font-size: 0.7rem; font-weight: 600; text-transform: uppercase;
    letter-spacing: 0.05em; color: var(--text-muted);
    white-space: nowrap;
  }
  .sort-btn:hover { color: var(--text); }
  .sort-btn.sorted { color: var(--text); }
  .sort-arrow { font-size: 0.65rem; opacity: 0.4; }
  .sort-btn.sorted .sort-arrow { opacity: 1; color: var(--accent); }

  tbody tr {
    border-bottom: 1px solid var(--border);
    transition: background 0.1s;
  }
  tbody tr:hover { background: var(--surface-2); }
  tbody tr.unused { opacity: 0.45; }
  tbody tr.unused:hover { opacity: 0.75; }
  tbody tr.next-addr {
    opacity: 1;
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  tbody tr.next-addr:hover {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }

  td { padding: 7px 8px; vertical-align: middle; }

  .col-index { color: var(--text-muted); font-family: monospace; width: 36px; }
  .col-address { min-width: 0; }
  .col-txs { width: 42px; text-align: right; color: var(--text-muted); font-variant-numeric: tabular-nums; }
  .col-balance { width: 120px; text-align: right; font-family: monospace; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .col-label { min-width: 120px; }
  .col-category { width: 130px; }

  th.col-txs .sort-btn,
  th.col-balance .sort-btn { justify-content: flex-end; }

  .addr-cell { display: flex; align-items: center; gap: 8px; min-width: 0; }

  .next-badge {
    flex-shrink: 0;
    font-size: 0.7rem; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.06em; color: var(--accent);
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
    border-radius: 999px; padding: 2px 8px;
  }

  /* Middle-truncation: prefix shrinks with ellipsis, last 8 chars stay pinned. */
  .addr-text {
    font-family: monospace; display: inline-flex; max-width: 100%;
    min-width: 0; vertical-align: bottom;
  }
  .addr-start {
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    min-width: 0; flex-shrink: 1;
  }
  .addr-end { flex-shrink: 0; white-space: nowrap; }

  .label-btn {
    background: none; border: 1px solid transparent; border-radius: 3px;
    color: var(--text-muted); cursor: pointer; font-size: 0.78rem;
    padding: 2px 4px; width: 100%; text-align: left;
    display: flex; align-items: center; justify-content: space-between; gap: 4px;
    min-height: 24px;
    transition: border-color 0.15s, color 0.15s;
  }
  .label-btn.has-label { color: var(--text); }
  .label-btn:hover { border-color: var(--border); color: var(--text); }
  .label-edit-hint { opacity: 0; font-size: 0.7rem; flex-shrink: 0; }
  .label-btn:hover .label-edit-hint { opacity: 0.6; }
  .label-empty { color: var(--text-muted); opacity: 0.6; }
  .label-btn:hover .label-empty { color: var(--accent); opacity: 1; }

  .label-input {
    width: 100%; background: var(--surface-2); border: 1px solid var(--accent);
    border-radius: 3px; color: var(--text); font-size: 0.78rem;
    padding: 2px 5px; outline: none;
  }

  /* Hover-revealed on devices with hover; always visible on touch. */
  .copy-btn {
    flex-shrink: 0;
    background: none; border: 1px solid var(--border); border-radius: 3px;
    color: var(--text-muted); cursor: pointer; font-size: 0.8rem;
    padding: 2px 6px; transition: opacity 0.12s, color 0.15s;
  }
  .copy-btn:hover { color: var(--text); border-color: var(--text-muted); }
  .copy-btn.copied { color: var(--accent); border-color: var(--accent); }

  @media (hover: hover) {
    .copy-btn { opacity: 0; }
    tr:hover .copy-btn { opacity: 1; }
    .copy-btn:focus-visible { opacity: 1; outline: 2px solid var(--accent); outline-offset: 1px; }
    /* Keep the confirmation tick visible regardless of hover so the user
       gets feedback after a click. */
    .copy-btn.copied { opacity: 1; }
  }

  .empty { color: var(--text-muted); padding: 16px 8px; font-size: 0.85rem; }

  @media (max-width: 640px) {
    /* Reflow table into card rows */
    table, tbody { display: block; }
    thead { display: none; }

    tbody tr {
      display: flex; flex-wrap: wrap; align-items: center;
      gap: 0 8px; padding: 10px 0; position: relative;
    }

    td { padding: 0 0 2px; border: none; font-size: 0.72rem; }

    /* Address spans the row; copy button stays in the flex cell. */
    .col-address { width: 100%; order: 0; }
    .addr-text { width: 100%; }

    /* Meta items flow on the second line — mirrors the desktop column
       order: # → label → txs → balance, so users get a consistent
       left-to-right reading rhythm across breakpoints. */
    .col-index    { order: 1; }
    .col-category { order: 2; }
    .col-label    { order: 3; flex: 1; min-width: 80px; }
    .col-txs      { order: 4; text-align: left; }
    .col-balance  { order: 5; text-align: left; white-space: nowrap; }

    /* Label editor still needs full width */
    .label-btn, .label-input { font-size: 0.72rem; }
  }
</style>
