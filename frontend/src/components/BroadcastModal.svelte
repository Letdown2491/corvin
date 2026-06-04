<script lang="ts">
  import { api } from '../lib/api'
  import { bytesToBase64 } from '../lib/utils'
  import type { DecodedTx } from '../lib/types'
  import { displayUnit, mempoolUrl } from '../stores/settings'
  import QrScanModal from './QrScanModal.svelte'
  import BusyButton from './ui/BusyButton.svelte'
  import Modal from './ui/Modal.svelte'

  let { onClose }: { onClose: () => void } = $props()

  type View = 'input' | 'preview' | 'success'

  let view = $state<View>('input')
  let input = $state('')
  let decoded = $state<DecodedTx | null>(null)
  let txid = $state('')
  let broadcasting = $state(false)
  let error = $state('')
  let scanOpen = $state(false)

  let modalTitle = $derived(
    view === 'preview' ? 'Review transaction' : view === 'success' ? 'Broadcast successful' : 'Broadcast transaction',
  )

  function buildPayload(raw: string) {
    const trimmed = raw.trim().replace(/\s+/g, '')
    // Sniff format:
    //   - "cHNidP" prefix → base64-encoded PSBT (the magic bytes "psbt\xff" encode to that)
    //   - all hex, even length, starts with "01"/"02" tx version → raw hex tx
    //   - fall back to PSBT (assume the user knows what they pasted)
    if (trimmed.startsWith('cHNidP')) return { psbt: trimmed }
    if (/^[0-9a-fA-F]+$/.test(trimmed) && trimmed.length % 2 === 0) return { raw_hex: trimmed }
    return { psbt: trimmed }
  }

  async function decode() {
    const trimmed = input.trim()
    if (!trimmed) return
    error = ''
    try {
      decoded = await api.broadcast.decode(buildPayload(trimmed))
      view = 'preview'
    } catch (e) {
      error = e instanceof Error ? e.message : 'Unknown error'
    }
  }

  async function handleFileImport(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0]
    if (!file) return
    ;(e.target as HTMLInputElement).value = '' // allow re-picking the same file
    try {
      const buf = new Uint8Array(await file.arrayBuffer())
      // Binary PSBT magic: "psbt\xff" = 70 73 62 74 ff
      if (buf.length >= 5 && buf[0] === 0x70 && buf[1] === 0x73 && buf[2] === 0x62 && buf[3] === 0x74 && buf[4] === 0xff) {
        input = bytesToBase64(buf)
      } else {
        // Assume text — could be base64 PSBT or hex raw tx
        input = new TextDecoder().decode(buf).trim()
      }
      error = ''
    } catch (e) {
      error = e instanceof Error ? e.message : 'Could not read file'
    }
  }

  async function broadcast() {
    const trimmed = input.trim()
    broadcasting = true; error = ''
    try {
      const result = await api.broadcast.broadcast(buildPayload(trimmed))
      txid = result.txid
      view = 'success'
    } catch (e) {
      error = e instanceof Error ? e.message : 'Unknown error'
    } finally {
      broadcasting = false }
  }

  function formatAmount(sats: number): string {
    if ($displayUnit === 'btc') return (sats / 1e8).toFixed(8) + ' BTC'
    return sats.toLocaleString() + ' sats'
  }

  function formatFeeRate(r: number): string {
    return r.toFixed(1) + ' sat/vB'
  }

  function truncate(s: string, n = 12): string {
    return s.length > n * 2 + 3 ? s.slice(0, n) + '…' + s.slice(-n) : s
  }

  const HIGH_FEE_RATE = 200
</script>

<Modal open onclose={onClose} title={modalTitle}>
    {#if view === 'input'}
      <p class="modal-desc">Paste a signed PSBT (base64) or a raw transaction (hex) to review and broadcast it — or load a <code class="inline-code">.psbt</code> file.</p>

      <textarea
        class="tx-input"
        bind:value={input}
        placeholder="cHNidP8B… or 0200000001…"
        rows="6"
        spellcheck="false"
        autocomplete="off"
      ></textarea>

      <div class="input-aux">
        <label class="file-load-btn">
          ↓ Load .psbt file
          <input type="file" accept=".psbt,.txt" onchange={handleFileImport} />
        </label>
        <button type="button" class="file-load-btn" onclick={() => { scanOpen = true; error = '' }}>
          ⊞ Scan QR
        </button>
      </div>

      {#if error}
        <p class="msg-err">{error}</p>
      {/if}

      {#if scanOpen}
        <QrScanModal
          title="Scan transaction"
          hint="Point your camera at a PSBT or transaction QR — single or animated (BBQr / UR)."
          onResult={(value) => { input = value; scanOpen = false; error = '' }}
          onCancel={() => { scanOpen = false }}
        />
      {/if}

      <div class="modal-foot">
        <button class="btn-ghost" onclick={() => onClose()}>Cancel</button>
        <BusyButton idle="Decode" busyLabel="Decoding…" disabled={!input.trim()} onclick={decode} />
      </div>

    {:else if view === 'preview' && decoded}
      <div class="summary">

        <div class="section-label">Outputs</div>
        <div class="outputs">
          {#each decoded.outputs as out, i (i)}
            <div class="output-row">
              <span class="output-addr mono">{out.address ?? 'OP_RETURN'}</span>
              <span class="output-val">{formatAmount(out.value_sat)}</span>
            </div>
          {/each}
        </div>

        <div class="meta-grid">
          <div class="meta-row">
            <span class="meta-label">Fee</span>
            <span class="meta-value">
              {#if decoded.fee_sat != null}
                {formatAmount(decoded.fee_sat)}
                {#if decoded.fee_rate_sat_vb != null}
                  <span class="meta-muted">({formatFeeRate(decoded.fee_rate_sat_vb)})</span>
                {/if}
                {#if decoded.fee_rate_sat_vb != null && decoded.fee_rate_sat_vb > HIGH_FEE_RATE}
                  <span class="badge-warn">High fee</span>
                {/if}
              {:else}
                <span class="meta-muted">Unknown (input values missing)</span>
              {/if}
            </span>
          </div>
          <div class="meta-row">
            <span class="meta-label">Size</span>
            <span class="meta-value">
              {decoded.vsize} vB
              {#if decoded.vsize_approximate}
                <span class="meta-muted">(estimated — unsigned tx)</span>
              {/if}
            </span>
          </div>
          <div class="meta-row">
            <span class="meta-label">RBF</span>
            <span class="meta-value">
              {#if decoded.is_rbf}
                <span class="badge-ok">Enabled</span>
              {:else}
                <span class="meta-muted">Not signalled</span>
              {/if}
            </span>
          </div>
          <div class="meta-row">
            <span class="meta-label">TXID</span>
            <span class="meta-value mono">{truncate(decoded.txid, 10)}</span>
          </div>
        </div>

      </div>

      {#if decoded.vsize_approximate}
        <p class="msg-warn-block">⚠ This PSBT isn't fully signed yet — the network will reject it until every required signature is present. Finish signing it first (hardware wallet, cosigners, or the send flow).</p>
      {/if}

      {#if error}
        <p class="msg-err">{error}</p>
      {/if}

      <div class="modal-foot">
        <button class="btn-ghost" onclick={() => { view = 'input'; error = '' }}>← Back</button>
        <button class="btn-primary" onclick={broadcast} disabled={broadcasting || decoded.vsize_approximate}>
          {broadcasting ? 'Broadcasting…' : 'Broadcast'}
        </button>
      </div>

    {:else if view === 'success'}
      <div class="success-body">
        <div class="success-icon">✓</div>
        <p class="success-label">Transaction submitted to the network</p>
        <div class="txid-box">
          <span class="txid-label">TXID</span>
          <span class="txid-val mono">{txid}</span>
        </div>
        {#if $mempoolUrl}
          <a
            class="mempool-link"
            href={new URL('/tx/' + txid, $mempoolUrl).href}
            target="_blank"
            rel="noopener noreferrer"
          >
            View on mempool explorer ↗
          </a>
        {/if}
      </div>

      <div class="modal-foot">
        <button class="btn-primary" onclick={() => onClose()}>Done</button>
      </div>
    {/if}
</Modal>

<style>
  .modal-desc { font-size: 0.82rem; color: var(--text-muted); margin: 0; line-height: 1.5; }

  .tx-input {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); padding: 10px 12px; font-size: 0.8rem;
    font-family: monospace; resize: vertical; width: 100%;
    line-height: 1.5;
  }
  .tx-input:focus { outline: 1px solid var(--accent); outline-offset: -1px; }
  .input-aux {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .file-load-btn {
    align-self: flex-start;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px;
    color: var(--text-muted); cursor: pointer; font-size: 0.78rem; padding: 5px 12px;
    transition: border-color 0.12s, color 0.12s;
  }
  .file-load-btn:hover { border-color: var(--accent); color: var(--accent); }
  .file-load-btn input[type="file"] { display: none; }
  .inline-code { font-family: monospace; font-size: 0.78rem; background: var(--surface-2); padding: 1px 4px; border-radius: 3px; }

  .summary { display: flex; flex-direction: column; gap: 14px; }

  .section-label {
    font-size: 0.72rem; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.08em; color: var(--text-muted);
  }

  .outputs {
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 6px; overflow: hidden;
  }
  .output-row {
    display: flex; justify-content: space-between; align-items: baseline;
    gap: 12px; padding: 9px 12px; font-size: 0.82rem;
  }
  .output-row + .output-row { border-top: 1px solid var(--border); }
  .output-addr {
    color: var(--text); font-family: monospace; font-size: 0.74rem;
    word-break: break-all; line-height: 1.4; flex: 1; min-width: 0;
  }
  .output-val { color: var(--text); font-weight: 600; font-variant-numeric: tabular-nums; flex-shrink: 0; }

  .meta-grid { display: flex; flex-direction: column; gap: 8px; }
  .meta-row { display: flex; gap: 12px; font-size: 0.82rem; }
  .meta-label { color: var(--text-muted); width: 48px; flex-shrink: 0; }
  .meta-value { color: var(--text); display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .meta-muted { color: var(--text-muted); font-size: 0.78rem; }

  .badge-ok {
    font-size: 0.7rem; color: #52a875; background: color-mix(in srgb, #52a875 12%, var(--surface-1));
    border: 1px solid color-mix(in srgb, #52a875 30%, transparent);
    border-radius: 20px; padding: 1px 7px;
  }
  .badge-warn {
    font-size: 0.7rem; color: #e09c52; background: color-mix(in srgb, #e09c52 12%, var(--surface-1));
    border: 1px solid color-mix(in srgb, #e09c52 30%, transparent);
    border-radius: 20px; padding: 1px 7px;
  }

  .mono { font-family: monospace; font-size: 0.78rem; }

  .modal-foot {
    display: flex; justify-content: flex-end; gap: 8px;
    padding-top: 4px; border-top: 1px solid var(--border); margin-top: 4px;
  }

  .msg-err { font-size: 0.82rem; color: var(--error); margin: 0; }
  .msg-warn-block {
    font-size: 0.8rem; color: #e09c52; margin: 0; line-height: 1.5;
    padding: 8px 12px; border-radius: 5px;
    background: color-mix(in srgb, #e09c52 8%, var(--surface-1));
    border: 1px solid color-mix(in srgb, #e09c52 30%, transparent);
  }

  /* Success view */
  .success-body {
    display: flex; flex-direction: column; align-items: center;
    gap: 14px; padding: 12px 0;
  }
  .success-icon {
    width: 48px; height: 48px; border-radius: 50%; background: color-mix(in srgb, #52a875 15%, var(--surface-1));
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
