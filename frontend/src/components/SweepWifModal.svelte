<script lang="ts">
  import { onMount } from 'svelte'
  import { api } from '../lib/api'
  import type { AddressInfo, WalletEntry } from '../lib/types'
  import { displayUnit, mempoolUrl, feeRates as feeRatesStore } from '../stores/settings'
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

  type View = 'input' | 'preview' | 'success'

  type Found = {
    script_type: string
    address: string
    utxos_found: number
    input_sats: number
    output_sats: number
    fee_sats: number
    signed_tx_hex: string
    txid: string
  }

  let view = $state<View>('input')
  let wif = $state('')
  let destination = $state('')
  let feeRate = $state(1)
  let found = $state<Found[]>([])
  let empty = $state<string[]>([])
  let searching = $state(false)
  let broadcastingIdx = $state<number | null>(null)
  // Candidates already broadcast — so when the user comes "back to remaining"
  // a swept tx is marked done instead of offering a redundant re-broadcast.
  let broadcastedIdxs = $state(new Set<number>())
  let lastTxid = $state('')
  let error = $state('')
  let modalTitle = $derived(
    view === 'preview' ? 'Sweep preview' : view === 'success' ? 'Broadcast successful' : 'Sweep private key',
  )

  onMount(() => {
    // Default destination to a fresh receive address on the current wallet.
    const unused = addresses.find(a => a.kind === 'external' && !a.used)
    if (unused) destination = unused.address
    // Default fee rate to mempool's half-hour estimate (or 1 sat/vB if unavailable).
    const rates = $feeRatesStore
    if (rates?.halfHourFee) feeRate = Math.max(1, Math.round(rates.halfHourFee))
  })

  async function findFunds() {
    if (!wif.trim() || !destination.trim()) return
    searching = true; error = ''; found = []; empty = []
    try {
      const result = await api.sweep({
        wif: wif.trim(),
        destination: destination.trim(),
        fee_rate_sat_vb: feeRate,
      })
      // Zeroize the WIF in component state once it has left the page.
      wif = ''
      found = result.found
      empty = result.empty
      view = 'preview'
    } catch (e) {
      error = e instanceof Error ? e.message : 'Unknown error'
    } finally {
      searching = false
    }
  }

  async function broadcastOne(idx: number) {
    const f = found[idx]
    if (!f) return
    broadcastingIdx = idx; error = ''
    try {
      const result = await api.broadcast.broadcast({ raw_hex: f.signed_tx_hex, wallet_id: wallet.id })
      lastTxid = result.txid
      broadcastedIdxs = new Set(broadcastedIdxs).add(idx)
      view = 'success'
    } catch (e) {
      error = e instanceof Error ? e.message : 'Unknown error'
    } finally {
      broadcastingIdx = null
    }
  }

  function formatAmount(sats: number): string {
    if ($displayUnit === 'btc') return (sats / 1e8).toFixed(8) + ' BTC'
    return sats.toLocaleString() + ' sats'
  }

  function truncate(s: string, n = 10): string {
    return s.length > n * 2 + 3 ? s.slice(0, n) + '…' + s.slice(-n) : s
  }

  function totalFound(): number {
    return found.reduce((acc, f) => acc + f.output_sats, 0)
  }

  function labelForKind(k: string): string {
    if (k.startsWith('p2pkh')) return 'Legacy (P2PKH)'
    if (k.startsWith('p2wpkh')) return 'SegWit (P2WPKH)'
    if (k.startsWith('p2sh-p2wpkh')) return 'Wrapped SegWit (P2SH-P2WPKH)'
    return k
  }
</script>

<Modal open onclose={onClose} title={modalTitle} width="560px">
    {#if view === 'input'}
      <p class="modal-desc">
        Paste a private key in WIF format. Corvin will scan its addresses across legacy, SegWit, and wrapped-SegWit script types, then build a transaction that drains everything into the destination below. The key is held in memory only and zeroized after signing.
      </p>

      <label class="field-label" for="wif-input">Private key (WIF)</label>
      <textarea
        id="wif-input"
        class="tx-input"
        bind:value={wif}
        placeholder="L… or K… or 5… (uncompressed)"
        rows="2"
        spellcheck="false"
        autocomplete="off"
        autocapitalize="off"
      ></textarea>

      <label class="field-label" for="dest-input">Destination (defaults to fresh address on "{wallet.label}")</label>
      <input
        id="dest-input"
        class="tx-input"
        type="text"
        bind:value={destination}
        spellcheck="false"
        autocomplete="off"
      />

      <label class="field-label" for="fee-input">Fee rate (sat/vB)</label>
      <input
        id="fee-input"
        class="tx-input fee-input"
        type="number"
        min="1"
        bind:value={feeRate}
      />

      {#if error}
        <p class="msg-err">{error}</p>
      {/if}

      <div class="modal-foot">
        <button class="btn-ghost" onclick={() => onClose()}>Cancel</button>
        <button
          class="btn-primary"
          onclick={findFunds}
          disabled={searching || !wif.trim() || !destination.trim()}
        >
          {searching ? 'Scanning…' : 'Find funds'}
        </button>
      </div>

    {:else if view === 'preview'}

      {#if found.length === 0}
        <div class="empty-state">
          <p>No UTXOs found at any candidate address.</p>
          <p class="empty-detail">Scanned: {empty.join(', ')}</p>
        </div>
      {:else}
        <p class="modal-desc">
          Found {formatAmount(totalFound())} across {found.length} address{found.length === 1 ? '' : 'es'}. Broadcast each transaction to move the funds.
        </p>
        <div class="candidates">
          {#each found as f, i (i)}
            <div class="candidate">
              <div class="candidate-head">
                <span class="kind-badge">{labelForKind(f.script_type)}</span>
                <span class="candidate-amount">{formatAmount(f.output_sats)}</span>
              </div>
              <div class="candidate-meta">
                <span class="meta-row-c"><span class="meta-label-c">From</span><span class="mono">{truncate(f.address, 12)}</span></span>
                <span class="meta-row-c"><span class="meta-label-c">UTXOs</span><span>{f.utxos_found}</span></span>
                <span class="meta-row-c"><span class="meta-label-c">Fee</span><span>{formatAmount(f.fee_sats)}</span></span>
                <span class="meta-row-c"><span class="meta-label-c">TXID</span><span class="mono">{truncate(f.txid, 10)}</span></span>
              </div>
              {#if broadcastedIdxs.has(i)}
                <div class="candidate-done">✓ Broadcast</div>
              {:else}
                <button
                  class="btn-primary btn-broadcast"
                  onclick={() => broadcastOne(i)}
                  disabled={broadcastingIdx !== null}
                >
                  {broadcastingIdx === i ? 'Broadcasting…' : 'Broadcast'}
                </button>
              {/if}
            </div>
          {/each}
        </div>
        {#if empty.length > 0}
          <p class="empty-detail">Also scanned (empty): {empty.join(', ')}</p>
        {/if}
      {/if}

      {#if error}
        <p class="msg-err">{error}</p>
      {/if}

      <div class="modal-foot">
        <button class="btn-ghost" onclick={() => onClose()}>Done</button>
      </div>

    {:else if view === 'success'}
      <div class="success-body">
        <div class="success-icon">✓</div>
        <p class="success-label">Sweep transaction submitted to the network</p>
        <div class="txid-box">
          <span class="txid-label">TXID</span>
          <span class="txid-val mono">{lastTxid}</span>
        </div>
        {#if $mempoolUrl}
          <a class="mempool-link" href={new URL('/tx/' + lastTxid, $mempoolUrl).href} target="_blank" rel="noopener noreferrer">
            View on mempool explorer ↗
          </a>
        {/if}
      </div>
      <div class="modal-foot">
        {#if found.length > 1}
          <button class="btn-ghost" onclick={() => { view = 'preview'; lastTxid = '' }}>← Back to remaining</button>
        {/if}
        <button class="btn-primary" onclick={() => onClose()}>Done</button>
      </div>
    {/if}
</Modal>

<style>
  .modal-desc { font-size: 0.82rem; color: var(--text-muted); margin: 0; line-height: 1.5; }
  .field-label { font-size: 0.76rem; color: var(--text-muted); margin: 0 0 -8px 0; }

  .tx-input {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); padding: 9px 11px; font-size: 0.82rem;
    font-family: monospace; resize: vertical; width: 100%;
    line-height: 1.5; box-sizing: border-box;
  }
  .tx-input:focus { outline: 1px solid var(--accent); outline-offset: -1px; }
  .fee-input { font-family: inherit; width: 120px; }

  .candidates { display: flex; flex-direction: column; gap: 10px; }
  .candidate {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 7px;
    padding: 12px 14px; display: flex; flex-direction: column; gap: 8px;
  }
  .candidate-head { display: flex; align-items: center; justify-content: space-between; }
  .kind-badge {
    font-size: 0.72rem; padding: 2px 8px; border-radius: 4px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent); font-weight: 600;
  }
  .candidate-amount { font-size: 0.95rem; font-weight: 600; color: var(--text); font-variant-numeric: tabular-nums; }
  .candidate-meta { display: grid; grid-template-columns: 1fr 1fr; gap: 4px 14px; font-size: 0.78rem; }
  .meta-row-c { display: flex; justify-content: space-between; }
  .meta-label-c { color: var(--text-muted); }
  .mono { font-family: monospace; font-size: 0.74rem; }
  .btn-broadcast { align-self: flex-end; padding: 5px 14px; font-size: 0.8rem; }
  .candidate-done { align-self: flex-end; font-size: 0.8rem; font-weight: 600; color: #52a875; padding: 5px 0; }

  .empty-state { padding: 18px 0; text-align: center; }
  .empty-state p { color: var(--text-muted); font-size: 0.88rem; margin: 0 0 6px 0; }
  .empty-detail { color: var(--text-muted); font-size: 0.76rem; margin: 4px 0 0 0; }

  .modal-foot { display: flex; justify-content: flex-end; gap: 8px; padding-top: 4px; border-top: 1px solid var(--border); margin-top: 4px; }

  .msg-err { font-size: 0.82rem; color: var(--error); margin: 0; }

  .success-body { display: flex; flex-direction: column; align-items: center; gap: 14px; padding: 12px 0; }
  .success-icon {
    width: 48px; height: 48px; border-radius: 50%;
    background: color-mix(in srgb, #52a875 15%, var(--surface-1));
    border: 1px solid color-mix(in srgb, #52a875 35%, transparent);
    display: flex; align-items: center; justify-content: center;
    font-size: 1.4rem; color: #52a875;
  }
  .success-label { font-size: 0.88rem; color: var(--text-muted); margin: 0; }
  .txid-box {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px;
    padding: 10px 14px; width: 100%; display: flex; flex-direction: column; gap: 4px;
  }
  .txid-label { font-size: 0.72rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.06em; }
  .txid-val { font-size: 0.78rem; color: var(--text); word-break: break-all; }
  .mempool-link { font-size: 0.82rem; color: var(--accent); text-decoration: none; }
  .mempool-link:hover { text-decoration: underline; }
</style>
