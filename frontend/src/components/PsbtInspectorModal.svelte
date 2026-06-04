<script lang="ts">
  import { api } from '../lib/api'
  import { displayUnit, balancesHidden } from '../stores/settings'
  import { utxoKey } from '../lib/utils'
  import type { AddressInfo, DecodedTx, UtxoRecord, WalletEntry } from '../lib/types'
  import Modal from './ui/Modal.svelte'

  let {
    wallet,
    addresses,
    utxos,
    onClose,
  }: {
    wallet: WalletEntry
    addresses: AddressInfo[]
    utxos: UtxoRecord[]
    onClose: () => void
  } = $props()

  let psbtInput = $state('')
  let loading = $state(false)
  let error = $state('')
  let decoded = $state<DecodedTx | null>(null)

  let myAddresses = $derived(new Set(addresses.map(a => a.address)))
  let myOutpoints = $derived(new Set(utxos.map(u => utxoKey(u.txid, u.vout))))

  function formatSats(sats: number | null): string {
    if (sats === null) return 'unknown'
    if ($balancesHidden) return '•••'
    if ($displayUnit === 'btc') return (sats / 1e8).toFixed(8) + ' BTC'
    return sats.toLocaleString() + ' sats'
  }

  async function inspect() {
    const trimmed = psbtInput.trim()
    if (!trimmed) return
    loading = true; error = ''; decoded = null
    try {
      decoded = await api.broadcast.decode({ psbt: trimmed })
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to decode PSBT'
    } finally {
      loading = false
    }
  }

  let summary = $derived.by(() => {
    if (!decoded) return null
    let to_external = 0
    let to_mine = 0
    for (const o of decoded.outputs) {
      if (o.address && myAddresses.has(o.address)) to_mine += o.value_sat
      else to_external += o.value_sat
    }
    let from_mine = 0
    let from_unknown = 0
    let any_unknown_value = false
    for (const i of decoded.inputs) {
      if (i.value_sat === null) any_unknown_value = true
      const key = utxoKey(i.txid, i.vout)
      if (myOutpoints.has(key) && i.value_sat !== null) from_mine += i.value_sat
      else if (i.value_sat !== null) from_unknown += i.value_sat
    }
    return { to_external, to_mine, from_mine, from_unknown, any_unknown_value }
  })
</script>

<Modal open onclose={onClose} title="PSBT inspector" width="820px"
  desc="Paste any PSBT to see what it spends, where it sends, and how much it pays in fees — without signing or broadcasting.">


    <div class="input-row">
      <label for="psbt-input" class="sr-only">PSBT base64</label>
      <textarea
        id="psbt-input"
        class="psbt-input"
        bind:value={psbtInput}
        placeholder="cHNidP8BA…"
        rows="4"
        spellcheck="false"
        autocapitalize="off"
        autocomplete="off"
      ></textarea>
      <div class="actions">
        {#if psbtInput}
          <button class="btn-secondary" onclick={() => { psbtInput = ''; decoded = null; error = '' }}>Clear</button>
        {/if}
        <button
          class="btn-primary"
          onclick={inspect}
          disabled={loading || !psbtInput.trim()}
        >
          {loading ? 'Decoding…' : 'Inspect'}
        </button>
      </div>
    </div>

    {#if error}
      <p class="error">{error}</p>
    {/if}

    {#if decoded && summary}
      <div class="summary-grid">
        <div class="summary-card">
          <span class="summary-label">To external</span>
          <span class="summary-value sent">{formatSats(summary.to_external)}</span>
          <span class="summary-sub">{decoded.outputs.filter(o => !o.address || !myAddresses.has(o.address)).length} output{decoded.outputs.filter(o => !o.address || !myAddresses.has(o.address)).length === 1 ? '' : 's'}</span>
        </div>
        <div class="summary-card">
          <span class="summary-label">Change to you</span>
          <span class="summary-value received">{formatSats(summary.to_mine)}</span>
          <span class="summary-sub">{decoded.outputs.filter(o => o.address && myAddresses.has(o.address)).length} output{decoded.outputs.filter(o => o.address && myAddresses.has(o.address)).length === 1 ? '' : 's'}</span>
        </div>
        <div class="summary-card">
          <span class="summary-label">Fee</span>
          <span class="summary-value">{formatSats(decoded.fee_sat)}</span>
          <span class="summary-sub">
            {#if decoded.fee_rate_sat_vb}
              ~{decoded.fee_rate_sat_vb.toFixed(1)} sat/vB {decoded.vsize_approximate ? '(approx)' : ''}
            {/if}
          </span>
        </div>
      </div>

      {#if summary.from_unknown > 0 || summary.any_unknown_value}
        <div class="advisory">
          <span class="advisory-icon" aria-hidden="true">!</span>
          <div>
            <strong>Some inputs aren't from this wallet's known UTXOs.</strong>
            {#if wallet.kind === 'multisig'}
              Expected for multisig — the input values come from each cosigner's UTXO set.
            {:else}
              This PSBT may have been built for a different wallet, or with inputs that haven't been synced yet.
            {/if}
          </div>
        </div>
      {/if}

      <div class="detail-grid">
        <section class="detail-panel">
          <h3 class="detail-title">Inputs ({decoded.inputs.length})</h3>
          <ul class="io-list">
            {#each decoded.inputs as inp, i (i)}
              {@const key = utxoKey(inp.txid, inp.vout)}
              {@const mine = myOutpoints.has(key)}
              <li class:mine>
                <div class="io-top">
                  <span class="io-outpoint mono">{inp.txid.slice(0, 8)}…{inp.txid.slice(-8)}:{inp.vout}</span>
                  {#if mine}<span class="io-badge">yours</span>{/if}
                </div>
                <div class="io-bottom">
                  <span class="io-amount">{formatSats(inp.value_sat)}</span>
                </div>
              </li>
            {/each}
          </ul>
        </section>

        <section class="detail-panel">
          <h3 class="detail-title">Outputs ({decoded.outputs.length})</h3>
          <ul class="io-list">
            {#each decoded.outputs as out, i (i)}
              {@const mine = out.address ? myAddresses.has(out.address) : false}
              <li class:mine>
                <div class="io-top">
                  <span class="io-addr mono" title={out.address ?? ''}>
                    {#if out.address}
                      {out.address.slice(0, -8)}…{out.address.slice(-8)}
                    {:else}
                      <em>non-standard script</em>
                    {/if}
                  </span>
                  {#if mine}<span class="io-badge">yours</span>{/if}
                </div>
                <div class="io-bottom">
                  <span class="io-amount" class:received={mine}>{formatSats(out.value_sat)}</span>
                </div>
              </li>
            {/each}
          </ul>
        </section>
      </div>

      <div class="meta">
        <span>txid: <code class="mono">{decoded.txid.slice(0, 16)}…</code></span>
        <span>vsize: {decoded.vsize.toLocaleString()} vB{decoded.vsize_approximate ? ' (unsigned)' : ''}</span>
        {#if decoded.is_rbf}<span class="meta-flag">RBF enabled</span>{/if}
      </div>
    {/if}
</Modal>

<style>
  .input-row { display: flex; flex-direction: column; gap: 8px; }
  .psbt-input {
    width: 100%; box-sizing: border-box;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 6px; color: var(--text);
    font-family: monospace; font-size: 0.78rem;
    padding: 10px; resize: vertical;
    word-break: break-all;
  }
  .psbt-input:focus { outline: none; border-color: var(--accent); }
  .actions { display: flex; gap: 8px; justify-content: flex-end; }

  .error {
    margin: 12px 0 0; padding: 10px 12px;
    background: color-mix(in srgb, var(--error) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--error) 40%, transparent);
    border-radius: 6px;
    color: var(--error); font-size: 0.82rem;
  }

  .summary-grid {
    margin-top: 14px;
    display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px;
  }
  @media (max-width: 600px) {
    .summary-grid { grid-template-columns: 1fr; }
  }
  .summary-card {
    display: flex; flex-direction: column; gap: 2px;
    background: var(--surface-2);
    border: 1px solid var(--border); border-radius: 6px;
    padding: 10px 12px;
  }
  .summary-label {
    font-size: 0.7rem; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.05em;
  }
  .summary-value {
    font-size: 1rem; font-weight: 700; font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .summary-value.sent { color: #e05252; }
  .summary-value.received { color: #52a875; }
  .summary-sub { font-size: 0.72rem; color: var(--text-muted); }

  .advisory {
    display: flex; gap: 10px; align-items: flex-start;
    margin-top: 10px; padding: 10px 12px;
    background: color-mix(in srgb, #e09c52 12%, transparent);
    border: 1px solid color-mix(in srgb, #e09c52 35%, transparent);
    border-radius: 6px;
    font-size: 0.8rem; color: var(--text);
  }
  .advisory-icon {
    flex-shrink: 0;
    width: 22px; height: 22px; border-radius: 50%;
    display: inline-flex; align-items: center; justify-content: center;
    background: #e09c52; color: #000;
    font-weight: 700; font-size: 0.78rem;
  }

  .detail-grid {
    margin-top: 14px;
    display: grid; grid-template-columns: 1fr 1fr; gap: 12px;
  }
  @media (max-width: 720px) {
    .detail-grid { grid-template-columns: 1fr; }
  }
  .detail-panel {
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 6px; padding: 10px 12px;
  }
  .detail-title {
    margin: 0 0 8px; font-size: 0.78rem;
    text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-muted); font-weight: 700;
  }
  .io-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .io-list li {
    display: flex; flex-direction: column; gap: 2px;
    padding: 8px 10px;
    background: var(--surface-1);
    border: 1px solid var(--border); border-radius: 4px;
  }
  .io-list li.mine {
    border-left: 3px solid var(--accent);
    padding-left: 7px;
  }
  .io-top { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .io-bottom { display: flex; justify-content: flex-end; }
  .io-outpoint, .io-addr { font-size: 0.76rem; color: var(--text); }
  .io-amount {
    font-size: 0.82rem; font-weight: 600; font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .io-amount.received { color: #52a875; }
  .io-badge {
    flex-shrink: 0;
    font-size: 0.66rem; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.04em; padding: 1px 6px; border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
  }
  .mono { font-family: monospace; }

  .meta {
    display: flex; gap: 14px; flex-wrap: wrap;
    margin-top: 14px; padding-top: 12px;
    border-top: 1px solid var(--border);
    font-size: 0.74rem; color: var(--text-muted);
  }
  .meta code { font-size: 0.74rem; color: var(--text); }
  .meta-flag {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
    padding: 1px 6px; border-radius: 3px; font-weight: 700;
  }

  .sr-only {
    position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
    overflow: hidden; clip: rect(0,0,0,0); white-space: nowrap; border: 0;
  }
</style>
