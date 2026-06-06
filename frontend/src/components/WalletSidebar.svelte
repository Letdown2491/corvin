<script lang="ts">
  import { onMount } from 'svelte'
  import { goto } from '$app/navigation'
  import { page } from '$app/stores'
  import type { BackendStatusEntry, WalletEntry } from '../lib/types'
  import { activeWalletId, syncing } from '../stores/wallets'
  import { defaultWalletId, nodeStatus, nodeStatusAt, feeRates, feeRatesAt, backendStatuses, spStatuses, showCurrentPrice, currentBtcPrice, offline } from '../stores/settings'
  import { api } from '../lib/api'
  import { kindLabel } from '../lib/utils'

  let { wallets }: { wallets: WalletEntry[] } = $props()

  // Both refreshers keep the last-known value on transient failures
  // so a single bad poll doesn't blank the sidebar. `nodeStatus` / `feeRates`
  // feed other components (wallet page, send flow); the sidebar itself only
  // renders the per-wallet dots below.
  async function refreshStatus() {
    try {
      const s = await api.status()
      nodeStatus.set(s)
      nodeStatusAt.set(Date.now())
    } catch { /* keep last-known */ }
    try {
      backendStatuses.set(await api.backends.status())
    } catch { /* keep last-known */ }
    try {
      spStatuses.set(await api.silentPayments.status())
    } catch { /* keep last-known */ }
  }

  async function refreshFees() {
    try {
      const f = await api.proxy.fees()
      if (f === null) {
        feeRates.set(null) // mempool unconfigured — nothing to show
      } else {
        feeRates.set(f)
        feeRatesAt.set(Date.now())
      }
    } catch { /* keep last-known */ }
  }

  function refreshAll() { refreshStatus(); refreshFees() }

  // Fire-and-forget kick to break the backend out of backoff after wake.
  function kickAndRefresh() {
    api.reconnect().catch(() => { /* best-effort */ })
    refreshAll()
  }

  onMount(() => {
    refreshAll()
    const tStatus = setInterval(refreshStatus, 30_000)
    const tFees = setInterval(refreshFees, 60_000)

    // Re-poll immediately on tab focus / network resume.
    function onVisible() { if (document.visibilityState === 'visible') kickAndRefresh() }
    document.addEventListener('visibilitychange', onVisible)
    window.addEventListener('online', kickAndRefresh)

    return () => {
      clearInterval(tStatus); clearInterval(tFees)
      document.removeEventListener('visibilitychange', onVisible)
      window.removeEventListener('online', kickAndRefresh)
    }
  })

  let sorted = $derived.by(() => {
    const def = $defaultWalletId
    return [...wallets].sort((a, b) => {
      if (a.id === def) return -1
      if (b.id === def) return 1
      return a.label.localeCompare(b.label)
    })
  })

  // Live price of 1 BTC (global), shown in the footer when enabled in Display.
  let btcPriceDisplay = $derived(
    $showCurrentPrice && $currentBtcPrice
      ? `$${$currentBtcPrice.toLocaleString('en-US', { maximumFractionDigits: 0 })}`
      : null
  )

  // Per-backend live status, keyed by backend id ('' = default backend).
  let statusByKey = $derived.by(() => {
    const m = new Map<string, BackendStatusEntry>()
    for (const s of $backendStatuses) m.set(s.backend ?? '', s)
    return m
  })
  function walletConn(w: WalletEntry): BackendStatusEntry | undefined {
    // SP wallets sync via their Frigate scanner, not the Electrum backend, so their
    // status is keyed by wallet id in spStatuses (not by w.backend).
    if (w.kind === 'silent_payments') {
      const sp = $spStatuses.find(s => s.wallet_id === w.id)
      return sp ? { backend: null, connected: sp.connected, tip_height: null, error: sp.error } : undefined
    }
    return statusByKey.get(w.backend ?? '')
  }
  function connTitle(st: BackendStatusEntry | undefined): string {
    if (!st) return 'Connection status unknown'
    if (st.connected) return `Connected${st.tip_height ? ' · block ' + st.tip_height.toLocaleString() : ''}`
    return `Disconnected${st.error ? ': ' + st.error : ''}`
  }

  function toggleDefault(e: MouseEvent, id: string) {
    e.stopPropagation()
    defaultWalletId.update(cur => cur === id ? null : id)
  }

  let currentPath = $derived($page.url.pathname)
  let serverActive = $derived(currentPath.startsWith('/settings/backend'))
  let displayActive = $derived(currentPath.startsWith('/settings/display'))
  let importExportActive = $derived(currentPath.startsWith('/settings/import-export'))
  let securityActive = $derived(currentPath.startsWith('/settings/security'))
  let helpActive = $derived(currentPath.startsWith('/help'))

  function handleWalletClick(id: string) {
    activeWalletId.set(id)
    goto(`/wallet/${id}`)
  }
</script>

<aside class="sidebar">
  <div class="sidebar-header">
    <span class="brand">Corvin</span>
  </div>

  {#if $offline}
    <div class="offline-banner" role="status" title="Offline mode is on (Backend settings)">
      <span class="offline-icon" aria-hidden="true">⦸</span> Offline mode
    </div>
  {/if}

  <nav class="wallet-list" aria-label="Wallets">
    {#each sorted as w (w.id)}
      {@const st = walletConn(w)}
      <div class="wallet-row" class:is-default={$defaultWalletId === w.id}>
        <button
          class="wallet-item"
          class:active={$activeWalletId === w.id && !serverActive && !displayActive && !importExportActive && !securityActive}
          onclick={() => handleWalletClick(w.id)}
        >
          <span class="wallet-label">{w.label}</span>
          <div class="wallet-sub-row">
            {#if $syncing.has(w.id)}
              <span class="wallet-kind sync-text" role="status">
                <span class="sync-spinner" aria-hidden="true">⟳</span>
                Syncing…
              </span>
            {:else}
              <span class="wallet-kind">{kindLabel(w.kind)}</span>
            {/if}
            <span
              class="wallet-conn-dot"
              class:green={!$offline && st?.connected}
              class:red={!$offline && st && !st.connected}
              class:dim={$offline}
              title={$offline ? 'Offline mode' : connTitle(st)}
            ></span>
          </div>
        </button>
        <button
          class="star-btn"
          class:set={$defaultWalletId === w.id}
          onclick={(e) => toggleDefault(e, w.id)}
          title={$defaultWalletId === w.id ? 'Unpin default' : 'Set as default'}
          aria-label={$defaultWalletId === w.id ? 'Remove default' : 'Set as default'}
        >{$defaultWalletId === w.id ? '★' : '☆'}</button>
      </div>
    {/each}

    {#if wallets.length === 0}
      <p class="empty-hint">No wallets yet.</p>
    {/if}
  </nav>

  <div class="sidebar-footer">
    <button class="footer-btn" onclick={() => goto('/add-wallet')}>
      <span class="footer-icon">+</span>
      Add wallet
    </button>
    <button class="footer-btn" class:active={serverActive} onclick={() => goto('/settings/backend')}>
      <span class="footer-icon">◎</span>
      Backend
    </button>
    <button class="footer-btn" class:active={displayActive} onclick={() => goto('/settings/display')}>
      <span class="footer-icon">◑</span>
      Display
    </button>
    <button class="footer-btn" class:active={securityActive} onclick={() => goto('/settings/security')}>
      <span class="footer-icon">
        <svg viewBox="0 0 16 16" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <rect x="3.25" y="7" width="9.5" height="7" rx="1.5" />
          <path d="M5 7V5a3 3 0 0 1 6 0v2" />
        </svg>
      </span>
      Security
    </button>
    <button class="footer-btn" class:active={importExportActive} onclick={() => goto('/settings/import-export')}>
      <span class="footer-icon">⇅</span>
      Import / Export
    </button>
    <button class="footer-btn" class:active={helpActive} onclick={() => goto('/help')}>
      <span class="footer-icon">?</span>
      Help
    </button>
    <a class="footer-btn" href="/bitcoin.pdf" target="_blank" rel="noopener noreferrer">
      <span class="footer-icon">
        <svg viewBox="0 0 16 16" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M4 1.75h4.5L12.25 5.5v8.75H4z" />
          <path d="M8.25 1.75V5.5h3.75" />
          <path d="M5.9 9h4.2M5.9 11.25h4.2" />
        </svg>
      </span>
      Whitepaper
    </a>

    {#if btcPriceDisplay}
      <div class="price-row" title="Current price of 1 BTC">
        <span class="price-icon" aria-hidden="true">₿</span>
        <span class="price-value">{btcPriceDisplay}</span>
      </div>
    {/if}
  </div>
</aside>

<style>
  .sidebar {
    width: 210px;
    height: 100vh;
    background: var(--surface-1);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
  }

  .sidebar-header {
    padding: 16px 14px 14px;
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .brand {
    font-weight: 800;
    font-size: 1rem;
    letter-spacing: 0.06em;
    color: var(--accent);
  }


  .wallet-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px 0;
  }

  .wallet-row {
    position: relative;
    display: flex;
    align-items: stretch;
  }
  .wallet-row:hover .star-btn,
  .wallet-row.is-default .star-btn { opacity: 1; }

  .wallet-item {
    flex: 1;
    background: none;
    border: none;
    text-align: left;
    padding: 10px 32px 10px 14px;
    cursor: pointer;
    color: var(--text);
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-left: 3px solid transparent;
    min-width: 0;
  }
  .wallet-item:hover { background: var(--surface-hover); }
  .wallet-item.active {
    background: var(--surface-active);
    border-left-color: var(--accent);
    padding-left: 11px;
  }
  .wallet-label { font-size: 0.88rem; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .wallet-sub-row { display: flex; align-items: center; justify-content: space-between; gap: 6px; }
  .wallet-kind { font-size: 0.7rem; color: var(--text-muted); }
  .wallet-conn-dot {
    flex-shrink: 0;
    width: 6px; height: 6px; border-radius: 50%;
    background: var(--border);
  }
  .wallet-conn-dot.green { background: #52a875; }
  .wallet-conn-dot.red   { background: #e05252; }
  .wallet-conn-dot.dim   { background: var(--text-muted); opacity: 0.5; }

  .offline-banner {
    display: flex; align-items: center; gap: 6px;
    margin: 0 12px 8px; padding: 5px 10px;
    border: 1px solid var(--border); border-radius: 6px;
    background: var(--surface-2);
    color: var(--text-muted); font-size: 0.72rem; font-weight: 600;
    letter-spacing: 0.02em;
  }
  .offline-icon { font-size: 0.85rem; line-height: 1; }
  .sync-text {
    display: inline-flex; align-items: center; gap: 4px;
    color: var(--accent);
  }
  .sync-spinner {
    font-size: 0.85rem;
    animation: spin 1s linear infinite; display: inline-block;
  }
  @keyframes spin { to { transform: rotate(360deg); } }

  .star-btn {
    position: absolute;
    right: 6px;
    top: 50%;
    transform: translateY(-50%);
    background: none;
    border: none;
    cursor: pointer;
    padding: 4px;
    font-size: 0.72rem;
    line-height: 1;
    opacity: 0;
    transition: opacity 0.1s, color 0.1s;
    color: var(--text-muted);
    border-radius: 3px;
  }
  .star-btn:hover { color: var(--accent); background: var(--surface-2); }
  .star-btn.set { color: var(--accent); }

  .empty-hint { color: var(--text-muted); font-size: 0.8rem; padding: 16px 14px; }

  .sidebar-footer {
    border-top: 1px solid var(--border);
    padding: 4px 0;
  }
  .price-row {
    display: flex; align-items: center; gap: 9px;
    padding: 10px 14px 10px 11px; cursor: default;
    border-left: 3px solid transparent;
  }
  .price-icon {
    font-size: 1rem; line-height: 1; color: var(--text);
    width: 16px; text-align: center; flex-shrink: 0;
  }
  .price-value {
    font-size: 0.88rem; color: var(--text); font-variant-numeric: tabular-nums;
  }
  .footer-btn {
    width: 100%;
    background: none;
    border: none;
    border-left: 3px solid transparent;
    text-align: left;
    padding: 10px 14px 10px 11px;
    cursor: pointer;
    font-size: 0.88rem;
    font-weight: 500;
    color: var(--text);
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .footer-btn:hover { background: var(--surface-hover); }
  a.footer-btn { text-decoration: none; }
  .footer-btn.active {
    color: var(--accent);
    background: var(--surface-active);
    border-left-color: var(--accent);
  }
  .footer-icon {
    font-size: 1rem;
    line-height: 1;
    width: 16px;
    height: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
</style>
