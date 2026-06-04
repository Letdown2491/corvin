<script lang="ts">
  import { onDestroy } from 'svelte'
  import { goto } from '$app/navigation'
  import { api } from '../lib/api'
  import type { WalletEntry } from '../lib/types'
  import { wallets, activeWalletId } from '../stores/wallets'
  import { checkMnemonic, type MnemonicCheck } from '../lib/bip39'
  import BusyButton from './ui/BusyButton.svelte'
  import Modal from './ui/Modal.svelte'

  let {
    wallet,
    onClose,
  }: {
    /// The existing wallet whose seed we're adding another account from. Must
    /// be a seed-imported HD wallet (input string is the derivation path).
    wallet: WalletEntry
    onClose: () => void
  } = $props()


  // ── Parse the source wallet's path and script type ──────────────────────
  // For seed-imported wallets, `input` is the BIP32 derivation path (e.g.
  // "m/84'/0'/0'"). We swap the account-level component to populate sensible
  // defaults for the new account.
  function parsePath(p: string): { purpose: string; coin: string; account: number } | null {
    const m = p.match(/^m\/(\d+'?)\/(\d+'?)\/(\d+)'?$/)
    if (!m) return null
    const account = parseInt(m[3], 10)
    if (!Number.isFinite(account)) return null
    return { purpose: m[1], coin: m[2], account }
  }
  let parsedSource = $derived(parsePath(wallet.input))
  let sourceAccount = $derived(parsedSource?.account ?? 0)

  // Map InputKind back to the script_type the seed-import endpoint expects.
  function kindToScriptType(kind: string): string {
    switch (kind) {
      case 'taproot': return 'taproot'
      case 'ypub':    return 'wrapped_segwit'
      case 'xpub':    return 'legacy'
      default:        return 'native_segwit'  // zpub or anything else HD
    }
  }
  let scriptType = $derived(kindToScriptType(wallet.kind))
  let scriptTypeLabel = $derived(({
    taproot: 'Taproot (P2TR)',
    wrapped_segwit: 'Wrapped SegWit (P2SH-P2WPKH)',
    legacy: 'Legacy (P2PKH)',
    native_segwit: 'Native SegWit (P2WPKH)',
  } as Record<string, string>)[scriptType])

  // ── Form state ──────────────────────────────────────────────────────────
  // Default new account = source account + 1 (the natural "next" choice).
  // svelte-ignore state_referenced_locally
  let newAccount = $state(sourceAccount + 1)
  let label = $state('')
  let mnemonic = $state('')
  let passphrase = $state('')
  let mnemonicCheck = $state<MnemonicCheck | null>(null)
  let error = $state('')

  // Auto-fill label from the source wallet's name + new account index. Keeps
  // re-syncing as long as the user hasn't typed a custom label.
  $effect(() => {
    const auto = `${wallet.label} · account `
    if (label === '' || label.startsWith(auto)) {
      label = `${auto}${newAccount}`
    }
  })

  // BIP39 validation runs each time the mnemonic changes. checkMnemonic is
  // async (uses WebCrypto for the checksum), so we sequence with a counter to
  // discard out-of-order results.
  let checkSeq = 0
  $effect(() => {
    const m = mnemonic.trim()
    const seq = ++checkSeq
    if (!m) { mnemonicCheck = null; return }
    checkMnemonic(m).then((r) => {
      if (seq === checkSeq) mnemonicCheck = r
    })
  })

  let canSubmit = $derived(
    label.trim().length > 0 &&
    mnemonicCheck?.kind === 'ok' &&
    newAccount >= 0
  )

  // Refuse if source wallet's path can't be parsed — that means it isn't a
  // standard seed-imported wallet (could be a paste-xpub one with a custom
  // input string). We render a hint instead of the form in that case.
  let canUse = $derived(parsedSource !== null)

  async function submit() {
    if (!canSubmit) return
    error = ''
    try {
      const entry: WalletEntry = await api.wallets.seedImport({
        label: label.trim(),
        mnemonic: mnemonic.trim(),
        passphrase,
        script_type: scriptType,
        account_index: newAccount,
        custom_path: null,
      })
      wallets.update(ws => [...ws, entry])
      activeWalletId.set(entry.id)
      // Wipe secrets eagerly before we navigate.
      mnemonic = ' '.repeat(mnemonic.length); mnemonic = ''
      passphrase = ' '.repeat(passphrase.length); passphrase = ''
      onClose()
      goto(`/wallet/${entry.id}`)
    } catch (e) {
      error = e instanceof Error ? e.message : 'Could not add account'
    }
  }

  onDestroy(() => {
    mnemonic = ''
    passphrase = ''
  })
</script>

<Modal open onclose={onClose} title="Add account" width="520px"
  desc={`Derive a new account from the same seed as ${wallet.label}.`}>
    {#if !canUse}
      <p class="hint">
        This wallet wasn't created from a seed (or uses a custom derivation path),
        so we can't infer the next account index. Use the regular "Add wallet" flow
        and pick the script type + account index manually.
      </p>
    {:else}
      <div class="form-section">
        <label class="field-label" for="aa-label">Label</label>
        <input id="aa-label" class="text-input" type="text" bind:value={label} placeholder="My wallet · account 1" />
      </div>

      <div class="info-grid">
        <div>
          <span class="info-label">Script type</span>
          <span class="info-value">{scriptTypeLabel}</span>
        </div>
        <div>
          <span class="info-label">Path</span>
          <span class="info-value mono">m/{parsedSource?.purpose}/{parsedSource?.coin}/{newAccount}'</span>
        </div>
      </div>

      <div class="form-section account-row">
        <label class="field-label" for="aa-account">Account index</label>
        <input
          id="aa-account"
          class="text-input account-input"
          type="number"
          min="0"
          bind:value={newAccount}
        />
        <span class="hint-inline">Source wallet uses account {sourceAccount}; defaulting to the next.</span>
      </div>

      <div class="form-section">
        <label class="field-label" for="aa-mnemonic">
          Mnemonic
          <span class="warn-inline">· used once, never stored</span>
        </label>
        <textarea
          id="aa-mnemonic"
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
            {:else if mnemonicCheck.kind === 'bad_checksum'}Invalid checksum — re-check the last word
            {/if}
          </p>
        {/if}
      </div>

      <div class="form-section">
        <label class="field-label" for="aa-pass">Passphrase <span class="muted">(optional)</span></label>
        <input id="aa-pass" class="text-input" type="password" placeholder="Empty if your seed has no BIP39 passphrase" bind:value={passphrase} autocomplete="off" />
      </div>

      <BusyButton idle="Create account" busyLabel="Creating…" disabled={!canSubmit} onclick={submit} />

      {#if error}<div class="error-box">{error}</div>{/if}
    {/if}
</Modal>

<style>
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

  .account-row { display: grid; grid-template-columns: auto 80px 1fr; gap: 8px; align-items: center; }
  .account-row .field-label { margin-bottom: 0; }
  .account-input { width: 80px; }
  .hint-inline { font-size: 0.72rem; color: var(--text-muted); }

  .info-grid {
    display: grid; grid-template-columns: 1fr 1fr; gap: 12px;
    margin-bottom: 12px; padding: 9px 12px;
    background: var(--surface-2); border-radius: 5px;
  }
  .info-grid > div { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .info-label { font-size: 0.7rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.06em; }
  .info-value { font-size: 0.82rem; color: var(--text); word-break: break-all; }

  .word-count { margin: 5px 0 0; font-size: 0.72rem; color: #e09c52; }
  .word-count.ok { color: #52a875; }

  .error-box {
    margin-top: 10px; padding: 8px 10px;
    background: color-mix(in srgb, #e05252 8%, var(--surface-2));
    border: 1px solid color-mix(in srgb, #e05252 40%, var(--border));
    border-radius: 4px; font-size: 0.78rem; color: var(--text);
  }

  .hint { font-size: 0.78rem; color: var(--text-muted); line-height: 1.5; }

  @media (max-width: 768px) {
    .account-row { grid-template-columns: 1fr; }
    .info-grid { grid-template-columns: 1fr; gap: 8px; }
  }
</style>
