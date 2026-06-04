<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { api } from '../lib/api'
  import type { AddressInfo, WalletEntry } from '../lib/types'
  import BusyButton from './ui/BusyButton.svelte'
  import CopyButton from './ui/CopyButton.svelte'
  import Modal from './ui/Modal.svelte'

  let {
    wallet,
    addresses,
    onClose,
  }: {
    wallet: WalletEntry
    addresses: AddressInfo[]
    onClose: () => void
  } = $props()

  type Tab = 'sign' | 'verify'
  let tab = $state<Tab>('sign')

  // ── Sign tab ──────────────────────────────────────────────────────────────
  // BIP-322 single-key signing requires a real HD wallet (we need to derive
  // the address's private key from the supplied mnemonic). Multisig and
  // watch-only-address wallets can't participate in BIP-322 Simple.
  let canSign = $derived(wallet.kind !== 'multisig' && wallet.internal_descriptor !== null)

  // Show only "used" external addresses by default — those are the ones the
  // user is likely to be proving ownership of. They can switch to "all" to
  // include never-used ones.
  let externalAddrs = $derived(addresses.filter(a => a.kind === 'external'))
  let showAllAddrs = $state(false)
  let signableAddrs = $derived(
    showAllAddrs ? externalAddrs : externalAddrs.filter(a => a.used)
  )

  let selectedAddress = $state('')
  // Default to the first signable address once the list resolves.
  $effect(() => {
    if (!selectedAddress && signableAddrs.length > 0) {
      selectedAddress = signableAddrs[0].address
    }
    // If filter change drops the current selection, fall back to first.
    if (selectedAddress && !signableAddrs.some(a => a.address === selectedAddress) && signableAddrs.length > 0) {
      selectedAddress = signableAddrs[0].address
    }
  })

  let signMessage = $state('')
  let signMnemonic = $state('')
  // Masked by default (shoulder-surfing); Firefox ignores -webkit-text-security
  // on a textarea, so we mask with a password input and toggle its type.
  let signSeedRevealed = $state(false)
  let signPassphrase = $state('')
  let signing = $state(false)
  let signError = $state('')
  let signature = $state('')

  // Quick sanity check on the mnemonic input before hitting the backend. The
  // server does the authoritative validation; this just catches obvious typos
  // before we round-trip a request with secret material.
  let mnemonicWordCount = $derived(signMnemonic.trim().split(/\s+/).filter(w => w).length)
  let mnemonicLooksOk = $derived([12, 15, 18, 21, 24].includes(mnemonicWordCount))

  async function doSign() {
    if (signing) return
    signing = true
    signError = ''
    signature = ''
    try {
      const result = await api.messages.sign(wallet.id, {
        address: selectedAddress,
        message: signMessage,
        mnemonic: signMnemonic,
        passphrase: signPassphrase || undefined,
      })
      signature = result.signature
      // Wipe the mnemonic from local state as soon as we're done with it.
      // Replaces the characters before clearing length so the JS engine is
      // less likely to leave residue in string interning.
      signMnemonic = ' '.repeat(signMnemonic.length)
      signMnemonic = ''
      signPassphrase = ' '.repeat(signPassphrase.length)
      signPassphrase = ''
    } catch (e) {
      signError = e instanceof Error ? e.message : 'Signing failed'
    } finally {
      signing = false
    }
  }

  // ── Verify tab ────────────────────────────────────────────────────────────
  let verifyAddress = $state('')
  let verifyMessage = $state('')
  let verifySignature = $state('')
  let verifying = $state(false)
  let verifyResult = $state<{ valid: boolean; error?: string } | null>(null)

  async function doVerify() {
    verifyResult = null
    try {
      verifyResult = await api.messages.verify({
        address: verifyAddress.trim(),
        message: verifyMessage,
        signature: verifySignature.trim(),
      })
    } catch (e) {
      verifyResult = { valid: false, error: e instanceof Error ? e.message : 'Verification request failed' }
    }
  }

  let canVerify = $derived(
    verifyAddress.trim().length > 0 &&
    verifySignature.trim().length > 0
  )

  onMount(() => {
    if (!canSign) tab = 'verify'
  })

  onDestroy(() => {
    // Belt-and-braces: clear secret-bearing state on close too.
    signMnemonic = ''
    signPassphrase = ''
  })
</script>

<Modal open onclose={onClose} title="Sign & verify message" width="560px"
  desc="BIP-322 message signing — prove ownership of an address.">


    <div class="tabs" role="tablist">
      <button
        class="tab-btn"
        class:active={tab === 'sign'}
        role="tab"
        aria-selected={tab === 'sign'}
        disabled={!canSign}
        title={canSign ? '' : 'Signing requires an HD wallet (multisig and watch-only address wallets can\'t sign).'}
        onclick={() => tab = 'sign'}
      >Sign</button>
      <button
        class="tab-btn"
        class:active={tab === 'verify'}
        role="tab"
        aria-selected={tab === 'verify'}
        onclick={() => tab = 'verify'}
      >Verify</button>
    </div>

    {#if tab === 'sign'}
      {#if !canSign}
        <p class="hint">
          This wallet can't sign messages — multisig and watch-only address wallets
          aren't supported by single-key BIP-322. Use the Verify tab instead.
        </p>
      {:else}
        <div class="form-section">
          <label class="field-label" for="sign-addr">Address</label>
          <div class="addr-row">
            <select id="sign-addr" class="addr-select" bind:value={selectedAddress}>
              {#each signableAddrs as a (a.address)}
                <option value={a.address}>
                  {a.address} {a.used ? '· used' : '· unused'}
                </option>
              {/each}
              {#if signableAddrs.length === 0}
                <option value="">No addresses available</option>
              {/if}
            </select>
          </div>
          <label class="cb-row">
            <input type="checkbox" bind:checked={showAllAddrs} />
            <span>Show unused addresses too</span>
          </label>
        </div>

        <div class="form-section">
          <label class="field-label" for="sign-msg">Message</label>
          <textarea
            id="sign-msg"
            class="textarea"
            rows="3"
            placeholder="Type the message you want to sign"
            bind:value={signMessage}
            spellcheck="false"
          ></textarea>
        </div>

        <div class="form-section">
          <label class="field-label" for="sign-mnemonic">
            Mnemonic
            <span class="warn-inline" title="The mnemonic is used only to derive the signing key, then immediately wiped. It never touches disk.">
              · used once, never stored
            </span>
            <button type="button" class="seed-reveal" onclick={() => signSeedRevealed = !signSeedRevealed}>
              {signSeedRevealed ? 'Hide' : 'Show'}
            </button>
          </label>
          <input
            id="sign-mnemonic"
            class="text-input mnemonic-input"
            type={signSeedRevealed ? 'text' : 'password'}
            placeholder="12, 15, 18, 21, or 24 words separated by spaces"
            bind:value={signMnemonic}
            spellcheck="false"
            autocomplete="off"
            autocapitalize="off"
          />
          {#if signMnemonic.trim().length > 0}
            <p class="word-count" class:ok={mnemonicLooksOk}>
              {mnemonicWordCount} word{mnemonicWordCount === 1 ? '' : 's'}
              {#if !mnemonicLooksOk} — expected 12/15/18/21/24{/if}
            </p>
          {/if}
        </div>

        <div class="form-section">
          <label class="field-label" for="sign-pass">Passphrase <span class="muted">(optional)</span></label>
          <input
            id="sign-pass"
            class="text-input"
            type="password"
            placeholder="Empty if your seed has no BIP39 passphrase"
            bind:value={signPassphrase}
            autocomplete="off"
          />
        </div>

        <button
          class="btn-primary"
          onclick={doSign}
          disabled={signing || !selectedAddress || !mnemonicLooksOk}
        >
          {signing ? 'Signing…' : 'Sign'}
        </button>

        {#if signError}
          <div class="error-box">{signError}</div>
        {/if}

        {#if signature}
          <div class="result-box">
            <div class="result-header">
              <span class="result-label">Signature</span>
              <CopyButton text={signature} idle="⎘ Copy" copiedLabel="✓ Copied" class="btn-copy" />
            </div>
            <code class="result-code">{signature}</code>
            <p class="result-hint">
              Share this signature alongside the address and message. The recipient
              can paste all three into a BIP-322 verifier (Sparrow, Electrum, or
              this app's Verify tab) to confirm you control the address.
            </p>
          </div>
        {/if}
      {/if}
    {:else}
      <div class="form-section">
        <label class="field-label" for="ver-addr">Address</label>
        <input
          id="ver-addr"
          class="text-input mono"
          type="text"
          placeholder="bc1q… / bc1p… / 3…"
          bind:value={verifyAddress}
          spellcheck="false"
          autocomplete="off"
          autocapitalize="off"
        />
      </div>
      <div class="form-section">
        <label class="field-label" for="ver-msg">Message</label>
        <textarea
          id="ver-msg"
          class="textarea"
          rows="3"
          placeholder="The exact message that was signed"
          bind:value={verifyMessage}
          spellcheck="false"
        ></textarea>
      </div>
      <div class="form-section">
        <label class="field-label" for="ver-sig">Signature</label>
        <textarea
          id="ver-sig"
          class="textarea mono"
          rows="3"
          placeholder="Base64-encoded BIP-322 signature"
          bind:value={verifySignature}
          spellcheck="false"
          autocomplete="off"
        ></textarea>
      </div>

      <BusyButton bind:busy={verifying} idle="Verify" busyLabel="Verifying…" disabled={!canVerify} onclick={doVerify} />

      {#if verifyResult}
        {#if verifyResult.valid}
          <div class="result-box valid">
            <span class="result-icon">✓</span>
            <div>
              <div class="result-title">Signature is valid</div>
              <div class="result-sub">The signer controls <code class="mono">{verifyAddress.trim()}</code> and signed this exact message.</div>
            </div>
          </div>
        {:else}
          <div class="result-box invalid">
            <span class="result-icon">⚠</span>
            <div>
              <div class="result-title">Signature is NOT valid</div>
              <div class="result-sub">{verifyResult.error ?? 'The signature does not match this address and message.'}</div>
            </div>
          </div>
        {/if}
      {/if}

      <p class="hint">
        Note: BIP-322 covers P2TR (<code>bc1p…</code>), native SegWit
        (<code>bc1q…</code>), and wrapped SegWit (<code>3…</code>) addresses.
        Legacy <code>1…</code> addresses use a different older format that isn't
        supported here.
      </p>
    {/if}
</Modal>

<style>
  .tabs {
    display: flex; gap: 4px; border-bottom: 1px solid var(--border);
    margin-bottom: 14px;
  }
  .tab-btn {
    background: none; border: none; padding: 8px 14px;
    color: var(--text-muted); cursor: pointer; font-size: 0.85rem;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }
  .tab-btn:hover:not(:disabled) { color: var(--text); }
  .tab-btn.active { color: var(--accent); border-bottom-color: var(--accent); }
  .tab-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  .form-section { margin-bottom: 12px; }
  .field-label {
    display: block; font-size: 0.74rem; font-weight: 600;
    color: var(--text-muted); margin-bottom: 5px;
    text-transform: uppercase; letter-spacing: 0.06em;
  }
  .warn-inline { font-weight: 400; color: #e09c52; text-transform: none; letter-spacing: 0; }
  .seed-reveal {
    float: right; background: none; border: none; color: var(--accent); cursor: pointer;
    font-size: 0.74rem; padding: 0; text-transform: none; letter-spacing: 0; font-weight: 600;
  }
  .seed-reveal:hover { text-decoration: underline; }
  .muted { color: var(--text-muted); font-weight: 400; text-transform: none; letter-spacing: 0; }

  .text-input, .textarea, .addr-select {
    width: 100%; box-sizing: border-box;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 4px; color: var(--text); padding: 7px 10px;
    font-size: 0.82rem; outline: none;
  }
  .text-input:focus, .textarea:focus, .addr-select:focus { border-color: var(--accent); }
  .textarea { resize: vertical; font-family: inherit; }
  .mono { font-family: monospace; font-size: 0.78rem; }
  .mnemonic-input { font-family: monospace; }

  .word-count {
    margin: 5px 0 0; font-size: 0.72rem; color: #e09c52;
  }
  .word-count.ok { color: #52a875; }

  .cb-row {
    display: flex; align-items: center; gap: 6px; margin-top: 6px;
    font-size: 0.78rem; color: var(--text-muted); cursor: pointer;
  }
  .cb-row input { accent-color: var(--accent); }

  .error-box {
    margin-top: 10px; padding: 8px 10px;
    background: color-mix(in srgb, #e05252 8%, var(--surface-2));
    border: 1px solid color-mix(in srgb, #e05252 40%, var(--border));
    border-radius: 4px;
    font-size: 0.78rem; color: var(--text);
  }

  .result-box {
    margin-top: 12px; padding: 10px 12px;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 5px; display: flex; flex-direction: column; gap: 6px;
  }
  .result-box.valid, .result-box.invalid {
    flex-direction: row; align-items: flex-start; gap: 10px;
  }
  .result-box.valid {
    background: color-mix(in srgb, #52a875 8%, var(--surface-2));
    border-color: color-mix(in srgb, #52a875 35%, var(--border));
  }
  .result-box.invalid {
    background: color-mix(in srgb, #e05252 8%, var(--surface-2));
    border-color: color-mix(in srgb, #e05252 40%, var(--border));
  }
  .result-icon { font-size: 1.15rem; line-height: 1; flex-shrink: 0; }
  .result-box.valid .result-icon { color: #52a875; }
  .result-box.invalid .result-icon { color: #e05252; }
  .result-title { font-weight: 700; font-size: 0.88rem; color: var(--text); margin-bottom: 3px; }
  .result-sub { font-size: 0.78rem; color: var(--text-muted); line-height: 1.5; }
  .result-sub code { font-size: 0.74rem; }
  .result-header {
    display: flex; align-items: center; justify-content: space-between;
  }
  .result-label {
    font-size: 0.72rem; font-weight: 700; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.06em;
  }
  .result-code {
    display: block; font-family: monospace; font-size: 0.76rem; color: var(--text);
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 3px;
    padding: 7px 10px; word-break: break-all; line-height: 1.45;
  }
  .result-hint {
    margin: 4px 0 0; font-size: 0.74rem; color: var(--text-muted); line-height: 1.5;
  }

  .hint {
    margin: 12px 0 0; font-size: 0.74rem; color: var(--text-muted);
    line-height: 1.5;
  }
  .hint code { font-size: 0.72rem; }
</style>
