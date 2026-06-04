<script lang="ts">
  import { goto } from '$app/navigation'
  import { wallets, activeWalletId } from '../stores/wallets'
  import { defaultWalletId } from '../stores/settings'
  import MobileWalletList from '../components/MobileWalletList.svelte'
  import EmptyState from '../components/EmptyState.svelte'
  import { untrack } from 'svelte'
  import { isMobile } from '../lib/mobile'

  $effect(() => {
    if ($isMobile) return
    const ws = $wallets
    if (ws.length > 0) {
      const defId = $defaultWalletId
      const target = (defId && ws.find(w => w.id === defId)) ? defId : ws[0].id
      activeWalletId.set(target)
      untrack(() => goto(`/wallet/${target}`, { replaceState: true }))
    }
  })
</script>

{#if $isMobile}
  <MobileWalletList />
{:else if $wallets.length === 0}
  <div class="landing">
    <EmptyState
      icon="wallet"
      title="No wallets yet"
      description="Create a new wallet or import one you already have — from a seed, an xpub, a descriptor, or a hardware wallet."
    >
      {#snippet action()}
        <button class="btn-primary" onclick={() => goto('/add-wallet')}>Add your first wallet</button>
      {/snippet}
    </EmptyState>
  </div>
{/if}

<style>
  .landing {
    flex: 1; display: flex; flex-direction: column; align-items: center;
    justify-content: center;
  }
  .btn-primary {
    background: var(--accent); color: #000; border: none; border-radius: 4px;
    padding: 10px 22px; cursor: pointer; font-weight: 600; font-size: 0.9rem;
  }
</style>
