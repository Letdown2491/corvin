<script lang="ts">
  import type { WalletEntry } from '../lib/types'

  let {
    wallet,
    txCount,
    canTestBackup,
    onTaxReport,
    onExportCsv,
    onExportDescriptor,
    onExportSpKeys,
    onExportMultisigConfig,
    onAddressLookup,
    onPsbtInspector,
    onSignVerify,
    onTestBackup,
    onSweepWif,
  }: {
    wallet: WalletEntry
    txCount: number
    canTestBackup: boolean
    onTaxReport: () => void
    onExportCsv: () => void
    onExportDescriptor: () => void
    onExportSpKeys: () => void
    onExportMultisigConfig: () => void
    onAddressLookup: () => void
    onPsbtInspector: () => void
    onSignVerify: () => void
    onTestBackup: () => void
    onSweepWif: () => void
  } = $props()
</script>

<div class="tools-grid">
  <button class="tool-card" onclick={onTaxReport}>
    <div class="tool-card-header">
      <span class="tool-icon" aria-hidden="true">$</span>
      <h3>Tax report</h3>
    </div>
    <p class="tool-desc">
      Capital gains and losses for a chosen year, exported as CSV.
    </p>
  </button>

  <button class="tool-card" onclick={onExportCsv} disabled={txCount === 0}>
    <div class="tool-card-header">
      <span class="tool-icon" aria-hidden="true">↓</span>
      <h3>Export transactions</h3>
    </div>
    <p class="tool-desc">
      CSV of every transaction in this wallet — for spreadsheets or accounting.
    </p>
  </button>

  {#if wallet.kind === 'silent_payments'}
    <button class="tool-card" onclick={onExportSpKeys}>
      <div class="tool-card-header">
        <span class="tool-icon" aria-hidden="true">⎘</span>
        <h3>Export watch-only keys</h3>
      </div>
      <p class="tool-desc">
        Scan key + spend pubkey for a watch-only Silent Payments view elsewhere. The scan key reveals received payments.
      </p>
    </button>
  {:else}
    <button class="tool-card" onclick={onExportDescriptor}>
      <div class="tool-card-header">
        <span class="tool-icon" aria-hidden="true">⎘</span>
        <h3>Export descriptor</h3>
      </div>
      <p class="tool-desc">
        Plain-text descriptor for setting up a watch-only view in Sparrow, Specter, etc.
      </p>
    </button>
  {/if}

  {#if wallet.kind === 'multisig'}
    <button class="tool-card" onclick={onExportMultisigConfig}>
      <div class="tool-card-header">
        <span class="tool-icon" aria-hidden="true">⛁</span>
        <h3>Export multisig config</h3>
      </div>
      <p class="tool-desc">
        Coldcard/Specter setup file for restoring this multisig on another device.
      </p>
    </button>
  {/if}

  <button class="tool-card" onclick={onAddressLookup}>
    <div class="tool-card-header">
      <span class="tool-icon" aria-hidden="true">⌕</span>
      <h3>Address lookup</h3>
    </div>
    <p class="tool-desc">
      Paste an address to check if this wallet derived it and see its derivation path.
    </p>
  </button>

  <button class="tool-card" onclick={onPsbtInspector}>
    <div class="tool-card-header">
      <span class="tool-icon" aria-hidden="true">⊡</span>
      <h3>PSBT inspector</h3>
    </div>
    <p class="tool-desc">
      Decode any PSBT to see inputs, outputs, and fees before signing. Don't sign blind.
    </p>
  </button>

  <button class="tool-card" onclick={onSignVerify}>
    <div class="tool-card-header">
      <span class="tool-icon" aria-hidden="true">✎</span>
      <h3>Sign &amp; verify message</h3>
    </div>
    <p class="tool-desc">
      BIP-322 signing — prove you own an address, or verify someone else's signature.
    </p>
  </button>

  {#if wallet.kind !== 'address'}
    <button class="tool-card" onclick={onSweepWif}>
      <div class="tool-card-header">
        <span class="tool-icon" aria-hidden="true">↺</span>
        <h3>Sweep private key</h3>
      </div>
      <p class="tool-desc">
        Drain a WIF private key (e.g. a paper wallet) into this wallet. Scans legacy, SegWit, and wrapped-SegWit script types.
      </p>
    </button>
  {/if}

  {#if canTestBackup}
    <button class="tool-card" onclick={onTestBackup}>
      <div class="tool-card-header">
        <span class="tool-icon" aria-hidden="true">✓</span>
        <h3>Test backup</h3>
      </div>
      <p class="tool-desc">
        Re-derive this wallet's descriptor from a seed phrase to catch backup typos.
      </p>
    </button>
  {/if}
</div>

<style>
  .tools-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 14px;
    padding-top: 4px;
  }
  .tool-card {
    text-align: left;
    background: var(--surface-1); border: 1px solid var(--border);
    border-radius: 8px; padding: 16px 18px;
    cursor: pointer; color: var(--text);
    transition: border-color 0.12s, background 0.12s;
    display: flex; flex-direction: column; gap: 6px;
  }
  .tool-card:hover:not(:disabled) {
    border-color: var(--text-muted);
    background: var(--surface-hover);
  }
  .tool-card:disabled { opacity: 0.4; cursor: not-allowed; }
  .tool-card-header { display: flex; align-items: center; gap: 10px; }
  .tool-card h3 {
    margin: 0; font-size: 0.95rem; font-weight: 600; color: var(--text);
  }
  .tool-icon {
    display: inline-flex; align-items: center; justify-content: center;
    width: 28px; height: 28px; border-radius: 6px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    font-size: 0.95rem; font-weight: 700; flex-shrink: 0;
  }
  .tool-desc {
    margin: 0; font-size: 0.82rem; color: var(--text-muted);
    line-height: 1.45;
  }
</style>
