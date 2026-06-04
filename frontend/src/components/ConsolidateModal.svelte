<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { api } from '../lib/api'
  import { downloadBlob, psbtBlob, utxoKey, swallow } from '../lib/utils'
  import type { AddressInfo, ConsolidateResult, DecodedTx, MempoolBlock, UtxoRecord, WalletEntry } from '../lib/types'
  import { displayUnit, mempoolUrl, hwEnabled } from '../stores/settings'
  import { addToast } from '../stores/toasts'
  import QrSignFlow from './QrSignFlow.svelte'
  import Modal from './ui/Modal.svelte'
  import CopyButton from './ui/CopyButton.svelte'

  interface FeeRates { fastestFee: number; halfHourFee: number; hourFee: number }
  let {
    wallet,
    utxos,
    feeRates,
    onClose,
  }: {
    wallet: WalletEntry
    utxos: UtxoRecord[]
    feeRates: FeeRates | null
    onClose: () => void
  } = $props()

  let mempoolBlocks = $state<MempoolBlock[] | null>(null)

  // ── Fee rate ──────────────────────────────────────────────────────────────
  type FeePreset = 'hour' | 'halfhour' | 'fastest' | 'custom'
  let feePreset = $state<FeePreset>('hour')
  let customFeeRate = $state(10)

  let effectiveFeeRate = $derived.by((): number => {
    if (feePreset === 'custom') return Math.max(1, customFeeRate)
    if (!feeRates) return 10
    if (feePreset === 'fastest') return feeRates.fastestFee
    if (feePreset === 'halfhour') return feeRates.halfHourFee
    return feeRates.hourFee
  })

  let rateComparison = $derived.by((): string | null => {
    if (!feeRates || feePreset !== 'custom') return null
    const fastest = feeRates.fastestFee
    const rate = effectiveFeeRate
    if (rate >= fastest * 1.05) return `${(rate / fastest).toFixed(1)}× faster than current 'Next block' rate`
    if (rate < feeRates.hourFee * 0.5) return 'well below current network rates — may take many hours to confirm'
    if (rate < feeRates.hourFee) return 'below current ~1-hour rate'
    return null
  })

  // ── Destination ──────────────────────────────────────────────────────────
  type DestMode = 'auto' | 'choose' | 'wallet' | 'custom'
  let destMode    = $state<DestMode>('auto')
  let customDest  = $state('')
  let chooseIndex = $state(0)
  let addresses   = $state<AddressInfo[]>([])
  let loadingAddrs = $state(true)

  // Other-wallet destination state. We let the user pick another HD wallet
  // in this Corvin instance; the modal then fetches that wallet's next
  // unused address to use as the destination.
  let otherWallets = $state<{ id: string; label: string }[]>([])
  let targetWalletId = $state('')
  let targetWalletAddr = $state('')
  let loadingTarget = $state(false)
  let targetWalletError = $state('')

  let unusedExternal = $derived(
    addresses.filter(a => a.kind === 'external' && !a.used)
      .sort((a, b) => a.index - b.index)
  )
  let unusedExternalTop = $derived(unusedExternal.slice(0, 20))

  let effectiveDestination = $derived.by((): string => {
    if (destMode === 'custom') return customDest.trim()
    if (destMode === 'wallet') return targetWalletAddr
    if (destMode === 'choose') return unusedExternalTop[chooseIndex]?.address ?? ''
    return unusedExternalTop[0]?.address ?? ''
  })

  $effect(() => {
    if (destMode !== 'wallet') return
    if (otherWallets.length > 0) return
    api.wallets.list().then((ws) => {
      otherWallets = ws
        .filter(w => w.id !== wallet.id && w.kind !== 'address')
        .map(w => ({ id: w.id, label: w.label }))
    }).catch(() => {})
  })

  // Version counter so that a rapid wallet-pick → re-pick doesn't end up
  // showing the older fetch's result (whichever promise resolves last).
  let targetWalletReqSeq = 0
  $effect(() => {
    const wid = targetWalletId
    if (!wid) { targetWalletAddr = ''; loadingTarget = false; return }
    const seq = ++targetWalletReqSeq
    loadingTarget = true; targetWalletError = ''
    api.wallets.addresses(wid).then((addrs) => {
      if (seq !== targetWalletReqSeq) return
      const next = addrs
        .filter(a => a.kind === 'external' && !a.used)
        .sort((a, b) => a.index - b.index)[0]
      if (!next) {
        targetWalletError = 'Target wallet has no unused addresses — sync it first.'
        targetWalletAddr = ''
      } else {
        targetWalletAddr = next.address
      }
    }).catch((e) => {
      if (seq !== targetWalletReqSeq) return
      targetWalletError = e instanceof Error ? e.message : 'Failed to load target wallet'
      targetWalletAddr = ''
    }).finally(() => {
      if (seq === targetWalletReqSeq) loadingTarget = false
    })
  })

  // ── Preview ───────────────────────────────────────────────────────────────
  let previewLoading = $state(false)
  let previewError   = $state('')
  let preview        = $state<ConsolidateResult | null>(null)
  let previewTimer: ReturnType<typeof setTimeout> | null = null

  $effect(() => {
    const dest = effectiveDestination
    const rate = effectiveFeeRate

    if (previewTimer) clearTimeout(previewTimer)
    preview = null
    previewError = ''

    if (!dest) return

    previewTimer = setTimeout(async () => {
      previewLoading = true
      try {
        preview = await api.wallets.consolidatePsbt(wallet.id, {
          utxos: utxos.map(u => utxoKey(u.txid, u.vout)),
          fee_rate_sat_vb: rate,
          destination: dest,
        })
        previewError = ''
      } catch (e) {
        previewError = e instanceof Error ? e.message : 'Failed to build transaction'
        preview = null
      } finally {
        previewLoading = false
      }
    }, 400)
  })

  onDestroy(() => {
    if (previewTimer) clearTimeout(previewTimer)
    hwEs?.close()
  })

  onMount(async () => {
    const [addrsResult] = await Promise.allSettled([
      api.wallets.addresses(wallet.id),
      (async () => {
        if ($mempoolUrl) {
          try {
            mempoolBlocks = await api.proxy.mempoolBlocks()
          } catch (e) { swallow(e, 'mempool-blocks') }
        }
      })(),
    ])
    if (addrsResult.status === 'fulfilled') {
      addresses = addrsResult.value
    } else {
      addToast(addrsResult.reason instanceof Error ? addrsResult.reason.message : 'Failed to load addresses')
    }
    loadingAddrs = false
  })

  // ── Export ────────────────────────────────────────────────────────────────
  function exportPsbt() {
    if (!preview?.psbt) return
    const slug = wallet.label.replace(/[^a-z0-9]/gi, '_').toLowerCase()
    downloadBlob(psbtBlob(preview.psbt), `consolidation_${slug}.psbt`)
  }

  // ── Hardware wallet signing ───────────────────────────────────────────────
  type HwStatus = 'idle' | 'connecting' | 'pairing' | 'confirm' | 'signing' | 'signed' | 'error'
  let hwStatus = $state<HwStatus>('idle')
  let hwMessage = $state('')
  let hwPairingCode = $state('')
  let hwEs = $state<EventSource | null>(null)
  let signedPsbt = $state<string | null>(null)
  let broadcasting = $state(false)
  let broadcastTxid = $state('')
  let broadcastError = $state('')

  let qrOpen = $state(false)

  async function handleQrSigned(qrPsbt: string) {
    qrOpen = false
    if (!preview?.psbt) return
    try {
      const combined = await api.wallets.combinePsbt(wallet.id, { psbt_a: preview.psbt, psbt_b: qrPsbt })
      signedPsbt = combined.psbt
      hwStatus = 'signed'
    } catch (e) {
      hwMessage = e instanceof Error ? e.message : 'Failed to combine PSBT'
      hwStatus = 'error'
    }
  }

  async function startHwSign() {
    if (!preview?.psbt) return
    qrOpen = false
    hwStatus = 'connecting'; hwMessage = ''; hwPairingCode = ''
    signedPsbt = null; broadcastTxid = ''; broadcastError = ''
    let token: string
    try {
      ;({ token } = await api.hwi.signStart(preview.psbt, wallet.id))
    } catch (e) {
      hwMessage = e instanceof Error ? e.message : 'Failed to start signing'
      hwStatus = 'error'
      return
    }
    const es = new EventSource(`/api/hwi/sign/${token}`)
    hwEs = es
    es.addEventListener('connecting', () => { hwStatus = 'connecting' })
    es.addEventListener('pairing_code', (e) => { hwPairingCode = JSON.parse(e.data).code; hwStatus = 'pairing' })
    es.addEventListener('waiting_confirm', () => { hwStatus = 'confirm' })
    es.addEventListener('paired', () => { hwPairingCode = ''; hwStatus = 'confirm' })
    es.addEventListener('signing', () => { hwStatus = 'signing' })
    es.addEventListener('signed', (e) => {
      signedPsbt = JSON.parse(e.data).psbt; hwStatus = 'signed'
      es.close(); hwEs = null
    })
    es.addEventListener('hw_error', (e) => {
      hwMessage = JSON.parse(e.data).message; hwStatus = 'error'
      es.close(); hwEs = null
    })
    es.onerror = () => {
      if (hwStatus !== 'signed' && hwStatus !== 'error') { hwMessage = 'Connection lost.'; hwStatus = 'error' }
      es.close(); hwEs = null
    }
  }

  function cancelHwSign() {
    hwEs?.close(); hwEs = null
    hwStatus = 'idle'; hwMessage = ''; hwPairingCode = ''
  }

  async function broadcastSigned() {
    if (!signedPsbt) return
    broadcasting = true; broadcastError = ''
    try {
      const result = await api.broadcast.broadcast({ psbt: signedPsbt, wallet_id: wallet.id })
      broadcastTxid = result.txid
    } catch (e) {
      broadcastError = e instanceof Error ? e.message : 'Broadcast failed'
    } finally {
      broadcasting = false
    }
  }

  // ── Pre-broadcast verification ────────────────────────────────────────────
  let verifyDecoded = $state<DecodedTx | null>(null)
  let verifyLoading = $state(false)
  let verifyError = $state('')

  let recipientReady = $derived(!broadcastTxid && hwStatus === 'signed' && !!signedPsbt)

  /// Unified Transaction panel phase (same model as SendModal / FeeBumpModal).
  type TxPhase = 'compose' | 'sign' | 'verify' | 'done'
  let txPhase = $derived<TxPhase>(
    broadcastTxid ? 'done'
    : recipientReady ? 'verify'
    : (qrOpen || (hwStatus !== 'idle' && hwStatus !== 'error')) ? 'sign'
    : 'compose'
  )

  $effect(() => {
    if (!recipientReady) { verifyDecoded = null; verifyError = ''; return }
    if (!signedPsbt) return
    verifyLoading = true; verifyError = ''
    api.broadcast.decode({ psbt: signedPsbt })
      .then((d) => { verifyDecoded = d })
      .catch((e) => { verifyError = e instanceof Error ? e.message : 'Failed to decode signed transaction' })
      .finally(() => { verifyLoading = false })
  })

  let verifySummary = $derived.by(() => {
    if (!verifyDecoded) return null
    const target = effectiveDestination
    const destOut = verifyDecoded.outputs.find((o) => o.address === target)
    const otherOuts = verifyDecoded.outputs.filter((o) => o.address !== target)
    return { destOut, otherOuts, mismatch: !destOut }
  })

  function formatSats(sats: number): string {
    if ($displayUnit === 'btc') return (sats / 1e8).toFixed(8) + ' BTC'
    return sats.toLocaleString() + ' sats'
  }

  let totalInputSats = $derived(utxos.reduce((s, u) => s + u.amount_sats, 0))
</script>

<Modal open onclose={onClose} title="Consolidate UTXOs" width="520px"
  desc={`${utxos.length} inputs · ${formatSats(totalInputSats)}`}>
    <!-- Fee rate -->
    <section class="config-section">
      <h3 class="section-label">Fee rate</h3>
      <div class="toggle-group">
        <button class="toggle-btn" class:active={feePreset === 'hour'} onclick={() => feePreset = 'hour'}>
          1 hr
          {#if feeRates}<br><span class="toggle-sub">{feeRates.hourFee} sat/vB</span>{/if}
          {#if mempoolBlocks && mempoolBlocks.length > 0}
            {@const b = mempoolBlocks[Math.min(5, mempoolBlocks.length - 1)]}
            <span class="depth-track"><span class="depth-fill" style="width: {Math.min(100, b.blockVSize / 10_000).toFixed(0)}%"></span></span>
            <span class="depth-mb">{(b.blockVSize / 1e6).toFixed(1)} MB</span>
          {/if}
        </button>
        <button class="toggle-btn" class:active={feePreset === 'halfhour'} onclick={() => feePreset = 'halfhour'}>
          30 min
          {#if feeRates}<br><span class="toggle-sub">{feeRates.halfHourFee} sat/vB</span>{/if}
          {#if mempoolBlocks && mempoolBlocks.length > 0}
            {@const b = mempoolBlocks[Math.min(2, mempoolBlocks.length - 1)]}
            <span class="depth-track"><span class="depth-fill" style="width: {Math.min(100, b.blockVSize / 10_000).toFixed(0)}%"></span></span>
            <span class="depth-mb">{(b.blockVSize / 1e6).toFixed(1)} MB</span>
          {/if}
        </button>
        <button class="toggle-btn" class:active={feePreset === 'fastest'} onclick={() => feePreset = 'fastest'}>
          Next block
          {#if feeRates}<br><span class="toggle-sub">{feeRates.fastestFee} sat/vB</span>{/if}
          {#if mempoolBlocks && mempoolBlocks.length > 0}
            {@const b = mempoolBlocks[0]}
            <span class="depth-track"><span class="depth-fill" style="width: {Math.min(100, b.blockVSize / 10_000).toFixed(0)}%"></span></span>
            <span class="depth-mb">{(b.blockVSize / 1e6).toFixed(1)} MB</span>
          {/if}
        </button>
        <button class="toggle-btn" class:active={feePreset === 'custom'} onclick={() => feePreset = 'custom'}>
          Custom
        </button>
      </div>
      {#if feePreset === 'custom'}
        <div class="custom-fee-row">
          <input
            class="custom-fee-input"
            type="number"
            min="1"
            step="1"
            bind:value={customFeeRate}
            aria-label="Custom fee rate"
          />
          <span class="custom-fee-unit">sat/vB</span>
        </div>
        {#if rateComparison}
          <p class="rate-compare">{rateComparison}</p>
        {/if}
      {/if}
    </section>

    <!-- Destination -->
    <section class="config-section">
      <h3 class="section-label">Destination</h3>
      <div class="toggle-group">
        <button
          class="toggle-btn"
          class:active={destMode === 'auto'}
          onclick={() => destMode = 'auto'}
        >
          Next unused
        </button>
        {#if unusedExternal.length > 1}
          <button
            class="toggle-btn"
            class:active={destMode === 'choose'}
            onclick={() => destMode = 'choose'}
          >
            Choose
          </button>
        {/if}
        <button
          class="toggle-btn"
          class:active={destMode === 'wallet'}
          onclick={() => destMode = 'wallet'}
          title="Send the consolidated UTXO to another Corvin wallet (e.g. cold storage)"
        >
          Another wallet
        </button>
        <button
          class="toggle-btn"
          class:active={destMode === 'custom'}
          onclick={() => destMode = 'custom'}
        >
          Custom
        </button>
      </div>

      {#if destMode === 'auto'}
        {#if loadingAddrs}
          <p class="dest-hint">Loading…</p>
        {:else if unusedExternal.length > 0}
          <div class="dest-preview">
            <span class="dest-addr">{unusedExternal[0].address}</span>
            <span class="dest-meta">external · index {unusedExternal[0].index}</span>
          </div>
        {:else}
          <p class="dest-hint warn">No unused addresses — sync your wallet first.</p>
        {/if}
      {:else if destMode === 'choose'}
        {#if unusedExternalTop.length > 0}
          <select class="dest-select" bind:value={chooseIndex}>
            {#each unusedExternalTop as addr, i (addr.address)}
              <option value={i}>{addr.address} · index {addr.index}</option>
            {/each}
          </select>
          {#if unusedExternal.length > unusedExternalTop.length}
            <p class="dest-hint">Showing first {unusedExternalTop.length} of {unusedExternal.length} unused.</p>
          {/if}
        {:else}
          <p class="dest-hint warn">No unused addresses available. Either sync the wallet to reveal more, or use Custom.</p>
        {/if}
      {:else if destMode === 'wallet'}
        {#if otherWallets.length === 0}
          <p class="dest-hint">No other HD wallets in this Corvin to send to.</p>
        {:else}
          <select class="dest-select" bind:value={targetWalletId}>
            <option value="">— pick a target wallet —</option>
            {#each otherWallets as w (w.id)}
              <option value={w.id}>{w.label}</option>
            {/each}
          </select>
          {#if loadingTarget}
            <p class="dest-hint">Resolving target address…</p>
          {:else if targetWalletError}
            <p class="dest-hint warn">{targetWalletError}</p>
          {:else if targetWalletAddr}
            <div class="dest-preview">
              <span class="dest-addr">{targetWalletAddr}</span>
              <span class="dest-meta">next unused address in target wallet</span>
            </div>
          {/if}
        {/if}
      {:else}
        <input
          class="dest-input"
          type="text"
          placeholder="Enter Bitcoin address…"
          bind:value={customDest}
        />
      {/if}
    </section>

    <!-- Unified Transaction panel — same shape as SendModal / FeeBumpModal. -->
    <section class="config-section tx-section">
      <div
        class="tx-panel"
        class:phase-compose={txPhase === 'compose'}
        class:phase-sign={txPhase === 'sign'}
        class:phase-verify={txPhase === 'verify' && !verifySummary?.mismatch}
        class:phase-mismatch={txPhase === 'verify' && !!verifySummary?.mismatch}
        class:phase-done={txPhase === 'done'}
      >
        <div class="tx-panel-header">
          <span class="tx-phase-chip">
            {#if txPhase === 'compose'}Transaction preview
            {:else if txPhase === 'sign'}Transaction · signing…
            {:else if txPhase === 'verify' && verifySummary?.mismatch}⚠ Destination mismatch
            {:else if txPhase === 'verify'}Verify before broadcasting
            {:else if txPhase === 'done'}✓ Transaction broadcast
            {/if}
          </span>
        </div>

        {#if txPhase === 'compose' || txPhase === 'sign'}
          {#if previewLoading}
            <div class="tx-status">Calculating…</div>
          {:else if previewError && txPhase === 'compose'}
            <div class="tx-status tx-error-text">{previewError}</div>
          {:else if !preview && !effectiveDestination}
            <p class="tx-placeholder">Enter a destination address to preview.</p>
          {:else if preview}
            <div class="tx-body">
              <div class="preview-recipient">
                <span class="preview-recipient-label">To</span>
                <span class="preview-recipient-addr mono">{effectiveDestination}</span>
              </div>
              <div class="preview-divider"></div>
              <div class="preview-row">
                <span>{utxos.length} inputs</span>
                <span>{formatSats(preview.input_sats)}</span>
              </div>
              <div class="preview-row fee-row">
                <span>Network fee</span>
                <span>− {formatSats(preview.fee_sats)}</span>
              </div>
              <div class="preview-divider"></div>
              <div class="preview-row total-row">
                <span>You receive</span>
                <span>{formatSats(preview.output_sats)}</span>
              </div>
            </div>
          {/if}
        {:else if txPhase === 'verify'}
          {#if verifyLoading}
            <div class="tx-status">Decoding signed transaction…</div>
          {:else if verifyError}
            <div class="tx-status tx-error-text">{verifyError}</div>
          {:else if verifyDecoded && verifySummary}
            {#if verifySummary.mismatch}
              <div class="tx-body verify-mismatch-body">
                <div class="verify-mismatch-row">
                  <span class="verify-mismatch-label">You picked:</span>
                  <span class="mono verify-mismatch-addr">{effectiveDestination}</span>
                </div>
                <div class="verify-mismatch-row">
                  <span class="verify-mismatch-label">Signed tx sends to:</span>
                  {#each verifyDecoded.outputs as o, i (i)}
                    {#if o.address}
                      <span class="mono verify-mismatch-addr">{o.address}</span>
                    {/if}
                  {/each}
                </div>
              </div>
            {:else}
              <div class="tx-body">
                {#if verifySummary.destOut}
                  <div class="verify-row">
                    <span class="verify-label">Consolidating to</span>
                    <span class="verify-value">{formatSats(verifySummary.destOut.value_sat)}</span>
                  </div>
                  <div class="verify-row verify-row-addr">
                    <span class="verify-label">Address</span>
                    <span class="mono verify-addr">{verifySummary.destOut.address}</span>
                  </div>
                {/if}
                {#if verifyDecoded.fee_sat != null}
                  <div class="verify-row">
                    <span class="verify-label">Fee</span>
                    <span class="verify-value">{formatSats(verifyDecoded.fee_sat)}{#if verifyDecoded.fee_rate_sat_vb} <span class="verify-feerate">({verifyDecoded.fee_rate_sat_vb.toFixed(1)} sat/vB)</span>{/if}</span>
                  </div>
                {/if}
                <div class="verify-row verify-row-txid">
                  <span class="verify-label">Txid</span>
                  <span class="mono verify-txid">{verifyDecoded.txid}</span>
                </div>
              </div>
            {/if}
          {/if}
        {:else if txPhase === 'done'}
          <div class="tx-body tx-broadcast-body">
            <div class="tx-broadcast-label">Txid</div>
            <div class="tx-broadcast-txid mono">{broadcastTxid}</div>
            {#if $mempoolUrl}
              <a class="tx-broadcast-link" href={new URL('/tx/' + broadcastTxid, $mempoolUrl).href} target="_blank" rel="noopener noreferrer">View on mempool ↗</a>
            {/if}
          </div>
        {/if}
      </div>
    </section>

    <!-- Phase-driven action footer. -->
    <div class="modal-footer">
      {#if qrOpen}
        <QrSignFlow psbt={preview!.psbt} onSigned={handleQrSigned} onCancel={() => qrOpen = false} />
      {:else if txPhase === 'compose'}
        {#if $hwEnabled}<button class="hw-sign-btn" onclick={startHwSign} disabled={!preview}>⚿ Hardware wallet</button>{/if}
        <button class="qr-sign-btn" onclick={() => qrOpen = true} disabled={!preview}>⬡ QR code</button>
        <button class="export-btn" onclick={exportPsbt} disabled={!preview}>↓ Export PSBT</button>
        <CopyButton class="copy-psbt-btn" idle="⎘ Copy PSBT" copiedLabel="✓ Copied" text={() => preview?.psbt ?? ''} disabled={!preview} />
        {#if hwStatus === 'error'}
          <p class="hw-error">{hwMessage}</p>
        {:else}
          <p class="footer-hint">Sign this PSBT externally, then broadcast.</p>
        {/if}
      {:else if txPhase === 'sign'}
        <div class="hw-active">
          <div class="hw-status-msg">
            {#if hwStatus === 'connecting'}Connecting to device…
            {:else if hwStatus === 'pairing'}Verify pairing code: <code class="hw-code">{hwPairingCode}</code>
            {:else if hwStatus === 'confirm' || hwStatus === 'signing'}Confirm transaction on device…
            {/if}
          </div>
          {#if hwStatus !== 'signed'}
            <button class="btn-cancel-hw" onclick={cancelHwSign}>Cancel</button>
          {/if}
        </div>
        {#if broadcastError}<p class="hw-error">{broadcastError}</p>{/if}
      {:else if txPhase === 'verify' && !verifySummary?.mismatch && verifyDecoded}
        <button class="btn-broadcast btn-broadcast-confirm" onclick={broadcastSigned} disabled={broadcasting}>
          {broadcasting ? 'Broadcasting…' : 'Confirm and broadcast'}
        </button>
        {#if broadcastError}<p class="hw-error">{broadcastError}</p>{/if}
      {/if}
    </div>
</Modal>

<style>
  /* Sections */
  .config-section {
    display: flex; flex-direction: column; gap: 12px;
  }
  .section-label {
    margin: 0 0 10px;
    font-size: 0.72rem; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.07em; color: var(--text-muted);
  }

  /* Toggle button groups */
  .toggle-group {
    display: flex; gap: 6px; width: 100%;
  }
  .toggle-btn {
    flex: 1;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 6px; padding: 6px 4px;
    font-size: 0.78rem; color: var(--text-muted); cursor: pointer;
    text-align: center; line-height: 1.3; transition: all 0.12s;
  }
  .toggle-btn:hover { border-color: var(--accent); color: var(--text); }
  .toggle-btn.active {
    border-color: var(--accent); color: var(--text);
    background: color-mix(in srgb, var(--accent) 10%, var(--surface-2));
  }
  .toggle-sub { font-size: 0.7rem; color: var(--text-muted); }
  .toggle-btn.active .toggle-sub { color: var(--accent); }
  .depth-track {
    display: block; width: 100%; height: 2px;
    background: rgba(255,255,255,0.07); border-radius: 1px;
    margin-top: 5px; overflow: hidden;
  }
  .depth-fill {
    display: block; height: 100%;
    background: var(--accent); border-radius: 1px; opacity: 0.5;
  }
  .depth-mb {
    display: block; font-size: 0.6rem; color: var(--text-muted); margin-top: 2px;
  }

  /* Fee */
  .custom-fee-row {
    display: flex; align-items: center; gap: 8px; margin-top: 10px;
  }
  .custom-fee-input {
    background: var(--surface-2); border: 1px solid var(--accent);
    border-radius: 4px; color: var(--text); padding: 5px 8px;
    font-size: 0.85rem; width: 80px; outline: none;
    font-variant-numeric: tabular-nums;
  }
  .custom-fee-unit { font-size: 0.78rem; color: var(--text-muted); }
  .rate-compare { margin: 6px 0 0; font-size: 0.74rem; color: var(--text-muted); font-style: italic; }

  /* Destination */
  .dest-preview {
    margin-top: 8px; display: flex; flex-direction: column; gap: 2px;
  }
  .dest-addr {
    font-family: monospace; font-size: 0.78rem; color: var(--text);
    word-break: break-all;
  }
  .dest-meta { font-size: 0.7rem; color: var(--text-muted); }
  .dest-hint { margin: 8px 0 0; font-size: 0.78rem; color: var(--text-muted); }
  .dest-hint.warn { color: #e09c52; }
  .dest-select {
    margin-top: 8px; width: 100%;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 4px; color: var(--text); padding: 6px 8px;
    font-size: 0.8rem; font-family: monospace; outline: none;
  }
  .dest-select:focus { border-color: var(--accent); }
  .dest-input {
    margin-top: 8px; width: 100%; box-sizing: border-box;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 4px; color: var(--text); padding: 6px 10px;
    font-size: 0.82rem; font-family: monospace; outline: none;
  }
  .dest-input:focus { border-color: var(--accent); }

  /* ── Unified Transaction panel — same shape as SendModal / FeeBumpModal. ── */
  .tx-section { flex: 1; }
  .tx-panel {
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 6px; padding: 12px 14px;
    display: flex; flex-direction: column; gap: 8px;
    transition: border-color 0.15s, background 0.15s, opacity 0.15s;
  }
  .tx-panel.phase-sign { opacity: 0.78; }
  .tx-panel.phase-verify {
    border-color: color-mix(in srgb, var(--accent) 60%, var(--border));
    background: color-mix(in srgb, var(--accent) 4%, var(--surface-2));
  }
  .tx-panel.phase-mismatch {
    border-color: #e05252;
    background: color-mix(in srgb, #e05252 10%, var(--surface-2));
  }
  .tx-panel.phase-done {
    border-color: color-mix(in srgb, #52a875 50%, var(--border));
    background: color-mix(in srgb, #52a875 6%, var(--surface-2));
  }
  .tx-panel-header { margin-bottom: 2px; }
  .tx-phase-chip {
    font-size: 0.72rem; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.07em; color: var(--text-muted);
  }
  .tx-panel.phase-verify .tx-phase-chip { color: var(--accent); }
  .tx-panel.phase-mismatch .tx-phase-chip { color: #e05252; letter-spacing: 0.04em; }
  .tx-panel.phase-done .tx-phase-chip { color: #52a875; }

  .tx-status { font-size: 0.8rem; color: var(--text-muted); padding: 4px 0; }
  .tx-status.tx-error-text { color: #e05252; line-height: 1.5; }
  .tx-placeholder { font-size: 0.8rem; color: var(--text-muted); padding: 4px 0; margin: 0; }
  .tx-body { display: flex; flex-direction: column; gap: 8px; }

  .preview-recipient { display: flex; flex-direction: column; gap: 4px; }
  .preview-recipient-label { font-size: 0.72rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.07em; }
  .preview-recipient-addr {
    font-size: 0.78rem; color: var(--text);
    word-break: break-all; line-height: 1.4;
  }
  .preview-row {
    display: flex; justify-content: space-between; align-items: center;
    font-size: 0.82rem; color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .fee-row { color: #e05252; }
  .preview-divider { height: 1px; background: var(--border); }
  .total-row { font-weight: 700; font-size: 0.88rem; color: var(--text); }

  .verify-row {
    display: flex; justify-content: space-between; align-items: baseline;
    gap: 12px; font-size: 0.82rem; color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .verify-row-addr, .verify-row-txid { flex-direction: column; align-items: stretch; gap: 3px; }
  .verify-label { font-size: 0.72rem; color: var(--text-muted); }
  .verify-value { font-weight: 600; }
  .verify-feerate { color: var(--text-muted); font-weight: 400; font-size: 0.78rem; }
  .verify-addr {
    font-size: 0.85rem; color: var(--text); word-break: break-all; line-height: 1.4;
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    padding: 6px 8px; border-radius: 4px;
  }
  .verify-txid {
    font-size: 0.72rem; color: var(--text-muted); word-break: break-all;
  }
  .btn-broadcast-confirm { flex: 1; padding: 10px 18px; font-size: 0.88rem; }

  .verify-mismatch-body { display: flex; flex-direction: column; gap: 8px; }
  .verify-mismatch-row { display: flex; flex-direction: column; gap: 3px; }
  .verify-mismatch-label { font-size: 0.72rem; color: var(--text-muted); }
  .verify-mismatch-addr { font-size: 0.78rem; color: var(--text); word-break: break-all; line-height: 1.4; }

  .tx-broadcast-body { display: flex; flex-direction: column; gap: 4px; }
  .tx-broadcast-label { font-size: 0.72rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.07em; }
  .tx-broadcast-txid { font-size: 0.78rem; color: var(--text); word-break: break-all; }
  .tx-broadcast-link { font-size: 0.78rem; color: var(--accent); text-decoration: none; }
  .tx-broadcast-link:hover { text-decoration: underline; }

  /* Footer */
  .modal-footer {
    padding-top: 14px; margin-top: 4px;
    display: flex; flex-wrap: wrap; align-items: center; gap: 8px;
    border-top: 1px solid var(--border);
  }
  .export-btn {
    background: none; border: 1px solid var(--border); border-radius: 5px;
    color: var(--text-muted); cursor: pointer; font-size: 0.82rem; padding: 7px 10px;
  }
  .export-btn:hover:not(:disabled) { border-color: var(--text-muted); color: var(--text); }
  .export-btn:disabled { opacity: 0.35; cursor: not-allowed; }
  /* :global so the rule reaches the CopyButton child's <button> (scoped CSS
     doesn't cross component boundaries). */
  :global(.copy-psbt-btn) {
    background: none; border: 1px solid var(--border); border-radius: 5px;
    color: var(--text-muted); cursor: pointer; font-size: 0.82rem; padding: 7px 10px;
  }
  :global(.copy-psbt-btn:hover:not(:disabled)) { border-color: var(--text-muted); color: var(--text); }
  :global(.copy-psbt-btn:disabled) { opacity: 0.35; cursor: not-allowed; }
  .footer-hint {
    width: 100%; margin: 2px 0 0;
    font-size: 0.7rem; color: var(--text-muted); line-height: 1.4;
  }
  /* Hardware = primary (accent fill); QR = secondary accent outline. Same
     hierarchy as SendModal / FeeBumpModal. */
  .hw-sign-btn {
    background: var(--accent); color: #000; border: none; border-radius: 5px;
    padding: 7px 12px; cursor: pointer; font-weight: 600; font-size: 0.82rem;
  }
  .hw-sign-btn:hover:not(:disabled) { opacity: 0.88; }
  .hw-sign-btn:disabled { opacity: 0.35; cursor: not-allowed; }
  .qr-sign-btn {
    background: none; border: 1px solid var(--accent); border-radius: 5px;
    color: var(--accent); cursor: pointer; font-size: 0.82rem; font-weight: 600; padding: 7px 10px;
  }
  .qr-sign-btn:hover:not(:disabled) { opacity: 0.78; }
  .qr-sign-btn:disabled { opacity: 0.35; cursor: not-allowed; }

  .hw-active {
    width: 100%; display: flex; align-items: center; gap: 10px; flex-wrap: wrap;
  }
  .hw-status-msg { flex: 1; font-size: 0.82rem; color: var(--text-muted); }
  .hw-code {
    font-family: monospace; font-size: 0.85rem; color: var(--text);
    background: var(--surface-2); padding: 1px 6px; border-radius: 3px;
  }
  .btn-broadcast {
    background: var(--accent); color: #000; border: none; border-radius: 5px;
    padding: 8px 18px; cursor: pointer; font-weight: 600; font-size: 0.82rem;
    white-space: nowrap;
  }
  .btn-broadcast:hover:not(:disabled) { filter: brightness(1.08); }
  .btn-broadcast:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-cancel-hw {
    background: none; border: 1px solid var(--border); border-radius: 5px;
    color: var(--text-muted); cursor: pointer; font-size: 0.82rem; padding: 7px 12px;
    white-space: nowrap;
  }
  .btn-cancel-hw:hover { border-color: var(--text-muted); color: var(--text); }
  .hw-error { width: 100%; margin: 0; font-size: 0.75rem; color: var(--error); }

  .mono { font-family: monospace; }

  @media (max-width: 768px) {
    .toggle-btn { min-width: 0; }
  }
</style>
