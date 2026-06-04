<script lang="ts">
  import { onDestroy } from 'svelte'
  import { api } from '../lib/api'
  import type { WalletEntry } from '../lib/types'
  import { checkMnemonic, type MnemonicCheck } from '../lib/bip39'
  import BusyButton from './ui/BusyButton.svelte'
  import Modal from './ui/Modal.svelte'

  let {
    wallet,
    onClose,
  }: {
    wallet: WalletEntry
    onClose: () => void
  } = $props()


  let mnemonic = $state('')
  let passphrase = $state('')
  let mnemonicCheck = $state<MnemonicCheck | null>(null)
  let result = $state<{ matches: boolean; message: string } | null>(null)
  let error = $state('')

  // BIP39 validation is async (WebCrypto for SHA-256). Sequence-counter
  // guards against a late result clobbering a fresher one.
  let checkSeq = 0
  $effect(() => {
    const m = mnemonic.trim()
    const seq = ++checkSeq
    if (!m) { mnemonicCheck = null; return }
    checkMnemonic(m).then((r) => {
      if (seq === checkSeq) mnemonicCheck = r
    })
  })

  let canSubmit = $derived(mnemonicCheck?.kind === 'ok')

  async function submit() {
    if (!canSubmit) return
    error = ''
    result = null
    try {
      const r = await api.testBackup(wallet.id, {
        mnemonic: mnemonic.trim(),
        passphrase: passphrase || undefined,
      })
      result = r
      // Wipe secrets immediately — they're not needed after the test result
      // is known. Overwrite character storage before truncating so the JS
      // engine is less likely to leave residue in string interning.
      mnemonic = ' '.repeat(mnemonic.length); mnemonic = ''
      passphrase = ' '.repeat(passphrase.length); passphrase = ''
    } catch (e) {
      error = e instanceof Error ? e.message : 'Test failed'
    }
  }

  function startOver() {
    result = null
    error = ''
    mnemonic = ''
    passphrase = ''
    mnemonicCheck = null
  }

  onDestroy(() => {
    mnemonic = ''
    passphrase = ''
  })
</script>

<Modal open onclose={onClose} title="Test backup" width="540px"
  desc={`Verify your written-down mnemonic actually reproduces ${wallet.label}.`}>
    {#if result === null}
      <div class="instructions">
        <p>
          <strong>Type your mnemonic from your written-down copy</strong>, not from screen
          or memory. The whole point is to catch typos and missing words that would only
          surface when you actually need to recover.
        </p>
        <p class="instructions-note">
          Nothing is saved or modified — this is a pure check. The mnemonic is held in
          memory only for the duration of the test, then wiped.
        </p>
      </div>

      <div class="form-section">
        <label class="field-label" for="tb-mnemonic">
          Mnemonic
          <span class="warn-inline">· used once, never stored</span>
        </label>
        <textarea
          id="tb-mnemonic"
          class="textarea mono"
          rows="3"
          placeholder="12, 15, 18, 21, or 24 words separated by spaces"
          bind:value={mnemonic}
          spellcheck="false"
          autocomplete="off"
          autocapitalize="off"
        ></textarea>
        {#if mnemonicCheck && mnemonic.trim().length > 0}
          <p class="word-count" class:ok={mnemonicCheck.kind === 'ok'}>
            {#if mnemonicCheck.kind === 'ok'}✓ Valid BIP39 mnemonic
            {:else if mnemonicCheck.kind === 'invalid_words'}Unknown word(s): {mnemonicCheck.words.join(', ')}
            {:else if mnemonicCheck.kind === 'wrong_count'}Wrong word count ({mnemonicCheck.count}) — expected 12, 15, 18, 21, or 24
            {:else if mnemonicCheck.kind === 'bad_checksum'}Invalid BIP39 checksum — re-check the last word
            {/if}
          </p>
        {/if}
      </div>

      <div class="form-section">
        <label class="field-label" for="tb-pass">Passphrase <span class="muted">(if you set one)</span></label>
        <input
          id="tb-pass"
          class="text-input"
          type="password"
          placeholder="Empty if your seed has no BIP39 passphrase"
          bind:value={passphrase}
          autocomplete="off"
        />
      </div>

      <BusyButton idle="Verify backup" busyLabel="Checking…" disabled={!canSubmit} onclick={submit} />

      {#if error}
        <div class="error-box">{error}</div>
      {/if}
    {:else if result.matches}
      <div class="result-box result-match">
        <span class="result-icon" aria-hidden="true">✓</span>
        <div class="result-body">
          <div class="result-title">Your backup is valid</div>
          <div class="result-msg">{result.message}</div>
        </div>
      </div>
      <p class="post-result-hint">
        Re-test any time. We recommend doing this annually, and immediately if
        your written copy has ever been out of your direct possession.
      </p>
      <button class="btn-secondary" onclick={onClose}>Done</button>
    {:else}
      <div class="result-box result-mismatch">
        <span class="result-icon" aria-hidden="true">⚠</span>
        <div class="result-body">
          <div class="result-title">Backup does NOT match this wallet</div>
          <div class="result-msg">{result.message}</div>
        </div>
      </div>
      <p class="post-result-hint">
        <strong>Don't panic.</strong> The wallet in Corvin is fine and your funds
        are not at risk. What this means is that the copy you just typed in won't
        recover this wallet. Re-check your written words against the original — most
        often it's a single letter wrong, or a missing passphrase.
      </p>
      <div class="result-actions">
        <button class="btn-secondary" onclick={startOver}>Try again</button>
        <button class="btn-secondary" onclick={onClose}>Close</button>
      </div>
    {/if}
</Modal>

<style>
  .instructions {
    margin-bottom: 14px; padding: 10px 12px;
    background: var(--surface-2); border-radius: 6px;
    border: 1px solid var(--border);
    display: flex; flex-direction: column; gap: 8px;
    font-size: 0.78rem; line-height: 1.55; color: var(--text);
  }
  .instructions p { margin: 0; }
  .instructions-note { font-size: 0.74rem; color: var(--text-muted); }

  .form-section { margin-bottom: 12px; }
  .field-label {
    display: block; font-size: 0.74rem; font-weight: 600; color: var(--text-muted);
    margin-bottom: 5px; text-transform: uppercase; letter-spacing: 0.06em;
  }
  .warn-inline { font-weight: 400; color: #e09c52; text-transform: none; letter-spacing: 0; }
  .muted { color: var(--text-muted); font-weight: 400; text-transform: none; letter-spacing: 0; }

  .text-input, .textarea {
    width: 100%; box-sizing: border-box;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 4px; color: var(--text); padding: 7px 10px;
    font-size: 0.82rem; outline: none;
  }
  .text-input:focus, .textarea:focus { border-color: var(--accent); }
  .textarea { resize: vertical; font-family: inherit; }
  .mono { font-family: monospace; font-size: 0.78rem; }

  .word-count { margin: 5px 0 0; font-size: 0.72rem; color: #e09c52; }
  .word-count.ok { color: #52a875; }

  .error-box {
    margin-top: 10px; padding: 8px 10px;
    background: color-mix(in srgb, #e05252 8%, var(--surface-2));
    border: 1px solid color-mix(in srgb, #e05252 40%, var(--border));
    border-radius: 4px; font-size: 0.78rem; color: var(--text);
  }

  .result-box {
    display: flex; align-items: flex-start; gap: 12px;
    padding: 12px 14px;
    border: 1px solid; border-radius: 6px;
    margin-bottom: 12px;
  }
  .result-box.result-match {
    background: color-mix(in srgb, #52a875 10%, var(--surface-2));
    border-color: color-mix(in srgb, #52a875 45%, var(--border));
  }
  .result-box.result-mismatch {
    background: color-mix(in srgb, #e05252 10%, var(--surface-2));
    border-color: color-mix(in srgb, #e05252 50%, var(--border));
  }
  .result-icon { font-size: 1.4rem; line-height: 1.1; flex-shrink: 0; }
  .result-box.result-match .result-icon { color: #52a875; }
  .result-box.result-mismatch .result-icon { color: #e05252; }
  .result-body { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .result-title { font-weight: 700; font-size: 0.88rem; color: var(--text); }
  .result-msg { font-size: 0.8rem; color: var(--text); line-height: 1.5; }

  .post-result-hint {
    margin: 6px 0 0; font-size: 0.78rem; color: var(--text-muted); line-height: 1.55;
  }
  .result-actions { display: flex; gap: 8px; }
</style>
