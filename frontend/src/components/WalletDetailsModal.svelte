<script lang="ts">
  import { onMount } from 'svelte'
  import type { WalletEntry } from '../lib/types'
  import { parseDerivation } from '../lib/derivation'
  import type { BackendEntry } from '../lib/types'
  import { addToast } from '../stores/toasts'
  import { downloadBlob } from '../lib/utils'
  import { api } from '../lib/api'
  import Modal from './ui/Modal.svelte'
  import CopyButton from './ui/CopyButton.svelte'

  let {
    wallet,
    onClose,
  }: {
    wallet: WalletEntry
    onClose: () => void
  } = $props()

  let cosignersOpen = $state(false)

  let derivation = $derived(parseDerivation(wallet))

  let walletKindLabel = $derived.by(() => {
    if (wallet.kind === 'address') return 'Watch-only address'
    if (wallet.kind === 'multisig') return 'Multisig wallet'
    if (wallet.kind === 'silent_payments') return 'Silent Payments wallet (BIP-352)'
    if (wallet.kind === 'descriptor') return 'Policy wallet (miniscript)'
    return wallet.input.startsWith('m/') ? 'HD wallet (seed)' : 'HD wallet (xpub)'
  })

  let isSp = $derived(wallet.kind === 'silent_payments')

  // Which backend this wallet uses (read-only here — change it from the wallet's
  // ⋯ menu → "Change backend"). `None` = the default; otherwise a saved backend.
  let savedBackends = $state<BackendEntry[]>([])
  let backendLabel = $derived(
    wallet.backend == null
      ? (isSp ? 'Public · frigate.2140.dev' : 'Default')
      : (savedBackends.find(b => b.id === wallet.backend)?.label ?? wallet.backend)
  )

  // Spending-policy summary — fetched for descriptor/policy wallets so timelock
  // (recovery/vault) conditions are visible. Skipped for address/SP (no script).
  let policy = $state<{ policy: string; timelocks: { kind: string; value: number; blocks: boolean; label: string }[]; key_fingerprints: string[] } | null>(null)
  // Show the policy block when there's something interesting: a timelock or a
  // non-standard (miniscript) policy wallet.
  let showPolicy = $derived(!!policy && (policy.timelocks.length > 0 || wallet.kind === 'descriptor'))

  let exporting = $state(false)
  async function downloadDescriptor() {
    if (exporting) return
    exporting = true
    try {
      const { descriptor } = await api.wallets.exportDescriptor(wallet.id)
      const slug = wallet.label.replace(/[^a-z0-9]/gi, '_').toLowerCase() || 'wallet'
      downloadBlob(new Blob([descriptor + '\n'], { type: 'text/plain' }), `${slug}_descriptor.txt`)
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Failed to export descriptor')
    } finally {
      exporting = false
    }
  }

  onMount(async () => {
    if (wallet.kind !== 'address' && wallet.kind !== 'silent_payments') {
      try { policy = await api.wallets.policy(wallet.id) } catch { /* trivial / no policy */ }
    }
    try { savedBackends = await api.backends.list() } catch { /* registry unavailable */ }
  })
</script>

<Modal open onclose={onClose} title="Wallet details" width="560px" desc={wallet.label}>
    <section class="section">
      <div class="field">
        <span class="field-label">Type</span>
        <span class="field-value">{walletKindLabel}</span>
      </div>
      <div class="field">
        <span class="field-label">Backend</span>
        <span class="field-value">{backendLabel}</span>
      </div>
      {#if !isSp && derivation}
        <div class="field">
          <span class="field-label">Script</span>
          <span class="field-value">
            {#if derivation.threshold !== null}
              {derivation.threshold}-of-{derivation.cosigners.length}
            {/if}
            {derivation.scriptType.label} ({derivation.scriptType.bip})
          </span>
        </div>
      {/if}
      {#if isSp}
        <div class="field">
          <span class="field-label">Derivation</span>
          <span class="field-value mono">m/352'/coin'/account'/...</span>
        </div>
      {/if}
    </section>

    {#if isSp}
      <section class="section">
        <div class="desc-block">
          <div class="desc-head">
            <span class="field-label">Silent Payment address</span>
            <CopyButton class="wd-copy-link" idle="Copy" copiedLabel="Copied" text={wallet.input} />
          </div>
          <pre class="desc-value">{wallet.input}</pre>
        </div>
        <p class="sp-note">
          One reusable address — senders derive a fresh on-chain destination per payment.
          The <strong>scan secret</strong> is persisted on disk (encrypted only by filesystem permissions) so the background scanner can find incoming payments.
          The <strong>spend secret</strong> is intentionally not stored — re-derived from your mnemonic on demand at send time.
        </p>
      </section>
    {:else}
      {#if derivation && derivation.threshold === null}
        <section class="section">
          <div class="field">
            <span class="field-label">Fingerprint</span>
            <span class="field-value mono accent">[{derivation.cosigners[0].fingerprint}]</span>
          </div>
          <div class="field">
            <span class="field-label">Account path</span>
            <span class="field-value mono">{derivation.cosigners[0].path}</span>
          </div>
        </section>
      {/if}

      {#if derivation && derivation.threshold !== null}
        <section class="section">
          <button
            class="disclosure"
            onclick={() => cosignersOpen = !cosignersOpen}
            aria-expanded={cosignersOpen}
          >
            <span class="disclosure-arrow" class:open={cosignersOpen}>▸</span>
            <span>Cosigners ({derivation.cosigners.length})</span>
          </button>
          {#if cosignersOpen}
            <ul class="cosigner-list">
              {#each derivation.cosigners as c, i (i)}
                <li>
                  <span class="mono accent">[{c.fingerprint}]</span>
                  <span class="mono">{c.path}</span>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      {/if}

      {#if showPolicy && policy}
        <section class="section">
          <div class="field-label">Spending policy</div>
          {#each policy.timelocks as t, i (i)}
            <div class="policy-timelock">
              <span aria-hidden="true">⏳</span>
              <span>{t.kind === 'relative' ? 'Recovery path unlocks after' : 'Recovery path unlocks at'} <strong>{t.label}</strong></span>
            </div>
          {/each}
          <pre class="desc-value policy-str">{policy.policy}</pre>
        </section>
      {/if}

      <section class="section">
        <div class="desc-block">
          <div class="desc-head">
            <span class="field-label">Receive descriptor</span>
            <CopyButton class="wd-copy-link" idle="Copy" copiedLabel="Copied" text={wallet.external_descriptor} />
          </div>
          <pre class="desc-value">{wallet.external_descriptor}</pre>
        </div>
        {#if wallet.internal_descriptor}
          <div class="desc-block">
            <div class="desc-head">
              <span class="field-label">Change descriptor</span>
              <CopyButton class="wd-copy-link" idle="Copy" copiedLabel="Copied" text={wallet.internal_descriptor ?? ''} />
            </div>
            <pre class="desc-value">{wallet.internal_descriptor}</pre>
          </div>
        {/if}
        {#if wallet.kind !== 'address'}
          <button class="export-desc-btn" onclick={downloadDescriptor} disabled={exporting}>
            {exporting ? 'Exporting…' : '↓ Download descriptor'}
          </button>
          <p class="export-desc-hint">Portable multipath descriptor for importing into Coldcard, Sparrow, or Bitcoin Core (air-gapped signing).</p>
        {/if}
      </section>
    {/if}
</Modal>

<style>
  .section { margin-bottom: 14px; }
  .section:last-child { margin-bottom: 0; }

  .field { display: flex; align-items: baseline; gap: 12px; padding: 4px 0; }
  .field-label { font-size: 0.76rem; color: var(--text-muted); width: 110px; flex-shrink: 0; }
  .field-value { font-size: 0.84rem; color: var(--text); word-break: break-all; }

  /* On narrow screens the 110px label column squeezes long values
     (paths, fingerprints) into tiny widths. Stack label above value
     so the value gets the full row width. */
  @media (max-width: 480px) {
    .field { flex-direction: column; gap: 2px; align-items: stretch; }
    .field-label { width: auto; }
  }
  .mono { font-family: monospace; font-size: 0.8rem; }
  .accent { color: var(--accent); }

  .disclosure {
    display: flex; align-items: center; gap: 6px;
    background: none; border: none; cursor: pointer;
    color: var(--text); font-size: 0.84rem; padding: 4px 0;
  }
  .disclosure-arrow {
    display: inline-block; font-size: 0.7rem; color: var(--text-muted);
    transition: transform 0.12s;
  }
  .disclosure-arrow.open { transform: rotate(90deg); }
  .cosigner-list {
    list-style: none; margin: 6px 0 0; padding: 0 0 0 14px;
    display: flex; flex-direction: column; gap: 4px;
  }
  .cosigner-list li { display: flex; gap: 8px; }

  .desc-block { margin-bottom: 10px; }
  .desc-block:last-child { margin-bottom: 0; }
  .desc-head {
    display: flex; align-items: baseline; justify-content: space-between;
    margin-bottom: 4px;
  }
  /* :global so the rule reaches the CopyButton child's <button> (scoped CSS
     doesn't cross component boundaries). */
  :global(.wd-copy-link) {
    background: none; border: none; cursor: pointer;
    color: var(--accent); font-size: 0.74rem; padding: 0;
  }
  :global(.wd-copy-link:hover) { text-decoration: underline; }
  .desc-value {
    margin: 0;
    font-family: monospace; font-size: 0.74rem;
    color: var(--text); background: var(--surface-2);
    border: 1px solid var(--border); border-radius: 4px;
    padding: 8px 10px;
    word-break: break-all; white-space: pre-wrap;
    max-height: 140px; overflow-y: auto;
  }
  .export-desc-btn {
    margin-top: 10px; background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 6px; color: var(--text); padding: 8px 12px; cursor: pointer;
    font-size: 0.8rem;
  }
  .export-desc-btn:hover:not(:disabled) { border-color: var(--accent); color: var(--accent); }
  .export-desc-btn:disabled { opacity: 0.5; cursor: default; }
  .export-desc-hint { margin: 6px 0 0; font-size: 0.72rem; color: var(--text-muted); line-height: 1.5; }
  .policy-str { margin-top: 8px; }
  .policy-timelock {
    display: flex; align-items: flex-start; gap: 7px; margin: 6px 0 0;
    font-size: 0.8rem; color: var(--text-muted); line-height: 1.5;
    padding: 7px 10px; border-radius: 5px;
    background: color-mix(in srgb, #e09c52 8%, transparent);
    border: 1px solid color-mix(in srgb, #e09c52 28%, transparent);
  }
  .policy-timelock strong { color: var(--text); }
  .sp-note {
    margin: 10px 0 0;
    font-size: 0.76rem; color: var(--text-muted); line-height: 1.55;
  }
  .sp-note strong { color: var(--text); }
</style>
