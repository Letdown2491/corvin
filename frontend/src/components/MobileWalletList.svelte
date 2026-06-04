<script lang="ts">
  import { goto } from '$app/navigation'
  import { wallets, syncing, activeWalletId } from '../stores/wallets'
  import { defaultWalletId } from '../stores/settings'
  import { slideDir } from '../stores/ui'
  import { kindLabel } from '../lib/utils'
  import EmptyState from './EmptyState.svelte'

  let sorted = $derived.by(() => {
    const def = $defaultWalletId
    return [...$wallets].sort((a, b) => {
      if (a.id === def) return -1
      if (b.id === def) return 1
      return a.label.localeCompare(b.label)
    })
  })

  function selectWallet(id: string) {
    slideDir.set(1)
    activeWalletId.set(id)
    goto(`/wallet/${id}`)
  }

  function addWallet() {
    slideDir.set(1)
    goto('/add-wallet')
  }
</script>

<div class="wallet-list">
  {#if sorted.length === 0}
    <EmptyState
      icon="wallet"
      title="No wallets yet"
      description="Create a new wallet or import one you already have — from a seed, an xpub, a descriptor, or a hardware wallet."
    >
      {#snippet action()}
        <button class="btn-primary" onclick={addWallet}>Add your first wallet</button>
      {/snippet}
    </EmptyState>
  {:else}
    {#each sorted as w (w.id)}
      <button class="wallet-row" onclick={() => selectWallet(w.id)}>
        <div class="wallet-info">
          <span class="wallet-name">{w.label}</span>
          <span class="wallet-kind">
            {kindLabel(w.kind)}
            {#if $syncing.has(w.id)}<span class="sync-spinner" role="status" aria-label="Syncing"> ⟳</span>{/if}
          </span>
        </div>
        <span class="chevron" aria-hidden="true">›</span>
      </button>
    {/each}

    <div class="list-footer">
      <button class="btn-secondary" onclick={addWallet}>+ Add wallet</button>
    </div>
  {/if}
</div>

<style>
  .wallet-list { display: flex; flex-direction: column; }

  .wallet-row {
    display: flex; align-items: center; justify-content: space-between;
    padding: 16px 20px; background: none; border: none;
    border-bottom: 1px solid var(--border); cursor: pointer;
    color: var(--text); text-align: left; width: 100%; gap: 12px;
    min-height: 64px;
  }
  .wallet-row:active { background: var(--surface-hover); }

  .wallet-info { display: flex; flex-direction: column; gap: 3px; flex: 1; min-width: 0; }
  .wallet-name { font-size: 1rem; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .wallet-kind { font-size: 0.78rem; color: var(--text-muted); }

  .sync-spinner { font-size: 0.8rem; color: var(--accent); animation: spin 1s linear infinite; display: inline-block; }
  @keyframes spin { to { transform: rotate(360deg); } }

  .chevron { font-size: 1.6rem; color: var(--text-muted); flex-shrink: 0; line-height: 1; }

  .list-footer { padding: 20px; }

  /* Deliberate mobile override of the global buttons: full-width, larger touch
     targets, bigger radius. Inherits color/role from the global .btn-* classes. */
  .btn-primary {
    border-radius: 8px; padding: 14px 24px; font-size: 0.95rem; width: 100%;
  }
  .btn-secondary {
    border-radius: 8px; padding: 13px 24px; font-size: 0.9rem; font-weight: 500; width: 100%;
  }
</style>
