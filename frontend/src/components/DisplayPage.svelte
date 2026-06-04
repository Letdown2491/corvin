<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { api } from '../lib/api'
  import type { Settings } from '../lib/types'
  import { showPriceData, showCurrentPrice, showFiatBalance, displayUnit, balancesHidden, notificationsEnabled } from '../stores/settings'
  import { showOnboarding } from '../stores/onboarding'
  import { theme, ACCENT_COLORS } from '../stores/theme'

  let settings = $state<Settings | null>(null)
  let error = $state('')
  let notifPermission = $state(typeof Notification !== 'undefined' ? Notification.permission : 'denied')

  // Which section is flashing its "Autosaved ✓" header indicator.
  let flashed = $state<string | null>(null)
  let flashTimer: ReturnType<typeof setTimeout> | null = null
  function flash(section: string) {
    flashed = section
    if (flashTimer) clearTimeout(flashTimer)
    flashTimer = setTimeout(() => { flashed = null }, 1500)
  }
  onDestroy(() => { if (flashTimer) clearTimeout(flashTimer) })

  async function handleNotifToggle(e: Event) {
    const checked = (e.target as HTMLInputElement).checked
    if (!checked) { notificationsEnabled.set(false); flash('notifications'); return }
    if (Notification.permission === 'granted') {
      notificationsEnabled.set(true)
    } else if (Notification.permission === 'default') {
      const result = await Notification.requestPermission()
      notifPermission = result
      notificationsEnabled.set(result === 'granted')
    } else {
      notificationsEnabled.set(false)
    }
    flash('notifications')
  }

  onMount(async () => {
    try { settings = await api.settings.get() }
    catch (e) { error = e instanceof Error ? e.message : 'Unknown error' }
  })

  // Toggle the onboarding-complete flag. Unchecking replays the welcome wizard
  // right away (it's a full-screen overlay); re-checking just marks it done.
  async function toggleOnboarding(e: Event) {
    if (!settings) return
    const complete = (e.target as HTMLInputElement).checked
    settings.onboarding_complete = complete
    try {
      if (complete) await api.completeOnboarding()
      else { await api.resetOnboarding(); showOnboarding.set(true) }
      flash('onboarding')
    } catch (err) {
      error = err instanceof Error ? err.message : 'Unknown error'
      settings.onboarding_complete = !complete
    }
  }

  // Price toggles autosave on change. Re-fetch the latest config first and write
  // only the three fields this page owns, so we never clobber settings the
  // Backend page may have changed in the meantime.
  async function savePrice() {
    if (!settings) return
    error = ''
    try {
      const fresh = await api.settings.get()
      fresh.backend.show_price_data = settings.backend.show_price_data
      fresh.backend.show_current_price = settings.backend.show_current_price
      fresh.backend.show_fiat_balance = settings.backend.show_fiat_balance
      settings = await api.settings.update(fresh)
      showPriceData.set(settings.backend.show_price_data)
      showCurrentPrice.set(settings.backend.show_current_price)
      showFiatBalance.set(settings.backend.show_fiat_balance)
      flash('price')
    } catch (e) { error = e instanceof Error ? e.message : 'Unknown error' }
  }
</script>

<div class="page">
  <div class="page-inner">
    <h1>Display Settings</h1>
    {#if error}<p class="global-error">{error}</p>{/if}

    <div class="card">
      <div class="card-hd">
        <div class="card-title">Theme</div>
        {#if flashed === 'theme'}<span class="autosaved">Autosaved ✓</span>{/if}
      </div>

      <div class="setting-row">
        <span class="setting-label">Color scheme</span>
        <div class="seg-group">
          <button type="button" class:active={$theme.scheme === 'dark'}  onclick={() => { theme.update(t => ({ ...t, scheme: 'dark' })); flash('theme') }}>Dark</button>
          <button type="button" class:active={$theme.scheme === 'light'} onclick={() => { theme.update(t => ({ ...t, scheme: 'light' })); flash('theme') }}>Light</button>
          <button type="button" class:active={$theme.scheme === 'auto'}  onclick={() => { theme.update(t => ({ ...t, scheme: 'auto' })); flash('theme') }} title="Follow your OS preferences">Auto</button>
        </div>
      </div>

      <div class="accent-section">
        <span class="setting-label">Accent color</span>
        <div class="swatch-row">
          {#each ACCENT_COLORS as color (color.value)}
            <div class="swatch-item">
              <button
                type="button"
                class="swatch"
                class:selected={$theme.accent === color.value}
                style="background: {color.value}"
                onclick={() => { theme.update(t => ({ ...t, accent: color.value })); flash('theme') }}
                aria-label="{color.name}{$theme.accent === color.value ? ' (selected)' : ''}"
              >
                {#if $theme.accent === color.value}✓{/if}
              </button>
              <span class="swatch-name">{color.name}</span>
            </div>
          {/each}
        </div>
      </div>
    </div>

    <div class="card">
      <div class="card-hd">
        <div class="card-title">Preferences</div>
        {#if flashed === 'prefs'}<span class="autosaved">Autosaved ✓</span>{/if}
      </div>

      <div class="setting-row">
        <span class="setting-label">Amount unit</span>
        <div class="seg-group">
          <button type="button" class:active={$displayUnit === 'sats'} onclick={() => { displayUnit.set('sats'); flash('prefs') }}>sats</button>
          <button type="button" class:active={$displayUnit === 'btc'}  onclick={() => { displayUnit.set('btc'); flash('prefs') }}>BTC</button>
        </div>
      </div>

      <div class="check">
        <input id="d-hidden" type="checkbox" bind:checked={$balancesHidden} onchange={() => flash('prefs')} />
        <label for="d-hidden">
          Hide balances by default
          <span class="muted-text">— show ••• instead of amounts on load</span>
        </label>
      </div>
    </div>

    <div class="card">
      <div class="card-hd">
        <div class="card-title">Price data</div>
        {#if flashed === 'price'}<span class="autosaved">Autosaved ✓</span>{/if}
      </div>
      <p class="card-desc">Fetched from your mempool server. Used in transaction details and tax reports.</p>

      {#if !settings}
        <p class="loading">Loading…</p>
      {:else}
        <div class="checks">
          <div class="check">
            <input id="s-price" type="checkbox" bind:checked={settings.backend.show_price_data} onchange={() => savePrice()} />
            <label for="s-price">
              Show historical USD prices
              <span class="muted-text">— fetch BTC/USD rates for transaction details and tax reports</span>
            </label>
          </div>
          <div class="check">
            <input id="s-current-price" type="checkbox" bind:checked={settings.backend.show_current_price} onchange={() => savePrice()} />
            <label for="s-current-price">
              Show current BTC price
              <span class="muted-text">— show the price of 1 BTC in the sidebar</span>
            </label>
          </div>
          <div class="check">
            <input id="s-fiat-bal" type="checkbox" bind:checked={settings.backend.show_fiat_balance} onchange={() => savePrice()} />
            <label for="s-fiat-bal">
              Show fiat balance
              <span class="muted-text">— show your holdings' USD value on the wallet's status line</span>
            </label>
          </div>
        </div>
      {/if}
    </div>

    <div class="card">
      <div class="card-hd">
        <div class="card-title">Notifications</div>
        {#if flashed === 'notifications'}<span class="autosaved">Autosaved ✓</span>{/if}
      </div>
      <p class="card-desc">Browser alerts when new transactions are detected during a sync.</p>
      <div class="checks">
        <div class="check">
          <input
            id="d-notifs" type="checkbox"
            checked={$notificationsEnabled}
            onchange={handleNotifToggle}
            disabled={notifPermission === 'denied'}
          />
          <label for="d-notifs">
            Enable notifications
            <span class="muted-text">— browser notification when new transactions arrive</span>
          </label>
        </div>
        {#if notifPermission === 'denied'}
          <p class="perm-warn">Permission denied — enable notifications in your browser settings and reload.</p>
        {/if}
      </div>
    </div>

    <div class="card">
      <div class="card-hd">
        <div class="card-title">Onboarding</div>
        {#if flashed === 'onboarding'}<span class="autosaved">Autosaved ✓</span>{/if}
      </div>
      <p class="card-desc">The welcome guide shown on first run.</p>
      {#if settings}
        <div class="checks">
          <div class="check">
            <input
              id="d-onboarding" type="checkbox"
              checked={settings.onboarding_complete ?? false}
              onchange={toggleOnboarding}
            />
            <label for="d-onboarding">
              Onboarding complete
              <span class="muted-text">— uncheck to see the welcome guide again</span>
            </label>
          </div>
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .page {
    flex: 1; overflow-y: auto; background: var(--surface-2);
    padding: 32px 24px 48px;
  }
  .page-inner { max-width: 640px; margin: 0 auto; }
  h1 { font-size: 1.4rem; font-weight: 700; color: var(--text); margin: 0 0 24px; letter-spacing: -0.01em; }
  .global-error { color: var(--error); font-size: 0.82rem; margin: 0 0 12px; }
  .loading { color: var(--text-muted); font-size: 0.85rem; margin: 0; }

  .card {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 8px;
    padding: 20px 24px; display: flex; flex-direction: column; gap: 14px;
    margin-bottom: 12px;
  }

  .card-hd { display: flex; align-items: center; gap: 10px; }
  .card-title { font-size: 0.9rem; font-weight: 600; color: var(--text); }
  .card-desc { font-size: 0.82rem; color: var(--text-muted); margin: 0; line-height: 1.5; }

  /* Transient "Autosaved ✓" indicator in a card header. */
  .autosaved {
    margin-left: auto; font-size: 0.72rem; font-weight: 500; color: #52a875;
    white-space: nowrap;
  }
  .setting-row { display: flex; align-items: center; justify-content: space-between; }
  .setting-label { font-size: 0.85rem; color: var(--text-muted); }

  .seg-group {
    display: flex; border: 1px solid var(--border); border-radius: 5px; overflow: hidden;
  }
  .seg-group button {
    background: none; border: none; padding: 6px 18px; cursor: pointer;
    font-size: 0.82rem; color: var(--text-muted); font-weight: 500;
  }
  .seg-group button + button { border-left: 1px solid var(--border); }
  .seg-group button.active { background: var(--accent); color: #000; font-weight: 600; }

  .accent-section { display: flex; flex-direction: column; gap: 10px; }

  .swatch-row { display: flex; gap: 16px; flex-wrap: wrap; padding: 2px 0; }
  .swatch-item { display: flex; flex-direction: column; align-items: center; gap: 5px; }
  .swatch {
    width: 36px; height: 36px; border-radius: 50%; border: none; cursor: pointer;
    display: flex; align-items: center; justify-content: center;
    font-size: 0.9rem; font-weight: 800; color: rgba(255,255,255,0.9);
    text-shadow: 0 1px 2px rgba(0,0,0,0.4);
    transition: transform 0.12s, box-shadow 0.12s;
    outline: 3px solid transparent; outline-offset: 2px;
  }
  .swatch:hover { transform: scale(1.1); }
  .swatch.selected { outline-color: var(--text); transform: scale(1.05); }
  .swatch-name { font-size: 0.68rem; color: var(--text-muted); }

  .checks { display: flex; flex-direction: column; gap: 10px; }
  .check { display: flex; align-items: baseline; gap: 8px; }
  .check input[type="checkbox"] { width: auto; margin: 0; flex-shrink: 0; cursor: pointer; }
  .check label { font-size: 0.85rem; color: var(--text); cursor: pointer; display: inline; }
  .muted-text { color: var(--text-muted); font-size: 0.78rem; }

  .perm-warn { font-size: 0.78rem; color: #e09c52; margin: 0; }

  @media (max-width: 768px) {
    .page { padding: 16px 16px 32px; }
    h1 { display: none; }
  }
</style>
