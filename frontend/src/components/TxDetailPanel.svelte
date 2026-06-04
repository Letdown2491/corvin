<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { mempoolUrl, showPriceData } from '../stores/settings'
  import { labels, setLabel } from '../stores/labels'
  import { costBasis, setCostBasis, deleteCostBasis } from '../stores/cost_basis'
  import { api } from '../lib/api'
  import type { TxRecord, UtxoRecord, TxBreakdown } from '../lib/types'
  import FeeBumpModal from './FeeBumpModal.svelte'
  import TxFlowDiagram from './TxFlowDiagram.svelte'

  interface FeeRates { fastestFee: number; halfHourFee: number; hourFee: number }
  let {
    tx,
    walletId = null,
    feeRates = null,
    utxos = [],
    onClose,
  }: {
    tx: TxRecord
    walletId?: string | null
    feeRates?: FeeRates | null
    utxos?: UtxoRecord[]
    onClose: () => void
  } = $props()

  let feeBumpMode = $state<'rbf' | 'cpfp' | null>(null)

  // CPFP requires at least one unspent output from this tx that lands in our wallet.
  let hasChildUtxos = $derived(utxos.some(u => u.txid === tx.txid))

  let canRbf = $derived(tx.confirmations === 0 && tx.is_rbf && walletId != null)
  let canCpfp = $derived(tx.confirmations === 0 && walletId != null && hasChildUtxos)

  // ── RBF replacement info ───────────────────────────────────────────────
  let replacementTxid = $state<string | null>(null)

  $effect(() => {
    if (!$mempoolUrl || tx.confirmations > 0) { replacementTxid = null; return }
    const controller = new AbortController()
    fetch(`/api/proxy/tx-rbf/${tx.txid}`, { signal: controller.signal })
      .then(r => { if (!r.ok) throw new Error('no data'); return r.json() as Promise<{ replacements?: { tx?: { txid?: string } } }> })
      .then(j => { replacementTxid = j?.replacements?.tx?.txid ?? null })
      .catch(e => { if (e.name !== 'AbortError') replacementTxid = null })
    return () => controller.abort()
  })

  // ── CPFP package info ──────────────────────────────────────────────────
  let cpfpEffectiveRate = $state<number | null>(null)

  $effect(() => {
    if (!$mempoolUrl || tx.confirmations > 0) { cpfpEffectiveRate = null; return }
    const controller = new AbortController()
    fetch(`/api/proxy/tx-cpfp/${tx.txid}`, { signal: controller.signal })
      .then(r => { if (!r.ok) throw new Error('no data'); return r.json() as Promise<{ effectiveFeePerVsize?: number }> })
      .then(j => {
        const eff = j?.effectiveFeePerVsize
        // Only surface if it meaningfully exceeds the raw rate (>10% higher)
        if (eff != null && localFeeRate != null && eff > localFeeRate * 1.1) {
          cpfpEffectiveRate = Math.round(eff * 10) / 10
        } else {
          cpfpEffectiveRate = null
        }
      })
      .catch(e => { if (e.name !== 'AbortError') cpfpEffectiveRate = null })
    return () => controller.abort()
  })

  let dialogEl = $state<HTMLDialogElement | null>(null)
  // Element to restore focus to when the panel closes (the row that opened it).
  let returnFocus: HTMLElement | null = null
  onMount(() => {
    returnFocus = (document.activeElement as HTMLElement) ?? null
    dialogEl?.showModal()
  })

  let txid = $derived(tx.txid)
  let shortTxid = $derived(txid.slice(0, 16) + '…' + txid.slice(-16))

  // ── Local data (always available) ─────────────────────────────────────
  let isConfirmed = $derived(tx.confirmations > 0)

  let localFeeRate = $derived(
    tx.fee_sats != null && tx.vsize != null && tx.vsize > 0
      ? Math.round((tx.fee_sats / tx.vsize) * 10) / 10
      : null
  )

  let localDate = $derived(
    tx.timestamp
      ? new Date(tx.timestamp).toLocaleString(undefined, {
          year: 'numeric', month: 'short', day: 'numeric',
          hour: '2-digit', minute: '2-digit',
        })
      : null
  )

  // ── Mempool extras (inputs/outputs, augmented block time) ──────────────
  interface MempoolVout {
    scriptpubkey_address?: string
    scriptpubkey_type?: string
    value: number
  }
  interface MempoolTx {
    size: number
    weight: number
    fee: number
    vin: unknown[]
    vout: MempoolVout[]
    status: {
      confirmed: boolean
      block_height: number | null
      block_hash: string | null
      block_time: number | null
    }
  }

  let mempoolData = $state<MempoolTx | null>(null)
  let mempoolLoading = $state(false)
  let mempoolError = $state('')

  $effect(() => {
    if (!$mempoolUrl) return
    mempoolData = null; mempoolError = ''
    mempoolLoading = true
    const controller = new AbortController()
    fetch(`/api/proxy/tx/${txid}`, { signal: controller.signal })
      .then(r => { if (!r.ok) throw new Error(`HTTP ${r.status}`); return r.json() as Promise<MempoolTx> })
      .then(d => { mempoolData = d; mempoolLoading = false })
      .catch(e => { if (e.name !== 'AbortError') { mempoolError = e.message; mempoolLoading = false } })
    return () => controller.abort()
  })

  // Wallet-aware breakdown (#31): outputs classified is_mine, so the flow diagram
  // can distinguish change / received from recipient.
  let breakdown = $state<TxBreakdown | null>(null)
  $effect(() => {
    breakdown = null
    if (walletId == null) return
    const id = walletId
    api.wallets.txBreakdown(id, txid).then(b => { breakdown = b }).catch(() => { breakdown = null })
  })

  type FlowOut = { label: string; sats: number; kind: 'recipient' | 'change' | 'fee' | 'received' }
  let flowOutputs = $derived.by((): FlowOut[] | null => {
    if (!breakdown) return null
    const sent = tx.amount_sats < 0
    const outs: FlowOut[] = breakdown.outputs.map((o) => {
      if (o.is_mine) {
        return sent
          ? { label: 'Change', sats: o.value_sats, kind: 'change' as const }
          : { label: 'To you', sats: o.value_sats, kind: 'received' as const }
      }
      const label = o.address ? `${o.address.slice(0, 8)}…${o.address.slice(-6)}` : 'Output'
      return { label, sats: o.value_sats, kind: 'recipient' as const }
    })
    if (breakdown.fee_sats != null) {
      outs.push({ label: 'Network fee', sats: breakdown.fee_sats, kind: 'fee' as const })
    }
    return outs
  })
  let flowInputSats = $derived(
    breakdown ? breakdown.outputs.reduce((s, o) => s + o.value_sats, 0) + (breakdown.fee_sats ?? 0) : 0
  )

  // Use mempool block_time to enrich date if local timestamp is missing
  let displayDate = $derived(
    localDate ??
    (mempoolData?.status.block_time
      ? new Date(mempoolData.status.block_time * 1000).toLocaleString(undefined, {
          year: 'numeric', month: 'short', day: 'numeric',
          hour: '2-digit', minute: '2-digit',
        })
      : null)
  )

  // ── Note ──────────────────────────────────────────────────────────────
  let noteInput = $state('')
  let noteSaving = $state(false)

  $effect(() => { noteInput = $labels[txid] ?? '' })

  let saveNoteTimer: ReturnType<typeof setTimeout> | null = null
  function onNoteInput() {
    if (saveNoteTimer) clearTimeout(saveNoteTimer)
    saveNoteTimer = setTimeout(async () => {
      noteSaving = true
      try { await setLabel(txid, noteInput) } finally { noteSaving = false }
    }, 600)
  }
  onDestroy(() => {
    if (saveNoteTimer) clearTimeout(saveNoteTimer)
    if (saveBasisTimer) clearTimeout(saveBasisTimer)
    returnFocus?.focus?.()
    returnFocus = null
  })

  // ── Price & cost basis ─────────────────────────────────────────────────
  let txTimestamp = $derived.by(() => {
    if (tx.timestamp) return Math.floor(new Date(tx.timestamp).getTime() / 1000)
    if (mempoolData?.status.block_time) return mempoolData.status.block_time
    return null
  })

  let usdPrice = $state<number | null>(null)
  let priceLoading = $state(false)
  let priceGen = 0

  $effect(() => {
    const ts = txTimestamp
    if (!ts) { usdPrice = null; return }
    const gen = ++priceGen
    priceLoading = true
    api.price.historical(ts)
      .then(r => { if (gen === priceGen) { usdPrice = r.usd; priceLoading = false } })
      .catch(() => { if (gen === priceGen) { usdPrice = null; priceLoading = false } })
  })

  let btcAmount = $derived(Math.abs(tx.amount_sats) / 1e8)
  let usdValue = $derived(usdPrice != null ? btcAmount * usdPrice : null)

  let storedBasis = $derived($costBasis[txid] ?? null)
  let costBasisInput = $state('')
  let basisSaving = $state(false)

  $effect(() => {
    costBasisInput = storedBasis != null ? storedBasis.toFixed(2) : ''
  })

  let saveBasisTimer: ReturnType<typeof setTimeout> | null = null
  function onBasisInput() {
    if (saveBasisTimer) clearTimeout(saveBasisTimer)
    saveBasisTimer = setTimeout(async () => {
      const n = parseFloat(costBasisInput)
      basisSaving = true
      try {
        if (!costBasisInput.trim() || isNaN(n)) await deleteCostBasis(txid)
        else await setCostBasis(txid, n)
      } finally { basisSaving = false }
    }, 600)
  }

  let isSent = $derived(tx.amount_sats < 0)
  let effectiveBasis = $derived.by(() => {
    const n = parseFloat(costBasisInput)
    if (!isNaN(n) && costBasisInput.trim()) return n
    if (storedBasis != null) return storedBasis
    return isSent ? null : usdValue
  })
  let gainLoss = $derived(
    isSent && usdValue != null && effectiveBasis != null ? usdValue - effectiveBasis : null
  )

  function fmtUsd(n: number): string {
    return '$' + n.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })
  }
</script>

<dialog
  bind:this={dialogEl}
  class="overlay"
  aria-labelledby="tx-detail-title"
  onclose={onClose}
  onclick={(e) => { if (e.target === dialogEl) dialogEl?.close() }}
>
  <div class="panel">
    <div class="panel-header">
      <div>
        <h2 id="tx-detail-title">Transaction</h2>
        <span class="txid-display" title={txid}>{shortTxid}</span>
      </div>
      <button class="close" onclick={() => dialogEl?.close()} aria-label="Close">✕</button>
    </div>

    <!-- Always-visible local data -->
    <dl class="details">
      <div class="row">
        <dt>Status</dt>
        <dd class:confirmed={isConfirmed} class:unconfirmed={!isConfirmed}>
          {isConfirmed ? `✓ Confirmed (${tx.confirmations})` : '⏳ Unconfirmed'}
        </dd>
      </div>

      {#if displayDate}
        <div class="row">
          <dt>Date</dt>
          <dd>{displayDate}</dd>
        </div>
      {/if}

      {#if tx.block_height}
        <div class="row">
          <dt>Block</dt>
          <dd>{tx.block_height.toLocaleString()}</dd>
        </div>
      {/if}

      {#if tx.fee_sats != null}
        <div class="row">
          <dt>Fee</dt>
          <dd>
            {tx.fee_sats.toLocaleString()} sats
            {#if localFeeRate != null}
              <span class="muted">· {localFeeRate} sat/vB</span>
            {/if}
            {#if cpfpEffectiveRate != null}
              <span class="cpfp-rate">· {cpfpEffectiveRate} sat/vB effective</span>
            {/if}
          </dd>
        </div>
      {/if}

      {#if tx.vsize != null}
        <div class="row">
          <dt>Size</dt>
          <dd>{tx.vsize} vB</dd>
        </div>
      {/if}

      <!-- Mempool extras: inputs/outputs -->
      {#if mempoolData}
        <div class="row">
          <dt>Inputs / Outputs</dt>
          <dd>{mempoolData.vin.length} in · {mempoolData.vout.length} out</dd>
        </div>
      {:else if mempoolLoading}
        <div class="row">
          <dt>Inputs / Outputs</dt>
          <dd class="muted">fetching…</dd>
        </div>
      {/if}
    </dl>

    {#if flowOutputs && flowOutputs.length > 0}
      <div class="flow-section">
        <div class="flow-header">Flow</div>
        <TxFlowDiagram
          inputSats={flowInputSats}
          inputCount={breakdown?.input_count ?? null}
          outputs={flowOutputs}
          format={(s) => s.toLocaleString() + ' sats'}
        />
      </div>
    {/if}

    <!-- Plain outputs list is the fallback when the wallet-aware flow isn't
         available (non-HD wallets, or the breakdown didn't load). -->
    {#if !flowOutputs && mempoolData && mempoolData.vout.length > 0}
      <div class="outputs-section">
        <div class="outputs-header">Outputs</div>
        <ul class="output-list">
          {#each mempoolData.vout as out, i (i)}
            <li class="output-item">
              <span class="output-addr mono">
                {out.scriptpubkey_address ?? (out.scriptpubkey_type ?? 'unknown')}
              </span>
              <span class="output-amount">{out.value.toLocaleString()} sats</span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    <!-- USD / cost basis -->
    {#if txTimestamp}
      <div class="usd-section">
        <div class="usd-header">USD value at time</div>
        <dl class="details">
          {#if $showPriceData}
            {#if priceLoading}
              <div class="row"><dt>BTC price</dt><dd class="muted">Loading…</dd></div>
            {:else if usdPrice != null && usdValue != null}
              <div class="row"><dt>BTC price</dt><dd>{fmtUsd(usdPrice)}</dd></div>
              <div class="row"><dt>Transaction value</dt><dd>{fmtUsd(usdValue)}</dd></div>
            {:else}
              <div class="row"><dt>BTC price</dt><dd class="muted">Unavailable</dd></div>
            {/if}
          {/if}
          <div class="row basis-row">
            <dt>Cost basis <span class="optional">(your acquisition cost)</span></dt>
            <dd>
              <div class="basis-input-wrap">
                <span class="dollar-sign">$</span>
                <input
                  type="text"
                  inputmode="decimal"
                  class="basis-input"
                  bind:value={costBasisInput}
                  oninput={onBasisInput}
                  placeholder={usdValue != null ? usdValue.toFixed(2) : '0.00'}
                />
                {#if basisSaving}<span class="saving">saving…</span>{/if}
              </div>
            </dd>
          </div>
          {#if gainLoss != null}
            <div class="row">
              <dt>Gain / Loss</dt>
              <dd class:gain={gainLoss > 0} class:loss={gainLoss < 0} class:neutral={gainLoss === 0}>
                {gainLoss >= 0 ? '+' : ''}{fmtUsd(gainLoss)}
              </dd>
            </div>
          {/if}
        </dl>
      </div>
    {/if}

    <!-- Replaced by another transaction -->
    {#if replacementTxid}
      <div class="replaced-section">
        <div class="replaced-label">Replaced by another transaction</div>
        <div class="replaced-body">
          <span class="mono replaced-txid">{replacementTxid.slice(0, 16)}…{replacementTxid.slice(-16)}</span>
          {#if $mempoolUrl}
            <a href={new URL('/tx/' + replacementTxid, $mempoolUrl).href} target="_blank" rel="noopener noreferrer" class="replaced-link">View →</a>
          {/if}
        </div>
      </div>
    {/if}

    <!-- Speed up (RBF / CPFP) — hidden if already replaced or confirmed -->
    {#if !replacementTxid && tx.confirmations === 0 && walletId != null}
      <div class="speedup-section">
        <div class="speedup-label">Speed up this transaction</div>
        {#if cpfpEffectiveRate != null}
          <p class="cpfp-notice">
            A child transaction is already boosting this to {cpfpEffectiveRate} sat/vB — it should confirm without further action.
          </p>
        {/if}
        <div class="speedup-buttons">
          {#if canRbf}
            <button class="speedup-btn" onclick={() => feeBumpMode = 'rbf'}>
              ↑ Replace-by-Fee
            </button>
          {/if}
          {#if canCpfp}
            <button class="speedup-btn" onclick={() => feeBumpMode = 'cpfp'}>
              + Child-Pays-for-Parent
            </button>
          {/if}
        </div>
        {#if !canRbf && !tx.is_rbf}
          <p class="speedup-disabled-hint">
            <strong>RBF unavailable:</strong> this transaction was sent without RBF signaling (sequence numbers are final). It can only be sped up via CPFP.
          </p>
        {/if}
        {#if !canCpfp && !hasChildUtxos}
          <p class="speedup-disabled-hint">
            <strong>CPFP unavailable:</strong> none of this transaction's outputs landed in your wallet, so there's nothing for you to spend as a child. {#if !canRbf}You'll need the recipient to bump the fee.{/if}
          </p>
        {/if}
      </div>
    {/if}

    <!-- Note -->
    <div class="note-section">
      <label class="note-label" for="tx-note">
        Note {#if noteSaving}<span class="saving">saving…</span>{/if}
      </label>
      <input
        id="tx-note"
        type="text"
        class="note-input"
        bind:value={noteInput}
        oninput={onNoteInput}
        placeholder="Add a note…"
      />
    </div>

    <div class="panel-footer">
      {#if $mempoolUrl}
        <a href={new URL(`/tx/${txid}`, $mempoolUrl).href} target="_blank" rel="noopener noreferrer" class="link-out">
          Open in mempool explorer →
        </a>
      {:else if mempoolError}
        <p class="muted footer-note">Mempool explorer unavailable: {mempoolError}</p>
      {/if}
    </div>
  </div>
</dialog>

{#if feeBumpMode && walletId}
  <FeeBumpModal
    {walletId}
    {tx}
    {feeRates}
    mode={feeBumpMode}
    onClose={() => feeBumpMode = null}
  />
{/if}

<style>
  .overlay {
    position: fixed; inset: 0; width: 100%; height: 100%;
    max-width: 100%; max-height: 100%; margin: 0; padding: 0;
    border: none; background: transparent;
    display: flex; align-items: center; justify-content: center;
  }
  .overlay::backdrop { background: rgba(0,0,0,0.55); }
  .panel {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 8px;
    width: min(520px, 95vw); padding: 24px;
    display: flex; flex-direction: column; gap: 20px;
    max-height: 90vh; overflow-y: auto;
  }
  @media (max-width: 480px) {
    .panel { padding: 16px; gap: 14px; max-height: 92dvh; width: calc(100vw - 16px); }
  }
  .panel-header { display: flex; justify-content: space-between; align-items: flex-start; }
  h2 { margin: 0 0 4px; font-size: 1.05rem; }
  .txid-display { font-family: monospace; font-size: 0.78rem; color: var(--text-muted); }
  .close { background: none; border: none; cursor: pointer; font-size: 1rem; color: var(--text-muted); }
  .muted { color: var(--text-muted); }

  .details { display: flex; flex-direction: column; gap: 10px; margin: 0; }

  .flow-section, .outputs-section {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px;
    padding: 10px 14px; display: flex; flex-direction: column; gap: 6px;
  }
  .flow-header, .outputs-header {
    font-size: 0.72rem; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.06em; color: var(--text-muted);
  }
  .output-list { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 4px; }
  .output-item {
    display: flex; justify-content: space-between; gap: 12px; align-items: baseline;
    font-size: 0.78rem;
  }
  .output-addr { color: var(--text); word-break: break-all; flex: 1; min-width: 0; font-family: monospace; font-size: 0.72rem; line-height: 1.4; }
  .output-amount { color: var(--text); font-variant-numeric: tabular-nums; font-weight: 600; flex-shrink: 0; }
  .mono { font-family: monospace; }
  .row { display: flex; justify-content: space-between; align-items: baseline; gap: 16px; }
  dt { font-size: 0.8rem; color: var(--text-muted); flex-shrink: 0; }
  dd { font-size: 0.88rem; font-weight: 600; margin: 0; text-align: right; }
  dd.confirmed { color: #52a875; }
  dd.unconfirmed { color: #e09c52; }
  .cpfp-rate { color: #52a875; font-size: 0.82rem; font-weight: 600; }
  dd.gain { color: #52a875; }
  dd.loss { color: #e05252; }
  dd.neutral { color: var(--text-muted); }

  .usd-section {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px;
    padding: 12px 14px; display: flex; flex-direction: column; gap: 10px;
  }
  .usd-header { font-size: 0.78rem; color: var(--text-muted); font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }
  .optional { font-size: 0.72rem; font-weight: 400; opacity: 0.7; }
  .basis-row dd { flex: 1; }
  .basis-input-wrap { display: flex; align-items: center; gap: 3px; justify-content: flex-end; }
  .dollar-sign { font-size: 0.85rem; color: var(--text-muted); }
  .basis-input {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 4px;
    color: var(--text); padding: 3px 7px; font-size: 0.85rem; font-weight: 600;
    width: 110px; text-align: right;
  }
  .basis-input:focus { outline: 1px solid var(--accent); }
  .saving { font-size: 0.72rem; color: var(--text-muted); font-style: italic; }

  .replaced-section {
    background: color-mix(in srgb, #9b59b6 8%, var(--surface-1));
    border: 1px solid color-mix(in srgb, #9b59b6 25%, transparent);
    border-radius: 6px; padding: 12px 14px;
    display: flex; flex-direction: column; gap: 6px;
  }
  .replaced-label { font-size: 0.78rem; color: #c39bd3; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }
  .replaced-body { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .replaced-txid { font-size: 0.8rem; color: var(--text); flex: 1; min-width: 0; }
  .replaced-link { font-size: 0.78rem; color: var(--accent); text-decoration: none; white-space: nowrap; flex-shrink: 0; }
  .replaced-link:hover { text-decoration: underline; }

  .speedup-section {
    background: color-mix(in srgb, #e09c52 6%, var(--surface-1));
    border: 1px solid color-mix(in srgb, #e09c52 20%, transparent);
    border-radius: 6px; padding: 12px 14px;
    display: flex; flex-direction: column; gap: 8px;
  }
  .speedup-label { font-size: 0.78rem; color: var(--text-muted); font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }
  .cpfp-notice { font-size: 0.78rem; color: #52a875; margin: 0; line-height: 1.5; }
  .speedup-disabled-hint { font-size: 0.74rem; color: var(--text-muted); margin: 4px 0 0; line-height: 1.5; }
  .speedup-buttons { display: flex; gap: 8px; flex-wrap: wrap; }
  .speedup-btn {
    background: none; border: 1px solid var(--border); border-radius: 5px;
    color: var(--text-muted); cursor: pointer; font-size: 0.8rem; padding: 6px 14px;
    transition: all 0.12s;
  }
  .speedup-btn:hover { border-color: #e09c52; color: #e09c52; }

  .note-section { display: flex; flex-direction: column; gap: 5px; }
  .note-label { font-size: 0.78rem; color: var(--text-muted); display: flex; gap: 6px; align-items: center; }
  .note-input {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px;
    color: var(--text); padding: 7px 10px; font-size: 0.85rem; width: 100%; box-sizing: border-box;
  }
  .panel-footer { border-top: 1px solid var(--border); padding-top: 16px; }
  .link-out { color: var(--accent); font-size: 0.85rem; text-decoration: none; }
  .link-out:hover { text-decoration: underline; }
  .footer-note { font-size: 0.75rem; margin: 0; }
</style>
