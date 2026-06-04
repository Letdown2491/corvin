<script lang="ts">
  // One recipient row in the send flow. Owns only its own markup; SendFlow keeps
  // the recipients array, the shared scanner / wallet-picker / HRN state, and the
  // handlers — this bubbles events up via callbacks. `recipient` is bindable so
  // the address/amount inputs write straight back into the parent's array element.
  import { isSilentPaymentAddress, looksLikeHrn, chunkAddress, addressShapeHint, showAddressEcho } from '../../lib/send'
  import { kindLabel } from '../../lib/utils'
  import type { WalletEntry } from '../../lib/types'
  import AmountInput from '../ui/AmountInput.svelte'

  interface Recipient {
    address: string
    amount: string
    sendMax: boolean
    fromWallet?: string
    payjoinUri?: string
  }
  type HrnStatus = { hrn?: string; resolving?: boolean; error?: string }

  let {
    recipient = $bindable(),
    index,
    total,
    unit,
    hrn,
    otherWallets,
    scanActive,
    walletPickerOpen,
    coinControlSelected,
    onRemove,
    onInput,
    onPaste,
    onBlur,
    onFocus,
    onPasteClipboard,
    onScan,
    onToggleWalletPicker,
    onCloseWalletPicker,
    onPickWallet,
    onSetSendMax,
  }: {
    recipient: Recipient
    index: number
    total: number
    unit: 'btc' | 'sats'
    hrn: HrnStatus | undefined
    otherWallets: WalletEntry[]
    scanActive: boolean
    walletPickerOpen: boolean
    /// Non-null when coin control has a selection, for the send-max hint wording.
    coinControlSelected: { count: number; satsFormatted: string } | null
    onRemove: () => void
    onInput: () => void
    onPaste: (e: ClipboardEvent) => void
    onBlur: () => void
    onFocus: () => void
    onPasteClipboard: () => void
    onScan: () => void
    onToggleWalletPicker: () => void
    onCloseWalletPicker: () => void
    onPickWallet: (w: WalletEntry) => void
    onSetSendMax: (val: boolean) => void
  } = $props()

  let shapeHint = $derived(addressShapeHint(recipient.address))
</script>

<div class="recipient-row-block">
  {#if total > 1}
    <div class="recipient-row-header">
      <span class="recipient-row-num">#{index + 1}</span>
      <button
        type="button"
        class="recipient-remove-btn"
        onclick={onRemove}
        title="Remove this recipient"
        aria-label={`Remove recipient ${index + 1}`}
      >✕</button>
    </div>
  {/if}

  <div class="addr-row">
    <input
      class="addr-input"
      class:addr-bad={shapeHint === 'bad'}
      type="text"
      placeholder="bc1q… · 3… · 1… · ₿user@domain"
      bind:value={recipient.address}
      onpaste={(e) => onPaste(e)}
      onfocus={onFocus}
      oninput={onInput}
      onblur={onBlur}
      autocomplete="off"
      autocapitalize="off"
      spellcheck="false"
      aria-label={`Recipient ${index + 1} address`}
    />
    <button
      class="addr-scan-btn"
      onclick={onPasteClipboard}
      title="Paste from clipboard"
      aria-label="Paste address"
    >
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <rect x="9" y="2" width="6" height="4" rx="1"/>
        <path d="M9 4H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2h-2"/>
      </svg>
    </button>
    <button
      class="addr-scan-btn"
      class:active={scanActive}
      onclick={onScan}
      title="Scan address QR code"
      aria-label="Scan QR code"
    >
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z"/>
        <circle cx="12" cy="13" r="4"/>
      </svg>
    </button>
    {#if otherWallets.length > 0}
      <button
        class="addr-scan-btn"
        class:active={walletPickerOpen}
        onclick={onToggleWalletPicker}
        title="Send to one of your wallets"
        aria-label="Send to one of your wallets"
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M3 7h15a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z"/>
          <path d="M3 7V6a2 2 0 0 1 2-2h11"/>
          <circle cx="16" cy="13" r="1.2" fill="currentColor" stroke="none"/>
        </svg>
      </button>
    {/if}
    {#if walletPickerOpen}
      <button type="button" class="wallet-picker-backdrop" aria-label="Close wallet picker" onclick={onCloseWalletPicker}></button>
      <div class="wallet-picker" role="menu">
        <div class="wallet-picker-head">Send to my wallet</div>
        {#each otherWallets as w (w.id)}
          <button type="button" class="wallet-picker-item" role="menuitem" onclick={() => onPickWallet(w)}>
            <span class="wp-name">{w.label}</span>
            <span class="wp-kind">{kindLabel(w.kind)}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>
  {#if recipient.fromWallet}
    <p class="self-target-note">↪ Sending to your wallet <strong>{recipient.fromWallet}</strong> — this links the two wallets on-chain.</p>
  {/if}
  {#if hrn?.resolving}
    <p class="hrn-line">Resolving <strong>{recipient.address.trim()}</strong> via DNSSEC…</p>
  {:else if hrn?.error}
    <p class="addr-shape-warn">{hrn.error}</p>
  {:else if hrn?.hrn}
    <div class="hrn-badge" role="status">
      <span aria-hidden="true">🔒</span>
      <span>Resolved <strong>₿{hrn.hrn}</strong> via DNSSEC. Verify the address below before sending.</span>
    </div>
    <p class="hrn-line">Resolving this name queried your DNS resolver, which can see who you're paying. Change it in Backend settings under Name resolution.</p>
  {/if}
  {#if shapeHint === 'bad' && !hrn?.hrn && !looksLikeHrn(recipient.address)}
    <p class="addr-shape-warn">This doesn't look like a Bitcoin address. Bitcoin addresses start with bc1, 1, or 3 (mainnet) and are 26–62 characters.</p>
  {:else if isSilentPaymentAddress(recipient.address)}
    <div class="sp-recipient-badge" role="status">
      <span class="sp-recipient-icon" aria-hidden="true">⌬</span>
      <div class="sp-recipient-body">
        <strong>Silent Payment address detected</strong>
        <span class="sp-recipient-note">BIP-352 send: derives a one-time Taproot output for this recipient. Provide your seed below — it's used once at build time to compute the shared secret, then zeroized.</span>
      </div>
    </div>
  {/if}
  {#if showAddressEcho(recipient.address)}
    <div class="addr-echo" role="status">
      <span class="addr-echo-label">Sending to</span>
      <span class="addr-echo-value">
        {#each chunkAddress(recipient.address) as chunk, ci (ci)}<span class="addr-chunk">{chunk}</span>{/each}
      </span>
      <span class="addr-echo-hint">Check every group against your source — malware can swap a copied address.</span>
    </div>
  {/if}

  <div class="amount-row">
    <AmountInput
      bind:value={recipient.amount}
      {unit}
      disabled={recipient.sendMax}
      onfocus={onFocus}
      ariaLabel={`Amount for recipient ${index + 1}`}
    />
    <button
      class="max-btn"
      class:active={recipient.sendMax}
      onclick={() => onSetSendMax(!recipient.sendMax)}
      title="Send all remaining funds (after the other recipients and fee) to this address"
    >
      Send max
    </button>
  </div>
  {#if recipient.sendMax}
    <p class="amount-hint">
      {#if coinControlSelected}
        All <strong>selected</strong> UTXOs ({coinControlSelected.count}, {coinControlSelected.satsFormatted}) — minus the other recipients and fee — will go to this address.
      {:else if total > 1}
        Whatever is left after the other recipients and fee will go to this address.
      {:else}
        All spendable funds (minus fee) will go to this address.
      {/if}
    </p>
  {/if}
</div>

<style>
  .recipient-row-block {
    display: flex; flex-direction: column; gap: 8px;
    padding: 10px;
    border: 1px solid var(--border); border-radius: 6px;
    margin-bottom: 8px;
    background: var(--surface-2);
  }
  .recipient-row-block:last-of-type { margin-bottom: 0; }
  .recipient-row-header {
    display: flex; align-items: center; justify-content: space-between;
    font-size: 0.7rem; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.06em;
  }
  .recipient-row-num { font-weight: 700; }
  .recipient-remove-btn {
    background: none; border: 1px solid var(--border); border-radius: 3px;
    color: var(--text-muted); cursor: pointer; padding: 1px 7px;
    font-size: 0.75rem; line-height: 1;
  }
  .recipient-remove-btn:hover { border-color: #e05252; color: #e05252; }

  .addr-row { display: flex; gap: 6px; align-items: stretch; position: relative; }
  .addr-input {
    flex: 1; min-width: 0; box-sizing: border-box;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 4px; color: var(--text); padding: 7px 10px;
    font-size: 0.82rem; font-family: monospace; outline: none;
  }
  .addr-input:focus { border-color: var(--accent); }
  .addr-input.addr-bad { border-color: #e09c52; }
  .addr-shape-warn { margin: 6px 0 0; font-size: 0.74rem; color: #e09c52; line-height: 1.4; }
  .hrn-line { margin: 6px 0 0; font-size: 0.74rem; color: var(--text-muted); line-height: 1.4; }

  .addr-echo {
    margin: 8px 0 0; padding: 8px 10px;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px;
    display: flex; flex-direction: column; gap: 4px;
  }
  .addr-echo-label { font-size: 0.68rem; text-transform: uppercase; letter-spacing: 0.06em; color: var(--text-muted); }
  .addr-echo-value { display: flex; flex-wrap: wrap; gap: 3px 7px; font-family: var(--font-mono); font-size: 0.82rem; color: var(--text); }
  .addr-chunk { white-space: nowrap; }
  .addr-echo-hint { font-size: 0.7rem; color: var(--text-muted); line-height: 1.4; }

  .hrn-badge {
    margin: 6px 0 0; display: flex; align-items: flex-start; gap: 6px;
    font-size: 0.74rem; color: var(--text-muted); line-height: 1.45;
    padding: 6px 9px; border-radius: 5px;
    background: color-mix(in srgb, #52a875 8%, transparent);
    border: 1px solid color-mix(in srgb, #52a875 28%, transparent);
  }
  .hrn-badge strong { color: var(--text); }

  .sp-recipient-badge {
    margin-top: 6px;
    display: flex; align-items: flex-start; gap: 8px;
    padding: 8px 10px; border-radius: 5px;
    background: color-mix(in srgb, var(--accent) 5%, var(--surface-2));
    border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
    font-size: 0.76rem; line-height: 1.5;
  }
  .sp-recipient-icon { color: var(--accent); font-size: 1rem; line-height: 1.3; flex-shrink: 0; }
  .sp-recipient-body { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .sp-recipient-body strong { color: var(--accent); font-weight: 700; }
  .sp-recipient-note { color: var(--text-muted); }

  .addr-scan-btn {
    flex-shrink: 0; background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 4px; padding: 0 10px; cursor: pointer;
    color: var(--text-muted); display: flex; align-items: center; transition: all 0.12s;
  }
  .addr-scan-btn:hover { border-color: var(--accent); color: var(--accent); }
  .addr-scan-btn.active { border-color: var(--accent); color: var(--accent); background: color-mix(in srgb, var(--accent) 10%, var(--surface-2)); }

  .wallet-picker-backdrop {
    position: fixed; inset: 0; z-index: 10;
    background: none; border: none; padding: 0; cursor: default;
  }
  .wallet-picker {
    position: absolute; top: calc(100% + 4px); right: 0; z-index: 11;
    min-width: 220px; max-height: 260px; overflow-y: auto;
    background: var(--surface-1); border: 1px solid var(--border);
    border-radius: 7px; padding: 4px; box-shadow: 0 10px 30px rgba(0,0,0,0.45);
    display: flex; flex-direction: column; gap: 1px;
  }
  .wallet-picker-head {
    font-size: 0.7rem; color: var(--text-muted); text-transform: uppercase;
    letter-spacing: 0.04em; padding: 6px 8px 4px;
  }
  .wallet-picker-item {
    display: flex; align-items: baseline; justify-content: space-between; gap: 10px;
    background: none; border: none; cursor: pointer; text-align: left;
    padding: 8px 8px; border-radius: 5px; color: var(--text);
  }
  .wallet-picker-item:hover { background: var(--surface-active); }
  .wp-name { font-size: 0.84rem; font-weight: 500; }
  .wp-kind { font-size: 0.72rem; color: var(--text-muted); flex-shrink: 0; }
  .self-target-note {
    margin: 6px 0 0; font-size: 0.76rem; color: var(--text-muted); line-height: 1.5;
  }
  .self-target-note strong { color: var(--text); }

  .amount-row { display: flex; align-items: center; gap: 8px; }
  .max-btn {
    margin-left: auto; background: none; border: 1px solid var(--border);
    border-radius: 4px; color: var(--text-muted); cursor: pointer;
    font-size: 0.78rem; padding: 5px 12px; transition: all 0.12s;
  }
  .max-btn:hover { border-color: var(--accent); color: var(--accent); }
  .max-btn.active { border-color: var(--accent); color: var(--accent); background: color-mix(in srgb, var(--accent) 10%, var(--surface-2)); }
  .amount-hint { margin: 8px 0 0; font-size: 0.75rem; color: var(--text-muted); }
</style>
