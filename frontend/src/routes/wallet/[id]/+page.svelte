<script lang="ts">
  import { goto } from '$app/navigation'
  import { page } from '$app/stores'
  import { wallets, activeWalletId } from '../../../stores/wallets'
  import { api } from '../../../lib/api'
  import { addToast } from '../../../stores/toasts'
  import WalletDetail from '../../../components/WalletDetail.svelte'

  let walletId = $derived($page.params.id)
  let wallet = $derived($wallets.find(w => w.id === walletId) ?? null)

  $effect(() => {
    if (walletId) activeWalletId.set(walletId)
  })

  async function handleDelete() {
    if (!walletId) return
    try {
      await api.wallets.remove(walletId)
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Failed to remove wallet')
      return
    }
    // Clean up any persisted send-draft for this wallet so a future wallet
    // created with the same UUID (unlikely but possible after backup restore)
    // doesn't inherit stale local state.
    try { localStorage.removeItem(`corvin:send-draft:${walletId}`) } catch {}
    const remaining = $wallets.filter(w => w.id !== walletId)
    wallets.update(ws => ws.filter(w => w.id !== walletId))
    if (remaining.length > 0) {
      activeWalletId.set(remaining[0].id)
      goto(`/wallet/${remaining[0].id}`, { replaceState: true })
    } else {
      activeWalletId.set(null)
      goto('/', { replaceState: true })
    }
  }
</script>

{#if wallet}
  <WalletDetail {wallet} onDelete={handleDelete} />
{:else}
  <div class="not-found">Wallet not found.</div>
{/if}

<style>
  .not-found {
    flex: 1; display: flex; align-items: center; justify-content: center;
    color: var(--text-muted);
  }
</style>
