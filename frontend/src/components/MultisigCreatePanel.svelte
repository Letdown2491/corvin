<script lang="ts">
  // Multisig wallet creation. Self-contained like VaultCreatePanel/SpCreatePanel:
  // owns signer rows, the per-signer HW-connect EventSource, localStorage draft
  // persistence, validation, and the create call. `label` is bindable so a
  // restored draft can populate the parent's shared Name field.
  import { onMount, onDestroy } from 'svelte'
  import { api } from '../lib/api'
  import { parseImportFile, parseMultisigDescriptor } from '../lib/import'
  import type { WalletEntry, BackendEntry } from '../lib/types'
  import MsSignerCard from './MsSignerCard.svelte'
  import BusyButton from './ui/BusyButton.svelte'

  let { label = $bindable(), onCreated, savedBackends = [] }: {
    label: string
    onCreated: (e: WalletEntry) => void
    savedBackends?: BackendEntry[]
  } = $props()

  let backend = $state<string | null>(null)
  let error = $state('')

  interface MsSigner { name: string; fingerprint: string; path: string; xpub: string; accountIndex: number }
  let msThreshold = $state(2)
  let msTotal = $state(3)
  let msSigners = $state<MsSigner[]>([
    { name: '', fingerprint: '', path: '', xpub: '', accountIndex: 0 },
    { name: '', fingerprint: '', path: '', xpub: '', accountIndex: 0 },
    { name: '', fingerprint: '', path: '', xpub: '', accountIndex: 0 },
  ])

  // Paste-a-descriptor UX: textarea + parser that fills the signer cards from a
  // `wsh(sortedmulti(M, [fp/path]xpub/0/*, ...))` string. Mirrors the backend's
  // parse_wsh_sortedmulti so we can validate locally before submit.
  let msDescriptorInput = $state('')
  let msDescriptorImportError = $state('')

  function applyMultisigDescriptor() {
    msDescriptorImportError = ''
    const parsed = parseMultisigDescriptor(msDescriptorInput)
    if (!parsed) {
      msDescriptorImportError =
        "Couldn't parse — expected a wsh(sortedmulti(M, [fp/path]xpub.../0/*, ...)) descriptor. " +
        "Each cosigner needs the [fingerprint/path] origin tag."
      return
    }
    msThreshold = parsed.threshold
    msTotal = parsed.signers.length
    msSigners = parsed.signers
    msDescriptorInput = ''
  }

  let msActiveSignerIdx = $state(-1)
  let msHwEventSource: EventSource | null = null
  type MsHwStatus = 'idle' | 'connecting' | 'pairing' | 'confirm' | 'done' | 'error'
  let msHwStatus = $state<MsHwStatus>('idle')
  let msHwMessage = $state('')
  let msPairingCode = $state('')

  // Sync signer card count to N — only reads msTotal, never touches msThreshold.
  $effect(() => {
    const n = Math.max(2, Math.min(15, msTotal))
    if (msSigners.length < n) {
      msSigners = [
        ...msSigners,
        ...Array.from({ length: n - msSigners.length }, () => ({
          name: '', fingerprint: '', path: '', xpub: '', accountIndex: 0,
        })),
      ]
    } else if (msSigners.length > n) {
      msSigners = msSigners.slice(0, n)
    }
  })

  // Clamp M to [1, N] — runs when either changes, but never touches N.
  $effect(() => {
    if (msThreshold > msTotal) msThreshold = msTotal
    if (msThreshold < 1) msThreshold = 1
  })

  function handleSignerFileImport(i: number, e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0]
    if (file) importSignerFile(i, file)
    ;(e.target as HTMLInputElement).value = ''
  }

  function importSignerFile(i: number, file: File) {
    const reader = new FileReader()
    reader.onload = () => {
      const parsed = parseImportFile(reader.result as string)
      if (!parsed) { error = 'Could not extract an xpub from this file.'; return }
      if (!parsed.fingerprint || !parsed.path) {
        error = 'This file has an xpub but no origin info (fingerprint + path). Multisig signing requires the origin — try a descriptor file or the device\'s "Connect device" option.'
        return
      }
      const updated = [...msSigners]
      updated[i] = { ...updated[i], xpub: parsed.xpub, fingerprint: parsed.fingerprint, path: parsed.path }
      msSigners = updated
      error = ''
    }
    reader.readAsText(file)
  }

  // Live descriptor preview: build a representative wsh(sortedmulti(...)) from
  // the filled signer cards so the user can sanity-check before submitting.
  let msDescriptorPreview = $derived.by((): { external: string; internal: string } | null => {
    const filled = msSigners.filter(s => s.xpub.trim() && s.fingerprint.trim() && s.path.trim())
    if (filled.length === 0 || msThreshold < 1) return null
    const keys = filled.map(s => {
      const fp = s.fingerprint.trim().toLowerCase()
      const path = s.path.trim().replace(/^m\//, '')
      return `[${fp}/${path}]${s.xpub.trim()}`
    })
    return {
      external: `wsh(sortedmulti(${msThreshold},${keys.map(k => `${k}/0/*`).join(',')}))`,
      internal: `wsh(sortedmulti(${msThreshold},${keys.map(k => `${k}/1/*`).join(',')}))`,
    }
  })

  // Quick visual lint for the multisig signer set.
  let msSignerWarnings = $derived.by((): string[] => {
    const out: string[] = []
    const filled = msSigners.filter(s => s.xpub.trim() && s.fingerprint.trim() && s.path.trim())
    if (filled.length === 0) return out
    const xpubs = new Set<string>()
    for (const s of filled) {
      const x = s.xpub.trim()
      if (xpubs.has(x)) { out.push(`Duplicate xpub on multiple signers`); break }
      xpubs.add(x)
    }
    const fps = filled.map(s => s.fingerprint.trim().toLowerCase())
    if (new Set(fps).size !== fps.length) out.push('Duplicate fingerprint — make sure each signer is a distinct device')
    const paths = new Set(filled.map(s => s.path.trim().replace(/^m\//, '')))
    if (paths.size > 1) out.push('Signers use different derivation paths — usually they should all be the same (e.g. m/48\'/0\'/0\'/2\')')
    for (const s of filled) {
      const fp = s.fingerprint.trim()
      if (!/^[0-9a-fA-F]{8}$/.test(fp)) { out.push(`Fingerprint "${fp}" should be 8 hex characters`); break }
    }
    return out
  })

  function connectMsDevice(i: number) {
    // Close any prior stream before reassigning so its handlers can't fire late
    // and overwrite a different signer's data.
    msHwEventSource?.close()
    msHwEventSource = null

    msActiveSignerIdx = i
    msHwStatus = 'connecting'; msHwMessage = ''; msPairingCode = ''
    const idx = msSigners[i].accountIndex
    const url = `/api/hwi/xpub?account=multisig_p2wsh&account_index=${idx}`
    const es = new EventSource(url)
    msHwEventSource = es
    // Capture i so a late-arriving event after msActiveSignerIdx changed doesn't
    // clobber the wrong signer.
    const targetIdx = i
    es.addEventListener('connecting', () => { msHwStatus = 'connecting' })
    es.addEventListener('pairing_code', (e) => {
      msPairingCode = JSON.parse(e.data).code; msHwStatus = 'pairing'
    })
    es.addEventListener('waiting_confirm', () => { msHwStatus = 'confirm' })
    es.addEventListener('paired', () => { msHwStatus = 'confirm'; msPairingCode = '' })
    es.addEventListener('xpub', (e) => {
      const data = JSON.parse(e.data)
      const updated = [...msSigners]
      updated[targetIdx] = {
        ...updated[targetIdx],
        xpub: data.xpub ?? '',
        fingerprint: data.fingerprint ?? '',
        path: data.path ?? '',
      }
      msSigners = updated
      msHwStatus = 'done'
      es.close(); if (msHwEventSource === es) msHwEventSource = null
    })
    es.addEventListener('hw_error', (e) => {
      msHwMessage = JSON.parse(e.data).message; msHwStatus = 'error'
      es.close(); if (msHwEventSource === es) msHwEventSource = null
    })
    es.onerror = () => {
      if (msHwStatus !== 'done' && msHwStatus !== 'error') { msHwMessage = 'Connection lost.'; msHwStatus = 'error' }
      es.close(); if (msHwEventSource === es) msHwEventSource = null
    }
  }

  function cancelMsDevice() {
    msHwEventSource?.close(); msHwEventSource = null
    msHwStatus = 'idle'; msHwMessage = ''; msPairingCode = ''
  }

  // Draft persistence: stash signer rows in localStorage so a user who's halfway
  // through doesn't lose their work on a refresh.
  const MS_DRAFT_KEY = 'corvin:add-wallet:multisig-draft'
  function saveMsDraft() {
    try {
      const filled = msSigners.some(s => s.xpub || s.fingerprint || s.path || s.name)
      if (!filled) { localStorage.removeItem(MS_DRAFT_KEY); return }
      // Signer XPUBs are not persisted: localStorage lives in the (unencrypted)
      // webview profile, outside at-rest encryption, and an xpub derives the whole
      // wallet's address set + history. Keep the rest of the row; re-fetch the xpub.
      const safeSigners = msSigners.map(s => ({ ...s, xpub: '' }))
      localStorage.setItem(MS_DRAFT_KEY, JSON.stringify({ label, msThreshold, msTotal, msSigners: safeSigners }))
    } catch {} // localStorage may be disabled — silently skip
  }
  function loadMsDraft() {
    try {
      const raw = localStorage.getItem(MS_DRAFT_KEY)
      if (!raw) return
      const draft = JSON.parse(raw)
      if (typeof draft.msThreshold === 'number') msThreshold = draft.msThreshold
      if (typeof draft.msTotal === 'number') msTotal = draft.msTotal
      if (Array.isArray(draft.msSigners)) {
        // Force xpub empty (drops any xpub left by an older draft).
        msSigners = draft.msSigners.map((s: Partial<MsSigner>) => ({
          name: s.name ?? '',
          fingerprint: s.fingerprint ?? '',
          path: s.path ?? '',
          xpub: '',
          accountIndex: typeof s.accountIndex === 'number' ? s.accountIndex : 0,
        }))
      }
      if (typeof draft.label === 'string' && !label) label = draft.label
    } catch {}
  }
  function clearMsDraft() {
    try { localStorage.removeItem(MS_DRAFT_KEY) } catch {}
  }

  onMount(loadMsDraft)

  // Auto-save the draft whenever it changes.
  $effect(() => {
    void msSigners; void msThreshold; void msTotal; void label
    saveMsDraft()
  })

  onDestroy(() => { msHwEventSource?.close() })

  let canCreate = $derived.by(() => {
    if (!label.trim()) return false
    if (msThreshold < 1 || msThreshold > msTotal || msTotal < 2) return false
    return msSigners.every(s => s.xpub.trim().length > 0 && s.fingerprint.trim().length > 0 && s.path.trim().length > 0)
  })

  async function create() {
    if (!canCreate) return
    error = ''
    try {
      const entry = await api.wallets.multisigCreate({
        label: label.trim(),
        threshold: msThreshold,
        signers: msSigners.map(s => ({ fingerprint: s.fingerprint, path: s.path, xpub: s.xpub.trim() })),
        backend,
      })
      clearMsDraft()
      onCreated(entry)
    } catch (e) {
      error = e instanceof Error ? e.message : 'Unknown error'
    }
  }
</script>

<div class="ms-panel">
  <div class="field">
    <label for="ms-backend">Backend</label>
    <select id="ms-backend" bind:value={backend}>
      <option value={null}>Default</option>
      {#each savedBackends as b (b.id)}
        <option value={b.id}>{b.label}</option>
      {/each}
    </select>
    <p class="field-hint">
      Which server this wallet syncs and broadcasts through. “Default” uses your main backend; pick a saved backend to keep this wallet's activity off your other servers. Add backends in Backend settings.
    </p>
  </div>

  <!-- Paste-descriptor shortcut: replaces all the manual signer cards with one
       parsed from a wsh(sortedmulti(...)) string. -->
  <div class="ms-paste-block">
    <label for="ms-paste-desc" class="ms-paste-label">
      Paste full descriptor (optional shortcut)
    </label>
    <textarea
      id="ms-paste-desc"
      class="ms-paste-input"
      rows="3"
      placeholder="wsh(sortedmulti(2,[fp1/48'/0'/0'/2']xpub.../0/*,[fp2/48'/0'/0'/2']xpub.../0/*,...))"
      bind:value={msDescriptorInput}
      autocomplete="off"
      spellcheck="false"
    ></textarea>
    <div class="ms-paste-actions">
      <button
        type="button"
        class="ms-paste-btn"
        disabled={!msDescriptorInput.trim()}
        onclick={applyMultisigDescriptor}
      >Fill from descriptor</button>
      <span class="ms-paste-hint">Or fill the cards below manually.</span>
    </div>
    {#if msDescriptorImportError}
      <p class="ms-paste-error">{msDescriptorImportError}</p>
    {/if}
  </div>

  <div class="ms-threshold-row">
    <span class="field-label">Require</span>
    <input class="ms-thresh-input" type="number" min="1" max={msTotal} bind:value={msThreshold} />
    <span class="field-label">of</span>
    <input class="ms-thresh-input" type="number" min="2" max="15" bind:value={msTotal} />
    <span class="field-label">signers</span>
  </div>

  {#each msSigners as signer, i (i)}
    <MsSignerCard
      bind:signer={msSigners[i]}
      index={i}
      hwStatus={msHwStatus}
      active={msActiveSignerIdx === i}
      pairingCode={msPairingCode}
      hwMessage={msHwMessage}
      onConnect={() => connectMsDevice(i)}
      onCancel={cancelMsDevice}
      onFileImport={(e) => handleSignerFileImport(i, e)}
    />
  {/each}

  {#if msSignerWarnings.length > 0}
    <div class="ms-warnings">
      {#each msSignerWarnings as w, i (i)}
        <div class="ms-warning"><span class="ms-warn-icon">⚠</span> {w}</div>
      {/each}
    </div>
  {/if}

  {#if msDescriptorPreview}
    <details class="ms-descriptor-preview">
      <summary>Resulting descriptor preview</summary>
      <div class="ms-desc-block">
        <span class="ms-desc-label">External (receive)</span>
        <code class="ms-desc-code">{msDescriptorPreview.external}</code>
      </div>
      <div class="ms-desc-block">
        <span class="ms-desc-label">Internal (change)</span>
        <code class="ms-desc-code">{msDescriptorPreview.internal}</code>
      </div>
      <p class="ms-desc-hint">Verify each signer's <code class="mono-inline">[fingerprint/path]xpub</code> against the device's own export before adding the wallet.</p>
    </details>
  {/if}

  {#if error}<p class="error">{error}</p>{/if}

  <div class="actions">
    <BusyButton idle="Add wallet" busyLabel="Adding…" disabled={!canCreate} onclick={create} />
  </div>
</div>

<style>
  .ms-panel { display: flex; flex-direction: column; gap: 16px; }

  .field { display: flex; flex-direction: column; gap: 5px; }
  .field-label { font-size: 0.8rem; color: var(--text-muted); }
  label[for] { font-size: 0.8rem; color: var(--text-muted); }
  input, textarea, select {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); padding: 8px 10px; font-size: 0.85rem; width: 100%;
  }
  input:focus, textarea:focus, select:focus { outline: 1px solid var(--accent); }
  textarea { font-family: monospace; resize: none; }
  select { font-family: inherit; }
  input[type="number"] { appearance: textfield; -moz-appearance: textfield; }

  .field-hint { font-size: 0.75rem; color: var(--text-muted); line-height: 1.4; }
  .mono-inline { font-family: monospace; font-size: 0.7rem; background: var(--surface-1); padding: 1px 4px; border-radius: 3px; }

  .ms-paste-block {
    display: flex; flex-direction: column; gap: 6px;
    margin-bottom: 14px; padding: 10px 12px;
    border: 1px dashed var(--border); border-radius: 6px;
    background: color-mix(in srgb, var(--accent) 3%, transparent);
  }
  .ms-paste-label {
    font-size: 0.72rem; color: var(--text-muted); font-weight: 600;
    text-transform: uppercase; letter-spacing: 0.05em;
  }
  .ms-paste-input {
    width: 100%; box-sizing: border-box;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 4px; color: var(--text); padding: 6px 8px;
    font-family: monospace; font-size: 0.74rem; outline: none; resize: vertical;
  }
  .ms-paste-input:focus { border-color: var(--accent); }
  .ms-paste-actions { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .ms-paste-btn {
    background: var(--accent); color: #000; border: none; border-radius: 4px;
    padding: 5px 12px; cursor: pointer; font-size: 0.78rem; font-weight: 600;
  }
  .ms-paste-btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .ms-paste-hint { font-size: 0.72rem; color: var(--text-muted); }
  .ms-paste-error {
    margin: 4px 0 0; font-size: 0.74rem; color: #e05252; line-height: 1.4;
  }

  .ms-threshold-row { display: flex; align-items: center; gap: 8px; }
  .ms-thresh-input {
    width: 52px !important; text-align: center; font-weight: 600;
    padding: 6px 6px !important;
  }

  .ms-warnings {
    display: flex; flex-direction: column; gap: 4px;
    background: color-mix(in srgb, #e09c52 6%, var(--surface-2));
    border: 1px solid color-mix(in srgb, #e09c52 40%, var(--border));
    border-radius: 5px; padding: 8px 10px;
  }
  .ms-warning { font-size: 0.78rem; color: var(--text); display: flex; gap: 6px; align-items: flex-start; }
  .ms-warn-icon { color: #e09c52; flex-shrink: 0; }

  .ms-descriptor-preview {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    padding: 6px 12px;
  }
  .ms-descriptor-preview summary {
    cursor: pointer; font-size: 0.78rem; color: var(--text-muted);
    padding: 4px 0;
  }
  .ms-descriptor-preview summary:hover { color: var(--text); }
  .ms-descriptor-preview[open] summary { color: var(--text); margin-bottom: 8px; }
  .ms-desc-block { display: flex; flex-direction: column; gap: 3px; margin-bottom: 8px; }
  .ms-desc-label { font-size: 0.7rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .ms-desc-code {
    font-family: monospace; font-size: 0.7rem; color: var(--text);
    background: var(--surface-1); padding: 6px 8px; border-radius: 4px;
    word-break: break-all; line-height: 1.4;
  }
  .ms-desc-hint { font-size: 0.72rem; color: var(--text-muted); line-height: 1.5; margin: 0; }

  .error { color: var(--error); font-size: 0.85rem; margin: 0; }

  .actions { display: flex; justify-content: flex-end; margin-top: 6px; }
</style>
