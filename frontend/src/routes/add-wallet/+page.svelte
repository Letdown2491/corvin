<script lang="ts">
  import { goto } from '$app/navigation'
  import { onMount, onDestroy } from 'svelte'
  import { api } from '../../lib/api'
  import { wallets, activeWalletId } from '../../stores/wallets'
  import { hwEnabled } from '../../stores/settings'
  import { checkMnemonic, type MnemonicCheck } from '../../lib/bip39'
  import { BIP39_WORDS } from '../../lib/bip39_words'
  import SeedGeneratorPanel from '../../components/SeedGeneratorPanel.svelte'
  import KindPicker from '../../components/KindPicker.svelte'
  import VaultCreatePanel from '../../components/VaultCreatePanel.svelte'
  import SpCreatePanel from '../../components/SpCreatePanel.svelte'
  import MultisigCreatePanel from '../../components/MultisigCreatePanel.svelte'
  import { parseImportFile, detectPaste } from '../../lib/import'
  import type { WalletEntry, BackendEntry } from '../../lib/types'
  import HelpLink from '../../components/HelpLink.svelte'

  let label = $state('')
  let input = $state('')
  let error = $state('')
  let loading = $state(false)
  let method = $state<'paste' | 'file' | 'hardware' | 'seed' | 'multisig' | 'descriptor'>('seed')

  // Backend pin: which server this wallet syncs/broadcasts through. null = the
  // default backend. Saved backends are loaded from the registry.
  let savedBackends = $state<BackendEntry[]>([])
  let selectedBackend = $state<string | null>(null)

  // Raw descriptor import (method === 'descriptor'). Optional change descriptor
  // for wallets exported as a separate receive/change pair; multipath `<0;1>`
  // descriptors carry both and don't need it.
  let descriptorText = $state('')
  let changeDescriptorText = $state('')

  // A signable descriptor carries key origins like `[d34db33f/84h/0h/0h]`.
  // Without them a hardware wallet can't match its keys, so the import is
  // watch-only even if it otherwise looks like an HD wallet.
  let descriptorHasOrigin = $derived(/\[[0-9a-fA-F]{8}(\/|\])/.test(descriptorText))

  // Top-level wallet kind. `null` means the user hasn't picked yet — show the
  // kind picker. Once chosen, we route to the relevant sub-form and hide
  // tabs that don't belong to this kind.
  type WalletKind = 'single' | 'multisig' | 'sp' | 'address' | 'vault'
  let kind = $state<WalletKind | null>(null)
  // SP wallets pick a Frigate (BIP-352) backend; other kinds pick a regular one.
  let backendOptions = $derived(
    kind === 'sp' ? savedBackends.filter(b => b.frigate) : savedBackends.filter(b => !b.frigate)
  )

  // Shared by the self-contained kind panels (vault, SP) that own their own
  // create call and hand back the new wallet.
  function handleCreated(entry: WalletEntry) {
    wallets.update(ws => [...ws, entry])
    activeWalletId.set(entry.id)
    goto(`/wallet/${entry.id}`, { replaceState: true })
  }

  function pickKind(k: WalletKind) {
    kind = k
    error = ''
    selectedBackend = null // a pin from a previous kind may not fit this one
    if (k === 'single') method = 'seed'
    else if (k === 'multisig') method = 'multisig'
    else if (k === 'address') method = 'paste'
    // 'sp' has no methods yet; sub-flow renders a placeholder.
  }

  function backToKindPicker() {
    cancelBitBox()
    seedMnemonic = ''
    seedGenerated = false
    input = ''
    descriptorText = ''
    changeDescriptorText = ''
    error = ''
    kind = null
  }

  // Seed tab state — default to "Generate" since this is the more common
  // path for a first-time user opening "Add wallet".
  let seedMode = $state<'generate' | 'import'>('generate')
  let seedMnemonic = $state('')
  let seedPassphrase = $state('')
  let seedScriptType = $state('native_segwit')
  let seedAccount = $state(0)
  let seedCustomPath = $state('')
  let seedShowAdvanced = $state(false)
  let seedGenerated = $state(false)
  let showPassphraseHelp = $state(false)

  // Live BIP39 validation for the import flow.
  let seedImportCheck = $state<MnemonicCheck | null>(null)
  let seedCheckPending = $state(false)
  let seedCheckSeq = 0

  // Word-by-word import: alternative to the textarea. Better for typing (each
  // input has BIP39 autocomplete and catches typos immediately), worse for
  // paste. User picks which they prefer.
  let seedImportLayout = $state<'paste' | 'words'>('paste')
  let seedWordInputs = $state<string[]>(Array(12).fill(''))
  let seedImportWordCount = $state<12 | 24>(12)

  // When the user types in word-by-word mode, sync the textarea-driven
  // mnemonic so the validation effect and submit() use the same source.
  $effect(() => {
    if (seedImportLayout !== 'words') return
    seedMnemonic = seedWordInputs.map(w => w.trim().toLowerCase()).filter(Boolean).join(' ')
  })

  // Switching layouts: when going paste -> words, populate the word inputs
  // from the existing mnemonic. When going words -> paste, the $effect above
  // has already pushed it into seedMnemonic.
  function switchSeedLayout(next: 'paste' | 'words') {
    if (next === 'words') {
      const words = seedMnemonic.trim().toLowerCase().split(/\s+/).filter(Boolean)
      const n = (words.length === 24 || seedImportWordCount === 24) ? 24 : 12
      seedImportWordCount = n
      seedWordInputs = Array.from({ length: n }, (_, i) => words[i] ?? '')
    }
    seedImportLayout = next
  }

  function handleWordPaste(e: ClipboardEvent, fromIdx: number) {
    const text = e.clipboardData?.getData('text') ?? ''
    const words = text.trim().toLowerCase().split(/\s+/).filter(Boolean)
    if (words.length > 1) {
      e.preventDefault()
      const next = [...seedWordInputs]
      for (let i = 0; i < words.length && fromIdx + i < next.length; i++) {
        next[fromIdx + i] = words[i]
      }
      seedWordInputs = next
      // If they pasted enough to fill out a 24-word mnemonic but we're in 12 mode, switch.
      if (words.length > 12 && seedImportWordCount === 12) {
        seedImportWordCount = 24
        seedWordInputs = Array.from({ length: 24 }, (_, i) => next[i] ?? '')
      }
    }
  }

  function handleWordKeydown(e: KeyboardEvent, idx: number) {
    if (e.key === ' ' || e.key === 'Enter') {
      const target = e.currentTarget as HTMLInputElement
      const trimmed = target.value.trim()
      if (trimmed) {
        e.preventDefault()
        seedWordInputs[idx] = trimmed.toLowerCase()
        const nextInput = document.getElementById(`seed-word-${idx + 1}`) as HTMLInputElement | null
        nextInput?.focus()
      }
    }
  }

  function setSeedImportWordCount(n: 12 | 24) {
    seedImportWordCount = n
    seedWordInputs = Array.from({ length: n }, (_, i) => seedWordInputs[i] ?? '')
  }

  $effect(() => {
    const input = seedMnemonic
    if (seedMode !== 'import' || !input.trim()) {
      seedImportCheck = null; seedCheckPending = false
      return
    }
    const seq = ++seedCheckSeq
    seedCheckPending = true
    checkMnemonic(input).then((c) => {
      if (seq === seedCheckSeq) {
        seedImportCheck = c
        seedCheckPending = false
      }
    })
  })

  let seedWords = $derived(
    seedMnemonic.trim() ? seedMnemonic.trim().split(/\s+/).filter(Boolean) : []
  )

  // Hardware wallet state
  type HwStatus = 'idle' | 'connecting' | 'pairing' | 'confirm' | 'done' | 'error'
  let hwStatus = $state<HwStatus>('idle')
  let hwMessage = $state('')
  let pairingCode = $state('')
  let hwAccount = $state('native_segwit')
  let hwAccountIndex = $state(0)
  let hwEventSource: EventSource | null = null
  // Key origin info captured from device — used to embed derivation path in descriptor
  let hwFingerprint = $state('')
  let hwPath = $state('')

  let pasteKind = $derived(detectPaste(input))
  // Keep `detected` string around for compatibility with file-import display.
  let detected = $derived(
    pasteKind.kind === 'address' || pasteKind.kind === 'xpub' ? pasteKind.label : ''
  )

  // `seedVerified` is driven by SeedGeneratorPanel — it goes true only after
  // the user has both confirmed the checkbox AND completed the 3-word
  // verification challenge.
  let seedVerified = $state(false)

  let canSubmit = $derived.by(() => {
    if (!label.trim() || loading) return false
    if (method === 'seed') {
      if (seedMode === 'generate') return seedGenerated && seedVerified
      // Imported mnemonic must pass full BIP39 validation (wordlist + checksum).
      return seedImportCheck?.kind === 'ok'
    }
    if (method === 'descriptor') return !!descriptorText.trim()
    return !!input.trim()
  })

  async function submit() {
    if (!canSubmit) return
    loading = true; error = ''
    try {
      let entry
      if (method === 'seed') {
        entry = await api.wallets.seedImport({
          label: label.trim(),
          mnemonic: seedMnemonic.trim(),
          passphrase: seedPassphrase,
          script_type: seedScriptType,
          account_index: seedAccount,
          custom_path: seedCustomPath.trim() || null,
          backend: selectedBackend,
        })
      } else if (method === 'descriptor') {
        entry = await api.wallets.importDescriptor({
          label: label.trim(),
          descriptor: descriptorText.trim(),
          change_descriptor: changeDescriptorText.trim() || null,
          backend: selectedBackend,
        })
      } else if ((method === 'hardware' || method === 'file') && hwFingerprint && hwPath) {
        // File imports with origin info (Coldcard generic.json, Sparrow descriptor,
        // etc.) take this path too, so the resulting descriptor has the proper
        // [fp/path] origin and hardware-wallet signing works.
        entry = await api.wallets.hwImport({
          label: label.trim(),
          xpub: input.trim(),
          fingerprint: hwFingerprint,
          path: hwPath,
          account_type: hwAccount,
          backend: selectedBackend,
        })
      } else {
        entry = await api.wallets.add(label.trim(), input.trim())
      }
      wallets.update(ws => [...ws, entry])
      activeWalletId.set(entry.id)
      goto(`/wallet/${entry.id}`, { replaceState: true })
    } catch (e) {
      error = e instanceof Error ? e.message : 'Unknown error'
    } finally {
      loading = false
    }
  }


  let fileDropActive = $state(false)

  function handleFileImport(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0]
    if (file) importFile(file)
  }

  function handleDrop(e: DragEvent) {
    e.preventDefault()
    fileDropActive = false
    const file = e.dataTransfer?.files?.[0]
    if (file) importFile(file)
  }

  function importFile(file: File) {
    const reader = new FileReader()
    reader.onload = () => {
      const parsed = parseImportFile(reader.result as string)
      if (!parsed) { error = 'Could not extract an xpub from this file.'; return }
      input = parsed.xpub
      // If the file gives us origin info, capture it so the wallet is created
      // with a proper descriptor instead of a pasted-xpub watch-only.
      if (parsed.fingerprint && parsed.path) {
        hwFingerprint = parsed.fingerprint
        hwPath = parsed.path
        if (parsed.accountType) hwAccount = parsed.accountType
      } else {
        hwFingerprint = ''
        hwPath = ''
      }
      error = ''
    }
    reader.readAsText(file)
  }

  function connectBitBox() {
    hwStatus = 'connecting'; hwMessage = ''; pairingCode = ''
    const url = `/api/hwi/xpub?account=${hwAccount}&account_index=${hwAccountIndex}`
    hwEventSource = new EventSource(url)
    hwEventSource.addEventListener('connecting', () => { hwStatus = 'connecting' })
    hwEventSource.addEventListener('pairing_code', (e) => {
      pairingCode = JSON.parse(e.data).code; hwStatus = 'pairing'
    })
    hwEventSource.addEventListener('waiting_confirm', () => { hwStatus = 'confirm' })
    hwEventSource.addEventListener('paired', () => { hwStatus = 'confirm'; pairingCode = '' })
    hwEventSource.addEventListener('xpub', (e) => {
      const data = JSON.parse(e.data)
      input = data.xpub
      hwFingerprint = data.fingerprint ?? ''
      hwPath = data.path ?? ''
      hwStatus = 'done'
      hwEventSource?.close(); hwEventSource = null
    })
    hwEventSource.addEventListener('hw_error', (e) => {
      hwMessage = JSON.parse(e.data).message; hwStatus = 'error'
      hwEventSource?.close(); hwEventSource = null
    })
    hwEventSource.onerror = () => {
      if (hwStatus !== 'done' && hwStatus !== 'error') { hwMessage = 'Connection lost.'; hwStatus = 'error' }
      hwEventSource?.close(); hwEventSource = null
    }
  }

  function cancelBitBox() {
    hwEventSource?.close(); hwEventSource = null
    hwStatus = 'idle'; hwMessage = ''; pairingCode = ''
  }

  function switchMethod(m: typeof method) {
    method = m
    if (m !== 'hardware') cancelBitBox()
    if (m !== 'seed') { seedMnemonic = ''; seedGenerated = false }
    if (m !== 'descriptor') { descriptorText = ''; changeDescriptorText = '' }
    error = ''
  }

  function cancel() {
    history.back()
  }

  let labelInputEl = $state<HTMLInputElement | null>(null)
  let fileInputHidden = $state<HTMLInputElement | null>(null)
  $effect(() => {
    if (labelInputEl) labelInputEl.focus()
  })

  // beforeunload guard: warn before tab close / refresh while a generated
  // mnemonic exists that hasn't been confirmed-and-saved yet.
  function beforeUnloadHandler(e: BeforeUnloadEvent) {
    if (seedGenerated && !loading) {
      e.preventDefault()
      return ''
    }
  }

  onMount(() => {
    window.addEventListener('beforeunload', beforeUnloadHandler)
    api.backends.list().then(b => { savedBackends = b }).catch(() => {})
  })

  onDestroy(() => {
    hwEventSource?.close()
    window.removeEventListener('beforeunload', beforeUnloadHandler)
  })
</script>

<div class="page">
  <div class="form-card" class:picker={kind === null}>
    <h1>Add wallet <HelpLink anchor="add-wallet" /></h1>

    {#if kind === null}
      <KindPicker onPick={pickKind} />
    {:else}
    <button type="button" class="back-to-kind" onclick={backToKindPicker}>
      ← Wallet kind
    </button>

    <form onsubmit={(e) => { e.preventDefault(); submit() }}>

      <div class="field">
        <label for="wallet-label">Name</label>
        <input
          id="wallet-label"
          type="text"
          bind:value={label}
          bind:this={labelInputEl}
          placeholder="e.g. Cold storage"
        />
      </div>

      {#if kind === 'single'}
        <div class="field">
          <label for="wallet-backend">Backend</label>
          <select id="wallet-backend" bind:value={selectedBackend}>
            <option value={null}>Default</option>
            {#each backendOptions as b (b.id)}
              <option value={b.id}>{b.label}</option>
            {/each}
          </select>
          <p class="field-hint">
            Which server this wallet syncs and broadcasts through. “Default” uses your main backend; pick a saved backend to keep this wallet's activity off your other servers. Add backends in Backend settings.
          </p>
        </div>
      {/if}

      {#if kind === 'single'}
        <div class="method-tabs">
          <button type="button" class:active={method === 'seed'} onclick={() => switchMethod('seed')}>
            Seed phrase
          </button>
          <button type="button" class:active={method === 'paste'} onclick={() => switchMethod('paste')}>
            Paste xpub
          </button>
          <button type="button" class:active={method === 'file'} onclick={() => switchMethod('file')}>
            Import file
          </button>
          {#if $hwEnabled}
            <button type="button" class:active={method === 'hardware'} onclick={() => switchMethod('hardware')}>
              Hardware wallet
            </button>
          {/if}
          <button type="button" class:active={method === 'descriptor'} onclick={() => switchMethod('descriptor')}>
            Descriptor
          </button>
        </div>
      {:else if kind === 'address'}
        <div class="method-tabs">
          <button type="button" class:active={method === 'paste'} onclick={() => switchMethod('paste')}>
            Paste address
          </button>
          <button type="button" class:active={method === 'file'} onclick={() => switchMethod('file')}>
            Import file
          </button>
        </div>
      {/if}

      {#if kind === 'vault'}
        <VaultCreatePanel {label} savedBackends={backendOptions} onCreated={handleCreated} />
      {:else if kind === 'sp'}
        <SpCreatePanel {label} savedBackends={backendOptions} onCreated={handleCreated} />
      {:else if kind === 'multisig'}
        <MultisigCreatePanel bind:label savedBackends={backendOptions} onCreated={handleCreated} />
      {:else if method === 'paste'}
        <div class="method-body">
          <label for="wallet-input" class="field-label">Address or extended public key</label>
          <textarea
            id="wallet-input"
            bind:value={input}
            rows="3"
            placeholder="bc1q… (watch-only)  ·  xpub / ypub / zpub… (HD wallet)"
            spellcheck="false"
            autocapitalize="off"
          ></textarea>

          {#if pasteKind.kind === 'empty'}
            <span class="field-hint">Bitcoin address (watch-only) or BIP32 extended public key (HD)</span>
          {:else if pasteKind.kind === 'unknown'}
            <span class="field-hint warn">↳ Unrecognised format</span>
          {:else if pasteKind.kind === 'address'}
            <div class="paste-badge paste-badge-watch">
              <span class="paste-badge-title">👁 Watch-only wallet</span>
              <span class="paste-badge-msg">You'll be able to see balances and receive, but signing/sending is not available without a private key. To enable sending, add the wallet via Seed phrase, Hardware wallet, or Multisig instead.</span>
            </div>
          {:else if pasteKind.kind === 'xpub'}
            <div class="paste-badge paste-badge-info">
              <span class="paste-badge-title">↳ {pasteKind.label}</span>
              <span class="paste-badge-msg">
                Heads up: pasting an xpub creates a <strong>watch-only HD wallet</strong>.
                {#if $hwEnabled}
                  If this xpub came from a hardware wallet, use the
                  <button type="button" class="paste-badge-link" onclick={() => switchMethod('hardware')}>Hardware wallet tab</button>
                  instead so the device fingerprint + derivation path are captured — without them, signing won't work.
                {:else}
                  To sign, import the descriptor file (with its fingerprint + derivation path) or use a seed phrase — a bare xpub can't sign.
                {/if}
              </span>
            </div>
          {/if}
        </div>

      {:else if method === 'descriptor'}
        <div class="method-body">
          <label for="desc-input" class="field-label">Output descriptor</label>
          <textarea
            id="desc-input"
            bind:value={descriptorText}
            rows="4"
            placeholder="wpkh([fp/84h/0h/0h]xpub…/&lt;0;1&gt;/*)#checksum  ·  wsh(sortedmulti(2,…))"
            spellcheck="false"
            autocapitalize="off"
          ></textarea>
          <span class="field-hint">
            Paste a full descriptor from Sparrow, Bitcoin Core, BSMS, etc. Multipath
            (<code>&lt;0;1&gt;</code>) descriptors carry both keychains; for a
            separate receive/change pair, add the change descriptor below.
          </span>

          <label for="desc-change-input" class="field-label" style="margin-top:0.75rem">
            Change descriptor <span class="optional">(optional)</span>
          </label>
          <textarea
            id="desc-change-input"
            bind:value={changeDescriptorText}
            rows="3"
            placeholder="Leave empty for multipath or single-keychain descriptors"
            spellcheck="false"
            autocapitalize="off"
          ></textarea>

          {#if descriptorText.trim() && !descriptorHasOrigin}
            <div class="paste-badge paste-badge-info">
              <span class="paste-badge-title">↳ No key origin found</span>
              <span class="paste-badge-msg">This descriptor has no <code>[fingerprint/path]</code> origin, so a hardware wallet can't match its keys — it imports as <strong>watch-only with signing unavailable</strong>. Re-export from your wallet including key origin info to enable signing.</span>
            </div>
          {:else}
            <div class="paste-badge paste-badge-watch">
              <span class="paste-badge-title">👁 Watch-only</span>
              <span class="paste-badge-msg">Imported descriptors have no private keys — you'll see balances and receive, and sign by exporting a PSBT to your hardware wallet or external signer.</span>
            </div>
          {/if}
        </div>

      {:else if method === 'file'}
        <div class="method-body">
          <div
            class="drop-zone"
            class:active={fileDropActive}
            role="button"
            tabindex="0"
            ondragover={(e) => { e.preventDefault(); fileDropActive = true }}
            ondragleave={() => fileDropActive = false}
            ondrop={handleDrop}
            onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); fileInputHidden?.click() } }}
          >
            <span class="drop-icon">↓</span>
            <span class="drop-label">Drop file here or</span>
            <label class="file-pick-btn">
              browse
              <input type="file" accept=".json,.txt" onchange={handleFileImport} />
            </label>
          </div>
          <input type="file" accept=".json,.txt" onchange={handleFileImport} bind:this={fileInputHidden} style="display:none" />
          <p class="field-hint">Coldcard generic JSON · Sparrow/Specter descriptor · plain text containing an xpub.</p>
          {#if input && detected}
            <div class="paste-badge paste-badge-watch" style="margin-top: 10px">
              <span class="paste-badge-title">↳ {detected} — ready</span>
              {#if hwFingerprint && hwPath}
                <span class="paste-badge-msg">Origin captured: fingerprint <code class="mono-inline">{hwFingerprint}</code> · path <code class="mono-inline">{hwPath}</code> — hardware-wallet signing will work for this wallet.</span>
              {:else}
                <span class="paste-badge-msg">No fingerprint or derivation path found in this file — the wallet will be watch-only.{#if $hwEnabled} Use the Hardware wallet tab if you want to sign with a connected device.{/if}</span>
              {/if}
            </div>
          {/if}
        </div>

      {:else if method === 'seed'}
        <div class="method-body">
          <div class="seed-mode-tabs">
            <button type="button" class:active={seedMode === 'generate'} onclick={() => { seedMode = 'generate'; seedMnemonic = ''; seedGenerated = false; error = '' }}>Generate new</button>
            <button type="button" class:active={seedMode === 'import'} onclick={() => { seedMode = 'import'; seedMnemonic = ''; seedGenerated = false; error = '' }}>Import existing</button>
          </div>

          <div class="seed-row">
            <div class="field" style="flex:1">
              <label for="seed-script">Script type</label>
              <select id="seed-script" bind:value={seedScriptType}>
                <option value="native_segwit">Native SegWit (P2WPKH)</option>
                <option value="taproot">Taproot (P2TR)</option>
                <option value="wrapped_segwit">Wrapped SegWit (P2SH-P2WPKH)</option>
                <option value="legacy">Legacy (P2PKH)</option>
              </select>
            </div>
            <div class="field" style="width:72px;flex-shrink:0">
              <label for="seed-account">Account</label>
              <input id="seed-account" type="number" min="0" max="99" bind:value={seedAccount} />
            </div>
          </div>

          <button type="button" class="advanced-toggle" onclick={() => seedShowAdvanced = !seedShowAdvanced}>
            {seedShowAdvanced ? '▾' : '▸'} Advanced
          </button>
          {#if seedShowAdvanced}
            <div class="field">
              <label for="seed-path">Custom derivation path <span class="optional">(optional)</span></label>
              <input id="seed-path" type="text" bind:value={seedCustomPath} placeholder="m/84'/0'/0'" spellcheck="false" />
              <span class="field-hint">Leave empty to use the standard path for the selected script type.</span>
            </div>
          {/if}

          {#if seedMode === 'generate'}
            <SeedGeneratorPanel
              bind:mnemonic={seedMnemonic}
              bind:verified={seedVerified}
              bind:generated={seedGenerated}
              walletLabel={label}
            />

            {#if seedGenerated}
              <div class="field">
                <label for="seed-pass-gen">
                  BIP39 passphrase <span class="optional">(optional)</span>
                  <button type="button" class="info-btn" onclick={() => showPassphraseHelp = !showPassphraseHelp} aria-label="What is this?">?</button>
                </label>
                {#if showPassphraseHelp}
                  <p class="info-blurb">A passphrase (sometimes called a "25th word") is an extra secret combined with your seed to derive a different wallet. It's optional and not stored anywhere — if you set one, you must remember it to access your funds.</p>
                {/if}
                <input id="seed-pass-gen" type="password" bind:value={seedPassphrase} placeholder="Leave blank if unsure" autocomplete="new-password" />
              </div>
            {/if}

          {:else}
            <div class="field">
              <div class="seed-layout-row">
                <label for="seed-words">Mnemonic phrase</label>
                <div class="seed-layout-toggle" role="group" aria-label="Input style">
                  <button type="button" class:active={seedImportLayout === 'paste'} onclick={() => switchSeedLayout('paste')}>Paste</button>
                  <button type="button" class:active={seedImportLayout === 'words'} onclick={() => switchSeedLayout('words')}>Word by word</button>
                </div>
              </div>

              {#if seedImportLayout === 'paste'}
                <textarea
                  id="seed-words"
                  rows="3"
                  bind:value={seedMnemonic}
                  placeholder="Enter 12 or 24 words separated by spaces"
                  spellcheck="false"
                  autocomplete="off"
                  autocapitalize="off"
                ></textarea>
              {:else}
                <div class="seed-words-header">
                  <div class="word-count-btns" role="group" aria-label="Word count">
                    <button type="button" class:active={seedImportWordCount === 12} onclick={() => setSeedImportWordCount(12)}>12</button>
                    <button type="button" class:active={seedImportWordCount === 24} onclick={() => setSeedImportWordCount(24)}>24</button>
                  </div>
                  <span class="seed-words-hint">Press space or Enter to advance · paste fills from the current cell</span>
                </div>
                <div class="seed-words-grid" class:cols-4={seedImportWordCount === 24}>
                  {#each seedWordInputs as _, i (i)}
                    <label class="seed-word-cell">
                      <span class="seed-word-num">{i + 1}</span>
                      <input
                        id="seed-word-{i}"
                        type="text"
                        class="seed-word-input"
                        list="bip39-wordlist"
                        autocomplete="off"
                        autocapitalize="off"
                        spellcheck="false"
                        bind:value={seedWordInputs[i]}
                        onkeydown={(e) => handleWordKeydown(e, i)}
                        onpaste={(e) => handleWordPaste(e, i)}
                      />
                    </label>
                  {/each}
                </div>
                <!-- One shared datalist; the browser narrows it as the user types. -->
                <datalist id="bip39-wordlist">
                  {#each BIP39_WORDS as w (w)}
                    <option value={w}></option>
                  {/each}
                </datalist>
              {/if}

              {#if seedWords.length > 0}
                {#if seedCheckPending}
                  <span class="field-hint">{seedWords.length} words — checking…</span>
                {:else if seedImportCheck?.kind === 'ok'}
                  <span class="field-hint detected">{seedWords.length} words ✓ valid BIP39 mnemonic</span>
                {:else if seedImportCheck?.kind === 'wrong_count'}
                  <span class="field-hint warn">{seedImportCheck.count} words — must be 12, 15, 18, 21, or 24</span>
                {:else if seedImportCheck?.kind === 'invalid_words'}
                  <span class="field-hint warn">
                    Not in the BIP39 wordlist:
                    {#each seedImportCheck.words.slice(0, 3) as w, i (i)}
                      <code class="bad-word">{w}</code>
                    {/each}
                    {#if seedImportCheck.words.length > 3}…{/if}
                  </span>
                {:else if seedImportCheck?.kind === 'bad_checksum'}
                  <span class="field-hint warn">All words valid, but the checksum is wrong — usually means one word is in the wrong order or mistyped.</span>
                {/if}
              {/if}
            </div>
            <div class="field">
              <label for="seed-pass-imp">
                BIP39 passphrase <span class="optional">(optional)</span>
                <button type="button" class="info-btn" onclick={() => showPassphraseHelp = !showPassphraseHelp} aria-label="What is this?">?</button>
              </label>
              {#if showPassphraseHelp}
                <p class="info-blurb">A passphrase (sometimes called a "25th word") is an extra secret combined with your seed to derive a different wallet. It's optional and not stored anywhere — if your original wallet used one, you must enter the same passphrase to recover the same addresses.</p>
              {/if}
              <input id="seed-pass-imp" type="password" bind:value={seedPassphrase} placeholder="Leave blank if your wallet has no passphrase" autocomplete="new-password" />
            </div>
          {/if}
        </div>

      {:else if method === 'hardware'}
        <div class="method-body">
          <div class="hw-config">
            <div class="field hw-field">
              <label for="hw-account-type">Script type</label>
              <select id="hw-account-type" bind:value={hwAccount}>
                <option value="native_segwit">Native SegWit (zpub)</option>
                <option value="taproot">Taproot (P2TR)</option>
                <option value="p2sh_segwit">P2SH-SegWit (ypub)</option>
                <option value="legacy">Legacy (xpub)</option>
              </select>
            </div>
            <div class="field hw-field-idx">
              <label for="hw-account-idx">Account</label>
              <input id="hw-account-idx" type="number" min="0" max="99" bind:value={hwAccountIndex} />
            </div>
          </div>

          {#if hwStatus === 'idle' || hwStatus === 'error'}
            <button type="button" class="btn-connect" onclick={connectBitBox}>Connect device</button>
          {:else if hwStatus === 'done'}
            <button type="button" class="btn-connect" onclick={connectBitBox}>Re-connect</button>
          {:else}
            <button type="button" class="btn-cancel-hw" onclick={cancelBitBox}>Cancel</button>
          {/if}

          <div class="hw-status">
            {#if hwStatus === 'connecting'}
              <p class="hw-msg">Connecting to device…</p>
            {:else if hwStatus === 'pairing'}
              <p class="hw-msg">Verify pairing code on your BitBox02:</p>
              <pre class="pairing-code">{pairingCode}</pre>
            {:else if hwStatus === 'confirm'}
              <p class="hw-msg">Confirm xpub export on device…</p>
            {:else if hwStatus === 'done'}
              <p class="hw-msg ok">xpub imported successfully.</p>
            {:else if hwStatus === 'error'}
              <p class="hw-msg err">{hwMessage}</p>
            {:else}
              <p class="hw-msg muted">Plug in your hardware wallet, then click Connect.</p>
            {/if}
          </div>
        </div>
      {/if}

      {#if kind !== 'vault' && kind !== 'sp' && kind !== 'multisig'}
        {#if error}
          <p class="error">{error}</p>
        {/if}

        <div class="actions">
          <button type="button" class="btn-ghost" onclick={cancel}>Cancel</button>
          <button type="submit" class="btn-primary" disabled={!canSubmit}>
            {loading ? 'Adding…' : 'Add wallet'}
          </button>
        </div>
      {/if}

    </form>
    {/if}
  </div>
</div>

<style>
  .page {
    flex: 1;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    overflow-y: auto;
    background: var(--surface-2);
    padding: 40px 24px 48px;
  }

  .form-card {
    width: 100%;
    max-width: 560px;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 28px 32px 24px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  /* Widen only for the kind picker so the 3-column tile grid has room; the
     forms below stay at the comfortable 560px reading width. */
  .form-card.picker { max-width: 840px; }

  h1 { font-size: 1.2rem; font-weight: 700; color: var(--text); margin: 0; }

  form { display: flex; flex-direction: column; gap: 16px; }

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

  /* kind-picker tiles moved to KindPicker.svelte. */

  .back-to-kind {
    align-self: flex-start;
    background: none; border: none; padding: 0; margin-bottom: 6px;
    color: var(--text-muted); cursor: pointer; font-size: 0.82rem;
  }
  .back-to-kind:hover { color: var(--text); }

  .method-tabs { display: flex; gap: 4px; }
  .method-tabs button {
    flex: 1; background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    padding: 7px 10px; cursor: pointer; font-size: 0.8rem; color: var(--text-muted); font-weight: 500;
  }
  .method-tabs button:hover { color: var(--text); }
  .method-tabs button.active {
    background: var(--surface-active); border-color: var(--accent);
    color: var(--accent); font-weight: 600;
  }

  .method-body { display: flex; flex-direction: column; gap: 8px; }
  .detected { font-size: 0.75rem; color: var(--accent); }
  .field-hint { font-size: 0.75rem; color: var(--text-muted); line-height: 1.4; }

  .drop-zone {
    border: 2px dashed var(--border); border-radius: 6px;
    padding: 28px 20px; text-align: center;
    display: flex; align-items: center; justify-content: center; gap: 8px;
    cursor: default; transition: border-color 0.15s;
  }
  .drop-zone.active { border-color: var(--accent); }
  .drop-icon { font-size: 1.2rem; color: var(--text-muted); }
  .drop-label { font-size: 0.85rem; color: var(--text-muted); }
  .file-pick-btn {
    background: none; border: none; color: var(--accent); cursor: pointer;
    font-size: 0.85rem; font-weight: 600; text-decoration: underline; padding: 0;
  }
  .file-pick-btn input[type="file"] { display: none; }

  .seed-mode-tabs { display: flex; gap: 3px; }
  .seed-mode-tabs button {
    flex: 1; background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px;
    padding: 5px 10px; cursor: pointer; font-size: 0.78rem; color: var(--text-muted); font-weight: 500;
  }
  .seed-mode-tabs button.active { background: var(--surface-active); border-color: var(--accent); color: var(--accent); font-weight: 600; }

  .seed-row { display: flex; gap: 10px; align-items: flex-start; }

  .word-count-btns { display: flex; gap: 4px; }
  .word-count-btns button {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px;
    padding: 6px 14px; cursor: pointer; font-size: 0.82rem; color: var(--text-muted);
  }
  .word-count-btns button.active { background: var(--surface-active); border-color: var(--accent); color: var(--accent); font-weight: 600; }

  .advanced-toggle {
    background: none; border: none; color: var(--text-muted); cursor: pointer;
    font-size: 0.75rem; padding: 0; text-align: left; align-self: flex-start;
  }
  .advanced-toggle:hover { color: var(--text); }

  .info-btn {
    background: var(--surface-2); border: 1px solid var(--border);
    color: var(--text-muted); cursor: pointer; font-size: 0.7rem;
    width: 16px; height: 16px; border-radius: 50%; padding: 0;
    display: inline-flex; align-items: center; justify-content: center;
    margin-left: 4px; line-height: 1;
  }
  .info-btn:hover { border-color: var(--accent); color: var(--accent); }
  .info-blurb {
    margin: 4px 0 4px;
    font-size: 0.75rem; color: var(--text-muted); line-height: 1.5;
    background: var(--surface-2); border-left: 2px solid var(--accent);
    padding: 6px 10px; border-radius: 0 4px 4px 0;
  }

  .seed-layout-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .seed-layout-toggle { display: flex; gap: 2px; background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px; padding: 2px; }
  .seed-layout-toggle button {
    background: transparent; border: none; color: var(--text-muted);
    padding: 3px 10px; font-size: 0.75rem; cursor: pointer; border-radius: 3px;
  }
  .seed-layout-toggle button.active { background: var(--surface-1); color: var(--accent); }
  .seed-layout-toggle button:hover:not(.active) { color: var(--text); }

  .seed-words-header { display: flex; align-items: center; gap: 12px; margin-bottom: 8px; flex-wrap: wrap; }
  .seed-words-hint { font-size: 0.72rem; color: var(--text-muted); }
  .seed-words-grid {
    display: grid; grid-template-columns: repeat(3, 1fr); gap: 6px;
  }
  .seed-words-grid.cols-4 { grid-template-columns: repeat(4, 1fr); }
  .seed-word-cell {
    display: flex; align-items: center; gap: 5px;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px;
    padding: 4px 6px;
  }
  .seed-word-cell:focus-within { border-color: var(--accent); }
  .seed-word-num {
    font-size: 0.68rem; color: var(--text-muted);
    min-width: 18px; text-align: right; font-variant-numeric: tabular-nums;
  }
  .seed-word-input {
    flex: 1; min-width: 0;
    background: transparent; border: none;
    color: var(--text); padding: 2px 0;
    font-size: 0.82rem; font-family: monospace;
  }
  .seed-word-input:focus { outline: none; }

  .optional { font-size: 0.72rem; color: var(--text-muted); font-weight: normal; margin-left: 3px; }
  .warn { color: #e09c52; }
  .bad-word {
    font-family: monospace; font-size: 0.72rem;
    background: color-mix(in srgb, #e05252 15%, transparent);
    color: #e05252; padding: 1px 5px; border-radius: 3px; margin: 0 2px;
  }
  .field-hint.detected { color: #52a875; }

  .paste-badge {
    display: flex; flex-direction: column; gap: 4px;
    border-radius: 5px; padding: 8px 10px;
    border: 1px solid var(--border); background: var(--surface-2);
    margin-top: 4px;
  }
  .paste-badge-title { font-size: 0.82rem; font-weight: 600; color: var(--text); }
  .paste-badge-msg { font-size: 0.75rem; color: var(--text-muted); line-height: 1.5; }
  .paste-badge-link {
    background: none; border: none; padding: 0;
    color: var(--accent); text-decoration: underline; text-underline-offset: 2px;
    cursor: pointer; font-size: inherit; font-family: inherit;
  }
  .mono-inline { font-family: monospace; font-size: 0.7rem; background: var(--surface-1); padding: 1px 4px; border-radius: 3px; }
  .paste-badge-link:hover { filter: brightness(1.2); }
  .paste-badge-watch {
    border-color: color-mix(in srgb, #52a8d4 40%, var(--border));
    background: color-mix(in srgb, #52a8d4 6%, var(--surface-2));
  }
  .paste-badge-watch .paste-badge-title { color: #52a8d4; }
  .paste-badge-info {
    border-color: color-mix(in srgb, #e09c52 40%, var(--border));
    background: color-mix(in srgb, #e09c52 6%, var(--surface-2));
  }
  .paste-badge-info .paste-badge-title { color: #e09c52; }

  .hw-config { display: flex; gap: 10px; align-items: flex-end; }
  .hw-field { flex: 1; }
  .hw-field-idx { width: 80px; flex-shrink: 0; }
  .btn-connect {
    background: var(--accent); color: #000; border: none; border-radius: 5px;
    padding: 8px 16px; cursor: pointer; font-weight: 600; font-size: 0.85rem;
    align-self: flex-start;
  }
  .btn-cancel-hw {
    background: var(--surface-2); color: var(--text); border: 1px solid var(--border);
    border-radius: 5px; padding: 8px 14px; cursor: pointer; font-size: 0.85rem;
    align-self: flex-start;
  }
  .hw-status { min-height: 24px; }
  .hw-msg { margin: 0; font-size: 0.82rem; color: var(--text-muted); }
  .hw-msg.ok { color: #52a875; }
  .hw-msg.err { color: var(--error); }
  .hw-msg.muted { color: var(--text-muted); }
  .pairing-code {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px;
    padding: 10px 14px; font-family: monospace; font-size: 0.9rem;
    letter-spacing: 0.05em; margin: 4px 0 0; white-space: pre;
  }

  .error { color: var(--error); font-size: 0.82rem; margin: 0; }

  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  .btn-primary {
    background: var(--accent); color: #000; border: none; border-radius: 5px;
    padding: 8px 20px; cursor: pointer; font-weight: 600; font-size: 0.85rem;
  }
  .btn-primary:disabled { opacity: 0.4; cursor: not-allowed; }
  .btn-ghost {
    background: none; border: 1px solid var(--border); border-radius: 5px;
    padding: 8px 16px; cursor: pointer; color: var(--text-muted); font-size: 0.85rem;
  }
  .btn-ghost:hover { color: var(--text); border-color: var(--text-muted); }

  @media (max-width: 768px) {
    .page { padding: 16px 12px 32px; align-items: stretch; }
    .form-card { padding: 20px 16px 18px; border-radius: 8px; }
  }
</style>
