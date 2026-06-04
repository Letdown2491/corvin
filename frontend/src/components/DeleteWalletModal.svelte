<script lang="ts">
  import type { WalletEntry } from '../lib/types'
  import Modal from './ui/Modal.svelte'

  let { wallet, confirmedSats = 0, onClose, onConfirm }: {
    wallet: WalletEntry
    /// Confirmed balance in sats — if > 0 we require typing the name to confirm.
    confirmedSats?: number
    onClose: () => void
    onConfirm: () => void | Promise<void>
  } = $props()

  let confirmText = $state('')
  let busy = $state(false)
  let hasFunds = $derived(confirmedSats > 0)
  // Always require typing the wallet name — a deliberate speed bump for any delete.
  let canDelete = $derived(confirmText.trim() === wallet.label)

  async function confirmDelete() {
    if (!canDelete || busy) return
    busy = true
    try {
      await onConfirm()
    } finally {
      busy = false
    }
  }
</script>

<Modal open onclose={onClose} title={`Delete “${wallet.label}”?`} width="440px">

    <p class="warn-box">
      <strong>Removes this wallet from Corvin only.</strong> Your bitcoin stays on-chain
      and can be re-imported from the seed phrase or descriptor.
    </p>

    {#if hasFunds}
      <p class="funds-warn">
        ⚠ This wallet currently holds <strong>{(confirmedSats / 1e8).toFixed(8)} BTC</strong>.
        Make sure you have the seed / descriptor saved before deleting.
      </p>
    {/if}

    <label class="field-label" for="dw-confirm">Type the wallet name to confirm</label>
    <input
      id="dw-confirm"
      class="text-input"
      type="text"
      placeholder={wallet.label}
      bind:value={confirmText}
      spellcheck="false"
      autocapitalize="off"
      autocomplete="off"
    />

    <div class="actions">
      <button class="btn-secondary" onclick={onClose} disabled={busy}>Cancel</button>
      <button class="btn-danger" onclick={confirmDelete} disabled={!canDelete || busy}>
        {busy ? 'Deleting…' : 'Delete wallet'}
      </button>
    </div>
</Modal>

<style>
  .warn-box {
    margin: 0 0 12px; font-size: 0.82rem; color: var(--text); line-height: 1.55;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 6px; padding: 10px 12px;
  }
  .funds-warn {
    margin: 0 0 12px; font-size: 0.82rem; line-height: 1.55; color: var(--text);
    background: color-mix(in srgb, #e09c52 12%, var(--surface-2));
    border: 1px solid color-mix(in srgb, #e09c52 35%, var(--border));
    border-radius: 6px; padding: 10px 12px;
  }
  .field-label {
    display: block; font-size: 0.74rem; font-weight: 600; color: var(--text-muted);
    margin-bottom: 5px; text-transform: uppercase; letter-spacing: 0.06em;
  }
  .text-input {
    width: 100%; box-sizing: border-box;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 4px; color: var(--text); padding: 7px 10px;
    font-size: 0.82rem; outline: none; margin-bottom: 4px;
  }
  .text-input:focus { border-color: var(--accent); }

  .actions { display: flex; gap: 8px; margin-top: 14px; }
</style>
