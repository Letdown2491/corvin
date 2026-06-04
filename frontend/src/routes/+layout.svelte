<script lang="ts">
  import { onMount } from 'svelte'
  import { get } from 'svelte/store'
  import { beforeNavigate } from '$app/navigation'
  import { api, subscribeEvents } from '../lib/api'
  import { wallets, activeWalletId, syncing, lastSyncComplete, markWalletSynced } from '../stores/wallets'
  import { mempoolUrl, showPriceData, showCurrentPrice, showFiatBalance, currentBtcPrice, defaultWalletId, notificationsEnabled, hwEnabled } from '../stores/settings'
  import { addToast } from '../stores/toasts'
  import { lastPayjoinEvent } from '../stores/payjoin'
  import { slideDir } from '../stores/ui'
  import { serverReachable } from '../stores/server'
  import { isMobile } from '../lib/mobile'
  import '../stores/theme'
  import WalletSidebar from '../components/WalletSidebar.svelte'
  import ToastContainer from '../components/ToastContainer.svelte'
  import MobileLayout from '../components/MobileLayout.svelte'
  import OnboardingWizard from '../components/OnboardingWizard.svelte'
  import { showOnboarding } from '../stores/onboarding'
  import UnlockGate from '../components/UnlockGate.svelte'
  import { securityState } from '../stores/security'

  let { children } = $props()

  // Load wallets + settings. Deferred until after unlock when encryption is on.
  async function loadInitialData() {
    try {
      const [ws, s] = await Promise.all([api.wallets.list(), api.settings.get()])
      wallets.set(ws)
      if (ws.length > 0) {
        const defId = $defaultWalletId
        const target = (defId && ws.find((w: { id: string }) => w.id === defId)) ? defId : ws[0].id
        activeWalletId.set(target)
      }
      mempoolUrl.set(s.backend.mempool_url)
      showPriceData.set(s.backend.show_price_data)
      showCurrentPrice.set(s.backend.show_current_price)
      showFiatBalance.set(s.backend.show_fiat_balance)
      // Show the wizard until onboarding is completed or skipped. The flag is
      // the sole signal — finishing or skipping sets it, so it shows exactly
      // once (including for existing users upgrading into this version).
      if (!s.onboarding_complete) showOnboarding.set(true)
    } catch (e) {
      console.error('Failed to load initial data', e)
    }
  }

  function handleUnlocked() {
    securityState.set('unlocked')
    loadInitialData()
  }

  onMount(() => {

    ;(async () => {
      // Build capabilities (unauthenticated, so safe even while locked): hide the
      // USB hardware-wallet UI on builds without the `hw` feature (Start9/headless).
      api.version().then((v) => hwEnabled.set(v.hw_enabled)).catch(() => {})

      // Check the lock state first. When locked, the gate renders and the rest of
      // the init is deferred until unlock (protected API calls would 423 anyway).
      let state: 'off' | 'locked' | 'unlocked' = 'off'
      try {
        state = (await api.security.status()).state
      } catch {
        // Treat an unreachable status as "off" so the app still tries to load.
      }
      securityState.set(state)
      if (state !== 'locked') await loadInitialData()
    })()

    const unsubEvents = subscribeEvents({
      sync_started: (payload: unknown) => {
        const p = payload as { wallet_id: string }
        syncing.update(s => new Set(s).add(p.wallet_id))
      },
      sync_complete: (payload: unknown) => {
        const p = payload as { wallet_id: string; new_txs: number }
        syncing.update(s => { const n = new Set(s); n.delete(p.wallet_id); return n })
        // Mark as synced this session so opening this wallet won't redundantly
        // re-sync (the subscriber's startup sync counts too).
        markWalletSynced(p.wallet_id)
        lastSyncComplete.set({ id: p.wallet_id, at: Date.now() })
        if (p.new_txs > 0 && get(notificationsEnabled)) {
          const wallet = get(wallets).find((w: { id: string }) => w.id === p.wallet_id)
          const name = wallet?.label ?? 'Wallet'
          const count = p.new_txs
          const body = `${count} new transaction${count === 1 ? '' : 's'}`
          if (!document.hasFocus() && Notification.permission === 'granted') {
            new Notification(`Corvin — ${name}`, { body })
          } else {
            addToast(`${name}: ${body}`)
          }
        }
      },
      sp_output_discovered: (payload: unknown) => {
        // The SP scanner found a new BIP-352 output for `wallet_id`.
        // Bump lastSyncComplete so WalletDetail re-fetches the affected
        // wallet's data — same plumbing the regular sync flow uses, so SP
        // matches surface in the UI without any wallet-detail changes.
        const p = payload as { wallet_id: string; txid?: string; count?: number }
        lastSyncComplete.set({ id: p.wallet_id, at: Date.now() })
        if (get(notificationsEnabled)) {
          const wallet = get(wallets).find((w: { id: string }) => w.id === p.wallet_id)
          const name = wallet?.label ?? 'Wallet'
          const count = p.count ?? 1
          const body = `${count} new silent payment${count === 1 ? '' : 's'}`
          if (!document.hasFocus() && Notification.permission === 'granted') {
            new Notification(`Corvin — ${name}`, { body })
          } else {
            addToast(`${name}: ${body}`)
          }
        }
      },
      payjoin_proposal_ready: (payload: unknown) => {
        const p = payload as { wallet_id?: string; session_id: string }
        lastPayjoinEvent.set({ wallet_id: p.wallet_id, session_id: p.session_id, status: 'proposal_ready' })
      },
      payjoin_sent: (payload: unknown) => {
        const p = payload as { wallet_id?: string; session_id: string; txid: string }
        lastPayjoinEvent.set({ wallet_id: p.wallet_id, session_id: p.session_id, status: 'sent', txid: p.txid })
        addToast('Payjoin sent')
      },
      payjoin_fell_back: (payload: unknown) => {
        const p = payload as { wallet_id?: string; session_id: string; txid: string }
        lastPayjoinEvent.set({ wallet_id: p.wallet_id, session_id: p.session_id, status: 'fell_back', txid: p.txid })
        addToast('Payjoin timed out — sent the original transaction instead')
      },
      payjoin_receive_proposal: (payload: unknown) => {
        const p = payload as { wallet_id?: string; session_id: string }
        lastPayjoinEvent.set({ wallet_id: p.wallet_id, session_id: p.session_id, status: 'proposal_ready' })
        if (get(notificationsEnabled) && !document.hasFocus() && Notification.permission === 'granted') {
          new Notification('Corvin — Payjoin', { body: 'A payer paid your payjoin invoice — confirm to coordinate.' })
        } else {
          addToast('Payjoin payment received — confirm to coordinate')
        }
      },
      payjoin_receive_sent: (payload: unknown) => {
        const p = payload as { wallet_id?: string; session_id: string }
        lastPayjoinEvent.set({ wallet_id: p.wallet_id, session_id: p.session_id, status: 'sent' })
      },
      error: (payload: unknown) => {
        // Background-sync errors come through here. Surface them as a toast
        // so the user knows when an auto-sync failed silently.
        const p = payload as { wallet_id?: string; message?: string }
        const wallet = p.wallet_id ? get(wallets).find((w: { id: string }) => w.id === p.wallet_id) : null
        const name = wallet?.label ?? 'Wallet'
        // Also remove this wallet from the "syncing" set in case sync_started
        // fired but sync_complete didn't (the error path skips it).
        if (p.wallet_id) {
          syncing.update(s => { const n = new Set(s); n.delete(p.wallet_id!); return n })
        }
        addToast(`${name}: ${p.message ?? 'sync failed'}`)
      },
    })

    return () => {
      unsubEvents()
    }
  })

  $effect(() => {
    if (!$showCurrentPrice && !$showFiatBalance) return
    let controller = new AbortController()
    async function fetchPrice() {
      try {
        const j = await api.price.current(controller.signal)
        if (typeof j.usd === 'number') currentBtcPrice.set(j.usd)
      } catch { /* aborted poll or transient failure — keep last price */ }
    }
    fetchPrice()
    const t = setInterval(() => { controller.abort(); controller = new AbortController(); fetchPrice() }, 60_000)
    return () => { clearInterval(t); controller.abort() }
  })

  // Set slide direction before navigation (used by mobile transitions)
  beforeNavigate(({ from, to }) => {
    if (!from || !to) return
    const fromPath = from.url.pathname
    const toPath = to.url.pathname
    const onSettingsTab = (p: string) => p.startsWith('/settings')
    if (onSettingsTab(fromPath) !== onSettingsTab(toPath)) {
      slideDir.set(0); return
    }
    const depth = (p: string) => p === '/' ? 0 : p.split('/').filter(Boolean).length
    if (depth(toPath) > depth(fromPath)) slideDir.set(1)
    else if (depth(toPath) < depth(fromPath)) slideDir.set(-1)
    else slideDir.set(0)
  })

</script>

{#if $securityState === 'locked'}
  <UnlockGate onUnlocked={handleUnlocked} />
{:else}
  {#if !$serverReachable}
    <div class="offline-banner" role="status" aria-live="polite">
      Server unreachable — check that Corvin is running.
    </div>
  {/if}

  {#if $isMobile}
    <MobileLayout>
      {@render children()}
    </MobileLayout>
  {:else}
    <div class="app">
      <WalletSidebar wallets={$wallets} />
      <main class="main">
        <div class="content">
          {@render children()}
        </div>
      </main>
    </div>
  {/if}

  {#if $showOnboarding}
    <OnboardingWizard onClose={() => showOnboarding.set(false)} />
  {/if}
{/if}

<ToastContainer />


<style>
  :global(*, *::before, *::after) { box-sizing: border-box; margin: 0; padding: 0; }
  :global(.sr-only) {
    position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
    overflow: hidden; clip: rect(0,0,0,0); white-space: nowrap; border: 0;
  }
  :global(:root) {
    --accent:          #f7931a;
    --surface-1:       #0d0d0d;
    --surface-2:       #181818;
    --surface-hover:   #1c1c1c;
    --border:          #2a2a2a;
    --text:            #e8e8e8;
    --text-muted:      #c2c2c2;
    --error:           #e05252;
    --surface-active:  color-mix(in srgb, var(--accent) 10%, var(--surface-1));

    /* Static scales (theme-independent). Colors live in theme.ts (per-scheme);
       these don't change between light/dark. */
    --radius-sm: 4px;
    --radius:    6px;
    --radius-lg: 10px;
    --space-1: 4px;
    --space-2: 8px;
    --space-3: 12px;
    --space-4: 16px;
    --space-5: 24px;
    --space-6: 32px;
    --text-xs:   0.72rem;
    --text-sm:   0.82rem;
    --text-base: 0.9rem;
    --text-lg:   1.1rem;
    --font-mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  /* Keyboard focus ring, app-wide. Many components do `outline: none` on
     :focus to suppress the mouse-click outline; this guarantees keyboard
     users still get a visible ring. !important is deliberate — it must win
     over those per-component resets. */
  :global(:focus-visible) {
    outline: 2px solid var(--accent) !important;
    outline-offset: 2px;
  }

  /* Shared UI primitives. Components already use these class names; defining
     them globally lets components drop their duplicated copies. A component
     that keeps a local rule still wins (scoped styles outrank globals), so
     migration is opt-in per component. */
  :global(.btn-primary) {
    background: var(--accent); color: #000; border: none;
    border-radius: 5px; padding: 9px 20px; cursor: pointer;
    font-weight: 600; font-size: 0.88rem;
  }
  :global(.btn-primary:hover) { filter: brightness(1.08); }
  :global(.btn-primary:disabled) { opacity: 0.5; cursor: not-allowed; }
  :global(.btn-ghost) {
    background: none; border: 1px solid var(--border);
    border-radius: 5px; padding: 9px 16px; cursor: pointer;
    color: var(--text-muted); font-size: 0.88rem;
  }
  :global(.btn-ghost:hover) { border-color: var(--text-muted); color: var(--text); }
  :global(.btn-ghost:disabled) { opacity: 0.5; cursor: not-allowed; }
  /* Secondary = neutral bordered button (also used for "Cancel"). */
  :global(.btn-secondary) {
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 5px; padding: 9px 16px; cursor: pointer;
    color: var(--text); font-size: 0.88rem;
  }
  :global(.btn-secondary:hover) { border-color: var(--text-muted); }
  :global(.btn-secondary:disabled) { opacity: 0.5; cursor: not-allowed; }
  /* Danger = destructive confirm (delete, etc.). */
  :global(.btn-danger) {
    background: var(--error); color: #fff; border: none;
    border-radius: 5px; padding: 9px 20px; cursor: pointer;
    font-weight: 600; font-size: 0.88rem;
  }
  :global(.btn-danger:hover:not(:disabled)) { filter: brightness(1.08); }
  :global(.btn-danger:disabled) { opacity: 0.45; cursor: not-allowed; }
  :global(.close-btn) {
    background: none; border: none; cursor: pointer;
    color: var(--text-muted); font-size: var(--text-base);
    padding: 2px 6px; line-height: 1;
  }
  :global(.close-btn:hover) { color: var(--text); }
  /* Compact bordered copy/icon button — global so the shared CopyButton can wear
     it (scoped per-component styles don't cross into child components). */
  :global(.btn-copy) {
    background: none; border: 1px solid var(--border); border-radius: 4px;
    color: var(--text-muted); cursor: pointer; font-size: 0.72rem;
    padding: 3px 8px; line-height: 1.4;
  }
  :global(.btn-copy:hover) { border-color: var(--text-muted); color: var(--text); }
  :global(body) {
    background: var(--surface-2);
    color: var(--text);
    font-family: system-ui, -apple-system, sans-serif;
    font-size: 14px;
    line-height: 1.5;
  }
  .offline-banner {
    position: fixed; top: 0; left: 0; right: 0; z-index: 9999;
    background: #7a2e2e; color: #f5c0c0;
    padding: 6px 16px; font-size: 0.78rem; text-align: center;
    letter-spacing: 0.01em;
  }
  @media (max-width: 768px) {
    .offline-banner {
      top: calc(52px + env(safe-area-inset-top));
    }
  }

  .app { display: flex; height: 100vh; overflow: hidden; }
  .main { flex: 1; display: flex; flex-direction: column; overflow: hidden; }
  .content { flex: 1; display: flex; overflow: hidden; }
</style>
