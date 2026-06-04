<script lang="ts">
  import { api, ApiErr } from '../lib/api'
  import type { UtxoRecord, WalletEntry } from '../lib/types'
  import { displayUnit, mempoolUrl } from '../stores/settings'
  import Modal from './ui/Modal.svelte'

  interface FeeRates { fastestFee: number; halfHourFee: number; hourFee: number }
  let {
    wallet,
    utxos,
    feeRates,
    onClose,
    onBroadcast,
  }: {
    wallet: WalletEntry
    utxos: UtxoRecord[]
    feeRates: FeeRates | null
    onClose: () => void
    onBroadcast: () => void
  } = $props()

  type View = 'input' | 'success'
  let view = $state<View>('input')
  let modalTitle = $derived(view === 'success' ? 'Sent' : 'Send from Silent Payments wallet')

  let recipient = $state('')
  let amount = $state('')
  let sendMax = $state(false)
  let localUnit = $state<'btc' | 'sats'>($displayUnit)

  type FeePreset = 'hour' | 'halfhour' | 'fastest' | 'custom'
  let feePreset = $state<FeePreset>('hour')
  let customFeeRate = $state(2)
  let effectiveFeeRate = $derived.by((): number => {
    if (feePreset === 'custom') return Math.max(1, customFeeRate)
    if (!feeRates) return 2
    if (feePreset === 'fastest') return feeRates.fastestFee
    if (feePreset === 'halfhour') return feeRates.halfHourFee
    return feeRates.hourFee
  })

  let mnemonic = $state('')
  let passphrase = $state('')
  let seedRevealed = $state(false)
  let seedWords = $derived(mnemonic.trim().split(/\s+/).filter(Boolean).length)

  let sending = $state(false)
  let error = $state('')
  let result = $state<{ txid: string; input_sats: number; recipient_sats: number; change_sats: number; fee_sats: number } | null>(null)

  const spendable = $derived(utxos.reduce((s, u) => s + u.amount_sats, 0))

  function isSp(s: string): boolean {
    const l = s.toLowerCase()
    return l.startsWith('sp1') || l.startsWith('tsp1') || l.startsWith('sprt1')
  }
  function amountToSats(a: string): number {
    const n = parseFloat(a)
    if (isNaN(n) || n <= 0) return 0
    return localUnit === 'btc' ? Math.round(n * 1e8) : Math.floor(n)
  }
  function fmt(sats: number): string {
    return localUnit === 'btc' ? (sats / 1e8).toFixed(8).replace(/\.?0+$/, '') + ' BTC' : sats.toLocaleString() + ' sats'
  }

  let canSend = $derived.by(() => {
    if (!recipient.trim() || isSp(recipient)) return false
    if (!sendMax && amountToSats(amount) <= 0) return false
    if (seedWords < 12) return false
    return true
  })


  async function doSend() {
    if (!canSend || sending) return
    sending = true; error = ''
    try {
      const r = await api.wallets.spSpend(wallet.id, {
        outputs: [{ recipient: recipient.trim(), amount_sats: sendMax ? null : amountToSats(amount) }],
        fee_rate_sat_vb: effectiveFeeRate,
        mnemonic: mnemonic.trim(),
        passphrase: passphrase || undefined,
      })
      result = r
      view = 'success'
    } catch (e) {
      // The wrong-secret case gets a clearer, action-oriented message via the
      // stable error code rather than relying on the raw message text.
      if (e instanceof ApiErr && e.code === 'wrong_secret') {
        error = 'That seed or passphrase doesn\'t match this wallet. Double-check the words, passphrase, and account.'
      } else {
        error = e instanceof Error ? e.message : 'Send failed'
      }
    } finally {
      // Wipe seed material from component state once it's left the page.
      mnemonic = ''; passphrase = ''
      sending = false
    }
  }
</script>

<Modal open onclose={onClose} title={modalTitle} width="480px">
    {#if view === 'input'}
      <p class="desc">Spends coins received to this SP wallet. Change returns to your own Silent Payments change address. Recipients must be regular addresses (paying another <code>sp1…</code> address from here isn't supported yet).</p>

      <label class="lbl" for="sp-recip">Recipient address</label>
      <input id="sp-recip" class="inp" bind:value={recipient} placeholder="bc1… / 3… / 1…" spellcheck="false" autocapitalize="off" />
      {#if recipient.trim() && isSp(recipient)}
        <span class="warn-line">Paying a Silent Payment address from an SP wallet isn't supported yet — use a regular address.</span>
      {/if}

      <div class="amount-row">
        <div class="amount-field">
          <label class="lbl" for="sp-amt">Amount</label>
          <input id="sp-amt" class="inp" type="number" min="0" bind:value={amount} disabled={sendMax} placeholder="0" />
        </div>
        <div class="unit-toggle">
          <button type="button" class:active={localUnit === 'sats'} onclick={() => localUnit = 'sats'}>sats</button>
          <button type="button" class:active={localUnit === 'btc'} onclick={() => localUnit = 'btc'}>BTC</button>
        </div>
      </div>
      <label class="max-row">
        <input type="checkbox" bind:checked={sendMax} />
        Send max (drain {fmt(spendable)})
      </label>

      <div class="fee-row">
        <span class="lbl">Fee rate</span>
        <div class="fee-presets">
          <button type="button" class:active={feePreset === 'hour'} onclick={() => feePreset = 'hour'}>Slow{#if feeRates} · {feeRates.hourFee}{/if}</button>
          <button type="button" class:active={feePreset === 'halfhour'} onclick={() => feePreset = 'halfhour'}>Mid{#if feeRates} · {feeRates.halfHourFee}{/if}</button>
          <button type="button" class:active={feePreset === 'fastest'} onclick={() => feePreset = 'fastest'}>Fast{#if feeRates} · {feeRates.fastestFee}{/if}</button>
          <button type="button" class:active={feePreset === 'custom'} onclick={() => feePreset = 'custom'}>Custom</button>
        </div>
        {#if feePreset === 'custom'}
          <input class="inp fee-custom" type="number" min="1" bind:value={customFeeRate} aria-label="Custom fee rate (sat/vB)" /> <span class="unit-lbl">sat/vB</span>
        {/if}
      </div>

      <div class="seed-head">
        <label class="lbl" for="sp-seed">Seed phrase <span class="muted">— used once to sign, then wiped</span></label>
        <button type="button" class="reveal" onclick={() => seedRevealed = !seedRevealed}>{seedRevealed ? 'Hide' : 'Show'}</button>
      </div>
      <input id="sp-seed" class="inp" type={seedRevealed ? 'text' : 'password'} bind:value={mnemonic} placeholder="twelve or twenty-four words" spellcheck="false" autocapitalize="off" autocomplete="off" />
      <input class="inp" type="password" bind:value={passphrase} placeholder="BIP39 passphrase (optional)" aria-label="BIP39 passphrase (optional)" autocomplete="new-password" />

      <p class="broadcast-note">⚠ This builds, signs, and <strong>broadcasts immediately</strong> at {effectiveFeeRate} sat/vB.</p>
      {#if error}<p class="err">{error}</p>{/if}

      <div class="foot">
        <button class="btn-ghost" onclick={() => onClose()}>Cancel</button>
        <button class="btn-primary" onclick={doSend} disabled={!canSend || sending}>{sending ? 'Sending…' : 'Sign & send'}</button>
      </div>

    {:else if view === 'success' && result}
      <div class="success">
        <div class="success-icon">✓</div>
        <div class="summary">
          <div class="srow"><span>Sent</span><span>{fmt(result.recipient_sats)}</span></div>
          <div class="srow"><span>Network fee</span><span>{fmt(result.fee_sats)}</span></div>
          {#if result.change_sats > 0}<div class="srow"><span>Change (to your SP change addr)</span><span>{fmt(result.change_sats)}</span></div>{/if}
        </div>
        <div class="txid-box"><span class="txid-lbl">TXID</span><span class="txid mono">{result.txid}</span></div>
        {#if $mempoolUrl}
          <a class="mempool-link" href={new URL('/tx/' + result.txid, $mempoolUrl).href} target="_blank" rel="noopener noreferrer">View on mempool ↗</a>
        {/if}
        <p class="desc">Change is re-discovered after the scanner restarts.</p>
      </div>
      <div class="foot">
        <button class="btn-primary" onclick={() => { onBroadcast(); onClose() }}>Done</button>
      </div>
    {/if}
</Modal>

<style>
  .desc { font-size: 0.8rem; color: var(--text-muted); margin: 0; line-height: 1.5; }
  .lbl { font-size: 0.76rem; color: var(--text-muted); font-weight: 600; }
  .muted { color: var(--text-muted); font-weight: 400; }
  .inp {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); padding: 8px 10px; font-size: 0.85rem; width: 100%; box-sizing: border-box;
    font-family: monospace;
  }
  .inp:focus { outline: 1px solid var(--accent); outline-offset: -1px; }
  .amount-row { display: flex; gap: 8px; align-items: flex-end; }
  .amount-field { flex: 1; display: flex; flex-direction: column; gap: 4px; }
  .unit-toggle { display: flex; }
  .unit-toggle button {
    background: var(--surface-2); border: 1px solid var(--border); color: var(--text-muted);
    padding: 8px 10px; cursor: pointer; font-size: 0.78rem;
  }
  .unit-toggle button:first-child { border-radius: 5px 0 0 5px; }
  .unit-toggle button:last-child { border-radius: 0 5px 5px 0; border-left: none; }
  .unit-toggle button.active { background: var(--surface-active); color: var(--text); border-color: var(--accent); }
  .max-row { display: flex; align-items: center; gap: 6px; font-size: 0.8rem; color: var(--text); }
  .fee-row { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .fee-presets { display: flex; }
  .fee-presets button {
    background: var(--surface-2); border: 1px solid var(--border); color: var(--text-muted);
    padding: 6px 10px; cursor: pointer; font-size: 0.76rem; border-right: none;
  }
  .fee-presets button:first-child { border-radius: 5px 0 0 5px; }
  .fee-presets button:last-child { border-radius: 0 5px 5px 0; border-right: 1px solid var(--border); }
  .fee-presets button.active { background: var(--surface-active); color: var(--text); border-color: var(--accent); }
  .fee-custom { width: 90px; font-family: inherit; }
  .unit-lbl { font-size: 0.76rem; color: var(--text-muted); }
  .seed-head { display: flex; align-items: baseline; justify-content: space-between; }
  .reveal { background: none; border: none; color: var(--accent); cursor: pointer; font-size: 0.76rem; padding: 0; }
  .reveal:hover { text-decoration: underline; }
  .warn-line { font-size: 0.74rem; color: #e09c52; }
  .broadcast-note {
    margin: 2px 0 0; font-size: 0.76rem; color: #e09c52; line-height: 1.5;
    padding: 7px 10px; border-radius: 5px;
    background: color-mix(in srgb, #e09c52 8%, transparent);
    border: 1px solid color-mix(in srgb, #e09c52 30%, transparent);
  }
  .broadcast-note strong { color: var(--text); }
  .err { font-size: 0.8rem; color: var(--error); margin: 0; }
  .foot { display: flex; justify-content: flex-end; gap: 8px; padding-top: 6px; border-top: 1px solid var(--border); margin-top: 4px; }
  .success { display: flex; flex-direction: column; align-items: center; gap: 12px; padding: 8px 0; }
  .success-icon {
    width: 44px; height: 44px; border-radius: 50%; display: flex; align-items: center; justify-content: center;
    background: color-mix(in srgb, #52a875 18%, transparent); color: #52a875; font-size: 1.4rem;
  }
  .summary { width: 100%; display: flex; flex-direction: column; gap: 5px; }
  .srow { display: flex; justify-content: space-between; font-size: 0.82rem; color: var(--text-muted); font-variant-numeric: tabular-nums; }
  .txid-box { width: 100%; display: flex; flex-direction: column; gap: 3px; }
  .txid-lbl { font-size: 0.72rem; color: var(--text-muted); }
  .txid { font-size: 0.74rem; word-break: break-all; color: var(--text); }
  .mono { font-family: monospace; }
  .mempool-link { font-size: 0.8rem; color: var(--accent); text-decoration: none; }
  .mempool-link:hover { text-decoration: underline; }
</style>
