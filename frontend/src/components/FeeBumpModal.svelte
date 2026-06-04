<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import Modal from './ui/Modal.svelte'
  import { api } from '../lib/api'
  import { downloadBlob, psbtBlob, swallow } from '../lib/utils'
  import type { DecodedTx, FeeBumpResult, MempoolBlock, TxRecord } from '../lib/types'
  import { mempoolUrl, displayUnit, hwEnabled } from '../stores/settings'
  import { addToast } from '../stores/toasts'
  import QrSignFlow from './QrSignFlow.svelte'
  import CopyButton from './ui/CopyButton.svelte'

  interface FeeRates { fastestFee: number; halfHourFee: number; hourFee: number }
  let {
    walletId,
    tx,
    feeRates,
    mode,
    onClose,
  }: {
    walletId: string
    tx: TxRecord
    feeRates: FeeRates | null
    mode: 'rbf' | 'cpfp'
    onClose: () => void
  } = $props()

  let mempoolBlocks = $state<MempoolBlock[] | null>(null)

  onMount(async () => {
    if ($mempoolUrl) {
      try {
        mempoolBlocks = await api.proxy.mempoolBlocks()
      } catch (e) { swallow(e, 'mempool-blocks') }
    }
  })

  let currentFeeRate = $derived(
    tx.fee_sats != null && tx.vsize != null && tx.vsize > 0
      ? Math.ceil(tx.fee_sats / tx.vsize)
      : null
  )

  let minFeeRate = $derived(
    mode === 'rbf' && currentFeeRate != null ? currentFeeRate + 1 : 1
  )

  // ── Fee rate selection ──────────────────────────────────────────────────────
  type FeePreset = 'hour' | 'halfhour' | 'fastest' | 'custom'
  let feePreset = $state<FeePreset>('fastest')
  let customFeeRate = $state(10)

  let effectiveFeeRate = $derived.by((): number => {
    if (feePreset === 'custom') return Math.max(minFeeRate, customFeeRate)
    if (!feeRates) return Math.max(minFeeRate, 10)
    if (feePreset === 'fastest') return Math.max(minFeeRate, feeRates.fastestFee)
    if (feePreset === 'halfhour') return Math.max(minFeeRate, feeRates.halfHourFee)
    return Math.max(minFeeRate, feeRates.hourFee)
  })

  // ── Build preview ───────────────────────────────────────────────────────────
  let building = $state(false)
  let buildError = $state('')
  let result = $state<FeeBumpResult | null>(null)
  let buildTimer: ReturnType<typeof setTimeout> | null = null

  // Decode the unsigned PSBT each build so we can show outputs in the preview —
  // critical for RBF because the user needs to verify the same recipient is
  // still in the bumped tx.
  let previewDecoded = $state<DecodedTx | null>(null)

  $effect(() => {
    const rate = effectiveFeeRate
    if (buildTimer) clearTimeout(buildTimer)
    result = null; buildError = ''; previewDecoded = null
    buildTimer = setTimeout(async () => {
      building = true
      try {
        const body = { txid: tx.txid, fee_rate_sat_vb: rate }
        result = mode === 'rbf'
          ? await api.wallets.rbfPsbt(walletId, body)
          : await api.wallets.cpfpPsbt(walletId, body)
        buildError = ''
        // Fire-and-forget decode of the new PSBT. Compare the captured psbt
        // back to the current result so a slow-resolving decode for a previous
        // fee rate doesn't overwrite a fresh result with stale data.
        if (result?.psbt) {
          const expectedPsbt = result.psbt
          api.broadcast.decode({ psbt: expectedPsbt })
            .then((d) => { if (result?.psbt === expectedPsbt) previewDecoded = d })
            .catch(() => { if (result?.psbt === expectedPsbt) previewDecoded = null })
        }
      } catch (e) {
        buildError = e instanceof Error ? e.message : 'Failed to build transaction'
        result = null
      } finally {
        building = false
      }
    }, 400)
  })

  // Comparison to current network rate: "1.2× fastest" or "below recommended".
  let rateComparison = $derived.by((): string | null => {
    if (!feeRates) return null
    const fastest = feeRates.fastestFee
    const rate = effectiveFeeRate
    if (rate >= fastest * 1.05) return `${(rate / fastest).toFixed(1)}× faster than 'Next block' rate`
    if (rate >= fastest * 0.95) return 'matches current Next-block rate'
    if (rate >= feeRates.halfHourFee * 0.95) return 'matches current ~30-min rate'
    if (rate >= feeRates.hourFee * 0.95) return 'matches current ~1-hour rate'
    return 'below current network rates — may be slow to confirm'
  })

  onDestroy(() => {
    if (buildTimer) clearTimeout(buildTimer)
    if (hwStuckTimer) clearTimeout(hwStuckTimer)
    hwEs?.close()
  })

  // ── Export / copy ───────────────────────────────────────────────────────────
  function exportPsbt() {
    if (!result?.psbt) return
    const fileLabel = mode === 'rbf' ? 'rbf' : 'cpfp'
    downloadBlob(psbtBlob(result.psbt), `${fileLabel}_${tx.txid.slice(0, 8)}.psbt`)
  }

  // ── Hardware wallet signing ─────────────────────────────────────────────────
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
    if (!result?.psbt) return
    try {
      const combined = await api.wallets.combinePsbt(walletId, { psbt_a: result.psbt, psbt_b: qrPsbt })
      signedPsbt = combined.psbt
      hwStatus = 'signed'
    } catch (e) {
      hwMessage = e instanceof Error ? e.message : 'Failed to combine PSBT'
      hwStatus = 'error'
    }
  }

  async function startHwSign() {
    if (!result?.psbt) return
    qrOpen = false
    hwStatus = 'connecting'; hwMessage = ''; hwPairingCode = ''
    signedPsbt = null; broadcastTxid = ''; broadcastError = ''
    let token: string
    try {
      ;({ token } = await api.hwi.signStart(result.psbt, walletId))
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
      const r = await api.broadcast.broadcast({ psbt: signedPsbt, wallet_id: walletId })
      broadcastTxid = r.txid
    } catch (e) {
      broadcastError = e instanceof Error ? e.message : 'Broadcast failed'
    } finally {
      broadcasting = false
    }
  }

  // ── Pre-broadcast verification ────────────────────────────────────────────
  // Decode the signed PSBT before broadcast so the user can confirm the
  // outputs (recipient, amounts, fee) match what they intended. Catches PSBT
  // tampering between signing and broadcast.
  let verifyDecoded = $state<DecodedTx | null>(null)
  let verifyLoading = $state(false)
  let verifyError = $state('')

  let recipientReady = $derived(!broadcastTxid && hwStatus === 'signed' && !!signedPsbt)

  /// Phase the unified Transaction panel is currently in. Same shape as
  /// SendModal — compose → sign → verify → done. RBF/CPFP doesn't have a
  /// recipient-mismatch case (no user-entered recipient to compare against),
  /// so verify here is just "look it over before broadcasting."
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

  function formatSats(sats: number): string {
    if ($displayUnit === 'btc') return (sats / 1e8).toFixed(8) + ' BTC'
    return sats.toLocaleString() + ' sats'
  }

  // ── HW "are you sure your device is plugged in?" hint ─────────────────────
  let hwStuckHint = $state(false)
  let hwStuckTimer: ReturnType<typeof setTimeout> | null = null
  $effect(() => {
    if (hwStuckTimer) { clearTimeout(hwStuckTimer); hwStuckTimer = null }
    hwStuckHint = false
    if (hwStatus === 'connecting') {
      hwStuckTimer = setTimeout(() => { hwStuckHint = true }, 5000)
    }
  })

  let title = $derived(mode === 'rbf' ? 'Replace-by-Fee (RBF)' : 'Child-Pays-for-Parent (CPFP)')
  let subtitle = $derived(mode === 'rbf'
    ? 'Replace the unconfirmed transaction with a higher-fee version'
    : 'Create a child transaction that spends your unconfirmed outputs at a high fee, dragging the parent into the next block.')
  let feeRateHint = $derived(mode === 'rbf'
    ? null
    : 'CPFP fee rate applies to the new child only — miners care about the effective rate of the parent+child package.')
</script>

<Modal open onclose={onClose} title={title} desc={subtitle} width="480px">
    <!-- Current fee info -->
    <section class="config-section">
      <h3 class="section-label">Current transaction</h3>
      <dl class="info-grid">
        <div class="info-row">
          <dt>Transaction ID</dt>
          <dd class="mono truncate">{tx.txid.slice(0, 20)}…{tx.txid.slice(-12)}</dd>
        </div>
        {#if currentFeeRate != null}
          <div class="info-row">
            <dt>Current fee rate</dt>
            <dd>{currentFeeRate} sat/vB</dd>
          </div>
        {/if}
        {#if tx.fee_sats != null}
          <div class="info-row">
            <dt>Current fee</dt>
            <dd>{tx.fee_sats.toLocaleString()} sats</dd>
          </div>
        {/if}
      </dl>
    </section>

    <!-- New fee rate -->
    <section class="config-section">
      <h3 class="section-label">New fee rate</h3>
      <div class="toggle-group">
        <button class="toggle-btn" class:active={feePreset === 'hour'} onclick={() => feePreset = 'hour'}>
          <div>~1 hour</div>
          {#if feeRates}<div class="toggle-sub">{Math.max(minFeeRate, feeRates.hourFee)} sat/vB</div>{/if}
          {#if mempoolBlocks && mempoolBlocks.length > 0}
            {@const b = mempoolBlocks[Math.min(5, mempoolBlocks.length - 1)]}
            <span class="depth-track"><span class="depth-fill" style="width: {Math.min(100, b.blockVSize / 10_000).toFixed(0)}%"></span></span>
            <span class="depth-mb">{(b.blockVSize / 1e6).toFixed(1)} MB</span>
          {/if}
        </button>
        <button class="toggle-btn" class:active={feePreset === 'halfhour'} onclick={() => feePreset = 'halfhour'}>
          <div>~30 min</div>
          {#if feeRates}<div class="toggle-sub">{Math.max(minFeeRate, feeRates.halfHourFee)} sat/vB</div>{/if}
          {#if mempoolBlocks && mempoolBlocks.length > 0}
            {@const b = mempoolBlocks[Math.min(2, mempoolBlocks.length - 1)]}
            <span class="depth-track"><span class="depth-fill" style="width: {Math.min(100, b.blockVSize / 10_000).toFixed(0)}%"></span></span>
            <span class="depth-mb">{(b.blockVSize / 1e6).toFixed(1)} MB</span>
          {/if}
        </button>
        <button class="toggle-btn" class:active={feePreset === 'fastest'} onclick={() => feePreset = 'fastest'}>
          <div>Next block</div>
          {#if feeRates}<div class="toggle-sub">{Math.max(minFeeRate, feeRates.fastestFee)} sat/vB</div>{/if}
          {#if mempoolBlocks && mempoolBlocks.length > 0}
            {@const b = mempoolBlocks[0]}
            <span class="depth-track"><span class="depth-fill" style="width: {Math.min(100, b.blockVSize / 10_000).toFixed(0)}%"></span></span>
            <span class="depth-mb">{(b.blockVSize / 1e6).toFixed(1)} MB</span>
          {/if}
        </button>
        <button class="toggle-btn" class:active={feePreset === 'custom'} onclick={() => feePreset = 'custom'}>
          <div>Custom</div>
          <div class="toggle-sub">manual</div>
        </button>
      </div>
      {#if feeRateHint}
        <p class="fee-rate-hint">{feeRateHint}</p>
      {/if}
      {#if feePreset === 'custom'}
        <div class="custom-fee-row">
          <input
            type="number"
            class="custom-fee-input"
            min={minFeeRate}
            step="1"
            bind:value={customFeeRate}
          />
          <span class="custom-fee-unit">sat/vB</span>
          {#if minFeeRate > 1}
            <span class="min-hint">min {minFeeRate}</span>
          {/if}
        </div>
      {/if}
    </section>

    <!-- Unified Transaction panel — single source of truth across phases. -->
    <section class="config-section tx-section">
      <div
        class="tx-panel"
        class:phase-compose={txPhase === 'compose'}
        class:phase-sign={txPhase === 'sign'}
        class:phase-verify={txPhase === 'verify'}
        class:phase-done={txPhase === 'done'}
      >
        <div class="tx-panel-header">
          <span class="tx-phase-chip">
            {#if txPhase === 'compose'}Fee preview
            {:else if txPhase === 'sign'}Transaction · signing…
            {:else if txPhase === 'verify'}Verify before broadcasting
            {:else if txPhase === 'done'}✓ Transaction broadcast
            {/if}
          </span>
        </div>

        {#if txPhase === 'compose' || txPhase === 'sign'}
          {#if building}
            <p class="tx-status">Building transaction…</p>
          {:else if buildError && txPhase === 'compose'}
            <p class="tx-status tx-error-text">{buildError}</p>
          {:else if result}
            <div class="tx-body">
              {#if previewDecoded}
                {#each previewDecoded.outputs as o, i (i)}
                  {#if o.address}
                    <div class="preview-row out-row">
                      <span class="out-label">{mode === 'rbf' ? 'Output' : 'Sends to'}</span>
                      <span class="out-addr mono">{o.address}</span>
                      <span class="out-amount">{o.value_sat.toLocaleString()} sats</span>
                    </div>
                  {/if}
                {/each}
                <div class="preview-divider"></div>
              {/if}
              <div class="preview-row fee-row">
                <span>New network fee</span>
                <span>− {result.fee_sats.toLocaleString()} sats</span>
              </div>
              {#if tx.fee_sats != null}
                <div class="preview-row extra-row">
                  <span>Additional cost vs. original</span>
                  <span>+ {(result.fee_sats - tx.fee_sats).toLocaleString()} sats</span>
                </div>
              {/if}
              <div class="preview-row rate-row">
                <span>Effective fee rate</span>
                <span>{effectiveFeeRate} sat/vB</span>
              </div>
              {#if rateComparison && txPhase === 'compose'}
                <p class="rate-compare">{rateComparison}</p>
              {/if}
            </div>
          {/if}
        {:else if txPhase === 'verify'}
          {#if verifyLoading}
            <p class="tx-status">Decoding signed transaction…</p>
          {:else if verifyError}
            <p class="tx-status tx-error-text">{verifyError}</p>
          {:else if verifyDecoded}
            <div class="tx-body">
              {#each verifyDecoded.outputs as o, i (i)}
                {#if o.address}
                  <div class="verify-row verify-row-addr">
                    <span class="verify-label">{mode === 'rbf' ? 'Output' : 'Change'}</span>
                    <span class="mono verify-addr">{o.address}</span>
                    <span class="verify-value">{formatSats(o.value_sat)}</span>
                  </div>
                {/if}
              {/each}
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
        <QrSignFlow psbt={result!.psbt} onSigned={handleQrSigned} onCancel={() => qrOpen = false} />
      {:else if txPhase === 'compose'}
        {#if $hwEnabled}<button class="hw-sign-btn" onclick={startHwSign} disabled={!result}>⚿ Hardware wallet</button>{/if}
        <button class="qr-sign-btn" onclick={() => qrOpen = true} disabled={!result}>⬡ QR code</button>
        <button class="export-btn" onclick={exportPsbt} disabled={!result}>↓ Export PSBT</button>
        <CopyButton class="copy-psbt-btn" idle="⎘ Copy PSBT" copiedLabel="✓ Copied" text={() => result?.psbt ?? ''} disabled={!result} />
        {#if hwStatus === 'error'}
          <p class="hw-error">{hwMessage}</p>
        {:else}
          <p class="footer-hint">Sign this PSBT externally, then broadcast via ··· → Broadcast.</p>
        {/if}
      {:else if txPhase === 'sign'}
        <div class="hw-active">
          <div class="hw-status-msg">
            {#if hwStatus === 'connecting'}
              Connecting to device…
              {#if hwStuckHint}<span class="hw-stuck-hint">Make sure your device is plugged in via USB and unlocked.</span>{/if}
            {:else if hwStatus === 'pairing'}Verify pairing code: <code class="hw-code">{hwPairingCode}</code>
            {:else if hwStatus === 'confirm' || hwStatus === 'signing'}Confirm transaction on device…
            {/if}
          </div>
          {#if hwStatus !== 'signed'}
            <button class="btn-cancel-hw" onclick={cancelHwSign}>Cancel</button>
          {/if}
        </div>
        {#if broadcastError}<p class="hw-error">{broadcastError}</p>{/if}
      {:else if txPhase === 'verify' && verifyDecoded}
        <button class="btn-broadcast btn-broadcast-confirm" onclick={broadcastSigned} disabled={broadcasting}>
          {broadcasting ? 'Broadcasting…' : 'Confirm and broadcast'}
        </button>
        {#if broadcastError}<p class="hw-error">{broadcastError}</p>{/if}
      {/if}
    </div>
</Modal>

<style>
  .config-section { display: flex; flex-direction: column; gap: 12px; }
  .section-label {
    margin: 0 0 10px; font-size: 0.72rem; font-weight: 700;
    text-transform: uppercase; letter-spacing: 0.07em; color: var(--text-muted);
  }

  .info-grid { display: flex; flex-direction: column; gap: 6px; margin: 0; }
  .info-row { display: flex; justify-content: space-between; align-items: baseline; gap: 12px; }
  dt { font-size: 0.78rem; color: var(--text-muted); flex-shrink: 0; }
  dd { font-size: 0.82rem; font-weight: 600; margin: 0; text-align: right; }
  .mono { font-family: monospace; }
  .truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 220px; }

  .toggle-group { display: flex; gap: 6px; width: 100%; }
  .toggle-btn {
    flex: 1;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 6px; padding: 6px 4px; font-size: 0.78rem;
    color: var(--text-muted); cursor: pointer; text-align: center;
    line-height: 1.3; transition: all 0.12s;
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
  .custom-fee-row { display: flex; align-items: center; gap: 8px; margin-top: 10px; }
  .custom-fee-input {
    background: var(--surface-2); border: 1px solid var(--accent);
    border-radius: 4px; color: var(--text); padding: 5px 8px;
    font-size: 0.85rem; width: 90px; outline: none;
  }
  .custom-fee-unit { font-size: 0.78rem; color: var(--text-muted); }
  .min-hint { font-size: 0.72rem; color: var(--text-muted); }

  /* ── Unified Transaction panel — same shape as SendModal. ─────────────── */
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
  .tx-panel.phase-done .tx-phase-chip { color: #52a875; }

  .tx-status { font-size: 0.8rem; color: var(--text-muted); margin: 0; padding: 4px 0; }
  .tx-status.tx-error-text { color: #e05252; line-height: 1.5; }
  .tx-body { display: flex; flex-direction: column; gap: 8px; }

  .preview-row {
    display: flex; justify-content: space-between; align-items: center;
    font-size: 0.82rem; font-variant-numeric: tabular-nums;
  }
  .fee-row { color: #e05252; }
  .extra-row { color: var(--text-muted); font-size: 0.78rem; }
  .rate-row { color: var(--text-muted); }
  .preview-divider { height: 1px; background: var(--border); margin: 4px 0; }

  .tx-broadcast-body { display: flex; flex-direction: column; gap: 4px; }
  .tx-broadcast-label { font-size: 0.72rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.07em; }
  .tx-broadcast-txid { font-size: 0.78rem; color: var(--text); word-break: break-all; }
  .tx-broadcast-link { font-size: 0.78rem; color: var(--accent); text-decoration: none; }
  .tx-broadcast-link:hover { text-decoration: underline; }
  .out-row { display: grid; grid-template-columns: 70px 1fr auto; gap: 8px; align-items: baseline; }
  .out-label { font-size: 0.72rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .out-addr { font-family: monospace; font-size: 0.74rem; color: var(--text); word-break: break-all; line-height: 1.4; }
  .out-amount { font-size: 0.8rem; color: var(--text); font-variant-numeric: tabular-nums; }
  .rate-compare { margin: 4px 0 0; font-size: 0.74rem; color: var(--text-muted); font-style: italic; }
  .fee-rate-hint { margin: 8px 0 0; font-size: 0.74rem; color: var(--text-muted); line-height: 1.5; }

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
  /* Hardware = primary (accent fill); QR = secondary accent outline; export
     / copy stay as ghost buttons. Same hierarchy as SendModal. */
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
  .footer-hint { width: 100%; margin: 2px 0 0; font-size: 0.7rem; color: var(--text-muted); line-height: 1.4; }

  .hw-active { width: 100%; display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .hw-status-msg { flex: 1; font-size: 0.82rem; color: var(--text-muted); }
  .hw-code {
    font-family: monospace; font-size: 0.85rem; color: var(--text);
    background: var(--surface-2); padding: 1px 6px; border-radius: 3px;
  }
  .btn-broadcast {
    background: var(--accent); color: #000; border: none; border-radius: 5px;
    padding: 8px 18px; cursor: pointer; font-weight: 600; font-size: 0.82rem;
  }
  .btn-broadcast:hover:not(:disabled) { filter: brightness(1.08); }
  .btn-broadcast:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-cancel-hw {
    background: none; border: 1px solid var(--border); border-radius: 5px;
    color: var(--text-muted); cursor: pointer; font-size: 0.82rem; padding: 7px 12px;
  }
  .btn-cancel-hw:hover { border-color: var(--text-muted); color: var(--text); }
  .hw-error { width: 100%; margin: 0; font-size: 0.75rem; color: #e05252; }

  .hw-stuck-hint { display: block; font-size: 0.72rem; color: #e09c52; margin-top: 4px; }

  /* Verify-mode rows inside the unified panel */
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
  .verify-txid { font-size: 0.72rem; color: var(--text-muted); word-break: break-all; }
  .mono { font-family: monospace; }
  .btn-broadcast-confirm { flex: 1; padding: 10px 18px; font-size: 0.88rem; }
</style>
