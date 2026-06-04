<script lang="ts">
  import { onDestroy } from 'svelte'
  import { parseImportFile } from '../lib/import'
  import { hwEnabled } from '../stores/settings'

  export interface PolicySig { fingerprint: string; path: string; xpub: string; accountIndex: number }

  let {
    signer = $bindable(),
    title,
    onRemove,
    recovery = false,
    pathPlaceholder = "m/48'/0'/0'/2'",
    hwAccount = 'multisig_p2wsh',
  }: {
    signer: PolicySig
    title: string
    onRemove?: () => void
    recovery?: boolean
    pathPlaceholder?: string
    // Which derivation branch "Connect device" pulls: BIP-48 P2WSH for wsh
    // policies, BIP-86 for taproot.
    hwAccount?: 'multisig_p2wsh' | 'taproot'
  } = $props()

  type HwStatus = 'idle' | 'connecting' | 'pairing' | 'confirm' | 'done' | 'error'
  let hwStatus = $state<HwStatus>('idle')
  let hwMessage = $state('')
  let hwPairingCode = $state('')
  let error = $state('')
  let source: EventSource | null = null

  // Account is only relevant for device connect; keep it tucked away unless the
  // user opts into a non-default derivation account.
  let showAccount = $state(false)

  let connecting = $derived(hwStatus === 'connecting' || hwStatus === 'pairing' || hwStatus === 'confirm')

  function connectDevice() {
    source?.close(); source = null
    hwStatus = 'connecting'; hwMessage = ''; hwPairingCode = ''; error = ''
    const es = new EventSource(`/api/hwi/xpub?account=${hwAccount}&account_index=${signer.accountIndex}`)
    source = es
    es.addEventListener('connecting', () => { hwStatus = 'connecting' })
    es.addEventListener('pairing_code', (e) => { hwPairingCode = JSON.parse(e.data).code; hwStatus = 'pairing' })
    es.addEventListener('waiting_confirm', () => { hwStatus = 'confirm' })
    es.addEventListener('paired', () => { hwStatus = 'confirm'; hwPairingCode = '' })
    es.addEventListener('xpub', (e) => {
      const data = JSON.parse(e.data)
      signer.xpub = data.xpub ?? ''
      signer.fingerprint = data.fingerprint ?? signer.fingerprint
      signer.path = data.path ?? signer.path
      hwStatus = 'done'
      es.close(); if (source === es) source = null
    })
    es.addEventListener('hw_error', (e) => {
      hwMessage = JSON.parse(e.data).message; hwStatus = 'error'
      es.close(); if (source === es) source = null
    })
    es.onerror = () => {
      if (hwStatus !== 'done' && hwStatus !== 'error') { hwMessage = 'Connection lost.'; hwStatus = 'error' }
      es.close(); if (source === es) source = null
    }
  }

  function cancelDevice() {
    source?.close(); source = null
    hwStatus = 'idle'; hwMessage = ''; hwPairingCode = ''
  }

  function handleFile(e: Event) {
    const fileInput = e.target as HTMLInputElement
    const file = fileInput.files?.[0]
    fileInput.value = ''
    if (!file) return
    const reader = new FileReader()
    reader.onload = () => {
      const parsed = parseImportFile(reader.result as string)
      if (!parsed) { error = 'Could not extract an xpub from this file.'; return }
      if (!parsed.fingerprint || !parsed.path) {
        error = 'This file has an xpub but no origin (fingerprint + path). Signing needs the origin — export a descriptor file or use Connect device.'
        return
      }
      signer.xpub = parsed.xpub
      signer.fingerprint = parsed.fingerprint
      signer.path = parsed.path
      error = ''
    }
    reader.readAsText(file)
  }

  onDestroy(() => { source?.close() })
</script>

<div class="sig-card" class:recovery>
  <div class="sig-card-head">
    <span class="sig-num">{title}{#if signer.xpub}<span class="ready" title="key set"> ✓</span>{/if}</span>
    {#if onRemove}<button type="button" class="rm" onclick={onRemove} aria-label="Remove">✕</button>{/if}
  </div>
  <div class="sig-row">
    <input class="inp fp" bind:value={signer.fingerprint} placeholder="fingerprint (8 hex)" spellcheck="false" />
    <input class="inp path" bind:value={signer.path} placeholder={pathPlaceholder} spellcheck="false" />
  </div>
  <input class="inp" bind:value={signer.xpub} placeholder="xpub / Zpub…" spellcheck="false" autocapitalize="off" />

  <div class="device-row">
    <label class="btn-file" title="Import xpub from a Coldcard/Sparrow/Specter export">
      ↓ File
      <input type="file" accept=".json,.txt" onchange={handleFile} />
    </label>
    {#if $hwEnabled}
      {#if connecting}
        <button type="button" class="btn-cancel" onclick={cancelDevice}>Cancel</button>
      {:else}
        <button type="button" class="btn-connect" onclick={connectDevice}>Connect device</button>
      {/if}
      {#if !(showAccount || signer.accountIndex !== 0)}
        <button type="button" class="acct-toggle" onclick={() => showAccount = true} title="Use a non-default derivation account (advanced)">Account…</button>
      {/if}
    {/if}
  </div>
  {#if showAccount || signer.accountIndex !== 0}
    <div class="acct-row">
      <label for="acct-{title}">Device account</label>
      <input id="acct-{title}" type="number" min="0" max="99" bind:value={signer.accountIndex} />
      <span class="acct-hint">derivation account on the connected device</span>
    </div>
  {/if}

  {#if hwStatus !== 'idle' && hwStatus !== 'done'}
    <div class="hw-status">
      {#if hwStatus === 'connecting'}<p class="hw-msg">Connecting…</p>
      {:else if hwStatus === 'pairing'}<p class="hw-msg">Verify pairing code:</p><pre class="pairing-code">{hwPairingCode}</pre>
      {:else if hwStatus === 'confirm'}<p class="hw-msg">Confirm xpub export on device…</p>
      {:else if hwStatus === 'error'}<p class="hw-msg err">{hwMessage}</p>{/if}
    </div>
  {/if}
  {#if error}<p class="hw-msg err">{error}</p>{/if}
</div>

<style>
  .sig-card { background: var(--surface-2); border: 1px solid var(--border); border-radius: 7px; padding: 10px 12px; display: flex; flex-direction: column; gap: 7px; }
  .sig-card.recovery { border-color: color-mix(in srgb, #e09c52 35%, var(--border)); }
  .sig-card-head { display: flex; align-items: center; justify-content: space-between; }
  .sig-num { font-size: 0.74rem; font-weight: 600; color: var(--text-muted); }
  .ready { color: var(--accent); }
  .rm { background: none; border: none; color: var(--text-muted); cursor: pointer; font-size: 0.85rem; padding: 0; }
  .rm:hover { color: var(--error); }
  .sig-row { display: flex; gap: 7px; }
  .inp { background: var(--surface-1); border: 1px solid var(--border); border-radius: 5px; color: var(--text); padding: 7px 9px; font-size: 0.8rem; font-family: monospace; width: 100%; box-sizing: border-box; }
  .inp:focus { outline: 1px solid var(--accent); outline-offset: -1px; }
  .inp.fp { flex: 0 0 40%; }
  .inp.path { flex: 1; }
  .device-row { display: flex; align-items: center; gap: 8px; margin-top: 2px; }
  .acct-toggle { background: none; border: none; color: var(--text-muted); cursor: pointer; font-size: 0.74rem; padding: 6px 4px; }
  .acct-toggle:hover { color: var(--text); text-decoration: underline; }
  .acct-row { display: flex; align-items: center; gap: 8px; margin-top: 2px; }
  .acct-row label { font-size: 0.72rem; color: var(--text-muted); }
  .acct-row input { width: 56px; background: var(--surface-1); border: 1px solid var(--border); border-radius: 5px; color: var(--text); padding: 5px 7px; font-size: 0.78rem; text-align: center; }
  .acct-row input::-webkit-inner-spin-button, .acct-row input::-webkit-outer-spin-button { -webkit-appearance: none; margin: 0; }
  .acct-row input { -moz-appearance: textfield; appearance: textfield; }
  .acct-hint { font-size: 0.7rem; color: var(--text-muted); }
  .btn-file, .btn-connect, .btn-cancel {
    font-size: 0.74rem; padding: 6px 10px; border-radius: 5px; cursor: pointer;
    border: 1px solid var(--border); background: var(--surface-1); color: var(--text);
  }
  .btn-file { position: relative; overflow: hidden; }
  .btn-file input { position: absolute; inset: 0; opacity: 0; cursor: pointer; }
  .btn-connect { border-color: var(--accent); color: var(--accent); margin-left: auto; }
  .btn-connect:hover { background: color-mix(in srgb, var(--accent) 12%, transparent); }
  .btn-cancel { margin-left: auto; }
  .btn-file:hover, .btn-cancel:hover { background: var(--surface-active); }
  .hw-status { font-size: 0.76rem; }
  .hw-msg { margin: 4px 0 0; color: var(--text-muted); }
  .hw-msg.err { color: var(--error); }
  .pairing-code { margin: 4px 0 0; font-family: monospace; font-size: 0.9rem; color: var(--text); }
</style>
