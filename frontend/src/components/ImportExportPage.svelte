<script lang="ts">
  import { onDestroy } from 'svelte'
  import { api } from '../lib/api'
  import { downloadBlob } from '../lib/utils'
  import { encryptBackup, decryptBackup, isEncryptedEnvelope } from '../lib/backup_crypto'
  import { securityState } from '../stores/security'
  import BusyButton from './ui/BusyButton.svelte'

  // At-rest encryption protects this device's config dir, not an exported file.
  let atRestOn = $derived($securityState === 'unlocked')

  let importing = $state(false)
  let decrypting = $state(false)
  let importError = $state('')
  let importSuccess = $state(false)
  let exportError = $state('')
  let exportPassphrase = $state('')
  let fileInputEl = $state<HTMLInputElement | null>(null)
  let successTimer: ReturnType<typeof setTimeout> | null = null

  // BIP-329 label interop
  let bip329FileInputEl = $state<HTMLInputElement | null>(null)
  let bip329Importing = $state(false)
  let bip329ImportError = $state('')
  let bip329ImportReplace = $state(false)
  let bip329ImportSummary = $state<{ tx_labels: number; address_labels: number; utxo_labels: number; frozen_changes: number; skipped: number } | null>(null)

  // Restore dry-run state — what's in the file before applying.
  let stagedFile = $state<{
    name: string
    parsed: unknown
    encrypted: boolean
    summary: { wallets: number; labels: number; cost_basis: number; address_labels: number; silent_payments: number } | null
  } | null>(null)
  let importPassphrase = $state('')

  onDestroy(() => { if (successTimer) clearTimeout(successTimer) })

  async function exportBackup() {
    exportError = ''
    // Move the passphrase into a local and clear the bound state immediately
    // so it's not retained in the reactive store / DOM input value any longer
    // than necessary. JS engines may still keep the original string in heap
    // until GC, but at least we drop our reference.
    const pass = exportPassphrase
    exportPassphrase = ''
    try {
      const blob = await api.backup.export()
      // A backup carrying Silent Payments keys (scan secret) must not leave the
      // machine in the clear — require a passphrase before allowing download.
      if (!pass && await backupHasSilentPayments(blob)) {
        exportError = 'This backup contains Silent Payments keys — set a passphrase to encrypt it before exporting.'
        return
      }
      if (pass) {
        const plaintext = await blob.text()
        const envelope = await encryptBackup(plaintext, pass)
        const encBlob = new Blob([JSON.stringify(envelope, null, 2)], { type: 'application/json' })
        downloadBlob(encBlob, `corvin-backup-${new Date().toISOString().split('T')[0]}-encrypted.json`)
      } else {
        downloadBlob(blob, `corvin-backup-${new Date().toISOString().split('T')[0]}.json`)
      }
    } catch (e) {
      exportError = e instanceof Error ? e.message : 'Export failed'
    }
  }

  /// Parses the file and surfaces what's in it before any destructive action.
  async function handleImportFile(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0]
    if (!file) return
    importError = ''; stagedFile = null
    const text = await file.text()
    let data: unknown
    try { data = JSON.parse(text) } catch {
      importError = 'Invalid JSON file.'
      if (fileInputEl) fileInputEl.value = ''
      return
    }
    if (isEncryptedEnvelope(data)) {
      stagedFile = { name: file.name, parsed: data, encrypted: true, summary: null }
    } else {
      stagedFile = { name: file.name, parsed: data, encrypted: false, summary: summariseBackup(data) }
    }
  }

  function summariseBackup(data: unknown): { wallets: number; labels: number; cost_basis: number; address_labels: number; silent_payments: number } | null {
    if (!data || typeof data !== 'object') return null
    const d = data as Record<string, unknown>
    const count = (v: unknown) => (typeof v === 'object' && v !== null ? Object.keys(v).length : 0)
    return {
      wallets: Array.isArray(d.wallets) ? d.wallets.length : 0,
      labels: count(d.labels),
      cost_basis: count(d.cost_basis),
      address_labels: count(d.address_labels),
      silent_payments: count(d.silent_payments),
    }
  }

  // Peek at a freshly-exported (plaintext) backup blob to see whether it carries
  // Silent Payments key material. Best-effort: a parse failure means "don't know"
  // → false, so we never block a legitimate plain export on a transient error.
  async function backupHasSilentPayments(blob: Blob): Promise<boolean> {
    try {
      const d = JSON.parse(await blob.text()) as Record<string, unknown>
      return typeof d.silent_payments === 'object'
        && d.silent_payments !== null
        && Object.keys(d.silent_payments).length > 0
    } catch {
      return false
    }
  }

  async function tryDecrypt() {
    if (!stagedFile || !stagedFile.encrypted || decrypting) return
    importError = ''
    // Hand the passphrase to decrypt then clear the bound state regardless
    // of outcome — even a failed attempt shouldn't leave it in the input.
    const pass = importPassphrase
    importPassphrase = ''
    decrypting = true
    try {
      const plaintext = await decryptBackup(stagedFile.parsed as Parameters<typeof decryptBackup>[0], pass)
      const parsed = JSON.parse(plaintext)
      stagedFile = {
        ...stagedFile,
        parsed,
        encrypted: false,
        summary: summariseBackup(parsed),
      }
    } catch (e) {
      importError = e instanceof Error ? e.message : 'Decryption failed'
    } finally {
      decrypting = false
    }
  }

  async function confirmRestore() {
    if (!stagedFile || stagedFile.encrypted) return
    importing = true; importError = ''; importSuccess = false
    try {
      await api.backup.restore(stagedFile.parsed)
      importSuccess = true
      stagedFile = null
      if (successTimer) clearTimeout(successTimer)
      successTimer = setTimeout(() => (importSuccess = false), 4000)
    } catch (e) {
      importError = e instanceof Error ? e.message : 'Import failed'
    } finally {
      importing = false
      if (fileInputEl) fileInputEl.value = ''
    }
  }

  function cancelStagedImport() {
    stagedFile = null
    importPassphrase = ''
    importError = ''
    if (fileInputEl) fileInputEl.value = ''
  }

  async function exportBip329() {
    try {
      const blob = await api.bip329.export()
      downloadBlob(blob, `corvin-labels-${new Date().toISOString().split('T')[0]}.jsonl`)
    } catch (e) {
      bip329ImportError = e instanceof Error ? e.message : 'Export failed'
    }
  }

  async function handleBip329FileImport(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0]
    if (!file) return
    bip329ImportError = ''; bip329ImportSummary = null; bip329Importing = true
    try {
      const jsonl = await file.text()
      bip329ImportSummary = await api.bip329.import({ jsonl, replace: bip329ImportReplace })
    } catch (err) {
      bip329ImportError = err instanceof Error ? err.message : 'Import failed'
    } finally {
      bip329Importing = false
      if (bip329FileInputEl) bip329FileInputEl.value = ''
    }
  }
</script>

<div class="page">
  <div class="page-inner">
    <h1>Import / Export</h1>

    <div class="action-card">
      <div class="action-icon">↓</div>
      <div class="action-body">
        <div class="action-title">Backup metadata</div>
        <p class="action-desc">
          Exports wallet descriptors (xpubs, paths, fingerprints), labels, cost-basis annotations, UTXO freezes, Silent Payments keys, and settings as a JSON file.
          <strong>Your seed phrases are NOT included</strong> — those are your responsibility to back up separately.
          To recover spending ability, you need both this file and your seeds.
          <strong>Silent Payments scan keys ARE included</strong>, so a passphrase is required when any SP wallet is present.
        </p>
        <details class="export-encrypt">
          <summary>Encrypt with a passphrase (recommended)</summary>
          <div class="export-encrypt-body">
            <p class="export-encrypt-hint">
              Wallet descriptors (xpubs) reveal all addresses and transaction history.
              Encrypt the backup if you'll store it in cloud / email it / share over a chat app.
            </p>
            <input
              type="password"
              class="export-pass-input"
              bind:value={exportPassphrase}
              placeholder="Passphrase (leave blank for plain JSON)"
              autocomplete="new-password"
            />
            {#if exportPassphrase && exportPassphrase.length < 8}
              <p class="msg-warn">Use 8+ characters for a meaningful passphrase.</p>
            {/if}
          </div>
        </details>
        {#if atRestOn && !exportPassphrase}
          <p class="msg-warn">
            Your at-rest encryption protects this device only. This export will be saved as plain
            JSON — set a passphrase above to encrypt the file too.
          </p>
        {/if}
        <BusyButton
          idle={exportPassphrase ? '🔒 Export encrypted backup' : 'Export backup'}
          busyLabel="Encrypting…"
          onclick={exportBackup}
        />
        {#if exportError}
          <p class="msg-err">{exportError}</p>
        {/if}
      </div>
    </div>

    <div class="action-card restore-card">
      <div class="action-icon">↑</div>
      <div class="action-body">
        <div class="action-title">Restore</div>
        <p class="action-desc">Import a previously exported backup file. <strong>This replaces all current wallets, labels, cost basis, and settings.</strong></p>

        {#if !stagedFile}
          <p class="action-warn">⚠ Destructive — all existing data will be overwritten.</p>
          <button type="button" class="btn-restore" onclick={() => fileInputEl?.click()} disabled={importing}>
            {importing ? 'Importing…' : 'Choose backup file'}
          </button>
        {:else if stagedFile.encrypted}
          <div class="staged-preview">
            <div class="staged-title">🔒 Encrypted backup: <span class="mono">{stagedFile.name}</span></div>
            <p class="staged-hint">Enter the passphrase used when this backup was created.</p>
            <input
              type="password"
              class="export-pass-input"
              bind:value={importPassphrase}
              placeholder="Backup passphrase"
              onkeydown={(e) => e.key === 'Enter' && tryDecrypt()}
              autocomplete="current-password"
            />
            <div class="staged-actions">
              <button type="button" class="btn-restore" onclick={cancelStagedImport}>Cancel</button>
              <button type="button" class="btn-primary" onclick={tryDecrypt} disabled={!importPassphrase || decrypting}>
                {decrypting ? 'Decrypting…' : 'Decrypt'}
              </button>
            </div>
          </div>
        {:else if stagedFile.summary}
          <div class="staged-preview">
            <div class="staged-title">Preview: <span class="mono">{stagedFile.name}</span></div>
            <ul class="staged-list">
              <li>{stagedFile.summary.wallets} wallet{stagedFile.summary.wallets !== 1 ? 's' : ''}</li>
              {#if stagedFile.summary.silent_payments > 0}
                <li>{stagedFile.summary.silent_payments} Silent Payments key set{stagedFile.summary.silent_payments !== 1 ? 's' : ''} <span class="muted-inline">(scanning resumes after restart)</span></li>
              {/if}
              <li>{stagedFile.summary.labels} transaction label{stagedFile.summary.labels !== 1 ? 's' : ''}</li>
              <li>{stagedFile.summary.address_labels} address label{stagedFile.summary.address_labels !== 1 ? 's' : ''}</li>
              <li>{stagedFile.summary.cost_basis} cost-basis override{stagedFile.summary.cost_basis !== 1 ? 's' : ''}</li>
            </ul>
            <p class="action-warn">⚠ Restoring this file will overwrite your current Corvin data.</p>
            <div class="staged-actions">
              <button type="button" class="btn-restore" onclick={cancelStagedImport}>Cancel</button>
              <button type="button" class="btn-primary" onclick={confirmRestore} disabled={importing}>
                {importing ? 'Restoring…' : 'Yes, restore'}
              </button>
            </div>
          </div>
        {/if}

        <input
          type="file"
          accept=".json,application/json"
          bind:this={fileInputEl}
          onchange={handleImportFile}
          style="display:none"
        />
        {#if importSuccess}
          <p class="msg-ok">✓ Restored successfully — reload to apply changes.</p>
          <button type="button" class="btn-reload" onclick={() => window.location.reload()}>Reload now</button>
        {/if}
        {#if importError}
          <p class="msg-err">{importError}</p>
        {/if}
      </div>
    </div>

    <!-- BIP-329 label interop -->
    <div class="action-card">
      <div class="action-icon">↔</div>
      <div class="action-body">
        <div class="action-title">Labels — share with other wallets</div>
        <p class="action-desc">
          Round-trip transaction, address, and UTXO labels (plus freeze flags) with any
          wallet that speaks the <a class="inline-link" href="https://github.com/bitcoin/bips/blob/master/bip-0329.mediawiki" target="_blank" rel="noopener noreferrer">BIP-329 standard</a> — Sparrow, Nunchuk, etc.
          Useful when migrating between wallets without losing context.
          <strong>Does not include cost-basis overrides</strong> (those live outside BIP-329 and stay in Corvin's own backup).
        </p>

        <div class="bip329-row">
          <BusyButton idle="Export labels (.jsonl)" busyLabel="Exporting…" onclick={exportBip329} />
          <label class="btn-restore" class:loading={bip329Importing}>
            {bip329Importing ? 'Importing…' : 'Import labels…'}
            <input type="file" accept=".jsonl,.json,.txt" bind:this={bip329FileInputEl} onchange={handleBip329FileImport} />
          </label>
        </div>

        <label class="bip329-replace">
          <input type="checkbox" bind:checked={bip329ImportReplace} />
          Replace mode <span class="muted-inline">— wipe existing labels first (default: merge)</span>
        </label>

        {#if bip329ImportSummary}
          <div class="bip329-summary">
            <strong>✓ Imported:</strong>
            {bip329ImportSummary.tx_labels} tx,
            {bip329ImportSummary.address_labels} address,
            {bip329ImportSummary.utxo_labels} UTXO label{bip329ImportSummary.utxo_labels !== 1 ? 's' : ''}.
            {#if bip329ImportSummary.frozen_changes > 0}{bip329ImportSummary.frozen_changes} freeze change{bip329ImportSummary.frozen_changes !== 1 ? 's' : ''}.{/if}
            {#if bip329ImportSummary.skipped > 0}<span class="muted-inline">{bip329ImportSummary.skipped} entries skipped</span>{/if}
          </div>
        {/if}
        {#if bip329ImportError}
          <p class="msg-err">{bip329ImportError}</p>
        {/if}
      </div>
    </div>
  </div>
</div>

<style>
  .page {
    flex: 1; overflow-y: auto; background: var(--surface-2);
    padding: 32px 24px 48px;
  }
  .page-inner { max-width: 640px; margin: 0 auto; }
  h1 { font-size: 1.4rem; font-weight: 700; color: var(--text); margin: 0 0 24px; letter-spacing: -0.01em; }

  .action-card {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 8px;
    padding: 24px; display: flex; gap: 20px; align-items: flex-start;
    margin-bottom: 12px;
  }

  .action-icon {
    font-size: 1.4rem; font-weight: 700; color: var(--accent);
    width: 36px; height: 36px; border-radius: 8px;
    background: color-mix(in srgb, var(--accent) 12%, var(--surface-1));
    display: flex; align-items: center; justify-content: center;
    flex-shrink: 0; line-height: 1;
  }

  .action-body { flex: 1; display: flex; flex-direction: column; gap: 10px; }

  .action-title { font-size: 1rem; font-weight: 600; color: var(--text); }

  .action-desc {
    font-size: 0.85rem; color: var(--text-muted); line-height: 1.55; margin: 0;
  }
  .action-desc strong { color: var(--text); font-weight: 600; }

  .action-warn {
    font-size: 0.8rem; color: #e09c52; margin: 0; line-height: 1.4;
    padding: 8px 12px; background: color-mix(in srgb, #e09c52 8%, var(--surface-1));
    border: 1px solid color-mix(in srgb, #e09c52 30%, transparent);
    border-radius: 5px;
  }

  /* Visual style is the global .btn-primary; only the layout override is local. */
  .btn-primary { align-self: flex-start; }

  .btn-restore {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    padding: 9px 20px; cursor: pointer; color: var(--text); font-size: 0.88rem;
    font-weight: 500; align-self: flex-start;
  }
  .btn-restore:hover { border-color: var(--text-muted); }
  .btn-restore:disabled { opacity: 0.5; cursor: not-allowed; }

  .msg-ok  { font-size: 0.82rem; color: #52a875; margin: 0; }
  .msg-err { font-size: 0.82rem; color: var(--error); margin: 0; }
  .btn-reload {
    background: none; border: 1px solid #52a875; border-radius: 5px;
    color: #52a875; cursor: pointer; font-size: 0.82rem;
    padding: 6px 14px; align-self: flex-start;
  }
  .btn-reload:hover { background: color-mix(in srgb, #52a875 10%, transparent); }

  .msg-warn { font-size: 0.78rem; color: #e09c52; margin: 0; }

  .export-encrypt {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    padding: 8px 12px;
  }
  .export-encrypt summary {
    cursor: pointer; font-size: 0.82rem; color: var(--text-muted); padding: 2px 0;
  }
  .export-encrypt summary:hover { color: var(--text); }
  .export-encrypt[open] summary { color: var(--text); margin-bottom: 8px; }
  .export-encrypt-body { display: flex; flex-direction: column; gap: 6px; }
  .export-encrypt-hint { font-size: 0.74rem; color: var(--text-muted); line-height: 1.5; margin: 0; }
  .export-pass-input {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 4px;
    color: var(--text); padding: 6px 10px; font-size: 0.85rem; width: 100%; box-sizing: border-box;
    font-family: monospace;
  }
  .export-pass-input:focus { outline: 1px solid var(--accent); }

  .staged-preview {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    padding: 12px 14px; display: flex; flex-direction: column; gap: 8px;
  }
  .staged-title { font-size: 0.85rem; font-weight: 600; color: var(--text); }
  .staged-title .mono { font-family: monospace; font-weight: 400; color: var(--text-muted); }
  .staged-hint { font-size: 0.78rem; color: var(--text-muted); line-height: 1.5; margin: 0; }
  .staged-list {
    margin: 0; padding: 0 0 0 1.2em;
    font-size: 0.82rem; color: var(--text); line-height: 1.6;
  }
  .staged-actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 4px; }

  .bip329-row { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
  .btn-restore.loading { opacity: 0.5; pointer-events: none; }
  .btn-restore input[type="file"] { display: none; }
  .bip329-replace {
    display: flex; align-items: center; gap: 6px;
    font-size: 0.78rem; color: var(--text);
  }
  .bip329-replace input { accent-color: var(--accent); }
  .muted-inline { color: var(--text-muted); font-size: 0.74rem; }
  .bip329-summary {
    background: color-mix(in srgb, #52a875 8%, var(--surface-2));
    border: 1px solid color-mix(in srgb, #52a875 30%, var(--border));
    border-radius: 5px; padding: 8px 12px;
    font-size: 0.78rem; color: var(--text); line-height: 1.5;
  }
  .inline-link { color: var(--accent); text-decoration: underline; text-underline-offset: 2px; }
  .inline-link:hover { filter: brightness(1.2); }

  @media (max-width: 768px) {
    .page { padding: 16px 16px 32px; }
    h1 { display: none; }
    .action-card { padding: 18px; }
  }
</style>
