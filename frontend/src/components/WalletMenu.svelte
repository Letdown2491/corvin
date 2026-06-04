<script lang="ts">
  // The wallet-header kebab menu. Owns its open/close + keyboard nav; emits a
  // semantic callback per action and the parent drives the resulting modal/state.
  import type { WalletEntry } from '../lib/types'

  let {
    wallet,
    syncing,
    offline,
    canAddAnotherAccount,
    onSync,
    onRename,
    onChangeBackend,
    onBroadcast,
    onConsolidate,
    onAddAccount,
    onDelete,
  }: {
    wallet: WalletEntry
    syncing: boolean
    offline: boolean
    canAddAnotherAccount: boolean
    onSync: () => void
    onRename: () => void
    onChangeBackend: () => void
    onBroadcast: () => void
    onConsolidate: () => void
    onAddAccount: () => void
    onDelete: () => void
  } = $props()

  let open = $state(false)
  let dropdownEl = $state<HTMLElement | null>(null)

  $effect(() => {
    if (open && dropdownEl) {
      const first = dropdownEl.querySelector<HTMLElement>('[role="menuitem"]:not([disabled])')
      first?.focus()
    }
  })

  function pick(fn: () => void) {
    open = false
    fn()
  }
</script>

<div class="menu-wrap">
  <button
    class="menu-trigger"
    class:spinning={syncing}
    onclick={(e) => { e.stopPropagation(); open = !open }}
    aria-haspopup="menu"
    aria-expanded={open}
    aria-label="Wallet options"
  >
    <span aria-hidden="true">
      {#if syncing}
        <svg class="sync-spin" viewBox="0 0 16 16" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">
          <path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9" />
          <path d="M13.5 2.5V5H11" />
        </svg>
      {:else}···{/if}
    </span>
  </button>

  {#if open}
    <button class="menu-backdrop" aria-label="Close menu" tabindex="-1" onclick={() => open = false}></button>
    <div class="menu-dropdown" role="menu" tabindex="-1"
      bind:this={dropdownEl}
      onkeydown={(e) => {
        if (e.key === 'Escape') { open = false; return }
        const items = (e.currentTarget as HTMLElement).querySelectorAll<HTMLElement>('[role="menuitem"]:not([disabled])')
        const idx = Array.from(items).indexOf(document.activeElement as HTMLElement)
        if (e.key === 'ArrowDown') { items[(idx + 1) % items.length]?.focus(); e.preventDefault() }
        else if (e.key === 'ArrowUp') { items[(idx - 1 + items.length) % items.length]?.focus(); e.preventDefault() }
        else if (e.key === 'Home') { items[0]?.focus(); e.preventDefault() }
        else if (e.key === 'End') { items[items.length - 1]?.focus(); e.preventDefault() }
      }}
    >
      <!-- ── Wallet ─────────────────────────────────────────────── -->
      <div class="menu-section-label" role="presentation">Wallet</div>
      <button class="menu-item" role="menuitem" onclick={() => pick(onSync)} disabled={syncing || offline} title={offline ? 'Unavailable in offline mode' : ''}>
        <span class="menu-icon" aria-hidden="true">↺</span> Sync
      </button>
      <button class="menu-item" role="menuitem" onclick={() => pick(onRename)}>
        <span class="menu-icon" aria-hidden="true">
          <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="M2.5 13.5l3-.8 7.2-7.2a1.4 1.4 0 0 0-2-2L3.3 10.7z" />
            <path d="M9.8 3.4l2.4 2.4" />
          </svg>
        </span> Rename
      </button>
      <button class="menu-item" role="menuitem" onclick={() => pick(onChangeBackend)}>
        <span class="menu-icon" aria-hidden="true">◎</span> Change backend…
      </button>

      <!-- ── Operations ─────────────────────────────────────────── -->
      <div class="menu-section-label" role="presentation">Operations</div>
      {#if wallet.internal_descriptor !== null}
        <button class="menu-item" role="menuitem" onclick={() => pick(onBroadcast)}>
          <span class="menu-icon" aria-hidden="true">↑</span> Broadcast…
        </button>
        <button class="menu-item" role="menuitem" onclick={() => pick(onConsolidate)}>
          <span class="menu-icon" aria-hidden="true">
            <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
              <path d="M8 2.5V6M5.8 4 8 6 10.2 4" />
              <path d="M8 13.5V10M5.8 12 8 10 10.2 12" />
            </svg>
          </span> Consolidate UTXOs
        </button>
      {/if}

      {#if canAddAnotherAccount}
        <div class="menu-section-label" role="presentation">Accounts &amp; features</div>
        <button class="menu-item" role="menuitem" onclick={() => pick(onAddAccount)}>
          <span class="menu-icon" aria-hidden="true">＋</span> Add account…
        </button>
      {/if}

      <!-- ── Danger ─────────────────────────────────────────────── -->
      <div class="menu-section-label menu-section-danger" role="presentation">Danger zone</div>
      <button class="menu-item danger" role="menuitem" onclick={() => pick(onDelete)}>
        <span class="menu-icon" aria-hidden="true">×</span> Delete wallet
      </button>
    </div>
  {/if}
</div>

<style>
  .menu-wrap { position: relative; }
  .menu-trigger {
    background: none; border: 1px solid transparent; border-radius: 5px;
    color: var(--text); cursor: pointer; padding: 4px 10px;
    font-size: 1.1rem; font-weight: 700; letter-spacing: 0.05em;
    line-height: 1; transition: all 0.15s;
  }
  .menu-trigger:hover { border-color: var(--border); color: var(--text); background: var(--surface-2); }
  .menu-trigger.spinning { color: var(--accent); }
  .menu-trigger svg { display: block; }
  .sync-spin { animation: spin 1s linear infinite; transform-origin: center; }
  @keyframes spin { to { transform: rotate(360deg); } }

  .menu-backdrop {
    position: fixed; inset: 0; z-index: 9;
    background: transparent; border: none; cursor: default; padding: 0;
  }
  .menu-dropdown {
    position: absolute; top: calc(100% + 6px); right: 0;
    background: var(--surface-1); border: 1px solid var(--border);
    border-radius: 7px; min-width: 210px; z-index: 10;
    box-shadow: 0 8px 24px rgba(0,0,0,0.4);
    padding: 4px;
    overflow: hidden;
  }
  .menu-item {
    width: 100%; background: none; border: none; border-radius: 4px;
    text-align: left; padding: 8px 12px; cursor: pointer;
    font-size: 0.85rem; color: var(--text); white-space: nowrap;
    display: flex; align-items: center; gap: 8px;
  }
  .menu-item:hover { background: var(--surface-hover); }
  .menu-item:disabled { opacity: 0.4; cursor: not-allowed; }
  .menu-item.danger { color: #e05252; }
  .menu-item.danger:hover { background: color-mix(in srgb, #e05252 10%, transparent); }
  .menu-icon {
    font-size: 1rem; line-height: 1; width: 16px; height: 16px; flex-shrink: 0;
    display: inline-flex; align-items: center; justify-content: center;
  }
  /* Section labels grouping menu items by purpose. Subtle but scannable —
     uppercase + muted color, small font, slight top spacing so each group
     visually detaches from the previous block. The danger label gets red
     accent to mirror the destructive item below it. */
  .menu-section-label {
    padding: 8px 12px 3px;
    font-size: 0.64rem; font-weight: 700; letter-spacing: 0.08em;
    color: var(--text-muted); text-transform: uppercase;
    pointer-events: none;
  }
  .menu-section-label:first-child { padding-top: 4px; }
  .menu-section-danger { color: #e05252; }
</style>
