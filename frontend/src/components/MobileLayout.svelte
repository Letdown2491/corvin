<script lang="ts">
  import { onMount } from 'svelte'
  import { fly } from 'svelte/transition'
  import { cubicOut } from 'svelte/easing'
  import { goto } from '$app/navigation'
  import { page } from '$app/stores'
  import { wallets, activeWalletId } from '../stores/wallets'
  import { nodeStatus } from '../stores/settings'
  import { slideDir } from '../stores/ui'
  import { api } from '../lib/api'

  let { children } = $props()

  let currentPath = $derived($page.url.pathname)

  let activeTab = $derived(currentPath.startsWith('/settings') ? 'settings' : 'wallets')

  let canGoBack = $derived(currentPath !== '/' && currentPath !== '/settings')

  let headerTitle = $derived.by((): string => {
    if (currentPath === '/') return 'Corvin'
    if (currentPath.startsWith('/wallet/')) {
      const id = currentPath.split('/')[2]
      return $wallets.find(w => w.id === id)?.label ?? 'Wallet'
    }
    if (currentPath === '/add-wallet') return 'Add wallet'
    if (currentPath === '/settings') return 'Settings'
    if (currentPath === '/settings/backend') return 'Backend'
    if (currentPath === '/settings/display') return 'Display'
    if (currentPath === '/settings/import-export') return 'Import / Export'
    if (currentPath.startsWith('/help')) return 'Help'
    return 'Corvin'
  })

  function goBack() {
    slideDir.set(-1)
    history.back()
  }

  function switchTab(tab: 'wallets' | 'settings') {
    slideDir.set(0)
    if (tab === 'wallets') {
      const id = $activeWalletId
      goto(id ? `/wallet/${id}` : '/')
    } else {
      goto('/settings')
    }
  }

  async function refreshStatus() {
    try { nodeStatus.set(await api.status()) }
    catch { nodeStatus.set(null) }
  }

  onMount(() => {
    refreshStatus()
    const t = setInterval(refreshStatus, 30_000)
    return () => clearInterval(t)
  })

  const SLIDE_PX = 340
  const SLIDE_MS = 260
  const FADE_MS  = 160

  function flyIn()  { return { x: $slideDir * SLIDE_PX,  duration: $slideDir === 0 ? FADE_MS : SLIDE_MS, opacity: $slideDir === 0 ? 0 : 1, easing: cubicOut } }
  function flyOut() { return { x: -$slideDir * SLIDE_PX, duration: $slideDir === 0 ? FADE_MS : SLIDE_MS, opacity: $slideDir === 0 ? 0 : 1, easing: cubicOut } }

  let showBottomNav = $derived(currentPath !== '/add-wallet')
</script>

<div class="mobile-app">
  <header class="mobile-header">
    <div class="header-side left">
      {#if canGoBack}
        <button class="back-btn" onclick={goBack} aria-label="Back">‹</button>
      {/if}
    </div>

    <span class="header-title">{headerTitle}</span>

    <div class="header-side right">
      {#if currentPath === '/' || currentPath.startsWith('/wallet/')}
        <span
          role="img"
          class="conn-dot"
          class:green={$nodeStatus?.connected}
          class:red={$nodeStatus && !$nodeStatus.connected}
          aria-label={$nodeStatus?.connected ? 'Connected' : 'Disconnected'}
        ></span>
      {/if}
    </div>
  </header>

  <main class="mobile-content">
    {#key currentPath}
      <div class="screen-slide"
        in:fly={flyIn()}
        out:fly={flyOut()}
      >
        {@render children()}
      </div>
    {/key}
  </main>

  {#if showBottomNav}
    <nav class="mobile-nav" aria-label="Main navigation">
      <button class="nav-btn" class:active={activeTab === 'wallets'} onclick={() => switchTab('wallets')}>
        <span class="nav-icon" aria-hidden="true">◈</span>
        <span class="nav-label">Wallets</span>
      </button>
      <button class="nav-btn" class:active={activeTab === 'settings'} onclick={() => switchTab('settings')}>
        <span class="nav-icon" aria-hidden="true">⊙</span>
        <span class="nav-label">Settings</span>
      </button>
    </nav>
  {/if}
</div>

<style>
  .mobile-app {
    display: flex; flex-direction: column;
    height: 100dvh;
    background: var(--surface-2); overflow: hidden;
  }

  .mobile-header {
    flex-shrink: 0;
    height: 52px;
    padding-top: env(safe-area-inset-top);
    background: var(--surface-1);
    border-bottom: 1px solid var(--border);
    display: flex; align-items: center;
  }

  .header-side {
    width: 56px; flex-shrink: 0;
    display: flex; align-items: center;
  }
  .header-side.left  { justify-content: flex-start; }
  .header-side.right { justify-content: flex-end; padding-right: 16px; }

  .header-title {
    flex: 1; text-align: center;
    font-size: 1rem; font-weight: 700; color: var(--text);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    padding: 0 4px;
  }

  .back-btn {
    background: none; border: none; cursor: pointer;
    color: var(--accent); font-size: 2.2rem; line-height: 1;
    padding: 0 4px 0 12px; height: 52px;
    display: flex; align-items: center;
  }

  .conn-dot {
    width: 8px; height: 8px; border-radius: 50%; background: var(--border);
  }
  .conn-dot.green { background: #52a875; box-shadow: 0 0 5px #52a87566; }
  .conn-dot.red   { background: #e05252; }

  .mobile-content {
    flex: 1;
    position: relative;
    overflow: hidden;
  }

  .screen-slide {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    overflow-x: hidden;
    -webkit-overflow-scrolling: touch;
  }

  .mobile-nav {
    flex-shrink: 0;
    padding-bottom: env(safe-area-inset-bottom);
    background: var(--surface-1);
    border-top: 1px solid var(--border);
    display: flex;
  }

  .nav-btn {
    flex: 1; background: none; border: none; cursor: pointer;
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: 3px; padding: 10px 0;
    color: var(--text-muted);
    transition: color 0.1s;
  }
  .nav-btn.active { color: var(--accent); }

  .nav-icon  { font-size: 1.3rem; line-height: 1; }
  .nav-label { font-size: 0.65rem; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }
</style>
