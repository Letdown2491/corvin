<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  // Lazy-load qrcode to keep it out of the initial bundle.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let qrcodeMod = $state<any>(null)
  import { api } from '../lib/api'
  import type { AddressInfo, WalletEntry } from '../lib/types'
  import { addToast } from '../stores/toasts'
  import { lastPayjoinEvent } from '../stores/payjoin'
  import { showFiatBalance, currentBtcPrice, hwEnabled } from '../stores/settings'
  import { swallow } from '../lib/utils'
  import HelpLink from './HelpLink.svelte'
  import Modal from './ui/Modal.svelte'
  import CopyButton from './ui/CopyButton.svelte'
  import AmountInput from './ui/AmountInput.svelte'

  let {
    wallet,
    addressLabels = {},
    onClose,
  }: {
    wallet: WalletEntry
    addressLabels?: Record<string, string>
    onClose: () => void
  } = $props()

  let canvasEl  = $state<HTMLCanvasElement | null>(null)

  let unusedAddresses = $state<AddressInfo[]>([])
  let addrIndex = $state(0)
  let loading   = $state(true)
  let amount    = $state('')
  let amountUnit = $state<'sats' | 'btc'>('sats')
  let label     = $state('')
  let message   = $state('')
  let editingLabel = $state(false)
  let savingLabel  = $state(false)
  let showIndexHelp = $state(false)

  // Hardware-wallet address verification state. The `canVerifyOnDevice`
  // derived value is declared after `currentIndex` below.
  let verifyEs = $state<EventSource | null>(null)
  type VerifyStatus = 'idle' | 'connecting' | 'pairing' | 'confirm' | 'verifying' | 'matched' | 'mismatched' | 'error'
  let verifyStatus = $state<VerifyStatus>('idle')
  let verifyPairingCode = $state('')
  let verifyMessage = $state('')
  let verifyDeviceAddress = $state('')

  // SP wallets have a single reusable sp1q… address that lives in wallet.input,
  // like the Address watch-only kind — so they're treated as non-HD here.
  let isHd = $derived(wallet.kind !== 'address' && wallet.kind !== 'silent_payments')

  // Silent Payments: fetched on mount, shown as a secondary address panel if
  // the wallet has SP enabled. Independent from the standard receive flow —
  // SP addresses are static (no BIP21 amount/label semantics) and rendered in
  // their own block.
  let spAddress = $state<string | null>(null)

  // ── Payjoin receive (BIP-77) ───────────────────────────────────────────────
  // Available for native-segwit / taproot software wallets on an RPC backend
  // when payjoin is enabled. Provision returns a pj= URI; when a payer pays it,
  // the SSE flips us to the confirm step (sign our contributed input).
  let payjoinReceiveAvailable = $state(false)
  type PjrPhase = 'idle' | 'waiting' | 'proposal' | 'done'
  let pjrPhase = $state<PjrPhase>('idle')
  let pjrSessionId = $state<string | null>(null)
  let pjrUri = $state('')
  let pjrBusy = $state(false)
  let pjrError = $state('')
  let pjrMnemonic = $state('')
  let pjrPassphrase = $state('')
  let pjrSeedRevealed = $state(false)
  let pjrCanvasEl = $state<HTMLCanvasElement | null>(null)
  let pjrSeedReady = $derived(pjrMnemonic.trim().split(/\s+/).filter(Boolean).length >= 12)

  async function startPayjoinReceive() {
    pjrError = ''; pjrBusy = true
    try {
      const res = await api.wallets.payjoinReceive.provision(wallet.id, {})
      pjrSessionId = res.session_id
      pjrUri = res.uri
      pjrPhase = 'waiting'
    } catch (e) {
      pjrError = e instanceof Error ? e.message : 'Failed to start payjoin receive'
    } finally {
      pjrBusy = false
    }
  }
  async function confirmPayjoinReceive() {
    if (!pjrSessionId) return
    pjrError = ''; pjrBusy = true
    try {
      await api.wallets.payjoinReceive.confirm(wallet.id, pjrSessionId, {
        mnemonic: pjrMnemonic.trim(),
        passphrase: pjrPassphrase || undefined,
      })
      pjrPhase = 'done'
      pjrMnemonic = ''; pjrPassphrase = ''
    } catch (e) {
      pjrError = e instanceof Error ? e.message : 'Failed to sign payjoin'
    } finally {
      pjrBusy = false
    }
  }
  async function cancelPayjoinReceive() {
    if (pjrBusy) return
    pjrBusy = true
    try {
      if (pjrSessionId) {
        try { await api.wallets.payjoinReceive.cancel(wallet.id, pjrSessionId) } catch {}
      }
      pjrSessionId = null; pjrUri = ''; pjrPhase = 'idle'; pjrError = ''
    } finally {
      pjrBusy = false
    }
  }

  function copyText(text: string) {
    navigator.clipboard?.writeText(text).catch(() => addToast('Could not copy to clipboard'))
  }
  // SSE: a payer paid the pj= URI (proposal ready) or it was posted (done).
  $effect(() => {
    const ev = $lastPayjoinEvent
    if (ev && pjrSessionId && ev.session_id === pjrSessionId) {
      if (ev.status === 'proposal_ready') pjrPhase = 'proposal'
      else if (ev.status === 'sent') pjrPhase = 'done'
    }
  })
  // Render the pj= URI QR once we have it (clearing qrcode's inline sizing,
  // same fix as the main address QR).
  $effect(() => {
    if (pjrPhase === 'waiting' && pjrUri && pjrCanvasEl && qrcodeMod) {
      qrcodeMod.toCanvas(pjrCanvasEl, pjrUri, { width: 200, margin: 1 }, () => {
        if (pjrCanvasEl) { pjrCanvasEl.style.width = ''; pjrCanvasEl.style.height = '' }
      })
    }
  })

  // BIP-352 labeled addresses. Same scan key as the base address; the receiver
  // can tell which label a payment used, but base + labels are linkable.
  let spLabels = $state<{ m: number; name: string; address: string }[]>([])
  let showLabelForm = $state(false)
  let newLabelName = $state('')
  let addingLabel = $state(false)
  let labelError = $state('')

  async function addLabel() {
    const name = newLabelName.trim()
    if (!name || addingLabel) return
    addingLabel = true; labelError = ''
    try {
      const label = await api.silentPayments.addLabel(wallet.id, name)
      spLabels = [...spLabels, label]
      newLabelName = ''
      showLabelForm = false
    } catch (e) {
      labelError = e instanceof Error ? e.message : 'Failed to add label'
    } finally {
      addingLabel = false
    }
  }

  let currentAddress = $derived(
    isHd ? (unusedAddresses[addrIndex]?.address ?? '') : wallet.input
  )

  let currentIndex = $derived(
    isHd ? (unusedAddresses[addrIndex]?.index ?? null) : null
  )

  // The current wallet is HW-eligible if it has a derivation index visible
  // (i.e. HD wallet). The backend will further reject paste-xpub wallets
  // that lack origin info in the descriptor.
  let canVerifyOnDevice = $derived($hwEnabled && isHd && currentIndex != null && wallet.kind !== 'multisig')

  // Stored label = persisted label, distinct from the editable BIP21 label.
  // When the user is editing, we show the textbox; otherwise we show the
  // persisted value as a chip and offer "Edit" / "Add label".
  // svelte-ignore state_referenced_locally
  let storedLabels = $state<Record<string, string>>({ ...addressLabels })
  let currentLabel = $derived(storedLabels[currentAddress] ?? null)

  // Live fiat estimate for the entered amount, shown next to the input.
  let amountFiat = $derived.by((): string | null => {
    if (!$showFiatBalance || !$currentBtcPrice) return null
    const n = parseFloat(amount)
    if (isNaN(n) || n <= 0) return null
    const btc = amountUnit === 'sats' ? n / 1e8 : n
    const usd = btc * $currentBtcPrice
    if (usd < 0.01) return null
    return `≈ $${usd.toLocaleString(undefined, { maximumFractionDigits: 2 })}`
  })

  let bitcoinUri = $derived.by(() => {
    if (!currentAddress) return ''
    const params: string[] = []
    const n = parseFloat(amount)
    if (amount && !isNaN(n) && n > 0) {
      const btc = amountUnit === 'sats' ? n / 1e8 : n
      const fmt = btc.toFixed(8).replace(/\.?0+$/, '')
      params.push(`amount=${fmt}`)
    }
    if (label.trim()) params.push(`label=${encodeURIComponent(label.trim())}`)
    if (message.trim()) params.push(`message=${encodeURIComponent(message.trim())}`)
    const qs = params.length ? `?${params.join('&')}` : ''
    return `bitcoin:${currentAddress}${qs}`
  })

  let hasUriExtras = $derived(amount.trim() !== '' || label.trim() !== '' || message.trim() !== '')

  $effect(() => {
    if (!qrcodeMod) {
      import('qrcode').then(m => { qrcodeMod = m }).catch(console.error)
    }
  })

  $effect(() => {
    const uri    = bitcoinUri
    const canvas = canvasEl
    const QR     = qrcodeMod
    if (!canvas || !uri || !QR) return
    // Render at high resolution and let CSS scale it down to the box width —
    // downscaling a crisp QR stays sharp at any display size.
    QR.toCanvas(canvas, uri, {
      width: 512,
      margin: 1,
      color: { dark: '#000000', light: '#ffffff' },
    }).then(() => {
      // qrcode writes inline width/height styles on the canvas; inline beats our
      // stylesheet, and with max-width capping the width but not the height the
      // QR stretches. Clear them so the CSS fixed 248px square wins (the backing
      // store stays 512px, so it's still crisp).
      canvas.style.width = ''
      canvas.style.height = ''
    }).catch(console.error)
  })

  let mounted = true
  onDestroy(() => {
    mounted = false
    verifyEs?.close()
    // A waiting invoice persists like a payment request (until paid, cancelled,
    // or 24h crate expiry) and is resumed on reopen — so closing the modal does
    // not cancel it. Use the explicit Cancel button to drop it.
  })

  function startDeviceVerify() {
    if (!canVerifyOnDevice || currentIndex == null) return
    verifyStatus = 'connecting'
    verifyPairingCode = ''
    verifyMessage = ''
    verifyDeviceAddress = ''
    // Capture the address being verified so a mid-flight navigation doesn't
    // make the match check compare against a different address.
    const verifyingAddress = currentAddress
    const params = new URLSearchParams({
      wallet_id: wallet.id,
      address_index: String(currentIndex),
      keychain: '0',
    })
    const es = new EventSource(`/api/hwi/show-address?${params}`)
    verifyEs = es
    es.addEventListener('connecting', () => { verifyStatus = 'connecting' })
    es.addEventListener('pairing_code', (e) => {
      verifyPairingCode = JSON.parse(e.data).code
      verifyStatus = 'pairing'
    })
    es.addEventListener('waiting_confirm', () => { verifyStatus = 'confirm' })
    es.addEventListener('paired', () => { verifyPairingCode = ''; verifyStatus = 'confirm' })
    es.addEventListener('verifying', () => { verifyStatus = 'verifying' })
    es.addEventListener('address', (e) => {
      const data = JSON.parse(e.data)
      verifyDeviceAddress = data.address
      verifyStatus = data.address === verifyingAddress ? 'matched' : 'mismatched'
      es.close(); verifyEs = null
    })
    es.addEventListener('hw_error', (e) => {
      verifyMessage = JSON.parse(e.data).message
      verifyStatus = 'error'
      es.close(); verifyEs = null
    })
    es.onerror = () => {
      if (verifyStatus !== 'matched' && verifyStatus !== 'mismatched' && verifyStatus !== 'error') {
        verifyMessage = 'Connection lost.'
        verifyStatus = 'error'
      }
      es.close(); verifyEs = null
    }
  }
  function cancelDeviceVerify() {
    verifyEs?.close(); verifyEs = null
    verifyStatus = 'idle'
    verifyMessage = ''
    verifyPairingCode = ''
  }

  onMount(async () => {
    if (isHd) {
      try {
        const all = await api.wallets.addresses(wallet.id)
        if (!mounted) return
        const unused = all
          .filter(a => a.kind === 'external' && !a.used)
          .sort((a, b) => a.index - b.index)
        // if everything is used, fall back to showing the last derived external address
        unusedAddresses = unused.length > 0
          ? unused
          : all.filter(a => a.kind === 'external').sort((a, b) => a.index - b.index).slice(-1)
      } catch (e) {
        addToast(e instanceof Error ? e.message : 'Failed to load addresses')
      }
      // Best-effort fetch of SP info — if not enabled or backend errors, just
      // skip the SP panel quietly.
      try {
        const info = await api.silentPayments.get(wallet.id)
        if (mounted && info.enabled && info.address) {
          spAddress = info.address
          try {
            const labels = await api.silentPayments.listLabels(wallet.id)
            if (mounted) spLabels = labels
          } catch (e) { swallow(e, 'sp listLabels') }
        }
      } catch {}
      // Payjoin receive availability (off by default; needs RPC + native-segwit
      // or taproot).
      try {
        const s = await api.settings.get()
        if (mounted) {
          payjoinReceiveAvailable = s.backend.payjoin_enabled
            && s.backend.type === 'rpc'
            && (wallet.kind === 'zpub' || wallet.kind === 'taproot')
        }
        // Resume an existing receive session (newest first): a proposal that
        // arrived while the modal was closed → jump to confirm; or an invoice
        // still awaiting payment → re-display it. One active invoice per wallet,
        // so this also avoids piling up duplicates.
        if (mounted && payjoinReceiveAvailable) {
          try {
            const sessions = await api.wallets.payjoinReceive.list(wallet.id)
            const proposal = sessions.find(s2 => s2.status === 'proposal_ready')
            const waiting = sessions.find(s2 => s2.status === 'negotiating' && s2.uri)
            if (mounted && proposal) {
              pjrSessionId = proposal.session_id
              pjrPhase = 'proposal'
            } else if (mounted && waiting) {
              pjrSessionId = waiting.session_id
              pjrUri = waiting.uri!
              pjrPhase = 'waiting'
            }
          } catch (e) { swallow(e, 'payjoin session resume') }
        }
      } catch {}
    }
    if (mounted) loading = false
  })


  // Inline label persistence — saves to /address-labels so it shows up in the
  // Addresses tab and the UTXO list next time the user sees this address.
  let labelEditValue = $state('')
  function startEditLabel() {
    labelEditValue = currentLabel ?? ''
    editingLabel = true
  }
  function cancelEditLabel() {
    editingLabel = false
    labelEditValue = ''
  }
  async function saveLabel() {
    if (!currentAddress) return
    const trimmed = labelEditValue.trim()
    savingLabel = true
    try {
      if (trimmed) {
        await api.addressLabels.set(currentAddress, trimmed)
        storedLabels = { ...storedLabels, [currentAddress]: trimmed }
      } else if (currentLabel) {
        await api.addressLabels.delete(currentAddress)
        const { [currentAddress]: _, ...rest } = storedLabels
        storedLabels = rest
      }
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Failed to save label')
    } finally {
      savingLabel = false
      editingLabel = false
    }
  }
  function handleLabelKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') { e.preventDefault(); saveLabel() }
    if (e.key === 'Escape') cancelEditLabel()
  }

  function focusOnMount(node: HTMLInputElement) {
    node.focus()
  }
</script>

<Modal open onclose={onClose} title="Receive" width="440px">
  {#snippet help()}<HelpLink anchor="receive" />{/snippet}

    {#if loading}
      <p class="hint">Loading…</p>
    {:else if !currentAddress}
      <p class="hint">No addresses found. Sync the wallet first.</p>
    {:else}

      <!-- QR code -->
      <div class="qr-wrap" role="img" aria-label="QR code for {currentAddress}">
        <canvas bind:this={canvasEl}></canvas>
      </div>

      <!-- Address + copy -->
      <div class="addr-row">
        <span class="addr-text">{currentAddress}</span>
        <CopyButton class="rcv-copy-btn" idle="Copy" copiedLabel="✓ Copied" text={() => currentAddress} disabled={!currentAddress} />
      </div>

      {#if hasUriExtras}
        <CopyButton class="rcv-copy-uri-btn" idle="↗ Copy BIP21 URI (with amount/label/message)" copiedLabel="✓ Copied URI" text={() => bitcoinUri} disabled={!bitcoinUri} />
      {/if}

      <!-- Index + label -->
      <div class="addr-meta">
        {#if currentIndex != null}
          <span class="addr-index">
            index #{currentIndex}
            <button type="button" class="addr-index-help" onclick={() => showIndexHelp = !showIndexHelp} aria-label="What is this?">?</button>
          </span>
        {/if}
        {#if editingLabel}
          <input
            type="text"
            class="label-edit-input"
            placeholder="Label this address"
            bind:value={labelEditValue}
            onkeydown={handleLabelKeydown}
            onblur={saveLabel}
            disabled={savingLabel}
            use:focusOnMount
          />
        {:else if currentLabel}
          <span class="addr-label">{currentLabel}</span>
          <button type="button" class="label-edit-btn" onclick={startEditLabel}>Edit</button>
        {:else}
          <button type="button" class="label-add-btn" onclick={startEditLabel}>+ Add label</button>
        {/if}
      </div>
      {#if showIndexHelp}
        <p class="info-blurb">This is the BIP32 derivation index of the address (m/.../0/{currentIndex}). HD wallets generate addresses sequentially; a fresh index per payment improves privacy because watchers can't link your activity.</p>
      {/if}

      {#if spAddress}
        <!-- Silent Payments static address. Reusable + private — each sender
             derives a unique on-chain destination from this string. Scanner
             support arrives in Phase 2; the address itself is shareable now. -->
        <div class="sp-block">
          <div class="sp-header">
            <span class="sp-label">⌬ Silent Payment address <HelpLink anchor="sp-concept" /></span>
            <span class="sp-static">static · reusable</span>
          </div>
          <div class="sp-addr-row">
            <code class="sp-addr">{spAddress}</code>
            <CopyButton class="rcv-copy-btn" idle="Copy" copiedLabel="✓ Copied" text={() => spAddress ?? ''} disabled={!spAddress} />
          </div>
          <p class="sp-hint">
            One address, fresh on-chain destination per payment. Senders need a Silent Payments-capable wallet.
          </p>

          <!-- BIP-352 labeled addresses -->
          <div class="sp-labels">
            <div class="sp-labels-head">
              <span class="sp-labels-title">Labeled addresses</span>
              {#if !showLabelForm}
                <button type="button" class="sp-label-add" onclick={() => { showLabelForm = true; labelError = '' }}>+ Add label</button>
              {/if}
            </div>

            {#each spLabels as l (l.m)}
              <div class="sp-label-row">
                <span class="sp-label-name">{l.name}</span>
                <code class="sp-label-addr">{l.address}</code>
                <CopyButton class="rcv-copy-btn" idle="Copy" copiedLabel="✓" text={l.address} />
              </div>
            {/each}

            {#if showLabelForm}
              <div class="sp-label-form">
                <p class="sp-label-warn">
                  ⚠ A labeled address shares this wallet's scan key — the base and labeled addresses are <strong>linkable to each other</strong>, and each extra label permanently increases scanning cost. Scanning for a new label begins after the next restart.
                </p>
                <div class="sp-label-input-row">
                  <input
                    type="text"
                    class="sp-label-input"
                    bind:value={newLabelName}
                    placeholder="Label name — e.g. Donations"
                    maxlength="40"
                    onkeydown={(e) => e.key === 'Enter' && addLabel()}
                  />
                  <button type="button" class="copy-btn" onclick={addLabel} disabled={addingLabel || !newLabelName.trim()}>
                    {addingLabel ? '…' : 'Create'}
                  </button>
                  <button type="button" class="sp-label-cancel" onclick={() => { showLabelForm = false; newLabelName = ''; labelError = '' }}>Cancel</button>
                </div>
                {#if labelError}<p class="sp-label-err">{labelError}</p>{/if}
              </div>
            {/if}
          </div>
        </div>
      {/if}

      <!-- Payjoin receive (BIP-77) -->
      {#if payjoinReceiveAvailable}
        <div class="pjr-block">
          {#if pjrPhase === 'idle'}
            <div class="pjr-head">
              <span class="pjr-label">⚡ Payjoin</span>
            </div>
            <p class="pjr-hint">Share a payjoin invoice instead of a plain address — the payment is coordinated privately and one of your inputs joins the transaction, breaking chain-analysis heuristics.</p>
            {#if pjrError}<p class="pjr-err">{pjrError}</p>{/if}
            <button type="button" class="pjr-btn" onclick={startPayjoinReceive} disabled={pjrBusy}>
              {pjrBusy ? 'Starting…' : 'Create payjoin invoice'}
            </button>
          {:else if pjrPhase === 'waiting'}
            <div class="pjr-head">
              <span class="pjr-label">⚡ Payjoin invoice</span>
              <span class="pjr-waiting">waiting for payment…</span>
            </div>
            <div class="pjr-qr"><canvas bind:this={pjrCanvasEl}></canvas></div>
            <code class="pjr-uri">{pjrUri}</code>
            <div class="pjr-actions">
              <button type="button" class="pjr-link" onclick={() => copyText(pjrUri)}>Copy invoice</button>
              <button type="button" class="pjr-link" onclick={cancelPayjoinReceive} disabled={pjrBusy}>Cancel</button>
            </div>
            <p class="pjr-hint">Keep this open — when the payer pays, you'll confirm here to coordinate the payjoin.</p>
          {:else if pjrPhase === 'proposal'}
            <div class="pjr-head"><span class="pjr-label">⚡ Payment received</span></div>
            <p class="pjr-hint">A payer paid your invoice. Enter your seed for <strong>{wallet.label}</strong> to sign your contributed input and finish the payjoin. It's used once and never stored.</p>
            <div class="pjr-seed-row">
              <label for="pjr-seed" class="pjr-field-label">Seed phrase</label>
              <button type="button" class="pjr-link" onclick={() => pjrSeedRevealed = !pjrSeedRevealed}>{pjrSeedRevealed ? 'Hide' : 'Show'}</button>
            </div>
            <input id="pjr-seed" class="pjr-input" type={pjrSeedRevealed ? 'text' : 'password'} bind:value={pjrMnemonic} placeholder="twelve or twenty-four words" spellcheck="false" autocapitalize="off" autocomplete="off" />
            <input class="pjr-input" type="password" bind:value={pjrPassphrase} placeholder="BIP39 passphrase (optional)" aria-label="BIP39 passphrase (optional)" autocomplete="new-password" />
            {#if pjrError}<p class="pjr-err">{pjrError}</p>{/if}
            <button type="button" class="pjr-btn" onclick={confirmPayjoinReceive} disabled={!pjrSeedReady || pjrBusy}>
              {pjrBusy ? 'Signing…' : 'Confirm payjoin'}
            </button>
          {:else if pjrPhase === 'done'}
            <div class="pjr-head"><span class="pjr-label">✓ Payjoin coordinated</span></div>
            <p class="pjr-hint">Signed and sent back to the payer — they'll broadcast the payjoin transaction.</p>
          {/if}
        </div>
      {/if}

      <!-- Verify on hardware device -->
      {#if canVerifyOnDevice && verifyStatus === 'idle'}
        <button type="button" class="verify-device-btn" onclick={startDeviceVerify}>
          ⚿ Verify address on hardware device
        </button>
      {:else if verifyStatus !== 'idle'}
        <div class="verify-panel" class:verify-matched={verifyStatus === 'matched'} class:verify-mismatched={verifyStatus === 'mismatched'} class:verify-error={verifyStatus === 'error'}>
          {#if verifyStatus === 'connecting'}
            <span class="verify-spinner">⚿</span>
            <span class="verify-msg">Connecting to device…</span>
            <button type="button" class="verify-cancel-btn" onclick={cancelDeviceVerify}>Cancel</button>
          {:else if verifyStatus === 'pairing'}
            <span class="verify-msg">Verify pairing code on device:</span>
            <code class="verify-pairing-code">{verifyPairingCode}</code>
            <button type="button" class="verify-cancel-btn" onclick={cancelDeviceVerify}>Cancel</button>
          {:else if verifyStatus === 'confirm' || verifyStatus === 'verifying'}
            <span class="verify-msg">Check the address shown on your device matches the one above. Tap confirm on the device when satisfied.</span>
            <button type="button" class="verify-cancel-btn" onclick={cancelDeviceVerify}>Cancel</button>
          {:else if verifyStatus === 'matched'}
            <span class="verify-icon">✓</span>
            <span class="verify-msg">Address verified — the device confirms it controls this address.</span>
            <button type="button" class="verify-cancel-btn" onclick={cancelDeviceVerify}>Close</button>
          {:else if verifyStatus === 'mismatched'}
            <span class="verify-icon">⚠</span>
            <span class="verify-msg"><strong>Mismatch.</strong> The device returned a different address: <code class="mono-inline">{verifyDeviceAddress}</code>. DO NOT use this address — your descriptor may be wrong or compromised.</span>
            <button type="button" class="verify-cancel-btn" onclick={cancelDeviceVerify}>Close</button>
          {:else if verifyStatus === 'error'}
            <span class="verify-icon">⚠</span>
            <span class="verify-msg">{verifyMessage}</span>
            <button type="button" class="verify-cancel-btn" onclick={cancelDeviceVerify}>Close</button>
          {/if}
        </div>
      {/if}

      <!-- Navigation (HD wallets with multiple unused addresses) -->
      {#if isHd && unusedAddresses.length > 1}
        <div class="nav-row">
          <button
            class="nav-btn"
            onclick={() => addrIndex--}
            disabled={addrIndex === 0}
            aria-label="Previous address"
          >←</button>
          <span class="nav-label">
            {#if unusedAddresses.length > 5}
              <select
                class="jump-select"
                aria-label="Jump to address"
                bind:value={addrIndex}
              >
                {#each unusedAddresses as a, i (a.address)}
                  <option value={i}>Address {i + 1} of {unusedAddresses.length}{storedLabels[a.address] ? ` · ${storedLabels[a.address]}` : ''}</option>
                {/each}
              </select>
            {:else}
              Address {addrIndex + 1} of {unusedAddresses.length}
            {/if}
          </span>
          <button
            class="nav-btn"
            onclick={() => addrIndex++}
            disabled={addrIndex === unusedAddresses.length - 1}
            aria-label="Next address"
          >→</button>
        </div>
      {/if}

      <details class="bip21-extras">
        <summary>More options</summary>
        <!-- Optional amount — tucked away since the common Receive action is
             just copying the address; only matters for a request-for-payment URI. -->
        <div class="amount-row">
          <div class="amount-field">
            <label for="recv-amount">
              Amount <span class="optional">(optional)</span>
            </label>
            <AmountInput id="recv-amount" bind:value={amount} bind:unit={amountUnit} showToggle ariaLabel="Amount" />
            {#if amountFiat}<span class="amount-fiat">{amountFiat}</span>{/if}
          </div>
        </div>
        <div class="field">
          <label for="recv-uri-label">Label <span class="optional">(included in BIP21 URI)</span></label>
          <input id="recv-uri-label" type="text" bind:value={label} placeholder="e.g. Alice payment" maxlength="100" />
        </div>
        <div class="field">
          <label for="recv-uri-message">Message <span class="optional">(included in BIP21 URI)</span></label>
          <input id="recv-uri-message" type="text" bind:value={message} placeholder="e.g. Order #42" maxlength="200" />
        </div>
        <p class="extras-hint">These fields are encoded into the BIP21 URI (the QR/copyable link). Save to the local address label using the "+ Add label" button above instead if you want it to persist in Corvin.</p>
      </details>

    {/if}
</Modal>

<style>
  .qr-wrap {
    display: flex; justify-content: center; align-items: center;
    align-self: center;            /* hug the QR and center in the modal */
    background: #fff; border-radius: 10px; padding: 14px;
    border: none; margin: 4px 0;
  }
  /* Medium QR, centered. Renders at 512px, downscaled to a fixed square box —
     width AND height pinned so flex layout can never stretch it. */
  .qr-wrap canvas { display: block; width: 248px; height: 248px; max-width: 100%; aspect-ratio: 1 / 1; }
  .jump-select {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 3px;
    color: var(--text); font-size: 0.72rem; padding: 1px 4px;
    margin-left: 4px;
  }

  .addr-row {
    display: flex; align-items: center; gap: 8px;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 6px; padding: 8px 10px;
  }
  .addr-text {
    flex: 1; min-width: 0;
    font-family: monospace; font-size: 0.7rem; color: var(--text-muted);
    word-break: break-all;
  }
  /* :global so the rules reach the CopyButton child's <button> (scoped CSS
     doesn't cross component boundaries). */
  :global(.rcv-copy-btn) {
    flex-shrink: 0; white-space: nowrap;
    background: none; border: 1px solid var(--border); border-radius: 4px;
    padding: 3px 9px; cursor: pointer; font-size: 0.75rem; color: var(--text);
    transition: border-color 0.12s, color 0.12s;
  }
  :global(.rcv-copy-btn:hover) { border-color: var(--accent); color: var(--accent); }

  :global(.rcv-copy-uri-btn) {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--accent); font-size: 0.75rem; padding: 6px 10px;
    cursor: pointer; transition: background 0.12s, border-color 0.12s;
    text-align: left;
  }
  :global(.rcv-copy-uri-btn:hover) { background: color-mix(in srgb, var(--accent) 10%, var(--surface-2)); border-color: var(--accent); }

  .addr-meta {
    display: flex; align-items: center; gap: 8px; min-height: 18px;
  }
  .addr-index { font-size: 0.72rem; color: var(--text-muted); font-family: monospace; display: inline-flex; align-items: center; gap: 4px; }
  .addr-index-help {
    background: var(--surface-2); border: 1px solid var(--border);
    color: var(--text-muted); cursor: pointer; font-size: 0.66rem; font-family: inherit;
    width: 14px; height: 14px; border-radius: 50%; padding: 0;
    display: inline-flex; align-items: center; justify-content: center; line-height: 1;
  }
  .addr-index-help:hover { border-color: var(--accent); color: var(--accent); }
  .addr-label {
    font-size: 0.72rem; color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    padding: 1px 6px; border-radius: 3px;
  }
  .label-edit-btn, .label-add-btn {
    background: none; border: none; cursor: pointer;
    color: var(--text-muted); font-size: 0.7rem; padding: 1px 4px;
    text-decoration: underline; text-underline-offset: 2px;
  }
  .label-edit-btn:hover, .label-add-btn:hover { color: var(--accent); }
  .label-edit-input {
    background: var(--surface-2); border: 1px solid var(--accent); border-radius: 3px;
    color: var(--text); padding: 2px 6px; font-size: 0.78rem; flex: 1; min-width: 0;
  }
  .label-edit-input:focus { outline: none; }
  .info-blurb {
    margin: -4px 0 0; font-size: 0.74rem; color: var(--text-muted); line-height: 1.5;
    background: var(--surface-2); border-left: 2px solid var(--accent);
    padding: 6px 10px; border-radius: 0 4px 4px 0;
  }

  /* Silent Payments panel — a distinctly-styled block so users immediately
     see it's a different kind of address. Subtle accent border + label. */
  .sp-block {
    margin-top: 4px;
    background: color-mix(in srgb, var(--accent) 5%, var(--surface-2));
    border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
    border-radius: 6px;
    padding: 10px 12px;
    display: flex; flex-direction: column; gap: 6px;
  }
  .sp-header {
    display: flex; align-items: baseline; justify-content: space-between; gap: 8px;
  }
  .sp-label {
    font-size: 0.74rem; font-weight: 700; color: var(--accent);
    text-transform: uppercase; letter-spacing: 0.06em;
  }
  .sp-static {
    font-size: 0.66rem; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.04em;
  }
  .sp-addr-row {
    display: flex; align-items: flex-start; gap: 6px;
  }
  .sp-addr {
    flex: 1; min-width: 0;
    font-family: monospace; font-size: 0.78rem; color: var(--text);
    word-break: break-all; line-height: 1.4;
    background: var(--surface-1); border-radius: 3px;
    padding: 6px 8px;
  }
  .sp-hint {
    margin: 0; font-size: 0.72rem; color: var(--text-muted); line-height: 1.5;
  }

  /* Payjoin receive */
  .pjr-block {
    margin-top: 4px;
    background: color-mix(in srgb, var(--accent) 7%, var(--surface-2));
    border: 1px solid color-mix(in srgb, var(--accent) 35%, var(--border));
    border-radius: 6px;
    padding: 12px 14px;
    display: flex; flex-direction: column; gap: 8px;
  }
  .pjr-head { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; }
  .pjr-label { font-size: 0.74rem; font-weight: 700; color: var(--accent); text-transform: uppercase; letter-spacing: 0.06em; }
  .pjr-waiting { font-size: 0.68rem; color: var(--text-muted); }
  .pjr-hint { margin: 0; font-size: 0.74rem; color: var(--text-muted); line-height: 1.5; }
  .pjr-hint strong { color: var(--text); }
  .pjr-err { margin: 0; font-size: 0.74rem; color: #e09c52; line-height: 1.5; }
  .pjr-qr { display: flex; justify-content: center; }
  .pjr-qr canvas { width: 200px; height: 200px; border-radius: 4px; background: #fff; padding: 6px; box-sizing: border-box; }
  .pjr-uri {
    font-family: monospace; font-size: 0.72rem; color: var(--text); word-break: break-all;
    line-height: 1.4; background: var(--surface-1); border-radius: 3px; padding: 6px 8px;
  }
  .pjr-actions { display: flex; gap: 14px; }
  .pjr-link { background: none; border: none; color: var(--accent); cursor: pointer; font-size: 0.78rem; padding: 0; }
  .pjr-link:hover { text-decoration: underline; }
  .pjr-btn {
    background: var(--accent); color: var(--accent-contrast, #fff); border: none;
    border-radius: 6px; padding: 9px 14px; font-size: 0.86rem; font-weight: 600; cursor: pointer;
  }
  .pjr-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .pjr-seed-row { display: flex; align-items: baseline; justify-content: space-between; }
  .pjr-field-label { font-size: 0.74rem; color: var(--text-muted); }
  .pjr-input {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); padding: 8px 10px; font-size: 0.82rem; font-family: monospace;
    width: 100%; box-sizing: border-box; outline: none;
  }
  .pjr-input:focus { border-color: var(--accent); }

  .sp-labels {
    margin-top: 10px; padding-top: 10px; border-top: 1px solid var(--border);
    display: flex; flex-direction: column; gap: 8px;
  }
  .sp-labels-head { display: flex; align-items: center; justify-content: space-between; }
  .sp-labels-title { font-size: 0.78rem; font-weight: 600; color: var(--text); }
  .sp-label-add {
    background: none; border: none; color: var(--accent); cursor: pointer;
    font-size: 0.78rem; padding: 0;
  }
  .sp-label-add:hover { text-decoration: underline; }
  .sp-label-row {
    display: flex; align-items: center; gap: 8px; min-width: 0;
  }
  .sp-label-name { font-size: 0.78rem; color: var(--text); flex-shrink: 0; max-width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sp-label-addr {
    font-family: var(--font-mono); font-size: 0.72rem; color: var(--text-muted);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; min-width: 0;
  }
  .sp-label-form { display: flex; flex-direction: column; gap: 6px; }
  .sp-label-warn {
    margin: 0; font-size: 0.72rem; line-height: 1.5; color: #e09c52;
    padding: 6px 10px; border-radius: 5px;
    background: color-mix(in srgb, #e09c52 8%, transparent);
    border: 1px solid color-mix(in srgb, #e09c52 30%, transparent);
  }
  .sp-label-warn strong { color: var(--text); }
  .sp-label-input-row { display: flex; align-items: center; gap: 6px; }
  .sp-label-input {
    flex: 1; min-width: 0; background: var(--surface-2);
    border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); padding: 6px 9px; font-size: 0.8rem;
  }
  .sp-label-input:focus { outline: 1px solid var(--accent); outline-offset: -1px; }
  .sp-label-cancel {
    background: none; border: none; color: var(--text-muted); cursor: pointer; font-size: 0.78rem;
  }
  .sp-label-cancel:hover { color: var(--text); }
  .sp-label-err { margin: 0; font-size: 0.74rem; color: var(--error); }
  .amount-fiat {
    margin-top: 3px; font-size: 0.72rem; color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .bip21-extras {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    padding: 8px 12px;
  }
  .bip21-extras summary {
    cursor: pointer; font-size: 0.78rem; color: var(--text-muted);
    padding: 2px 0;
  }
  .bip21-extras summary:hover { color: var(--text); }
  .bip21-extras[open] summary { color: var(--text); margin-bottom: 8px; }
  .bip21-extras .field { display: flex; flex-direction: column; gap: 4px; margin-bottom: 8px; }
  .bip21-extras label { font-size: 0.75rem; color: var(--text-muted); }
  .extras-hint { font-size: 0.7rem; color: var(--text-muted); line-height: 1.5; margin: 4px 0 0; }

  .verify-device-btn {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); cursor: pointer; font-size: 0.78rem; padding: 7px 12px;
    transition: border-color 0.12s, color 0.12s;
  }
  .verify-device-btn:hover { border-color: var(--accent); color: var(--accent); }

  .verify-panel {
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 5px; padding: 9px 12px;
    display: flex; flex-direction: column; gap: 6px;
  }
  .verify-panel.verify-matched {
    border-color: color-mix(in srgb, #52a875 50%, var(--border));
    background: color-mix(in srgb, #52a875 6%, var(--surface-2));
  }
  .verify-panel.verify-mismatched, .verify-panel.verify-error {
    border-color: color-mix(in srgb, #e05252 50%, var(--border));
    background: color-mix(in srgb, #e05252 6%, var(--surface-2));
  }
  .verify-icon { font-size: 1.1rem; color: #e09c52; }
  .verify-matched .verify-icon { color: #52a875; }
  .verify-mismatched .verify-icon, .verify-error .verify-icon { color: #e05252; }
  .verify-msg { font-size: 0.78rem; color: var(--text); line-height: 1.45; }
  .verify-spinner { font-size: 1rem; color: var(--accent); }
  .verify-pairing-code {
    font-family: monospace; font-size: 0.95rem; color: var(--accent);
    background: var(--surface-1); padding: 4px 8px; border-radius: 3px;
    display: inline-block;
  }
  .verify-cancel-btn {
    background: none; border: 1px solid var(--border); border-radius: 4px;
    color: var(--text-muted); cursor: pointer; font-size: 0.72rem; padding: 3px 8px;
    align-self: flex-start;
  }
  .verify-cancel-btn:hover { border-color: var(--text-muted); color: var(--text); }
  .mono-inline { font-family: monospace; font-size: 0.7rem; background: var(--surface-1); padding: 1px 4px; border-radius: 3px; word-break: break-all; }

  .nav-row {
    display: flex; align-items: center; justify-content: space-between; gap: 8px;
  }
  .nav-btn {
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 4px; padding: 4px 12px; cursor: pointer;
    color: var(--text); font-size: 0.9rem; flex-shrink: 0;
  }
  .nav-btn:disabled { opacity: 0.3; cursor: not-allowed; }
  .nav-btn:not(:disabled):hover { border-color: var(--accent); color: var(--accent); }
  .nav-label { font-size: 0.75rem; color: var(--text-muted); text-align: center; }

  .amount-row {
    display: flex; align-items: flex-end; gap: 8px;
  }
  .amount-field { flex: 1; display: flex; flex-direction: column; gap: 5px; }
  .amount-field label { font-size: 0.78rem; color: var(--text-muted); }
  .optional { font-size: 0.72rem; opacity: 0.7; }

  .hint { color: var(--text-muted); font-size: 0.85rem; margin: 0; }
</style>
