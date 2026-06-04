<script lang="ts">
  import { hwEnabled } from '../stores/settings'
  // One cosigner row in the multisig add-wallet flow. Owns only its own markup;
  // the parent keeps the signer list, the HW-connect EventSource, and file import.
  interface MsSigner { name: string; fingerprint: string; path: string; xpub: string; accountIndex: number }
  type MsHwStatus = 'idle' | 'connecting' | 'pairing' | 'confirm' | 'done' | 'error'

  let {
    signer = $bindable(),
    index,
    hwStatus,
    active,
    pairingCode,
    hwMessage,
    onConnect,
    onCancel,
    onFileImport,
  }: {
    signer: MsSigner
    index: number
    hwStatus: MsHwStatus
    /// True when this card is the one currently driving the HW-connect flow.
    active: boolean
    pairingCode: string
    hwMessage: string
    onConnect: () => void
    onCancel: () => void
    onFileImport: (e: Event) => void
  } = $props()
</script>

<div class="ms-signer-card" class:ms-signer-ready={!!signer.xpub}>
  <div class="ms-signer-header">
    <span class="ms-signer-label">Signer {index + 1}</span>
    <input
      class="ms-signer-name"
      type="text"
      placeholder="e.g. Cold storage, Laptop, Phone…"
      bind:value={signer.name}
      aria-label="Signer {index + 1} name"
    />
    {#if signer.xpub}<span class="ms-ready-dot" title="xpub set">✓</span>{/if}
  </div>

  <textarea
    class="ms-xpub-input"
    rows="2"
    placeholder="xpub… — paste, drop a descriptor file, or use Connect device below"
    spellcheck="false"
    autocapitalize="off"
    bind:value={signer.xpub}
  ></textarea>

  <div class="ms-origin-row">
    <input class="ms-fp-input" type="text" placeholder="Fingerprint (8 hex)" bind:value={signer.fingerprint} spellcheck="false" autocapitalize="off" />
    <input class="ms-path-input" type="text" placeholder="m/48'/0'/0'/2'" bind:value={signer.path} spellcheck="false" autocapitalize="off" />
  </div>

  <div class="ms-device-row">
    <div class="field ms-acct-field">
      <label for="ms-acct-{index}">Account</label>
      <input id="ms-acct-{index}" type="number" min="0" max="99" bind:value={signer.accountIndex} />
    </div>
    <label class="btn-file-sm" title="Import xpub from a Coldcard/Sparrow/Specter export">
      ↓ File
      <input type="file" accept=".json,.txt" onchange={onFileImport} />
    </label>
    {#if $hwEnabled}
      {#if hwStatus === 'idle' || hwStatus === 'done' || hwStatus === 'error' || !active}
        <button type="button" class="btn-connect-sm" onclick={onConnect}>
          Connect device
        </button>
      {:else}
        <button type="button" class="btn-cancel-hw" onclick={onCancel}>Cancel</button>
      {/if}
    {/if}
  </div>

  {#if active && hwStatus !== 'idle' && hwStatus !== 'done'}
    <div class="hw-status">
      {#if hwStatus === 'connecting'}
        <p class="hw-msg">Connecting…</p>
      {:else if hwStatus === 'pairing'}
        <p class="hw-msg">Verify pairing code:</p>
        <pre class="pairing-code">{pairingCode}</pre>
      {:else if hwStatus === 'confirm'}
        <p class="hw-msg">Confirm xpub export on device…</p>
      {:else if hwStatus === 'error'}
        <p class="hw-msg err">{hwMessage}</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  /* Inputs + field layout (copied from the add-wallet page — scoped styles don't
     cross component boundaries). */
  input, textarea {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); padding: 8px 10px; font-size: 0.85rem; width: 100%;
    box-sizing: border-box;
  }
  textarea { font-family: monospace; resize: none; }
  .field { display: flex; flex-direction: column; gap: 5px; }
  label[for] { font-size: 0.8rem; color: var(--text-muted); }

  .ms-signer-card {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px;
    padding: 10px 12px; display: flex; flex-direction: column; gap: 7px;
  }
  .ms-signer-card.ms-signer-ready { border-color: color-mix(in srgb, var(--accent) 35%, var(--border)); }
  .ms-signer-header { display: flex; align-items: center; justify-content: space-between; }
  .ms-signer-label { font-size: 0.75rem; font-weight: 700; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  .ms-ready-dot { font-size: 0.75rem; color: #52a875; }
  .ms-signer-name {
    flex: 1; min-width: 0;
    background: transparent; border: none; border-bottom: 1px dashed var(--border);
    color: var(--text); font-size: 0.85rem; padding: 2px 4px; margin: 0 8px;
    border-radius: 0;
  }
  .ms-signer-name:focus { outline: none; border-bottom-color: var(--accent); }
  .ms-xpub-input {
    width: 100%; font-family: monospace; font-size: 0.75rem; resize: none;
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 4px;
    color: var(--text); padding: 6px 8px;
  }
  .ms-xpub-input:focus { outline: 1px solid var(--accent); }
  .ms-origin-row { display: flex; gap: 6px; }
  .ms-fp-input { width: 130px !important; font-family: monospace; font-size: 0.75rem; }
  .ms-path-input { flex: 1; font-family: monospace; font-size: 0.75rem; }
  .ms-device-row { display: flex; align-items: flex-end; gap: 8px; }
  .ms-acct-field { width: 72px; flex-shrink: 0; }
  .btn-connect-sm {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 4px;
    color: var(--text); cursor: pointer; font-size: 0.78rem; padding: 6px 10px; white-space: nowrap;
  }
  .btn-connect-sm:hover { border-color: var(--accent); color: var(--accent); }
  .btn-file-sm {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 4px;
    color: var(--text-muted); cursor: pointer; font-size: 0.78rem; padding: 6px 10px;
    white-space: nowrap; display: inline-block;
  }
  .btn-file-sm:hover { border-color: var(--accent); color: var(--accent); }
  .btn-file-sm input[type="file"] { display: none; }
  .btn-cancel-hw {
    background: var(--surface-2); color: var(--text); border: 1px solid var(--border);
    border-radius: 5px; padding: 8px 14px; cursor: pointer; font-size: 0.85rem;
    align-self: flex-start;
  }
  .hw-status { min-height: 24px; }
  .hw-msg { margin: 0; font-size: 0.82rem; color: var(--text-muted); }
  .hw-msg.err { color: var(--error); }
  .pairing-code {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px;
    padding: 10px 14px; font-family: monospace; font-size: 0.9rem;
    letter-spacing: 0.05em; margin: 4px 0 0; white-space: pre;
  }
</style>
