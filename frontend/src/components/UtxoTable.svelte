<script lang="ts">
  import { onDestroy } from 'svelte'
  import EmptyState from './EmptyState.svelte'
  import type { UtxoRecord } from '../lib/types'
  import { displayUnit, mempoolUrl, balancesHidden } from '../stores/settings'
  import { utxoLabels, setUtxoLabel } from '../stores/utxo_labels'
  import { frozenUtxos, freezeUtxo, unfreezeUtxo } from '../stores/utxo_freeze'
  import { utxoCategories, addressCategories, assignUtxoCategory } from '../stores/categories'
  import { addToast } from '../stores/toasts'
  import { utxoKey } from '../lib/utils'
  import CategoryPicker from './CategoryPicker.svelte'
  import CategoryChip from './CategoryChip.svelte'

  interface FeeRates { fastestFee: number; halfHourFee: number; hourFee: number }
  let {
    utxos,
    feeRates,
    consolidateMode = false,
    addressLabels = {},
    canConsolidate = false,
    onConsolidate = () => {},
    onStartConsolidate = () => {},
    onCancelConsolidate = () => {},
  }: {
    utxos: UtxoRecord[]
    feeRates: FeeRates | null
    consolidateMode?: boolean
    addressLabels?: Record<string, string>
    /** Show the "Consolidate" entry-point button (HD wallets only). */
    canConsolidate?: boolean
    onConsolidate?: (utxos: UtxoRecord[]) => void
    onStartConsolidate?: () => void
    onCancelConsolidate?: () => void
  } = $props()

  // ── Script type ──────────────────────────────────────────────────────────
  type ScriptType = 'p2wpkh' | 'taproot' | 'p2sh' | 'legacy' | 'unknown'

  function scriptType(address: string | null): ScriptType {
    if (!address) return 'unknown'
    if (address.startsWith('bc1q') || address.startsWith('tb1q')) return 'p2wpkh'
    if (address.startsWith('bc1p') || address.startsWith('tb1p')) return 'taproot'
    if (address.startsWith('3')    || address.startsWith('2'))    return 'p2sh'
    if (address.startsWith('1')    || address.startsWith('m') || address.startsWith('n')) return 'legacy'
    return 'unknown'
  }

  const TYPE_LABEL: Record<ScriptType, string> = {
    p2wpkh:  'P2WPKH',
    taproot: 'Taproot',
    p2sh:    'P2SH',
    legacy:  'Legacy',
    unknown: '?',
  }

  const TYPE_TITLE: Record<ScriptType, string> = {
    p2wpkh:  'Native SegWit (P2WPKH)',
    taproot: 'Taproot (P2TR) — latest script type',
    p2sh:    'Wrapped SegWit or multisig (P2SH)',
    legacy:  'Legacy (P2PKH) — highest spend cost',
    unknown: 'Unknown script type',
  }

  // ── Sorting ──────────────────────────────────────────────────────────────
  type SortCol = 'amount' | 'age'
  type SortDir = 'asc' | 'desc'
  let sortCol = $state<SortCol>('amount')
  let sortDir = $state<SortDir>('desc')

  function setSort(col: SortCol) {
    if (sortCol === col) {
      sortDir = sortDir === 'desc' ? 'asc' : 'desc'
    } else {
      sortCol = col
      sortDir = col === 'amount' ? 'desc' : 'asc'
    }
  }

  function sortIcon(col: SortCol) {
    if (sortCol !== col) return '↕'
    return sortDir === 'desc' ? '↓' : '↑'
  }

  // Type column only earns its space when UTXOs have mixed script types
  // (e.g. multi-account wallets with BIP-84 + BIP-86, or imported sweeps).
  // For the common case — a single-script wallet — every row would show
  // the same badge, so we hide the column entirely.
  let mixedTypes = $derived.by(() => {
    if (utxos.length < 2) return false
    const first = scriptType(utxos[0].address)
    return utxos.some(u => scriptType(u.address) !== first)
  })

  let sorted = $derived.by(() => {
    const list = [...utxos]
    if (sortCol === 'amount') {
      list.sort((a, b) => sortDir === 'desc'
        ? b.amount_sats - a.amount_sats
        : a.amount_sats - b.amount_sats)
    } else {
      list.sort((a, b) => sortDir === 'asc'
        ? (a.block_height ?? Infinity) - (b.block_height ?? Infinity)
        : (b.block_height ?? 0) - (a.block_height ?? 0))
    }
    return list
  })

  // ── Dust ─────────────────────────────────────────────────────────────────
  function inputVbytes(address: string | null): number {
    if (!address) return 68
    if (address.startsWith('bc1q') || address.startsWith('tb1q')) return 68
    if (address.startsWith('bc1p') || address.startsWith('tb1p')) return 58
    if (address.startsWith('3')    || address.startsWith('2'))    return 91
    if (address.startsWith('1')    || address.startsWith('m') || address.startsWith('n')) return 148
    return 68
  }

  function isDust(u: UtxoRecord): boolean {
    if (!feeRates) return false
    return u.amount_sats < inputVbytes(u.address) * feeRates.hourFee
  }
  // Server-flagged Silent Payments dust-attack output: a tiny payment to your reusable
  // SP address, used to probe or link your wallet. A privacy signal, distinct from the
  // fee-based economic dust above.
  function isDustAttack(u: UtxoRecord): boolean {
    return u.suspected_dust
  }
  function isFreezeableDust(u: UtxoRecord): boolean {
    return isDust(u) || isDustAttack(u)
  }

  let economicDustCount = $derived(feeRates ? utxos.filter(u => isDust(u)).length : 0)
  let attackDustCount = $derived(utxos.filter(u => isDustAttack(u)).length)
  let dustCount = $derived(utxos.filter(u => isFreezeableDust(u)).length)
  let showNudge = $derived(utxos.length >= 10 || dustCount > 0)

  // ── Shared address detection ──────────────────────────────────────────────
  let sharedAddresses = $derived.by(() => {
    const counts = new Map<string, number>()
    for (const u of utxos) {
      if (u.address) counts.set(u.address, (counts.get(u.address) ?? 0) + 1)
    }
    return new Set([...counts.entries()].filter(([, n]) => n > 1).map(([a]) => a))
  })

  // ── Footer totals ─────────────────────────────────────────────────────────
  let totalSats     = $derived(utxos.reduce((s, u) => s + u.amount_sats, 0))
  let frozenSats    = $derived(
    utxos.filter(u => $frozenUtxos.has(utxoKey(u.txid, u.vout)))
         .reduce((s, u) => s + u.amount_sats, 0)
  )
  let spendableSats = $derived(totalSats - frozenSats)

  // ── Age ───────────────────────────────────────────────────────────────────
  // Age from confirmations (~10 min/block) so it's always a human-readable
  // duration — no dependency on a known chain tip, no raw block-number fallback.
  function coinAge(_blockHeight: number | null, confirmations: number): string {
    if (confirmations === 0) return 'unconfirmed'
    if (confirmations < 6)   return `${confirmations} conf`
    const days = Math.floor(confirmations * 10 / 1440)
    if (days === 0)   return 'today'
    if (days === 1)   return '1 day ago'
    if (days < 30)    return `${days} days ago`
    if (days < 60)    return '1 month ago'
    if (days < 365)   return `${Math.floor(days / 30)} months ago`
    const years = Math.floor(days / 365)
    return years === 1 ? '1 year ago' : `${years} years ago`
  }

  function ageClass(_blockHeight: number | null, confirmations: number): string {
    if (confirmations === 0) return 'age-unconf'
    if (confirmations < 6)   return 'age-new'
    const days = Math.floor(confirmations * 10 / 1440)
    if (days === 0)   return 'age-new'
    if (days >= 180)  return 'age-old'
    return ''
  }

  // ── Format ────────────────────────────────────────────────────────────────
  function formatAmount(sats: number): string {
    if ($balancesHidden) return '•••'
    if ($displayUnit === 'btc') return (sats / 1e8).toFixed(8) + ' BTC'
    return sats.toLocaleString() + ' sats'
  }

  // ── Copy ──────────────────────────────────────────────────────────────────
  let copied = $state<string | null>(null)
  let copyTimer: ReturnType<typeof setTimeout> | null = null
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text)
      copied = text
      if (copyTimer) clearTimeout(copyTimer)
      copyTimer = setTimeout(() => (copied = null), 1500)
    } catch { addToast('Copy failed — clipboard unavailable') }
  }

  // ── Inline label editing ──────────────────────────────────────────────────
  let editingOutpoint = $state<string | null>(null)
  let editValue = $state('')
  const labelTimers = new Map<string, ReturnType<typeof setTimeout>>()

  function startEdit(outpoint: string) {
    editingOutpoint = outpoint
    editValue = $utxoLabels[outpoint] ?? ''
  }

  function onLabelInput(outpoint: string) {
    const t = labelTimers.get(outpoint)
    if (t) clearTimeout(t)
    labelTimers.set(outpoint, setTimeout(async () => {
      const [txid, voutStr] = outpoint.split(':')
      await setUtxoLabel(txid, Number(voutStr), editValue)
      labelTimers.delete(outpoint)
    }, 600))
  }

  // Flush on blur/Enter: cancel any pending debounce and persist the current
  // value immediately (an empty value deletes the note). Without this, clearing
  // a note and leaving the field never sent the delete — the debounce could be
  // cancelled by onDestroy on tab switch, so the note appeared undeletable.
  function commitLabel() {
    const op = editingOutpoint
    if (op) {
      const t = labelTimers.get(op)
      if (t) { clearTimeout(t); labelTimers.delete(op) }
      const [txid, voutStr] = op.split(':')
      void setUtxoLabel(txid, Number(voutStr), editValue)
    }
    editingOutpoint = null
  }

  onDestroy(() => {
    for (const t of labelTimers.values()) clearTimeout(t)
    if (copyTimer) clearTimeout(copyTimer)
  })

  function focusAndSelect(node: HTMLInputElement) { node.focus(); node.select() }

  // ── Freeze ────────────────────────────────────────────────────────────────
  async function toggleFreeze(u: UtxoRecord) {
    const key = utxoKey(u.txid, u.vout)
    try {
      if ($frozenUtxos.has(key)) await unfreezeUtxo(u.txid, u.vout)
      else await freezeUtxo(u.txid, u.vout)
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Failed to update freeze status')
    }
  }

  let bulkFreezing = $state(false)
  async function bulkFreezeDust() {
    if (bulkFreezing) return
    const dustOnes = utxos.filter(u => isFreezeableDust(u) && !$frozenUtxos.has(utxoKey(u.txid, u.vout)))
    if (dustOnes.length === 0) return
    bulkFreezing = true
    try {
      // Sequential to be gentle on the server (each is a small DB write).
      for (const u of dustOnes) {
        await freezeUtxo(u.txid, u.vout)
      }
      addToast(`Froze ${dustOnes.length} dust UTXO${dustOnes.length !== 1 ? 's' : ''}`)
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Failed to freeze dust')
    } finally {
      bulkFreezing = false
    }
  }

  // ── Consolidation selection ───────────────────────────────────────────────
  let selected = $state(new Set<string>())

  $effect(() => { if (!consolidateMode) selected = new Set() })

  let selectedUtxos = $derived(utxos.filter(u => selected.has(utxoKey(u.txid, u.vout))))
  let selectedSats  = $derived(selectedUtxos.reduce((s, u) => s + u.amount_sats, 0))
  let allSelected   = $derived(utxos.length > 0 && utxos.every(u => selected.has(utxoKey(u.txid, u.vout))))

  function toggleAll() {
    selected = allSelected ? new Set() : new Set(utxos.map(u => utxoKey(u.txid, u.vout)))
  }

  function toggleOne(outpoint: string) {
    const u = utxos.find(x => utxoKey(x.txid, x.vout) === outpoint)
    if (u && !u.is_mature) {
      addToast('Immature coinbase outputs can\'t be selected until they reach 100 confirmations.')
      return
    }
    const next = new Set(selected)
    next.has(outpoint) ? next.delete(outpoint) : next.add(outpoint)
    selected = next
  }

  /// Coinbase outputs are spendable only after 100 confirmations. Show how
  /// many more blocks are needed; null once mature or for non-coinbase UTXOs.
  function blocksUntilMature(u: UtxoRecord): number | null {
    if (!u.is_coinbase || u.is_mature) return null
    return Math.max(0, 100 - u.confirmations)
  }
</script>

{#if utxos.length === 0}
  <EmptyState icon="coins" compact title="No coins yet" description="Unspent outputs (your spendable coins) appear here once this wallet receives Bitcoin." />
{:else}
  {#if !consolidateMode && showNudge}
    <div class="nudge">
      <div class="nudge-text">
        {#if attackDustCount > 0}
          {attackDustCount} UTXO{attackDustCount !== 1 ? 's' : ''} may be a dust attack: a tiny output sent to your Silent Payments address to probe or link your wallet. Consider freezing.{economicDustCount > 0 ? ` (${economicDustCount} more cost more in fees than they're worth.)` : ''}
        {:else if dustCount > 0}
          {dustCount} UTXO{dustCount !== 1 ? 's are' : ' is'} dust: worth less than the fee to spend at current rates.{utxos.length >= 10 ? ` You have ${utxos.length} UTXOs total.` : ''}
        {:else}
          {utxos.length} UTXOs — consider consolidating when fees are low to reduce future costs.
        {/if}
      </div>
      <div class="nudge-actions">
        {#if dustCount > 0}
          <button class="nudge-action" onclick={bulkFreezeDust} disabled={bulkFreezing}>
            {bulkFreezing ? 'Freezing…' : `❄ Freeze ${dustCount} dust`}
          </button>
        {/if}
        {#if canConsolidate && utxos.length >= 2}
          <button class="nudge-action nudge-action-primary" onclick={onStartConsolidate}>
            ↕ Consolidate
          </button>
        {/if}
      </div>
    </div>
  {/if}

  <table class="utxo-table">
    <thead>
      <tr>
        {#if consolidateMode}
          <th class="col-check">
            <input type="checkbox" checked={allSelected} onchange={toggleAll} aria-label="Select all UTXOs" />
          </th>
        {/if}
        <th class="col-utxo">UTXO</th>
        {#if !consolidateMode}
          <th class="col-category">Category</th>
          <th class="col-note">Note</th>
        {/if}
        {#if mixedTypes}
          <th class="col-type">Type</th>
        {/if}
        <th class="col-age">
          <button class="th-btn" class:active={sortCol === 'age'} onclick={() => setSort('age')}>
            Age <span class="sort-icon" class:active={sortCol === 'age'}>{sortIcon('age')}</span>
          </button>
        </th>
        <th class="col-amount">
          <button class="th-btn" class:active={sortCol === 'amount'} onclick={() => setSort('amount')}>
            Amount <span class="sort-icon" class:active={sortCol === 'amount'}>{sortIcon('amount')}</span>
          </button>
        </th>
        {#if !consolidateMode}
          <th class="col-actions"></th>
        {/if}
      </tr>
    </thead>

    <tbody>
      {#each sorted as utxo (utxo.txid + ':' + utxo.vout)}
        {@const outpoint  = utxoKey(utxo.txid, utxo.vout)}
        {@const frozen    = $frozenUtxos.has(outpoint)}
        {@const dust      = isDust(utxo)}
        {@const dustAttack = utxo.suspected_dust}
        {@const label     = $utxoLabels[outpoint] ?? addressLabels[utxo.address ?? ''] ?? ''}
        {@const isSelected = selected.has(outpoint)}
        {@const stype     = scriptType(utxo.address)}
        {@const shared    = sharedAddresses.has(utxo.address ?? '')}
        {@const unconf    = utxo.confirmations === 0}
        {@const matureIn  = blocksUntilMature(utxo)}
        {@const immature  = matureIn !== null}
        {@const catId     = $utxoCategories[outpoint] ?? (utxo.address ? $addressCategories[utxo.address] : null) ?? null}
        {@const catInherited = !$utxoCategories[outpoint] && !!catId}
        <tr
          class:is-frozen={frozen}
          class:is-dust={(dust || dustAttack) && !frozen}
          class:is-selected={consolidateMode && isSelected}
          class:is-unconfirmed={unconf}
          class:is-immature={immature}
          class:selectable={consolidateMode && !immature}
          onclick={consolidateMode ? () => toggleOne(outpoint) : undefined}
        >
          {#if consolidateMode}
            <td class="col-check" onclick={(e) => e.stopPropagation()}>
              <input
                type="checkbox"
                checked={isSelected}
                disabled={immature}
                onchange={() => toggleOne(outpoint)}
                aria-label="Select {outpoint}"
                title={immature ? `Immature coinbase — matures in ${matureIn} more block${matureIn === 1 ? '' : 's'}` : undefined}
              />
            </td>
          {/if}

          <!-- UTXO identity -->
          <td class="col-utxo">
            <div class="utxo-identity">
              <div class="addr-row">
                {#if frozen}<span class="freeze-dot" aria-hidden="true">❄</span>{/if}
                {#if utxo.address}
                  <span class="addr" title={utxo.address}>
                    <span class="addr-start">{utxo.address.slice(0, -8)}</span><span class="addr-end">{utxo.address.slice(-8)}</span>
                  </span>
                {:else}
                  <span class="addr">—</span>
                {/if}
                {#if utxo.address && !consolidateMode}
                  <button class="copy-btn" title="Copy address" onclick={() => copy(utxo.address!)}>
                    {copied === utxo.address ? '✓' : '⎘'}
                  </button>
                {/if}
                <!-- "frozen" omitted; the labeled Freeze button shows it. -->
                {#if immature}
                  <span class="sflag sflag-immature" title={`Immature coinbase output — matures in ${matureIn} more block${matureIn === 1 ? '' : 's'} (must reach 100 confirmations to be spendable)`}>matures in {matureIn}b</span>
                {:else if utxo.is_coinbase}
                  <span class="sflag sflag-coinbase" title="Mining reward (coinbase) — fully matured">⛏ mined</span>
                {/if}
                {#if unconf}
                  <span class="sflag sflag-unconf" title="Unconfirmed — not yet included in a block">unconfirmed</span>
                {/if}
                {#if shared}
                  <span class="sflag sflag-reuse" title="Address reuse — spending multiple UTXOs from this address links them on-chain">reuse</span>
                {/if}
                {#if dustAttack}
                  <span class="sflag sflag-dust" title="Tiny output sent to your Silent Payments address: a possible dust attack to probe or link your wallet. Consider freezing.">dust attack?</span>
                {:else if dust}
                  <span class="sflag sflag-dust" title="Dust: costs more in fees to spend than its current value at this fee rate">dust</span>
                {/if}
              </div>
              <div class="outpoint-row">
                {#if $mempoolUrl}
                  <a
                    class="outpoint outpoint-link"
                    href={new URL('/tx/' + utxo.txid, $mempoolUrl).href}
                    target="_blank"
                    rel="noopener noreferrer"
                    title="View transaction in block explorer"
                  >{utxo.txid.slice(0, 8)}…{utxo.txid.slice(-8)}:{utxo.vout}<span class="outpoint-arrow" aria-hidden="true">↗</span></a>
                {:else}
                  <span class="outpoint">{utxo.txid.slice(0, 8)}…{utxo.txid.slice(-8)}:{utxo.vout}</span>
                {/if}
                {#if !consolidateMode}
                  <button class="copy-btn" title="Copy outpoint" onclick={() => copy(outpoint)}>
                    {copied === outpoint ? '✓' : '⎘'}
                  </button>
                {/if}
              </div>
              <!-- Mobile only — desktop has dedicated columns. -->
              <div class="mobile-meta">
                {#if mixedTypes}
                  <span class="type-badge type-{stype}" title={TYPE_TITLE[stype]}>{TYPE_LABEL[stype]}</span>
                {/if}
                <span class="mobile-age {ageClass(utxo.block_height, utxo.confirmations)}">
                  {coinAge(utxo.block_height, utxo.confirmations)}
                </span>
              </div>
              <!-- Note + category inline (mobile only; desktop uses dedicated
                   columns). Mirrors how type/age reflow into .mobile-meta. -->
              {#if !consolidateMode}
                <div class="note-cat-row note-cat-mobile">
                  {#if editingOutpoint === outpoint}
                    <input
                      class="label-input"
                      type="text"
                      bind:value={editValue}
                      oninput={() => onLabelInput(outpoint)}
                      onblur={commitLabel}
                      onkeydown={(e) => e.key === 'Enter' && commitLabel()}
                      placeholder="Add note…"
                      use:focusAndSelect
                    />
                  {:else}
                    <button class="label-btn" class:has-label={!!label} onclick={() => startEdit(outpoint)}>
                      {#if label}
                        <span class="label-text">{label}</span>
                        <span class="label-edit" aria-hidden="true">✎</span>
                      {:else}
                        <span class="label-empty">+ note</span>
                      {/if}
                    </button>
                  {/if}
                  <CategoryPicker current={catId} inherited={catInherited} onSelect={(id) => assignUtxoCategory(utxo.txid, utxo.vout, id)} />
                </div>
              {:else}
                {#if label}<span class="label-text">{label}</span>{/if}
                {#if catId}<CategoryChip categoryId={catId} />{/if}
              {/if}
            </div>
          </td>

          <!-- Category + Note (desktop columns; hidden on mobile, shown inline above) -->
          {#if !consolidateMode}
            <td class="col-category">
              <CategoryPicker current={catId} inherited={catInherited} onSelect={(id) => assignUtxoCategory(utxo.txid, utxo.vout, id)} />
            </td>
            <td class="col-note">
              {#if editingOutpoint === outpoint}
                <input
                  class="label-input"
                  type="text"
                  bind:value={editValue}
                  oninput={() => onLabelInput(outpoint)}
                  onblur={commitLabel}
                  onkeydown={(e) => e.key === 'Enter' && commitLabel()}
                  placeholder="Add note…"
                  use:focusAndSelect
                />
              {:else}
                <button class="label-btn" class:has-label={!!label} onclick={() => startEdit(outpoint)}>
                  {#if label}
                    <span class="label-text">{label}</span>
                    <span class="label-edit" aria-hidden="true">✎</span>
                  {:else}
                    <span class="label-empty">+ note</span>
                  {/if}
                </button>
              {/if}
            </td>
          {/if}

          <!-- Type (desktop only, mixed-type wallets only) -->
          {#if mixedTypes}
            <td class="col-type">
              <span class="type-badge type-{stype}" title={TYPE_TITLE[stype]}>{TYPE_LABEL[stype]}</span>
            </td>
          {/if}

          <!-- Age (desktop only) -->
          <td class="col-age">
            <span class={ageClass(utxo.block_height, utxo.confirmations)}>
              {coinAge(utxo.block_height, utxo.confirmations)}
            </span>
          </td>

          <!-- Amount -->
          <td class="col-amount">
            <span class="amount">{formatAmount(utxo.amount_sats)}</span>
          </td>

          {#if !consolidateMode}
            <td class="col-actions">
              <button
                class="freeze-btn"
                class:frozen
                onclick={() => toggleFreeze(utxo)}
                aria-pressed={frozen}
                aria-label={frozen ? `Unfreeze ${outpoint}` : `Freeze ${outpoint}`}
                title={frozen
                  ? 'Frozen — this UTXO will not be selected when sending. Click to unfreeze.'
                  : 'Freeze — mark this UTXO as unspendable to keep it out of future transactions.'}
              ><span class="freeze-icon" aria-hidden="true">❄</span>{frozen ? 'Frozen' : 'Freeze'}</button>
            </td>
          {/if}
        </tr>
      {/each}
    </tbody>

    <tfoot>
      <tr class="total-row">
        {#if consolidateMode}<td></td>{/if}
        <td class="col-utxo">
          {#if canConsolidate && utxos.length >= 2}
            <button class="footer-action" onclick={onStartConsolidate}>
              ↕ Consolidate {utxos.length} UTXOs
            </button>
          {:else}
            <span class="total-label">{utxos.length} UTXO{utxos.length !== 1 ? 's' : ''}</span>
          {/if}
        </td>
        {#if !consolidateMode}<td class="col-category"></td><td class="col-note"></td>{/if}
        {#if mixedTypes}<td class="col-type"></td>{/if}
        <td class="col-age"></td>
        <td class="col-amount">
          <span class="total-amount">{formatAmount(totalSats)}</span>
          {#if frozenSats > 0}
            <span class="spendable">{formatAmount(spendableSats)} spendable</span>
          {/if}
        </td>
        {#if !consolidateMode}<td></td>{/if}
      </tr>
    </tfoot>
  </table>

  {#if consolidateMode}
    <div class="consolidate-bar">
      <span class="bar-info">
        {#if selected.size >= 2}
          {selected.size} selected · {$balancesHidden ? '•••' : `${selectedSats.toLocaleString()} sats`}
        {:else if selected.size === 1}
          1 selected — pick at least one more
        {:else}
          Select UTXOs to consolidate
        {/if}
      </span>
      <div class="bar-actions">
        <button class="bar-cancel" onclick={onCancelConsolidate}>Cancel</button>
        <button
          class="bar-continue"
          disabled={selected.size < 2}
          onclick={() => onConsolidate(selectedUtxos)}
        >Continue →</button>
      </div>
    </div>
  {/if}
{/if}

<style>
  .empty { color: var(--text-muted); font-size: 0.9rem; padding: 24px 0; }

  .nudge {
    background: color-mix(in srgb, var(--accent) 6%, var(--surface-2));
    border: 1px solid color-mix(in srgb, var(--accent) 20%, var(--border));
    border-radius: 6px; padding: 9px 12px;
    font-size: 0.78rem; color: var(--text-muted);
    margin-bottom: 14px; line-height: 1.5;
    display: flex; align-items: center; gap: 12px; flex-wrap: wrap;
  }
  .nudge-text { flex: 1; min-width: 0; }
  .nudge-actions { display: flex; gap: 6px; flex-wrap: wrap; }
  .nudge-action {
    background: var(--surface-1); border: 1px solid var(--border);
    color: #52a8d4; padding: 5px 10px; border-radius: 4px;
    font-size: 0.74rem; cursor: pointer; white-space: nowrap;
    flex-shrink: 0;
  }
  .nudge-action:hover:not(:disabled) { border-color: #52a8d4; }
  .nudge-action:disabled { opacity: 0.5; cursor: not-allowed; }
  .nudge-action-primary {
    color: var(--accent);
  }
  .nudge-action-primary:hover:not(:disabled) { border-color: var(--accent); }

  /* ── Table ── */
  .utxo-table { width: 100%; border-collapse: collapse; }

  thead th {
    text-align: left; padding: 0 10px 8px 0;
    border-bottom: 1px solid var(--border);
    color: var(--text-muted); font-size: 0.72rem; font-weight: 700;
    text-transform: uppercase; letter-spacing: 0.06em; white-space: nowrap;
  }
  thead th:last-child { padding-right: 0; }

  .th-btn {
    background: none; border: none; cursor: pointer;
    color: var(--text-muted); font-size: 0.72rem; font-weight: 700;
    text-transform: uppercase; letter-spacing: 0.06em;
    padding: 0; display: flex; align-items: center; gap: 3px;
    transition: color 0.12s;
  }
  .th-btn:hover { color: var(--text); text-decoration: underline; text-underline-offset: 3px; }
  .th-btn.active { color: var(--text); }
  .sort-icon { font-size: 0.7rem; opacity: 0.4; }
  .sort-icon.active { opacity: 1; color: var(--accent); }

  /* ── Column widths ── */
  .col-check   { width: 32px; padding-right: 8px; }
  .col-utxo    { width: auto; }
  .col-category { width: 120px; vertical-align: middle; }
  .col-note     { width: 340px; vertical-align: middle; }
  .col-note .label-btn { width: 100%; }
  .col-type    { width: 90px; }
  .col-age     { width: 110px; text-align: right; white-space: nowrap; }
  .col-age .th-btn { display: inline-flex; margin-left: auto; }
  .col-amount  { width: 130px; text-align: right; white-space: nowrap; }
  .col-actions { width: 96px; text-align: right; white-space: nowrap; }

  /* ── Rows ── */
  tbody tr {
    border-bottom: 1px solid var(--border);
    border-left: 3px solid transparent;
    transition: background 0.1s;
  }
  tbody tr.is-frozen {
    border-left: 3px solid #52a8d4;
    background: color-mix(in srgb, #52a8d4 4%, transparent);
  }
  tbody tr.is-unconfirmed {
    border-left: 3px dashed color-mix(in srgb, #e05252 50%, transparent);
  }
  tbody tr.is-frozen.is-unconfirmed { border-left: 3px solid #52a8d4; }
  tbody tr.is-dust { opacity: 0.65; }
  tbody tr.is-immature {
    border-left: 3px solid #e09c52;
    background: color-mix(in srgb, #e09c52 4%, transparent);
  }
  tbody tr.is-selected {
    border-left-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 5%, transparent);
  }
  tbody tr.selectable { cursor: pointer; }
  tbody tr.selectable:hover { background: var(--surface-hover); }
  tbody tr.selectable.is-selected:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }

  tbody td {
    padding: 10px 10px 10px 0;
    vertical-align: middle;
  }
  tbody td:first-child { padding-left: 4px; }
  tbody td:last-child  { padding-right: 0; }

  /* ── Checkbox column ── */
  .col-check input[type="checkbox"] { cursor: pointer; accent-color: var(--accent); }

  /* ── UTXO identity ── */
  .utxo-identity { display: flex; flex-direction: column; gap: 2px; }
  .addr-row, .outpoint-row { display: flex; align-items: center; gap: 5px; min-width: 0; flex-wrap: wrap; }
  .freeze-dot { font-size: 0.72rem; color: #52a8d4; flex-shrink: 0; }
  /* Middle-truncation: last 8 chars stay pinned. */
  .addr {
    font-family: monospace; font-size: 0.82rem; color: var(--text);
    display: inline-flex; max-width: 220px; min-width: 0;
  }
  .addr .addr-start {
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    min-width: 0; flex-shrink: 1;
  }
  .addr .addr-end { flex-shrink: 0; white-space: nowrap; }
  .outpoint { font-family: monospace; font-size: 0.72rem; color: var(--text-muted); }

  .sflag {
    font-size: 0.6rem; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.04em; padding: 2px 4px; border-radius: 3px;
    white-space: nowrap; cursor: default;
  }
  .sflag-unconf { background: color-mix(in srgb, #e05252 15%, transparent); color: #e05252; }
  .sflag-reuse  { background: color-mix(in srgb, #e09c52 15%, transparent); color: #e09c52; }
  .sflag-dust   { background: color-mix(in srgb, #888 15%, transparent); color: var(--text-muted); }
  .sflag-immature  { background: color-mix(in srgb, #e09c52 18%, transparent); color: #e09c52; }
  .sflag-coinbase  { background: color-mix(in srgb, #a87fd4 15%, transparent); color: #a87fd4; }

  .copy-btn {
    background: none; border: 1px solid var(--border); border-radius: 3px;
    color: var(--text-muted); cursor: pointer; font-size: 0.72rem;
    padding: 1px 4px; flex-shrink: 0; line-height: 1.4;
  }
  .copy-btn:hover { color: var(--text); }

  /* Mobile: type + age shown inline, desktop columns hidden */
  .mobile-meta { display: none; }

  /* ── Label ── */
  /* Inline note+category is the mobile-only fallback; desktop uses the dedicated
     Category/Note columns. Hidden here, shown in the mobile media query. */
  .note-cat-row { display: none; align-items: center; gap: 8px; margin-top: 3px; flex-wrap: wrap; }
  .label-btn {
    background: none; border: none; cursor: pointer; padding: 0;
    display: flex; align-items: center; gap: 4px;
  }
  .label-empty { font-size: 0.72rem; color: var(--text-muted); opacity: 0.7; }
  .label-btn:hover .label-empty { color: var(--accent); opacity: 1; }
  .label-text { font-size: 0.75rem; color: var(--text); }
  .label-edit { font-size: 0.65rem; color: var(--text-muted); }
  .label-input {
    background: var(--surface-2); border: 1px solid var(--accent); border-radius: 3px;
    color: var(--text); padding: 2px 6px; font-size: 0.75rem; width: 180px; outline: none;
  }

  /* ── Type column ── */
  .col-type { vertical-align: middle; }
  .type-badge {
    display: inline-block;
    font-size: 0.62rem; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.04em; padding: 2px 5px; border-radius: 3px;
    white-space: nowrap; cursor: default;
  }
  .type-p2wpkh { background: color-mix(in srgb, #52a8d4 15%, transparent); color: #52a8d4; }
  .type-taproot { background: color-mix(in srgb, #a87fd4 15%, transparent); color: #a87fd4; }
  .type-p2sh    { background: color-mix(in srgb, #e09c52 15%, transparent); color: #e09c52; }
  .type-legacy  { background: color-mix(in srgb, #e05252 15%, transparent); color: #e05252; }
  .type-unknown { background: var(--surface-2); color: var(--text-muted); }

  /* ── Age column ── */
  .col-age { font-size: 0.78rem; font-weight: 600; }
  .age-unconf { color: #e05252; }
  .age-new    { color: #e09c52; }
  .age-old    { color: #52a875; }

  /* ── Amount column ── */
  .col-amount { display: table-cell; }
  .col-amount > * { display: inline; }
  .amount { font-size: 0.88rem; font-weight: 600; font-variant-numeric: tabular-nums; }

  /* ── Freeze button ── */
  .freeze-btn {
    display: inline-flex; align-items: center; gap: 5px;
    background: none; border: 1px solid var(--border); border-radius: 4px;
    color: var(--text-muted); cursor: pointer;
    font-size: 0.72rem; font-weight: 500;
    padding: 3px 8px; line-height: 1.2;
    transition: border-color 0.12s, color 0.12s, background 0.12s;
  }
  .freeze-icon { font-size: 0.78rem; line-height: 1; }
  .freeze-btn:hover { border-color: #52a8d4; color: #52a8d4; }
  .freeze-btn:focus-visible {
    outline: 2px solid var(--accent); outline-offset: 1px;
  }
  .freeze-btn.frozen {
    border-color: #52a8d4; color: #52a8d4;
    background: color-mix(in srgb, #52a8d4 10%, transparent);
  }

  /* Outpoint doubles as the block-explorer link. */
  .outpoint-link {
    color: var(--text-muted); text-decoration: none;
    border-radius: 2px;
  }
  .outpoint-link:hover { color: var(--text); }
  .outpoint-link:hover .outpoint-arrow { opacity: 1; }
  .outpoint-arrow {
    margin-left: 3px; font-size: 0.65rem;
    opacity: 0; transition: opacity 0.12s;
  }

  /* ── Footer ──
     Padding-right matches body so the Amount column aligns top-to-bottom. */
  .total-row td {
    padding: 8px 10px 4px 0;
    border-top: 1px solid var(--border);
  }
  .total-row td:first-child { padding-left: 4px; }
  .total-row .col-amount { padding-right: 10px; }
  .total-label  { font-size: 0.72rem; color: var(--text-muted); }
  .footer-action {
    background: none; border: none; cursor: pointer;
    color: var(--accent); font-size: 0.74rem; font-weight: 500;
    padding: 0;
  }
  .footer-action:hover { text-decoration: underline; }
  .total-amount { font-size: 0.82rem; font-weight: 700; color: var(--text); font-variant-numeric: tabular-nums; }
  .spendable {
    display: block; font-size: 0.7rem; color: var(--text-muted);
    font-variant-numeric: tabular-nums; margin-top: 1px;
  }

  /* ── Consolidate bar ── */
  .consolidate-bar {
    position: sticky; bottom: 0;
    display: flex; align-items: center; justify-content: space-between; gap: 12px;
    background: var(--surface-1); border-top: 1px solid var(--border);
    padding: 10px 0; margin-top: 2px;
  }
  .bar-info { font-size: 0.82rem; color: var(--text-muted); }
  .bar-actions { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
  .bar-cancel {
    background: none; border: none; cursor: pointer;
    font-size: 0.82rem; color: var(--text-muted); padding: 4px 8px;
  }
  .bar-cancel:hover { color: var(--text); }
  .bar-continue {
    background: var(--accent); color: #000; border: none; border-radius: 5px;
    padding: 6px 16px; cursor: pointer; font-weight: 600; font-size: 0.82rem;
    transition: opacity 0.12s;
  }
  .bar-continue:hover:not(:disabled) { opacity: 0.88; }
  .bar-continue:disabled { opacity: 0.35; cursor: not-allowed; }

  /* ── Mobile ── */
  @media (max-width: 640px) {
    /* Hide dedicated columns on mobile; type + age render inline in the
       UTXO cell via .mobile-meta. Status flags already render inline on
       all viewports via .utxo-flags. */
    .col-type, .col-age, .col-category, .col-note { display: none; }
    .note-cat-row { display: flex; }
    .mobile-meta {
      display: flex; align-items: center; gap: 6px; margin-top: 3px;
    }
    .mobile-age {
      font-size: 0.7rem; font-weight: 600; color: var(--text-muted);
    }
    .mobile-age.age-unconf { color: #e05252; }
    .mobile-age.age-new    { color: #e09c52; }
    .mobile-age.age-old    { color: #52a875; }

    /* Tighten layout on small screens */
    .addr { max-width: 160px; font-size: 0.78rem; }
    .col-amount { width: 100px; }
    .col-actions { width: 88px; }
    tbody td { padding: 8px 8px 8px 0; }

    /* Consolidate bar stacks on very small screens */
    .consolidate-bar { flex-wrap: wrap; gap: 8px; }
    .bar-info { flex: 1 1 100%; }
    .bar-actions { width: 100%; justify-content: flex-end; }
  }
</style>
