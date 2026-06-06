<script lang="ts">
  import { onMount } from 'svelte'
  import { api } from '../lib/api'
  import { downloadBlob } from '../lib/utils'
  import type { AddressInfo, Balance, BalancePoint, BackendEntry, TxRecord, UtxoRecord, WalletEntry } from '../lib/types'
  import { syncing, lastSyncComplete, walletBalances, wallets, markWalletSynced, unmarkWalletSynced, hasWalletSyncedThisSession } from '../stores/wallets'
  import { goto } from '$app/navigation'
  import { displayUnit, balancesHidden, showFiatBalance, currentBtcPrice, mempoolUrl, feeRates as feeRatesStore, backendStatuses, spStatuses, offline } from '../stores/settings'
  import { get } from 'svelte/store'
  import { labels, loadLabels } from '../stores/labels'
  import { parseQuery, filterTxs } from '../lib/search'
  import { loadCostBasis } from '../stores/cost_basis'
  import { loadUtxoLabels } from '../stores/utxo_labels'
  import { loadFrozenUtxos } from '../stores/utxo_freeze'
  import { loadCategories } from '../stores/categories'
  import { addressLabels, loadAddressLabels } from '../stores/address_labels'
  import { addToast } from '../stores/toasts'
  import TxRow from './TxRow.svelte'
  import TxDetailPanel from './TxDetailPanel.svelte'
  import AddressTable from './AddressTable.svelte'
  import UtxoTable from './UtxoTable.svelte'
  import BalanceChart from './BalanceChart.svelte'
  import ReceiveModal from './ReceiveModal.svelte'
  import EmptyState from './EmptyState.svelte'
  import ConsolidateModal from './ConsolidateModal.svelte'
  import BroadcastModal from './BroadcastModal.svelte'
  import MessagesModal from './MessagesModal.svelte'
  import BalanceHero from './BalanceHero.svelte'
  import SendFlow from './send/SendFlow.svelte'
  import WalletMenu from './WalletMenu.svelte'
  import TxSearchBar from './TxSearchBar.svelte'
  import AddAccountModal from './AddAccountModal.svelte'
  import TestBackupModal from './TestBackupModal.svelte'
  import AddressLookupModal from './AddressLookupModal.svelte'
  import PsbtInspectorModal from './PsbtInspectorModal.svelte'
  import SpSpendModal from './SpSpendModal.svelte'
  import WalletDetailsModal from './WalletDetailsModal.svelte'
  import ChangeBackendModal from './ChangeBackendModal.svelte'
  import DeleteWalletModal from './DeleteWalletModal.svelte'
  import WalletToolsTab from './WalletToolsTab.svelte'
  import SweepWifModal from './SweepWifModal.svelte'

  let { wallet, onDelete }: { wallet: WalletEntry; onDelete: () => void } = $props()

  // Fee rates are polled centrally by WalletSidebar and shared via the
  // `feeRates` store. Send / consolidate / fee-bump flows still need the
  // numeric values (for default presets), so we read from the store here
  // instead of polling independently.
  let feeRates = $derived($feeRatesStore)

  let balance = $state<Balance | null>(null)
  let savedBackends = $state<BackendEntry[]>([])
  // This wallet's backend connection state. SP wallets sync via their Frigate scanner
  // (keyed by wallet id in spStatuses), never the Electrum backend, so resolve them
  // separately; map the SP entry into the same shape the chip/banner expect.
  let conn = $derived.by(() => {
    if (wallet.kind === 'silent_payments') {
      const sp = $spStatuses.find(s => s.wallet_id === wallet.id)
      return sp ? { backend: null, connected: sp.connected, tip_height: null, error: sp.error } : undefined
    }
    return $backendStatuses.find(s => (s.backend ?? null) === (wallet.backend ?? null))
  })
  // Backend is unreachable (not a deliberate offline mode, not a transient sync):
  // drives the reassuring "degraded" banner. Distinct from $offline.
  let backendDown = $derived(
    !$offline && conn !== undefined && !conn.connected && !$syncing.has(wallet.id)
  )
  let serverLabel = $derived(
    wallet.backend != null
      ? (savedBackends.find(b => b.id === wallet.backend)?.label ?? wallet.backend)
      : wallet.kind === 'silent_payments'
        ? 'Public Frigate'
        : 'Default backend'
  )
  // Recommended mempool fee for the status line (matches the old sidebar
  // formatting). Global/mempool value, shown in wallet context.
  function fmtFee(n: number): string {
    if (n >= 100) return Math.round(n).toString()
    if (Number.isInteger(n)) return `${n}.0`
    return n.toFixed(1)
  }
  let feeDisplay = $derived.by(() => {
    const f = feeRates
    if (!f) return null
    const { hourFee, halfHourFee, fastestFee } = f
    const uniform = hourFee === halfHourFee && halfHourFee === fastestFee
    return uniform
      ? `${fmtFee(hourFee)} sat/vB`
      : `${fmtFee(hourFee)} / ${fmtFee(halfHourFee)} / ${fmtFee(fastestFee)} sat/vB`
  })

  // Fiat value of this wallet's holdings, shown on the status line. (The live
  // 1-BTC price is a separate, global thing shown in the sidebar.)
  let fiatText = $derived.by(() => {
    if ($balancesHidden || !$currentBtcPrice || !$showFiatBalance || !balance) return ''
    return `$${((balance.confirmed_sats / 1e8) * $currentBtcPrice).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`
  })
  let txs = $state<TxRecord[]>([])
  let addresses = $state<AddressInfo[]>([])
  let utxos = $state<UtxoRecord[]>([])
  let balanceHistory = $state<BalancePoint[]>([])
  let tab = $state<'txs' | 'addresses' | 'utxos' | 'charts' | 'tools'>('txs')
  let error = $state('')
  let deleteOpen = $state(false)
  let broadcastOpen = $state(false)
  let messagesOpen = $state(false)
  let addAccountOpen = $state(false)
  let testBackupOpen = $state(false)
  let addressLookupOpen = $state(false)
  let psbtInspectorOpen = $state(false)
  let sweepWifOpen = $state(false)
  // Only seed-imported wallets have a derivation path in `input` (paste-xpub
  // wallets put the xpub string there). Detect that signal to decide whether
  // "Add account" applies.
  let canAddAnotherAccount = $derived(
    wallet.kind !== 'address' &&
    wallet.kind !== 'multisig' &&
    wallet.kind !== 'silent_payments' &&
    /^m\//.test(wallet.input)
  )
  // Broader eligibility than Add-another-account: any HD singlesig wallet
  // whose descriptor carries origin info ([fingerprint/path]xpub) can be
  // verified — covers seed-imported wallets AND hardware-wallet-imported
  // wallets (where wallet.input is the xpub string, not the path, but the
  // path lives in the descriptor's origin tag).
  let canTestBackup = $derived(
    wallet.kind !== 'address' &&
    wallet.kind !== 'multisig' &&
    wallet.kind !== 'silent_payments' &&
    /\[[0-9a-fA-F]{8}\/[^\]]+\]/.test(wallet.external_descriptor ?? '')
  )

  let selectedTx = $state<TxRecord | null>(null)
  let searchQuery = $state('')
  // Debounced mirror of searchQuery: the input updates instantly, but the
  // (potentially heavy) filterTxs only re-runs ~180ms after typing settles.
  let debouncedQuery = $state('')
  let showReceive = $state(false)
  let showSend = $state(false)
  let showSpSpend = $state(false)
  let showDetails = $state(false)
  let changeBackendOpen = $state(false)
  let consolidateMode = $state(false)
  let consolidateUtxos = $state<UtxoRecord[] | null>(null)
  let editing = $state(false)
  let newLabel = $state('')
  let renameError = $state('')
  let renaming = $state(false)
  let renameInputEl = $state<HTMLInputElement | null>(null)

  $effect(() => {
    if (editing && renameInputEl) renameInputEl.focus()
  })

  onMount(() => {
    loadLabels()
    loadCostBasis()
    loadUtxoLabels()
    loadFrozenUtxos()
    loadCategories()
    loadAddressLabels()
    refreshConn()
    api.backends.list().then(b => savedBackends = b).catch(() => {})
    return () => {
      if (syncTickTimer) clearInterval(syncTickTimer)
    }
  })

  async function refreshConn() {
    try { backendStatuses.set(await api.backends.status()) } catch { /* keep last-known */ }
    try { spStatuses.set(await api.silentPayments.status()) } catch { /* keep last-known */ }
  }

  let dataGen = 0

  async function loadData(gen?: number) {
    const myGen = gen ?? dataGen
    try {
      const [b, t, u] = await Promise.all([
        api.wallets.balance(wallet.id),
        api.wallets.txs(wallet.id),
        api.wallets.utxos(wallet.id),
      ])
      if (myGen !== dataGen) return
      balance = b; txs = t; utxos = u
      if (balance) walletBalances.update(m => { const n = new Map(m); n.set(wallet.id, balance!); return n })
      error = ''
    } catch (e) {
      if (myGen !== dataGen) return
      error = e instanceof Error ? e.message : 'Failed to load wallet data'
    }
  }

  async function loadAddresses(gen?: number) {
    const myGen = gen ?? dataGen
    try {
      const [a, al] = await Promise.all([
        api.wallets.addresses(wallet.id),
        api.addressLabels.list(),
      ])
      if (myGen !== dataGen) return
      addresses = a; addressLabels.set(al)
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Failed to load addresses')
    }
  }
  async function loadUtxos(gen?: number) {
    const myGen = gen ?? dataGen
    try {
      const u = await api.wallets.utxos(wallet.id)
      if (myGen !== dataGen) return
      utxos = u
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Failed to load UTXOs')
    }
  }
  async function loadBalanceHistory(gen?: number) {
    const myGen = gen ?? dataGen
    try {
      const h = await api.wallets.balanceHistory(wallet.id)
      if (myGen !== dataGen) return
      balanceHistory = h
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Failed to load balance history')
    }
  }

  let syncStartedAt = $state<number | null>(null)
  let syncElapsedSec = $state(0)
  let syncTickTimer: ReturnType<typeof setInterval> | null = null

  async function sync() {
    const gen = ++dataGen
    syncing.update(s => new Set(s).add(wallet.id))
    syncStartedAt = Date.now()
    syncElapsedSec = 0
    if (syncTickTimer) clearInterval(syncTickTimer)
    syncTickTimer = setInterval(() => {
      if (syncStartedAt) syncElapsedSec = Math.floor((Date.now() - syncStartedAt) / 1000)
    }, 1000)
    try {
      await api.wallets.sync(wallet.id)
      if (gen !== dataGen) return
      addresses = []; utxos = []; balanceHistory = []
      await loadData(gen)
      if (wallet.internal_descriptor) await loadAddresses(gen)
    } catch (e) {
      if (gen === dataGen) error = e instanceof Error ? e.message : 'Sync failed'
    } finally {
      syncing.update(s => { const n = new Set(s); n.delete(wallet.id); return n })
      if (syncTickTimer) { clearInterval(syncTickTimer); syncTickTimer = null }
      syncStartedAt = null
    }
  }

  function startEdit() {
    newLabel = wallet.label
    renameError = ''
    editing = true
  }

  async function doRename() {
    const trimmed = newLabel.trim()
    if (!trimmed) { renameError = 'Name cannot be empty'; return }
    if (trimmed === wallet.label) { editing = false; return }
    // Warn (but don't block) on duplicate names so the user notices.
    const duplicate = $wallets.find(w => w.id !== wallet.id && w.label === trimmed)
    if (duplicate && !renameError.startsWith('Already used')) {
      renameError = `Already used by another wallet. Press Save again to keep this name anyway.`
      return
    }
    renaming = true; renameError = ''
    try {
      const updated = await api.wallets.rename(wallet.id, trimmed)
      wallets.update(ws => ws.map(w => w.id === updated.id ? updated : w))
      editing = false
    } catch (e) {
      renameError = e instanceof Error ? e.message : 'Rename failed'
    } finally {
      renaming = false }
  }

  function cancelEdit() { editing = false; renameError = '' }


  function exportCsv() {
    const rows: (string | number)[][] = [
      ['txid', 'date', 'type', 'amount_sats', 'amount_btc', 'fee_sats', 'confirmations', 'label'],
    ]
    for (const tx of txs) {
      rows.push([
        tx.txid,
        tx.timestamp ? new Date(tx.timestamp).toISOString().split('T')[0] : '',
        tx.amount_sats >= 0 ? 'received' : 'sent',
        Math.abs(tx.amount_sats),
        (Math.abs(tx.amount_sats) / 1e8).toFixed(8),
        tx.fee_sats ?? '',
        tx.confirmations,
        $labels[tx.txid] ?? '',
      ])
    }
    const csv = rows.map(r => r.map(v => `"${String(v).replace(/"/g, '""').replace(/[\r\n]/g, ' ')}"`).join(',')).join('\n')
    const blob = new Blob([csv], { type: 'text/csv' })
    downloadBlob(blob, `${wallet.label.replace(/[^a-z0-9]/gi, '_')}_transactions.csv`)
  }

  /// Download the wallet's external (+ change) descriptor as a .txt file.
  function exportDescriptor() {
    const lines = [
      `# Corvin wallet descriptor export`,
      `# Wallet: ${wallet.label}`,
      `# Kind:   ${wallet.kind}`,
      ``,
      `# External (receive) descriptor`,
      wallet.external_descriptor,
    ]
    if (wallet.internal_descriptor) {
      lines.push(``, `# Internal (change) descriptor`, wallet.internal_descriptor)
    }
    const blob = new Blob([lines.join('\n') + '\n'], { type: 'text/plain' })
    downloadBlob(blob, `${wallet.label.replace(/[^a-z0-9]/gi, '_')}_descriptor.txt`)
  }

  // SP wallets have no descriptor; their portable watch-only artifact is the
  // scan secret + spend pubkey, which re-import via Add wallet → Silent
  // Payments → Watch-only.
  async function exportSpKeys() {
    try {
      const k = await api.silentPayments.exportKeys(wallet.id)
      const lines = [
        `# Corvin Silent Payments watch-only export`,
        `# Wallet: ${wallet.label}`,
        `# Import via: Add wallet → Silent Payments → Watch-only`,
        `# WARNING: the scan secret reveals every payment received to this wallet.`,
        ``,
        `Network:      ${k.network}`,
        `Address:      ${k.address}`,
        `Scan secret:  ${k.scan_secret_hex}`,
        `Spend pubkey: ${k.spend_pubkey_hex}`,
      ]
      const blob = new Blob([lines.join('\n') + '\n'], { type: 'text/plain' })
      downloadBlob(blob, `${wallet.label.replace(/[^a-z0-9]/gi, '_')}_sp_watchonly.txt`)
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Export failed')
    }
  }

  /// Download the Coldcard-format multisig setup file. Cosigners load this on
  /// their devices once to register the wallet, then cooperative signing can
  /// proceed via the normal PSBT flow.
  async function exportMultisigConfig() {
    try {
      const res = await fetch(`/api/wallets/${wallet.id}/multisig-config`)
      if (!res.ok) {
        const err = await res.json().catch(() => ({}))
        throw new Error(err.error ?? `Server error (${res.status})`)
      }
      const blob = await res.blob()
      const slug = wallet.label.replace(/[^a-z0-9]/gi, '_').toLowerCase()
      const m = wallet.threshold ?? 0
      downloadBlob(blob, `${slug}-${m}-of-N-multisig.txt`)
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to export multisig config'
    }
  }

  // Balance formatting — split amount and unit for independent styling
  let balanceParts = $derived.by(() => {
    if (!balance) return null
    const sats = balance.confirmed_sats
    if ($displayUnit === 'btc') {
      const btc = (sats / 1e8).toFixed(8)
      return { amount: btc, unit: 'BTC' }
    }
    return { amount: sats.toLocaleString(), unit: 'sats' }
  })

  function formatSyncTime(ts: string): string {
    const diff = Math.floor((Date.now() - new Date(ts).getTime()) / 60_000)
    if (diff < 1) return 'synced just now'
    if (diff < 60) return `synced ${diff}m ago`
    const h = Math.floor(diff / 60)
    if (h < 24) return `synced ${h}h ago`
    return `synced ${new Date(ts).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}`
  }

  let txReplacements = $state<Map<string, string>>(new Map())

  $effect(() => {
    const url = $mempoolUrl
    const unconfirmed = txs.filter(t => t.confirmations === 0)
    if (!url || unconfirmed.length === 0) { txReplacements = new Map(); return }
    let cancelled = false
    const controller = new AbortController()
    ;(async () => {
      const results = await Promise.allSettled(
        unconfirmed.map(async t => {
          try {
            const r = await fetch(`/api/proxy/tx-rbf/${t.txid}`, { signal: controller.signal })
            if (!r.ok) return null
            const json = await r.json() as { replacements?: { tx?: { txid?: string } } }
            const rid = json?.replacements?.tx?.txid
            return rid ? { txid: t.txid, rid } : null
          } catch { return null }
        })
      )
      if (cancelled) return
      const map = new Map<string, string>()
      for (const r of results) {
        if (r.status === 'fulfilled' && r.value) map.set(r.value.txid, r.value.rid)
      }
      txReplacements = map
    })()
    return () => { cancelled = true; controller.abort() }
  })

  let activeTabId = $derived('tab-' + tab)
  $effect(() => {
    const q = searchQuery
    const t = setTimeout(() => { debouncedQuery = q }, 180)
    return () => clearTimeout(t)
  })
  let parsedSearch = $derived(debouncedQuery.trim() ? parseQuery(debouncedQuery.trim()) : null)
  let filteredTxs = $derived(parsedSearch ? filterTxs(txs, parsedSearch, $labels) : txs)

  interface TxGroup { label: string; txs: TxRecord[] }
  let txGroups = $derived.by((): TxGroup[] => {
    const groups: TxGroup[] = []
    let current = ''
    for (const tx of filteredTxs) {
      const label = tx.timestamp
        ? new Date(tx.timestamp).toLocaleDateString(undefined, { month: 'long', year: 'numeric' })
        : 'Unconfirmed'
      if (label !== current) { current = label; groups.push({ label, txs: [] }) }
      groups[groups.length - 1].txs.push(tx)
    }
    return groups
  })

  $effect(() => {
    void wallet // reactive dependency: reset when the active wallet changes
    balance = null
    txs = []
    addresses = []
    utxos = []
    balanceHistory = []
    addressLabels.set({})
    error = ''
    tab = 'txs'
    searchQuery = ''
    debouncedQuery = ''
    deleteOpen = false
    editing = false
    renameError = ''
    selectedTx = null
    consolidateMode = false
    consolidateUtxos = null
    showSend = false
    const gen = ++dataGen
    loadData(gen)
    if (wallet.internal_descriptor) loadAddresses(gen)
    // Auto-sync a wallet the first time it's opened this session (so data is
    // fresh after a restart), then leave it to the subscriber's push/periodic
    // sync. Avoids a redundant Electrum round-trip every time you switch wallets
    // or return from settings. Marked on fire so rapid re-opens before the
    // sync_complete arrives don't double-fire. Skipped in offline mode.
    if (!get(offline) && !hasWalletSyncedThisSession(wallet.id)) {
      const wid = wallet.id
      markWalletSynced(wid)
      api.wallets.sync(wid).catch(() => unmarkWalletSynced(wid))
    }
  })

  $effect(() => {
    if ($lastSyncComplete?.id === wallet.id) {
      const gen = ++dataGen
      loadData(gen)
      addresses = []; utxos = []; balanceHistory = []
      if (wallet.internal_descriptor) loadAddresses(gen)
    }
  })

  $effect(() => {
    if (tab === 'utxos' && utxos.length === 0) loadUtxos(dataGen)
    if (tab === 'charts' && balanceHistory.length === 0) loadBalanceHistory(dataGen)
    if (tab === 'addresses' && addresses.length === 0) loadAddresses(dataGen)
  })

  // Re-read every 30s + on tab focus, as a fallback for missed SSE events.
  $effect(() => {
    const timer = setInterval(() => { loadData(dataGen); refreshConn() }, 30_000)
    function onVisible() {
      if (document.visibilityState === 'visible') { loadData(dataGen); refreshConn() }
    }
    function onOnline() { loadData(dataGen); refreshConn() }
    document.addEventListener('visibilitychange', onVisible)
    window.addEventListener('online', onOnline)
    return () => {
      clearInterval(timer)
      document.removeEventListener('visibilitychange', onVisible)
      window.removeEventListener('online', onOnline)
    }
  })
</script>

<div class="detail">

  <!-- Wallet identity -->
  <div class="wallet-header">
    <div class="wallet-identity">
      {#if editing}
        <form class="rename-form" onsubmit={(e) => { e.preventDefault(); doRename() }}>
          <label for="wallet-rename-input" class="sr-only">Wallet name</label>
          <input
            id="wallet-rename-input"
            class="rename-input"
            bind:value={newLabel}
            bind:this={renameInputEl}
            disabled={renaming}
            onkeydown={(e) => e.key === 'Escape' && cancelEdit()}
          />
          <button type="submit" class="rename-btn confirm" disabled={renaming} aria-label="Save">✓</button>
          <button type="button" class="rename-btn cancel" onclick={cancelEdit} aria-label="Cancel">✕</button>
        </form>
        {#if renameError}<span class="rename-error">{renameError}</span>{/if}
      {:else}
        <div class="title-row">
          <h1 class="wallet-name">{wallet.label}</h1>
          <button
            class="info-btn"
            onclick={() => showDetails = true}
            title="Wallet details"
            aria-label="Show wallet details"
          >ⓘ</button>
        </div>
      {/if}
    </div>

    <div class="header-right">
      <button class="receive-btn" onclick={() => showReceive = true}>↓ Receive</button>
      {#if wallet.internal_descriptor !== null && wallet.kind !== 'silent_payments'}
        <button class="send-btn" onclick={() => showSend = true}>↑ Send</button>
      {:else if wallet.kind === 'silent_payments'}
        <button class="send-btn" onclick={() => showSpSpend = true}>↑ Send</button>
      {/if}

      <WalletMenu
        {wallet}
        syncing={$syncing.has(wallet.id)}
        offline={$offline}
        {canAddAnotherAccount}
        onSync={sync}
        onRename={startEdit}
        onChangeBackend={() => changeBackendOpen = true}
        onBroadcast={() => broadcastOpen = true}
        onConsolidate={() => { tab = 'utxos'; consolidateMode = true }}
        onAddAccount={() => addAccountOpen = true}
        onDelete={() => deleteOpen = true}
      />
    </div>
  </div>

  <!-- Balance hero -->
  <BalanceHero
    {balance}
    {balanceParts}
    {fiatText}
    {conn}
    {serverLabel}
    syncing={$syncing.has(wallet.id)}
    {syncElapsedSec}
    {feeDisplay}
  />

  {#if backendDown}
    <div class="degraded-banner" role="status">
      <span class="degraded-icon" aria-hidden="true">⚠</span>
      <span class="degraded-text">
        Can't reach {serverLabel}{conn?.error ? ` (${conn.error})` : ''}.
        {#if balance?.last_synced}Showing data from your last sync ({formatSyncTime(balance.last_synced)}).{:else}No synced data yet.{/if}
        You can still build, sign, and export transactions — syncing and broadcasting resume when the connection returns.
      </span>
      <button class="btn-secondary degraded-retry" onclick={sync} disabled={$syncing.has(wallet.id)}>Retry</button>
    </div>
  {/if}

  {#if error}
    <div class="error-banner" role="alert">
      <span class="error-icon">⚠</span>
      <span class="error-text">{error}</span>
      <button class="error-dismiss" onclick={() => error = ''} aria-label="Dismiss">✕</button>
    </div>
  {/if}

  <!-- Tabs -->
  <div class="tabs" role="tablist" aria-label="Wallet views" tabindex="-1"
    onkeydown={(e) => {
      const tabOrder = wallet.internal_descriptor !== null ? ['txs', 'addresses', 'utxos', 'charts', 'tools'] : ['txs', 'charts', 'tools']
      const idx = tabOrder.indexOf(tab)
      if (e.key === 'ArrowRight') { const next = tabOrder[(idx + 1) % tabOrder.length]; tab = next as typeof tab; e.preventDefault() }
      else if (e.key === 'ArrowLeft') { const prev = tabOrder[(idx - 1 + tabOrder.length) % tabOrder.length]; tab = prev as typeof tab; e.preventDefault() }
    }}
  >
    <button id="tab-txs" role="tab" aria-selected={tab === 'txs'} class:active={tab === 'txs'} tabindex={tab === 'txs' ? 0 : -1} onclick={() => tab = 'txs'}>
      Transactions {#if txs.length}<span class="tab-count">{txs.length}</span>{/if}
    </button>
    {#if wallet.internal_descriptor !== null}
      <button id="tab-addresses" role="tab" aria-selected={tab === 'addresses'} class:active={tab === 'addresses'} tabindex={tab === 'addresses' ? 0 : -1} onclick={() => tab = 'addresses'}>
        Addresses {#if addresses.length}<span class="tab-count">{addresses.length}</span>{/if}
      </button>
      <button id="tab-utxos" role="tab" aria-selected={tab === 'utxos'} class:active={tab === 'utxos'} tabindex={tab === 'utxos' ? 0 : -1} onclick={() => tab = 'utxos'}>
        UTXOs {#if utxos.length}<span class="tab-count">{utxos.length}</span>{/if}
      </button>
    {/if}
    <button id="tab-charts" role="tab" aria-selected={tab === 'charts'} class:active={tab === 'charts'} tabindex={tab === 'charts' ? 0 : -1} onclick={() => tab = 'charts'}>Charts</button>
    <button id="tab-tools" role="tab" aria-selected={tab === 'tools'} class:active={tab === 'tools'} tabindex={tab === 'tools' ? 0 : -1} onclick={() => tab = 'tools'}>Tools</button>
  </div>

  <!-- Tab content -->
  <div class="tab-content" role="tabpanel" aria-labelledby={activeTabId}>
    {#if tab === 'txs'}
      {#if txs.length > 0}
        <TxSearchBar bind:query={searchQuery} summary={parsedSearch?.summary ?? null} />
      {/if}
      {#if txs.length === 0}
        {#if $syncing.has(wallet.id) || !balance?.last_synced}
          <EmptyState icon="sync" title="Looking for transactions…" description="Corvin is scanning the chain for this wallet's history. New wallets can take a moment on the first sync." />
        {:else}
          <EmptyState
            icon="inbox"
            title="No transactions yet"
            description={wallet.internal_descriptor === null
              ? "This watch-only wallet has no activity yet. It will show transactions once its address receives or spends coins."
              : "Once you receive Bitcoin to this wallet, it'll show up here."}
          >
            {#snippet action()}
              {#if wallet.internal_descriptor !== null || wallet.kind === 'address' || wallet.kind === 'silent_payments'}
                <button class="es-btn" onclick={() => showReceive = true}>↓ Receive</button>
              {/if}
            {/snippet}
          </EmptyState>
        {/if}
      {:else if filteredTxs.length === 0}
        <EmptyState icon="search" compact title="No matching transactions" description="No transactions match your search. Try a different term or clear the search." />
      {:else}
        {#each txGroups as group (group.label)}
          <div class="tx-group-header">{group.label}</div>
          {#each group.txs as tx (tx.txid)}
            <TxRow {tx} replacedBy={txReplacements.get(tx.txid) ?? null} onSelect={(t) => selectedTx = t} />
          {/each}
        {/each}
      {/if}
    {:else if tab === 'addresses'}
      <AddressTable {addresses} {utxos} {wallet} />
    {:else if tab === 'utxos'}
      <UtxoTable
        {utxos}
        {feeRates}
        {consolidateMode}
        addressLabels={$addressLabels}
        canConsolidate={wallet.internal_descriptor !== null}
        onConsolidate={(selected) => { consolidateMode = false; consolidateUtxos = selected }}
        onStartConsolidate={() => { consolidateMode = true }}
        onCancelConsolidate={() => consolidateMode = false}
      />
    {:else if tab === 'charts'}
      <BalanceChart points={balanceHistory} txs={txs} utxos={utxos} />
    {:else if tab === 'tools'}
      <WalletToolsTab
        {wallet}
        txCount={txs.length}
        {canTestBackup}
        onTaxReport={() => goto(`/wallet/${wallet.id}/tax-report`)}
        onExportCsv={exportCsv}
        onExportDescriptor={exportDescriptor}
        onExportSpKeys={exportSpKeys}
        onExportMultisigConfig={exportMultisigConfig}
        onAddressLookup={() => { addressLookupOpen = true; if (addresses.length === 0) loadAddresses(dataGen) }}
        onPsbtInspector={() => { psbtInspectorOpen = true; if (addresses.length === 0) loadAddresses(dataGen) }}
        onSignVerify={() => { messagesOpen = true; if (addresses.length === 0) loadAddresses(dataGen) }}
        onTestBackup={() => testBackupOpen = true}
        onSweepWif={() => { sweepWifOpen = true; if (addresses.length === 0) loadAddresses(dataGen) }}
      />
    {/if}
  </div>
</div>

{#if selectedTx}
  <TxDetailPanel tx={selectedTx} walletId={wallet.id} {feeRates} {utxos} onClose={() => selectedTx = null} />
{/if}

{#if showReceive}
  <ReceiveModal {wallet} addressLabels={$addressLabels} onClose={() => showReceive = false} />
{/if}

{#if showSend}
  <SendFlow {wallet} {utxos} {feeRates} addressLabels={$addressLabels} onClose={() => showSend = false} />
{/if}

{#if showDetails}
  <WalletDetailsModal {wallet} onClose={() => showDetails = false} />
{/if}

{#if changeBackendOpen}
  <ChangeBackendModal
    {wallet}
    onClose={() => changeBackendOpen = false}
    onChanged={(updated) => wallets.update(ws => ws.map(w => w.id === updated.id ? updated : w))}
  />
{/if}

{#if deleteOpen}
  <DeleteWalletModal
    {wallet}
    confirmedSats={balance?.confirmed_sats ?? 0}
    onClose={() => deleteOpen = false}
    onConfirm={onDelete}
  />
{/if}

{#if messagesOpen}
  <MessagesModal {wallet} {addresses} onClose={() => messagesOpen = false} />
{/if}

{#if addAccountOpen}
  <AddAccountModal {wallet} onClose={() => addAccountOpen = false} />
{/if}

{#if testBackupOpen}
  <TestBackupModal {wallet} onClose={() => testBackupOpen = false} />
{/if}

{#if addressLookupOpen}
  <AddressLookupModal {wallet} {addresses} onClose={() => addressLookupOpen = false} />
{/if}

{#if sweepWifOpen}
  <SweepWifModal {wallet} {addresses} onClose={() => sweepWifOpen = false} />
{/if}

{#if psbtInspectorOpen}
  <PsbtInspectorModal {wallet} {addresses} {utxos} onClose={() => psbtInspectorOpen = false} />
{/if}

{#if broadcastOpen}
  <BroadcastModal onClose={() => broadcastOpen = false} />
{/if}

{#if consolidateUtxos !== null}
  <ConsolidateModal
    {wallet}
    utxos={consolidateUtxos}
    {feeRates}
    onClose={() => consolidateUtxos = null}
  />
{/if}


{#if showSpSpend}
  <SpSpendModal
    {wallet}
    {utxos}
    {feeRates}
    onClose={() => showSpSpend = false}
    onBroadcast={() => { showSpSpend = false; lastSyncComplete.set({ id: wallet.id, at: Date.now() }) }}
  />
{/if}

<style>
  .detail { flex: 1; padding: 28px 32px 20px; overflow-y: auto; }

  /* Wallet identity row */
  .wallet-header {
    display: flex; align-items: center; justify-content: space-between;
    margin-bottom: 20px; gap: 12px;
  }
  .wallet-identity { display: flex; flex-direction: column; gap: 4px; }
  .title-row { display: flex; align-items: center; gap: 6px; }
  .wallet-name { margin: 0; font-size: 1.1rem; font-weight: 700; color: var(--text); }
  .info-btn {
    background: none; border: none; cursor: pointer;
    color: var(--text-muted); font-size: 0.95rem; line-height: 1;
    padding: 2px 4px; border-radius: 3px;
    transition: color 0.12s, background 0.12s;
  }
  .info-btn:hover { color: var(--accent); background: var(--surface-2); }
  .info-btn:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }

  .header-right { display: flex; align-items: center; gap: 8px; }

  /* Receive and Send are equal-weight peers in the wallet header — both
     filled accent. Pre-rename the styles were asymmetric (Send was outline),
     but the user has chosen to treat them as equally important actions. */
  .receive-btn, .send-btn {
    background: transparent; color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 5px; padding: 6px 14px; cursor: pointer;
    font-weight: 600; font-size: 0.82rem; letter-spacing: 0.02em;
    transition: background 0.12s, color 0.12s;
  }
  .receive-btn:hover, .send-btn:hover {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }

  .error-banner {
    display: flex; align-items: flex-start; gap: 10px;
    margin: 0 0 14px; padding: 8px 12px;
    background: color-mix(in srgb, #e05252 8%, var(--surface-1));
    border: 1px solid color-mix(in srgb, #e05252 30%, var(--border));
    border-radius: 5px;
    font-size: 0.8rem; color: var(--text); line-height: 1.5;
  }
  .error-icon { color: #e05252; font-size: 1rem; line-height: 1; flex-shrink: 0; }
  .error-text { flex: 1; min-width: 0; }
  .error-dismiss {
    background: none; border: none; cursor: pointer;
    color: var(--text-muted); font-size: 0.85rem; padding: 0 4px;
    flex-shrink: 0;
  }
  .error-dismiss:hover { color: var(--text); }

  /* Softer amber treatment than the red error banner — this is a degraded state,
     not an error, and the wallet stays fully usable for signing/export. */
  .degraded-banner {
    display: flex; align-items: center; gap: 10px;
    margin: 0 0 14px; padding: 8px 12px;
    background: color-mix(in srgb, #d99a2b 8%, var(--surface-1));
    border: 1px solid color-mix(in srgb, #d99a2b 30%, var(--border));
    border-radius: 5px;
    font-size: 0.8rem; color: var(--text); line-height: 1.5;
  }
  .degraded-icon { color: #d99a2b; font-size: 1rem; line-height: 1; flex-shrink: 0; }
  .degraded-text { flex: 1; min-width: 0; }
  .degraded-retry { flex-shrink: 0; padding: 4px 12px; font-size: 0.78rem; }

  /* Tabs */
  .tabs {
    display: flex; gap: 2px; border-bottom: 1px solid var(--border); margin-bottom: 14px;
    overflow-x: auto; -webkit-overflow-scrolling: touch; scrollbar-width: none;
  }
  .tabs::-webkit-scrollbar { display: none; }
  .tabs button {
    background: none; border: none; border-bottom: 2px solid transparent;
    padding: 7px 12px; cursor: pointer; font-size: 0.82rem; color: var(--text);
    margin-bottom: -1px; display: flex; align-items: center; gap: 5px;
    flex-shrink: 0;
  }
  .tabs button.active { color: var(--accent); border-bottom-color: var(--accent); font-weight: 600; }
  .tab-count {
    background: var(--surface-2); border-radius: 10px;
    padding: 1px 6px; font-size: 0.7rem; color: var(--text-muted); font-weight: 400;
  }
  .tabs button.active .tab-count { background: color-mix(in srgb, var(--accent) 15%, transparent); color: var(--accent); }

  /* Transaction groups */
  .tx-group-header {
    font-size: 0.67rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.08em;
    color: var(--text-muted); padding: 14px 0 5px;
  }
  .tx-group-header:first-child { padding-top: 0; }

  .es-btn {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent); border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
    border-radius: 6px; padding: 8px 16px; cursor: pointer; font-size: 0.88rem; font-weight: 600;
  }
  .es-btn:hover { background: color-mix(in srgb, var(--accent) 22%, transparent); }

  /* Inline rename */
  .rename-form { display: flex; align-items: center; gap: 5px; }
  .rename-input {
    background: var(--surface-2); border: 1px solid var(--accent); border-radius: 4px;
    color: var(--text); padding: 3px 8px; font-size: 1.05rem; font-weight: 700;
    flex: 1; min-width: 0; outline: none;
  }
  .rename-input:disabled { opacity: 0.6; }
  .rename-btn {
    background: none; border: 1px solid var(--border); border-radius: 4px;
    cursor: pointer; padding: 3px 8px; font-size: 0.8rem;
    color: var(--text-muted); flex-shrink: 0; line-height: 1.4;
  }
  .rename-btn.confirm:hover { border-color: #52a875; color: #52a875; }
  .rename-btn.cancel:hover { border-color: var(--text-muted); color: var(--text); }
  .rename-btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .rename-error { font-size: 0.75rem; color: var(--error); display: block; margin-top: 3px; }

  @media (max-width: 768px) {
    .detail { padding: 16px 14px; }

    /* MobileLayout header already shows the wallet name — hide the duplicate */
    .wallet-identity { display: none; }
    .wallet-header { margin-bottom: 14px; justify-content: flex-end; }

    /* Bigger tap targets */
    .send-btn, .receive-btn { padding: 8px 16px; font-size: 0.85rem; }
  }
</style>
