<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { api } from '../../lib/api'
  import { downloadBlob, psbtBlob, bytesToBase64 } from '../../lib/utils'
  import { parseBip21, amountToSats as amountStrToSats, btcToSats, satsToBtcString } from '../../lib/amount'
  import { uriHasPayjoin, looksLikeHrn, addressFromScan, bip21AmountToField } from '../../lib/send'
  import type { DecodedTx, MempoolBlock, MultisigDetails, PayjoinProposalDiff, PayjoinStatusKind, SendResult, UtxoRecord, WalletEntry } from '../../lib/types'
  import { displayUnit, mempoolUrl, showFiatBalance, currentBtcPrice, nodeStatus, offline, hwEnabled } from '../../stores/settings'
  import { lastPayjoinEvent } from '../../stores/payjoin'
  import { addToast } from '../../stores/toasts'
  import { wallets } from '../../stores/wallets'
  import { utxoKey, swallow } from '../../lib/utils'
  import HelpLink from '../HelpLink.svelte'
  import QrSignFlow from '../QrSignFlow.svelte'
  import QrScanModal from '../QrScanModal.svelte'
  import RecipientRow from './RecipientRow.svelte'
  import FeePicker from './FeePicker.svelte'
  import CoinControlPanel from './CoinControlPanel.svelte'
  import TxFlowDiagram from '../TxFlowDiagram.svelte'

  interface FeeRates { fastestFee: number; halfHourFee: number; hourFee: number }
  let {
    wallet,
    utxos,
    feeRates,
    addressLabels = {},
    onClose,
  }: {
    wallet: WalletEntry
    utxos: UtxoRecord[]
    feeRates: FeeRates | null
    addressLabels?: Record<string, string>
    onClose: () => void
  } = $props()

  let dialogEl = $state<HTMLDialogElement | null>(null)
  // Element to restore focus to when the dialog closes (the trigger that opened it).
  let returnFocus: HTMLElement | null = null
  function onBackdrop(e: MouseEvent) { if (e.target === dialogEl) onClose() }

  // Two-step modal: 'compose' (recipients/fee/coin control) → 'review' (preview +
  // sign). The preview builds live regardless of step, so advancing is instant.
  let step = $state<'compose' | 'review'>('compose')

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

  // ── Recipients ─────────────────────────────────────────────────────────────
  // Each row is one PSBT output. The UI starts with a single row and the user
  // can add more. At most one row may be marked `sendMax` — that row receives
  // everything left over after the other recipients' fixed amounts + fees.
  interface Recipient {
    address: string
    amount: string
    sendMax: boolean
    // Set when the address was filled by picking one of the user's own wallets,
    // so we can show a "sending to yourself" chip. Cleared on manual edit/paste.
    fromWallet?: string
    // Full BIP-21 URI when the pasted invoice carried a `pj=` payjoin endpoint.
    // Cleared on manual edit / wallet-pick.
    payjoinUri?: string
  }
  let recipients = $state<Recipient[]>([{ address: '', amount: '', sendMax: false }])

  // ── Spending path (policy/vault wallets) ────────────────────────────────────
  // For a vault, the recovery branch is timelocked; the user picks which branch
  // to spend through. Only shown when the wallet's policy has a timelock.
  let vaultPolicy = $state<{ policy: string; timelocks: { kind: string; value: number; blocks: boolean; label: string }[]; requires_path: boolean } | null>(null)
  let spendPath = $state<'primary' | 'recovery'>('primary')
  // Vault = a spending CHOICE (primary OR timelocked recovery) → show selector.
  // Savings = a single timelocked path → no selector, gate the whole send.
  let isVault = $derived(!!vaultPolicy && vaultPolicy.timelocks.length > 0 && vaultPolicy.requires_path)
  let isSavings = $derived(!!vaultPolicy && vaultPolicy.timelocks.length > 0 && !vaultPolicy.requires_path)
  let isPolicyWallet = $derived(wallet.kind === 'descriptor')
  let timelock = $derived(vaultPolicy?.timelocks?.[0] ?? null)
  let recoveryTimelockLabel = $derived(timelock?.label ?? '')

  // Per-UTXO maturity for the timelocked path. `null` = can't pre-judge
  // (time-based lock) → don't gate. Mirrors core::timelock_spendable.
  function utxoTimelockOk(u: UtxoRecord): boolean | null {
    if (!timelock) return true
    if (timelock.kind === 'relative' && timelock.blocks) return u.confirmations >= timelock.value
    if (timelock.kind === 'absolute' && timelock.blocks) return ($nodeStatus?.tip_height ?? 0) >= timelock.value
    return null
  }
  // Coins spendable via the timelocked path right now (matured or un-gateable).
  let timelockMaturedCount = $derived(utxos.filter((u) => utxoTimelockOk(u) !== false).length)
  let timelockPathSpendable = $derived(timelockMaturedCount > 0)
  // Soonest unlock among locked coins (relative block timelocks only).
  let soonestUnlockBlocks = $derived.by(() => {
    if (!timelock || timelock.kind !== 'relative' || !timelock.blocks) return null
    const locked = utxos.filter((u) => u.confirmations < timelock.value)
    if (locked.length === 0) return null
    return Math.min(...locked.map((u) => timelock.value - u.confirmations))
  })
  // The send is blocked by the timelock when the chosen path is the timelocked
  // one and nothing has matured: savings always, or a vault with recovery picked.
  let timelockBlocked = $derived(
    (isSavings && !timelockPathSpendable) ||
    (isVault && spendPath === 'recovery' && !timelockPathSpendable)
  )
  let lockHint = $derived.by(() => {
    if (!timelock) return ''
    if (timelock.kind === 'absolute' && timelock.blocks) return `unlocks at block ${timelock.value}`
    if (soonestUnlockBlocks != null) return `earliest unlocks in ~${soonestUnlockBlocks} block${soonestUnlockBlocks === 1 ? '' : 's'}`
    return timelock.label
  })

  // ── Send to one of my own wallets ───────────────────────────────────────────
  // Picker per recipient row. Excludes the source wallet (self-send is the
  // consolidation flow). Fetches a fresh unused receive address for the target.
  let walletPickerOpenIdx = $state<number | null>(null)
  let otherWallets = $derived($wallets.filter((w) => w.id !== wallet.id))

  async function addressForWallet(w: WalletEntry): Promise<string> {
    // SP and watch-only-address wallets have a single static address in `input`.
    if (w.kind === 'address' || w.kind === 'silent_payments') return w.input
    const all = await api.wallets.addresses(w.id)
    const unused = all.filter((a) => a.kind === 'external' && !a.used).sort((a, b) => a.index - b.index)
    if (unused.length) return unused[0].address
    // Everything used — fall back to the highest-index external address.
    const ext = all.filter((a) => a.kind === 'external').sort((a, b) => b.index - a.index)
    return ext[0]?.address ?? ''
  }

  async function pickWallet(i: number, w: WalletEntry) {
    walletPickerOpenIdx = null
    try {
      const addr = await addressForWallet(w)
      if (!addr) { addToast(`Couldn't get a receive address for ${w.label}`); return }
      const next = [...recipients]
      next[i] = { ...next[i], address: addr, fromWallet: w.label }
      recipients = next
      activeRecipientIdx = i
      clearHrnAt(i)
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Failed to fetch address')
    }
  }

  function onRecipientInput(i: number) {
    clearHrnAt(i)
    // A manual edit means it's no longer the address we auto-filled, nor the
    // pasted payjoin invoice.
    if (recipients[i].fromWallet || recipients[i].payjoinUri) {
      const next = [...recipients]
      next[i] = { ...next[i], fromWallet: undefined, payjoinUri: undefined }
      recipients = next
    }
  }
  // Index of the recipient row currently driving the QR scanner / paste-from-
  // clipboard / etc. helpers. Set when one of those buttons is invoked from a
  // specific row.
  let activeRecipientIdx = $state(0)

  function addRecipient() {
    recipients = [...recipients, { address: '', amount: '', sendMax: false }]
  }
  function removeRecipient(i: number) {
    if (recipients.length <= 1) return
    recipients = recipients.filter((_, idx) => idx !== i)
    // Ensure active index stays in bounds.
    if (activeRecipientIdx >= recipients.length) activeRecipientIdx = recipients.length - 1
  }
  function setSendMax(i: number, val: boolean) {
    // At most one row may be send-max; turning one on clears the others.
    recipients = recipients.map((r, idx) => ({
      ...r,
      sendMax: idx === i ? val : (val ? false : r.sendMax),
      amount: idx === i && val ? '' : r.amount,
    }))
  }

  /// Quick shape check for a BIP-352 Silent Payment address. The bech32m
  /// HRPs are `sp` (mainnet), `tsp` (testnet / signet), `sprt` (regtest).
  /// Used to surface a recognizable badge on the recipient row; the backend
  /// does full bech32m validation.
  // Pure address/URI/format helpers now live in ../lib/send (unit-tested).

  function onRecipientPaste(i: number, e: ClipboardEvent) {
    const text = e.clipboardData?.getData('text') ?? ''
    handlePastedRecipient(i, text, e)
  }

  function handlePastedRecipient(i: number, text: string, e?: ClipboardEvent) {
    const parsed = parseBip21(text)
    const next = [...recipients]
    if (parsed) {
      e?.preventDefault()
      const payjoinUri = uriHasPayjoin(text) ? text.trim() : undefined
      next[i] = { ...next[i], address: parsed.address, fromWallet: undefined, payjoinUri }
      if (parsed.amount) {
        next[i].amount = bip21AmountToField(parsed.amount, localUnit)
        next[i].sendMax = false
      }
      recipients = next
    } else if (!e) {
      // Programmatic paste (button click) — set the recipient even if not BIP21
      next[i] = { ...next[i], address: text.trim(), fromWallet: undefined }
      recipients = next
    }
  }

  async function pasteFromClipboard(i: number) {
    try {
      const text = await navigator.clipboard.readText()
      if (text) handlePastedRecipient(i, text)
    } catch {
      addToast('Clipboard read failed — paste manually with Ctrl+V')
    }
  }

  // ── BIP-353 human-readable names (₿user@domain → bitcoin: URI) ──────────────
  let hrnStatus = $state<Record<number, { hrn?: string; resolving?: boolean; error?: string }>>({})

  /// Resolve an HRN typed into recipient row `i` via DNSSEC-validated lookup,
  /// then fill in the on-chain / Silent Payment address it points to.
  async function resolveHrnAt(i: number) {
    const val = recipients[i]?.address.trim() ?? ''
    if (!looksLikeHrn(val)) return
    hrnStatus = { ...hrnStatus, [i]: { resolving: true } }
    try {
      const r = await api.resolveName(val)
      const parsed = parseBip21(r.uri)
      const addr = parsed?.address?.trim() ?? ''
      if (!addr) {
        hrnStatus = { ...hrnStatus, [i]: { error: `${r.hrn} only has a Lightning offer, which Corvin can't pay.` } }
        return
      }
      const next = [...recipients]
      next[i] = { ...next[i], address: addr }
      if (parsed?.amount) {
        const sats = btcToSats(parsed.amount)
        if (sats !== null && sats > 0) {
          next[i].amount = bip21AmountToField(parsed.amount, localUnit)
          next[i].sendMax = false
        }
      }
      recipients = next
      hrnStatus = { ...hrnStatus, [i]: { hrn: r.hrn } }
    } catch (e) {
      hrnStatus = { ...hrnStatus, [i]: { error: e instanceof Error ? e.message : 'Name resolution failed' } }
    }
  }

  function clearHrnAt(i: number) {
    if (hrnStatus[i]) hrnStatus = { ...hrnStatus, [i]: {} }
  }


  // ── Address QR scanner ────────────────────────────────────────────────────
  let addrScanOpen = $state(false)

  function applyScannedAddress(raw: string) {
    const x = addressFromScan(raw)
    if (!x) return
    const i = Math.min(activeRecipientIdx, recipients.length - 1)
    const next = [...recipients]
    next[i] = { ...next[i], address: x.address }
    if (x.amount) {
      next[i].amount = bip21AmountToField(x.amount, localUnit)
      next[i].sendMax = false
    }
    recipients = next
    addrScanOpen = false
  }

  // ── Amount ────────────────────────────────────────────────────────────────
  // Per-modal unit override so the user can pick sats here without changing
  // their global default. Initialized from the global setting.
  let localUnit = $state<'btc' | 'sats'>($displayUnit)

  /// Convert a single recipient row's typed amount to sats using the current
  /// display unit. Returns 0 for empty / invalid / non-positive input.
  /// Delegates to the pure, tested helper in ../lib/amount.
  function amountToSats(amount: string): number {
    return amountStrToSats(amount, localUnit)
  }

  let totalFixedSats = $derived(
    recipients.reduce((s, r) => s + (r.sendMax ? 0 : amountToSats(r.amount)), 0)
  )

  // Fiat estimate for the total being sent across all fixed-amount recipients.
  // Send-max rows aren't counted since we don't know what they'll receive until
  // after the PSBT is built.
  let amountFiat = $derived.by((): string | null => {
    if (!$showFiatBalance || !$currentBtcPrice || totalFixedSats <= 0) return null
    const usd = (totalFixedSats / 1e8) * $currentBtcPrice
    if (usd < 0.01) return null
    return `≈ $${usd.toLocaleString(undefined, { maximumFractionDigits: 2 })}`
  })

  function setLocalUnit(u: 'btc' | 'sats') {
    if (u === localUnit) return
    // Preserve typed amounts when switching units — translate every row through
    // sats (exact, no float drift) via the tested helpers.
    const from = localUnit
    recipients = recipients.map((r) => {
      const sats = amountStrToSats(r.amount, from)
      if (sats <= 0) return r
      const next = u === 'btc' ? satsToBtcString(sats) : sats.toString()
      return { ...r, amount: next }
    })
    localUnit = u
  }

  /// Number of rows currently marked send-max. The UI guarantees at most one
  /// via setSendMax(), but defensive validation in canPreview still checks.
  let sendMaxCount = $derived(recipients.filter(r => r.sendMax).length)

  // ── Coin control ──────────────────────────────────────────────────────────
  let coinControlEnabled = $state(false)
  let selected = $state(new Set<string>())

  $effect(() => { if (!coinControlEnabled) selected = new Set() })

  // Used by the preview/build + send-max hint; selection itself lives in
  // CoinControlPanel (which binds `selected` + `coinControlEnabled`).
  let selectedUtxos = $derived(utxos.filter(u => selected.has(utxoKey(u.txid, u.vout))))
  let selectedSats  = $derived(selectedUtxos.reduce((s, u) => s + u.amount_sats, 0))

  // ── Preview ───────────────────────────────────────────────────────────────
  let previewLoading = $state(false)
  let previewError   = $state('')
  let preview        = $state<SendResult | null>(null)
  let previewTimer: ReturnType<typeof setTimeout> | null = null

  // The seed is needed to *build/sign* an SP send, but not to *estimate* the
  // fee — the backend computes coin selection + fee from a placeholder PSBT
  // before the seed is used. So a 12+ word seed gates the real build only.
  let spSeedReady = $derived.by(() => spMnemonic.trim().split(/\s+/).filter(Boolean).length >= 12)

  let canPreview = $derived.by(() => {
    if (recipients.length === 0) return false
    if (sendMaxCount > 1) return false
    for (const r of recipients) {
      if (r.address.trim().length === 0) return false
      if (!r.sendMax && amountToSats(r.amount) <= 0) return false
    }
    // SP send-max isn't supported in v1 (the crate needs explicit per-recipient
    // amounts to pick output indexes deterministically).
    if (hasSpRecipient && recipients.some(r => r.sendMax)) return false
    // Timelocked path with no matured coins → don't even try to build.
    if (timelockBlocked) return false
    return true
  })

  // True when the current preview is a fee-only estimate (SP send with no seed
  // yet) rather than a real, broadcastable build. Gates the signed-PSBT
  // promotion so we never try to broadcast an empty placeholder.
  let previewIsEstimate = $state(false)

  let coinControlList = $derived(
    coinControlEnabled ? Array.from(selected) : undefined
  )

  // Snapshot the recipients into a primitive-shaped value so the preview
  // effect re-runs on row edits without depending on the proxied array
  // reference (which can stay stable through mutations).
  let previewOutputs = $derived.by(() =>
    recipients.map(r => ({
      recipient: r.address.trim(),
      amount_sats: r.sendMax ? null : amountToSats(r.amount),
      sendMax: r.sendMax,
    }))
  )

  $effect(() => {
    const outs = previewOutputs
    const rate = effectiveFeeRate
    const cc = coinControlList
    const ok = canPreview
    // Track SP-mode inputs so retyping the mnemonic re-triggers the preview.
    const _mnem = spMnemonic
    const _pass = spPassphrase
    const _sp = hasSpRecipient
    const _path = spendPath
    void _mnem; void _pass; void _sp; void _path;

    // Freeze the build once signing begins: a background fee-rate poll (effectiveFeeRate
    // tracks the polled feeRates) must not rebuild and silently invalidate a PSBT that is
    // being or has been signed. Change the fee by cancelling back to compose.
    if (txPhase !== 'compose') return

    if (previewTimer) clearTimeout(previewTimer)
    preview = null
    previewError = ''

    if (!ok) return

    previewTimer = setTimeout(async () => {
      previewLoading = true
      try {
        const payload = {
          outputs: outs.map(o => ({
            recipient: o.recipient,
            amount_sats: o.sendMax ? null : o.amount_sats,
          })),
          fee_rate_sat_vb: rate,
          utxos: cc,
        }
        if (hasSpRecipient) {
          // Seedless fee estimate until the mnemonic is entered; then the real
          // (signed) build. estimate_only returns an empty PSBT we must not
          // promote to a broadcastable one.
          const estimate = !spSeedReady
          preview = await api.wallets.spSend(wallet.id, {
            ...payload,
            estimate_only: estimate,
            mnemonic: estimate ? undefined : spMnemonic.trim(),
            passphrase: estimate ? undefined : (spPassphrase || undefined),
          })
          previewIsEstimate = estimate
        } else {
          preview = await api.wallets.sendPsbt(wallet.id, { ...payload, spend_path: spendPath })
          previewIsEstimate = false
        }
        previewError = ''
      } catch (e) {
        previewError = e instanceof Error ? e.message : 'Failed to build transaction'
        preview = null
      } finally {
        previewLoading = false
      }
    }, 400)
  })

  let copyTimer: ReturnType<typeof setTimeout> | null = null
  onDestroy(() => {
    if (previewTimer) clearTimeout(previewTimer)
    if (copyTimer) clearTimeout(copyTimer)
    hwEs?.close()
    stopPjPolling()
    returnFocus?.focus?.()
    returnFocus = null
  })

  // ── Draft persistence ─────────────────────────────────────────────────────
  // Save recipient/amount/unit/fee/coin-control so an accidental Esc or
  // refresh doesn't destroy a half-composed send. Cleared on successful
  // broadcast or when user manually clears via "Reset draft".
  const draftKey = $derived(`corvin:send-draft:${wallet.id}`)
  // Guard so the autosave effect doesn't write an empty draft to localStorage
  // before we've had a chance to restore the saved one. Without this the first
  // effect-flush would clobber any persisted draft with the modal's initial
  // (empty) state.
  let draftLoaded = $state(false)

  function saveDraft() {
    if (!browserHasLocalStorage()) return
    if (broadcastTxid) return // never persist after a successful broadcast
    try {
      localStorage.setItem(draftKey, JSON.stringify({
        // Recipient ADDRESSES are intentionally not persisted: localStorage lives in
        // the (unencrypted) webview profile, outside at-rest encryption, and the
        // counterparty address is the most privacy-sensitive field. Keep only the
        // non-identifying parts; the user re-enters the address on restore.
        recipients: recipients.map((r) => ({ amount: r.amount, sendMax: r.sendMax })),
        localUnit,
        feePreset,
        customFeeRate,
        coinControlEnabled,
        selected: Array.from(selected),
      }))
    } catch {}
  }
  function loadDraft() {
    if (!browserHasLocalStorage()) return
    try {
      const raw = localStorage.getItem(draftKey)
      if (!raw) return
      const d = JSON.parse(raw)
      if (Array.isArray(d.recipients) && d.recipients.length > 0) {
        const restored: Recipient[] = []
        for (const r of d.recipients) {
          // Address is never restored from a draft (it isn't persisted anymore, and
          // any address in an older draft is dropped here too). Amount/sendMax only.
          if (typeof r?.amount === 'string' && typeof r?.sendMax === 'boolean') {
            restored.push({ address: '', amount: r.amount, sendMax: r.sendMax })
          }
        }
        if (restored.length > 0) {
          // Defensively cap to one send-max row (older drafts could conceivably
          // violate this if the structure ever changes).
          let seenMax = false
          recipients = restored.map((r) => {
            if (r.sendMax) {
              if (seenMax) return { ...r, sendMax: false }
              seenMax = true
            }
            return r
          })
        }
      } else if (typeof d.amountInput === 'string') {
        // Backward-compat: drafts from the old single-recipient schema (address dropped).
        recipients = [{
          address: '',
          amount: d.amountInput,
          sendMax: d.sendMax === true,
        }]
      }
      if (d.localUnit === 'btc' || d.localUnit === 'sats') localUnit = d.localUnit
      if (['hour','halfhour','fastest','custom'].includes(d.feePreset)) feePreset = d.feePreset
      if (typeof d.customFeeRate === 'number') customFeeRate = d.customFeeRate
      if (typeof d.coinControlEnabled === 'boolean') coinControlEnabled = d.coinControlEnabled
      if (Array.isArray(d.selected)) {
        // Filter against current UTXOs — a previously-selected UTXO may have
        // been spent since we saved the draft, in which case keeping it in
        // the selection breaks PSBT construction.
        const available = new Set(utxos.map(u => utxoKey(u.txid, u.vout)))
        selected = new Set((d.selected as string[]).filter(op => available.has(op)))
      }
    } catch {}
  }
  function clearDraft() {
    if (!browserHasLocalStorage()) return
    try { localStorage.removeItem(draftKey) } catch {}
  }
  function browserHasLocalStorage(): boolean {
    try { return typeof localStorage !== 'undefined' } catch { return false }
  }

  // Autosave on any change (cheap: localStorage write is sync-fast).
  $effect(() => {
    void recipients; void localUnit
    void feePreset; void customFeeRate; void coinControlEnabled; void selected
    if (!draftLoaded) return
    saveDraft()
  })

  onMount(async () => {
    returnFocus = (document.activeElement as HTMLElement) ?? null
    dialogEl?.showModal()
    loadDraft()
    draftLoaded = true
    if ($mempoolUrl) {
      try {
        mempoolBlocks = await api.proxy.mempoolBlocks()
      } catch (e) { swallow(e, 'mempool-blocks') }
    }
    // Policy/vault wallets: fetch the spending policy so we can offer a
    // primary/recovery path choice when there's a timelocked branch.
    if (wallet.kind === 'descriptor') {
      try { vaultPolicy = await api.wallets.policy(wallet.id) } catch { /* not a policy with paths */ }
    }
    // Payjoin availability (off by default) — gates the payjoin send path.
    try {
      const s = await api.settings.get()
      payjoinEnabled = s.backend.payjoin_enabled
      payjoinFallbackSecs = s.backend.payjoin_fallback_secs
    } catch { /* settings unavailable — payjoin stays off */ }
  })

  // ── Export / copy ─────────────────────────────────────────────────────────
  function exportPsbt() {
    const psbt = currentPsbt
    if (!psbt) return
    const slug = wallet.label.replace(/[^a-z0-9]/gi, '_').toLowerCase()
    downloadBlob(psbtBlob(psbt), `send_${slug}.psbt`)
  }

  // Offline air-gap output: download the *signed* PSBT so it can be carried to an
  // online machine to broadcast (offline mode can't broadcast itself).
  function exportSignedTx() {
    const psbt = isMultisig ? currentPsbt : signedPsbt
    if (!psbt) return
    const slug = wallet.label.replace(/[^a-z0-9]/gi, '_').toLowerCase()
    downloadBlob(psbtBlob(psbt), `signed_${slug}.psbt`)
  }

  let copied = $state(false)
  async function copyPsbt() {
    const psbt = currentPsbt
    if (!psbt) return
    try {
      await navigator.clipboard.writeText(psbt)
      copied = true
      if (copyTimer) clearTimeout(copyTimer)
      copyTimer = setTimeout(() => copied = false, 1500)
    } catch { addToast('Copy failed — clipboard unavailable') }
  }

  // ── Multisig tracking ─────────────────────────────────────────────────────
  let isMultisig = $derived(wallet.kind === 'multisig')

  // Silent Payments send mode — entered when any recipient is sp1q…/tsp1…/sprt1….
  // Switches the build flow to /sp-send (which requires the mnemonic at build
  // time) and hides HW/PSBT signing affordances since SP signing happens
  // server-side in software.
  let hasSpRecipient = $derived(
    recipients.some(r => {
      const s = r.address.trim().toLowerCase()
      return s.startsWith('sp1') || s.startsWith('tsp1') || s.startsWith('sprt1')
    })
  )
  let spMnemonic = $state('')
  let spPassphrase = $state('')
  // Mask the seed by default; the user can reveal it to double-check. Firefox
  // doesn't honor -webkit-text-security on a textarea, so we mask with a real
  // password input and toggle its type.
  let spSeedRevealed = $state(false)

  // ── Payjoin (BIP-77 send) ──────────────────────────────────────────────────
  // Software single-sig only, single recipient with a pj= endpoint. Reuses the
  // SP seed inputs (spMnemonic/spPassphrase): the user types the seed once, the
  // modal holds it through negotiation to re-sign the proposal, and it's gone
  // when the modal closes.
  const PAYJOIN_SOFTWARE_KINDS = ['xpub', 'ypub', 'zpub', 'taproot']
  let payjoinEnabled = $state(false)
  let payjoinFallbackSecs = $state(120)
  let payjoinOptOut = $state(false)
  let payjoinActive = $derived(
    payjoinEnabled
    && !payjoinOptOut
    && recipients.length === 1
    && !!recipients[0]?.payjoinUri
    && !recipients[0]?.sendMax
    && PAYJOIN_SOFTWARE_KINDS.includes(wallet.kind),
  )
  type PjPhase = 'idle' | 'negotiating' | 'proposal' | 'sent' | 'fellback'
  let pjPhase = $state<PjPhase>('idle')
  let pjSessionId = $state<string | null>(null)
  let pjDiff = $state<PayjoinProposalDiff | null>(null)
  let pjTxid = $state('')
  let pjError = $state('')
  let pjBusy = $state(false)
  let pjDeadline = $state(0)
  let pjNow = $state(Date.now())
  let pjPollTimer: ReturnType<typeof setInterval> | null = null
  let pjRemaining = $derived(Math.max(0, Math.ceil((pjDeadline - pjNow) / 1000)))

  function stopPjPolling() {
    if (pjPollTimer) { clearInterval(pjPollTimer); pjPollTimer = null }
  }

  function applyPjStatus(status: PayjoinStatusKind, txid?: string | null, diff?: PayjoinProposalDiff) {
    if (status === 'proposal_ready') { if (diff) pjDiff = diff; pjPhase = 'proposal'; stopPjPolling() }
    else if (status === 'sent') { pjTxid = txid ?? pjTxid; pjPhase = 'sent'; stopPjPolling() }
    else if (status === 'fell_back') { pjTxid = txid ?? pjTxid; pjPhase = 'fellback'; stopPjPolling() }
    else if (status === 'failed') { pjError = 'Payjoin failed — try sending normally.'; pjPhase = 'idle'; stopPjPolling() }
  }

  async function pollPjStatus() {
    if (!pjSessionId) return
    pjNow = Date.now()
    try {
      const s = await api.wallets.payjoinSend.status(wallet.id, pjSessionId)
      applyPjStatus(s.status, s.result_txid, s.diff)
    } catch { /* transient — keep polling */ }
  }

  async function startPayjoin() {
    const uri = recipients[0]?.payjoinUri
    if (!uri) return
    pjError = ''; pjBusy = true
    try {
      const res = await api.wallets.payjoinSend.build(wallet.id, {
        uri,
        fee_rate_sat_vb: effectiveFeeRate,
        utxos: coinControlList,
        mnemonic: spMnemonic.trim(),
        passphrase: spPassphrase || undefined,
      })
      if (res.result === 'v1_unsupported') {
        addToast("This invoice uses an older payjoin version Corvin can't coordinate — send it normally instead.")
        recipients[0] = { ...recipients[0], payjoinUri: undefined }
        return
      }
      pjSessionId = res.session_id
      pjPhase = 'negotiating'
      pjDeadline = Date.now() + payjoinFallbackSecs * 1000
      pjNow = Date.now()
      stopPjPolling()
      pjPollTimer = setInterval(pollPjStatus, 2500)
    } catch (e) {
      pjError = e instanceof Error ? e.message : 'Failed to start payjoin'
    } finally {
      pjBusy = false
    }
  }

  async function confirmPayjoin() {
    if (!pjSessionId) return
    pjError = ''; pjBusy = true
    try {
      const r = await api.wallets.payjoinSend.confirm(wallet.id, pjSessionId, {
        mnemonic: spMnemonic.trim(),
        passphrase: spPassphrase || undefined,
      })
      pjTxid = r.txid; pjPhase = 'sent'
    } catch (e) {
      pjError = e instanceof Error ? e.message : 'Failed to sign payjoin'
    } finally {
      pjBusy = false
    }
  }

  async function abandonPayjoin() {
    if (!pjSessionId) return
    pjError = ''; pjBusy = true
    try {
      const r = await api.wallets.payjoinSend.abandon(wallet.id, pjSessionId)
      pjTxid = r.txid; pjPhase = 'fellback'
      stopPjPolling()
    } catch (e) {
      pjError = e instanceof Error ? e.message : 'Failed to send original'
    } finally {
      pjBusy = false
    }
  }

  // Land the proposal/result instantly off the SSE, not just the poll tick.
  $effect(() => {
    const ev = $lastPayjoinEvent
    if (ev && pjSessionId && ev.session_id === pjSessionId) {
      applyPjStatus(ev.status, ev.txid)
    }
  })

  let sigsRequired = $derived(wallet.threshold ?? 1)
  let currentPsbt = $state<string | null>(null)
  let sigsPresent = $state(0)
  let combineReady = $state(false)
  // Which cosigner fingerprints have signed so far. Updated alongside
  // sigsPresent every time combine_psbt is called.
  let signedFingerprints = $state<string[]>([])
  // Static info: the cosigner roster (parsed from the descriptor server-side).
  let multisigInfo = $state<MultisigDetails | null>(null)
  $effect(() => {
    if (!isMultisig) return
    api.wallets.multisigInfo(wallet.id).then((d) => { multisigInfo = d }).catch(() => {})
  })

  $effect(() => {
    const newPsbt = preview?.psbt ?? null
    if (sigsPresent === 0) {
      currentPsbt = newPsbt
      combineReady = false
    }
  })

  // ── PSBT import (multisig) ────────────────────────────────────────────────
  let importPsbtText = $state('')
  let importingPsbt = $state(false)
  let importPsbtError = $state('')

  async function handlePsbtFileImport(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0]
    if (!file) return
    ;(e.target as HTMLInputElement).value = '' // reset so the same file can be re-picked
    try {
      const buf = new Uint8Array(await file.arrayBuffer())
      // Heuristic: binary PSBT starts with 'psbt' magic (0x70 0x73 0x62 0x74 0xff).
      // Text PSBT is base64 — usually starts with 'cHNidP' (base64 of 'psbt').
      if (buf.length >= 5 && buf[0] === 0x70 && buf[1] === 0x73 && buf[2] === 0x62 && buf[3] === 0x74 && buf[4] === 0xff) {
        importPsbtText = bytesToBase64(buf)
      } else {
        // Assume base64 text (whitespace allowed)
        const text = new TextDecoder().decode(buf).trim()
        importPsbtText = text.replace(/\s+/g, '')
      }
      await importSignedPsbt()
    } catch (err) {
      importPsbtError = err instanceof Error ? err.message : 'Could not read PSBT file'
    }
  }

  async function importSignedPsbt() {
    const b64 = importPsbtText.trim()
    if (!b64 || !currentPsbt) return
    importingPsbt = true; importPsbtError = ''
    try {
      const result = await api.wallets.combinePsbt(wallet.id, { psbt_a: currentPsbt, psbt_b: b64 })
      currentPsbt = result.psbt
      sigsPresent = result.sigs_present
      combineReady = result.ready
      signedFingerprints = result.signed_fingerprints
      importPsbtText = ''
    } catch (e) {
      importPsbtError = e instanceof Error ? e.message : 'Combine failed'
    } finally {
      importingPsbt = false
    }
  }

  // ── QR signing ────────────────────────────────────────────────────────────
  let qrOpen = $state(false)

  async function handleQrSigned(signedPsbtFromQr: string) {
    qrOpen = false
    importPsbtText = signedPsbtFromQr
    await importSignedPsbt()
    if (combineReady) {
      // For singlesig the combine endpoint finalized the PSBT into currentPsbt.
      // Mirror it to signedPsbt so broadcastSigned (which reads signedPsbt for
      // singlesig) and the verification decode use the same source of truth.
      if (!isMultisig) signedPsbt = currentPsbt
      hwStatus = 'signed'
    }
  }

  // ── Hardware wallet signing ───────────────────────────────────────────────
  type HwStatus = 'idle' | 'connecting' | 'pairing' | 'confirm' | 'verifying' | 'registering' | 'signing' | 'signed' | 'error'
  type HwBrand = 'bitbox' | 'ledger' | 'trezor' | 'unknown'
  let hwStatus = $state<HwStatus>('idle')
  // SP preview already produces a signed PSBT (we have the keys at build
  // time). Promote it into the same shape the HW-signed path uses so the
  // existing "ready to broadcast" gate (`recipientReady`) lights up.
  $effect(() => {
    if (hasSpRecipient && preview && !previewIsEstimate) {
      signedPsbt = preview.psbt
      hwStatus = 'signed'
    } else if (hasSpRecipient && (!preview || previewIsEstimate)) {
      // No real build yet — fee-only estimate, or mnemonic still being typed.
      // Clear any stale signed-PSBT state so broadcast stays gated.
      if (hwStatus === 'signed') hwStatus = 'idle'
      signedPsbt = null
    }
  })
  let hwBrand = $state<HwBrand>('unknown')
  let hwMessage = $state('')
  let hwPairingCode = $state('')
  let hwEs = $state<EventSource | null>(null)
  let signedPsbt = $state<string | null>(null)
  let broadcasting = $state(false)
  let broadcastTxid = $state('')
  let broadcastError = $state('')

  // "Connecting…" hint upgrade: if the device is unresponsive for >5s, suggest
  // that the user check the USB connection — covers the common "I forgot to
  // plug it in" case.
  let hwStuckHint = $state(false)
  let hwStuckTimer: ReturnType<typeof setTimeout> | null = null
  $effect(() => {
    if (hwStuckTimer) { clearTimeout(hwStuckTimer); hwStuckTimer = null }
    hwStuckHint = false
    if (hwStatus === 'connecting') {
      hwStuckTimer = setTimeout(() => { hwStuckHint = true }, 5000)
    }
  })

  async function startHwSign() {
    if (!currentPsbt) return
    qrOpen = false
    hwStatus = 'connecting'; hwMessage = ''; hwPairingCode = ''
    hwBrand = 'unknown'
    signedPsbt = null; broadcastTxid = ''; broadcastError = ''
    let token: string
    try {
      ;({ token } = await api.hwi.signStart(currentPsbt, wallet.id))
    } catch (e) {
      hwMessage = e instanceof Error ? e.message : 'Failed to start signing'
      hwStatus = 'error'
      return
    }
    const es = new EventSource(`/api/hwi/sign/${token}`)
    hwEs = es
    // Backend emits `device_type` once at flow start with the detected brand
    // so we can render brand-specific hints (Ledger "open the Bitcoin app",
    // Trezor "confirm PIN on device", BitBox pairing code, etc.).
    es.addEventListener('device_type', (e) => {
      const b = JSON.parse(e.data).brand
      if (b === 'bitbox' || b === 'ledger' || b === 'trezor') hwBrand = b
    })
    es.addEventListener('connecting', () => { hwStatus = 'connecting' })
    es.addEventListener('pairing_code', (e) => { hwPairingCode = JSON.parse(e.data).code; hwStatus = 'pairing' })
    es.addEventListener('waiting_confirm', () => { hwStatus = 'confirm' })
    es.addEventListener('paired', () => { hwPairingCode = ''; hwStatus = 'confirm' })
    es.addEventListener('verifying', () => { hwStatus = 'verifying' })
    es.addEventListener('registering', () => { hwStatus = 'registering' })
    es.addEventListener('signing', () => { hwStatus = 'signing' })
    es.addEventListener('signed', async (e) => {
      const hwPsbt = JSON.parse(e.data).psbt
      es.close(); hwEs = null
      if (isMultisig && currentPsbt) {
        try {
          const result = await api.wallets.combinePsbt(wallet.id, { psbt_a: currentPsbt, psbt_b: hwPsbt })
          currentPsbt = result.psbt
          sigsPresent = result.sigs_present
          combineReady = result.ready
          signedFingerprints = result.signed_fingerprints
        } catch (err) {
          hwMessage = err instanceof Error ? err.message : 'Combine failed'
          hwStatus = 'error'
          return
        }
      } else {
        signedPsbt = hwPsbt
      }
      hwStatus = 'signed'
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
    const psbt = isMultisig ? currentPsbt : signedPsbt
    if (!psbt) return
    broadcasting = true; broadcastError = ''
    try {
      const result = await api.broadcast.broadcast({ psbt, wallet_id: wallet.id })
      broadcastTxid = result.txid
      // The send has gone through — wipe any saved draft so reopening the
      // modal next time starts blank instead of restoring obsolete data.
      clearDraft()
    } catch (e) {
      broadcastError = e instanceof Error ? e.message : 'Broadcast failed'
    } finally {
      broadcasting = false
    }
  }

  // ── Pre-broadcast verification ────────────────────────────────────────────
  // Decode the signed PSBT so the user can confirm the recipient + amount + fee
  // match what they intended. Catches clipboard-malware address swaps and any
  // PSBT manipulation between signing and broadcast.
  let verifyDecoded = $state<DecodedTx | null>(null)
  let verifyLoading = $state(false)
  let verifyError = $state('')

  // Ready to broadcast in any of these cases:
  //  - singlesig: HW returned signedPsbt (hwStatus='signed') or QR/import filled signedPsbt
  //  - multisig: combine produced a finalized PSBT (combineReady), via HW, QR, or pasted text
  let recipientReady = $derived(
    !broadcastTxid && ((!isMultisig && hwStatus === 'signed' && !!signedPsbt) || (isMultisig && combineReady))
  )

  /// The Transaction panel walks through four phases: composing the send
  /// (compose), running the device signing flow (sign), verifying the signed
  /// PSBT before broadcast (verify), and the post-broadcast success state
  /// (done). The same panel renders in all four — only its chrome and
  /// data source change, which kills the "two near-identical panels"
  /// confusion the old design had.
  ///
  /// QR signing counts as 'sign' too even though hwStatus stays idle while
  /// the QR flow runs — what matters for the panel is "user is no longer
  /// composing, a signer is mid-flight."
  type TxPhase = 'compose' | 'sign' | 'verify' | 'done'
  let txPhase = $derived<TxPhase>(
    broadcastTxid ? 'done'
    : recipientReady ? 'verify'
    : (qrOpen || (hwStatus !== 'idle' && hwStatus !== 'error')) ? 'sign'
    : 'compose'
  )

  $effect(() => {
    if (!recipientReady) { verifyDecoded = null; verifyError = ''; return }
    const psbt = isMultisig ? currentPsbt : signedPsbt
    if (!psbt) return
    verifyLoading = true; verifyError = ''
    api.broadcast.decode({ psbt })
      .then((d) => { verifyDecoded = d })
      .catch((e) => { verifyError = e instanceof Error ? e.message : 'Failed to decode signed transaction' })
      .finally(() => { verifyLoading = false })
  })

  /// Returns the outputs matching each intended recipient address, the list
  /// of remaining (change) outputs, and a mismatch flag if any intended
  /// recipient is missing from the signed PSBT (which would indicate the PSBT
  /// was tampered with).
  let verifySummary = $derived.by(() => {
    if (!verifyDecoded) return null
    const targets = recipients
      .map((r) => r.address.trim())
      .filter((a) => a.length > 0)
    const targetSet = new Set(targets)
    const recipientOuts = verifyDecoded.outputs.filter(
      (o) => o.address && targetSet.has(o.address)
    )
    const changeOuts = verifyDecoded.outputs.filter(
      (o) => !o.address || !targetSet.has(o.address)
    )
    // Every intended recipient address must appear as an output. (Multiple
    // outputs to the same address collapse to one match, but missing any
    // intended target is a mismatch — keep the strict check.)
    const matchedAddrs = new Set(
      recipientOuts.map((o) => o.address).filter((a): a is string => !!a)
    )
    const mismatch = targets.some((t) => !matchedAddrs.has(t))
    return { recipientOuts, changeOuts, mismatch, targets }
  })

  // Fat-finger soft warnings shown in the verify step (informational, never
  // blocking — the user can still send). Two classic fund-loss shapes: a fee
  // that's a large fraction of what's being sent (misplaced decimal / wrong
  // unit), and spending nearly the whole wallet (often a surprise from
  // send-max or coin selection). Computed from the signed tx, so they reflect
  // what will actually broadcast.
  let sendSanityWarnings = $derived.by((): string[] => {
    const d = verifyDecoded
    const s = verifySummary
    if (!d || !s || s.mismatch) return []
    const out: string[] = []
    const sent = s.recipientOuts.reduce((a, o) => a + o.value_sat, 0)
    const fee = d.fee_sat ?? 0
    if (sent > 0 && fee > 0 && fee >= sent * 0.25) {
      out.push(`The fee (${formatSats(fee)}) is ${Math.round((fee / sent) * 100)}% of the amount you're sending. Double-check the fee rate.`)
    }
    const walletTotal = utxos.reduce((a, u) => a + u.amount_sats, 0)
    if (walletTotal > 0 && sent + fee >= walletTotal * 0.9) {
      out.push(`This spends ${Math.round(((sent + fee) / walletTotal) * 100)}% of this wallet's balance.`)
    }
    return out
  })

  // ── Format ────────────────────────────────────────────────────────────────
  function formatSats(sats: number): string {
    if ($displayUnit === 'btc') return (sats / 1e8).toFixed(8) + ' BTC'
    return sats.toLocaleString() + ' sats'
  }

</script>

<dialog
  bind:this={dialogEl}
  class="send-modal"
  onclose={onClose}
  onclick={onBackdrop}
  aria-labelledby="send-title"
>
  <div class="inner">
    <!-- Header -->
    <div class="modal-header">
      <div>
        <h2 id="send-title">Send Bitcoin</h2>
        <p class="subtitle">From: {wallet.label}</p>
      </div>
      <div class="header-actions">
        {#if recipients.some(r => r.address || r.amount || r.sendMax) || coinControlEnabled || sigsPresent > 0 || signedPsbt}
          <button
            type="button"
            class="reset-draft-btn"
            onclick={() => {
              recipients = [{ address: '', amount: '', sendMax: false }]
              activeRecipientIdx = 0
              coinControlEnabled = false; selected = new Set()
              feePreset = 'hour'; customFeeRate = 10
              // Also wipe any signing progress — otherwise a stale signed PSBT
              // would still trigger the "Confirm and broadcast" card after the
              // user just reset the inputs.
              sigsPresent = 0
              combineReady = false
              currentPsbt = null
              signedPsbt = null
              signedFingerprints = []
              hwStatus = 'idle'
              hwMessage = ''
              importPsbtText = ''
              importPsbtError = ''
              step = 'compose'
              clearDraft()
            }}
            title="Clear all fields, signing progress, and saved draft"
          >Reset</button>
        {/if}
        <button class="close-btn" onclick={onClose} aria-label="Close">✕</button>
      </div>
    </div>

    {#if step === 'compose'}

    {#if isVault}
      <!-- Spend path: vaults have a primary branch (now) and a timelocked
           recovery branch (only available once coins mature). -->
      <section class="config-section">
        <h3 class="section-label" style="margin: 0 0 8px">Spend via</h3>
        <div class="spend-path-toggle" role="group" aria-label="Spending path">
          <button type="button" class:active={spendPath === 'primary'} onclick={() => spendPath = 'primary'}>
            <span class="sp-path-title">Primary</span>
            <span class="sp-path-sub">spendable now</span>
          </button>
          <button
            type="button"
            class:active={spendPath === 'recovery'}
            disabled={!timelockPathSpendable}
            title={timelockPathSpendable ? '' : `Locked — ${lockHint}`}
            onclick={() => spendPath = 'recovery'}
          >
            <span class="sp-path-title">Recovery</span>
            <span class="sp-path-sub">{timelockPathSpendable ? `after ${recoveryTimelockLabel || 'timelock'}` : `🔒 ${lockHint}`}</span>
          </button>
        </div>
        {#if spendPath === 'recovery'}
          <p class="spend-path-note">Recovery spends only confirm once the timelock has elapsed. Signing uses the recovery key(s) — this branch is a script-path spend and may require software signing rather than a hardware wallet.</p>
        {/if}
      </section>
    {:else if isSavings}
      <!-- Timelocked savings: single locked path — surface lock status. -->
      <section class="config-section">
        {#if timelockPathSpendable}
          <p class="timelock-banner ok">🔓 Unlocked — {timelockMaturedCount} of {utxos.length} coin{utxos.length === 1 ? '' : 's'} spendable. Locked coins remain frozen until they mature.</p>
        {:else}
          <p class="timelock-banner">🔒 Locked savings — {lockHint}. The coins can't be spent until the timelock elapses (network-enforced).</p>
        {/if}
      </section>
    {/if}

    <!-- Recipients -->
    <section class="config-section">
      <div class="recipients-header">
        <h3 class="section-label" style="margin: 0">Recipients{recipients.length > 1 ? ` (${recipients.length})` : ''}</h3>
        <div class="unit-toggle" role="group" aria-label="Unit">
          <button type="button" class:active={localUnit === 'sats'} onclick={() => setLocalUnit('sats')}>sats</button>
          <button type="button" class:active={localUnit === 'btc'}  onclick={() => setLocalUnit('btc')}>BTC</button>
        </div>
      </div>

      {#each recipients as r, i (i)}
        <RecipientRow
          bind:recipient={recipients[i]}
          index={i}
          total={recipients.length}
          unit={localUnit}
          hrn={hrnStatus[i]}
          {otherWallets}
          scanActive={addrScanOpen && activeRecipientIdx === i}
          walletPickerOpen={walletPickerOpenIdx === i}
          coinControlSelected={coinControlEnabled && selected.size > 0 ? { count: selected.size, satsFormatted: formatSats(selectedSats) } : null}
          onRemove={() => removeRecipient(i)}
          onInput={() => onRecipientInput(i)}
          onPaste={(e) => onRecipientPaste(i, e)}
          onBlur={() => resolveHrnAt(i)}
          onFocus={() => activeRecipientIdx = i}
          onPasteClipboard={() => pasteFromClipboard(i)}
          onScan={() => { activeRecipientIdx = i; addrScanOpen = true }}
          onToggleWalletPicker={() => { activeRecipientIdx = i; walletPickerOpenIdx = walletPickerOpenIdx === i ? null : i }}
          onCloseWalletPicker={() => walletPickerOpenIdx = null}
          onPickWallet={(w) => pickWallet(i, w)}
          onSetSendMax={(v) => setSendMax(i, v)}
        />
      {/each}

      <div class="recipients-footer">
        <button type="button" class="add-recipient-btn" onclick={addRecipient}>
          + Add recipient
        </button>
        {#if amountFiat}<span class="amount-fiat-inline">{amountFiat}</span>{/if}
      </div>

      {#if sendMaxCount > 1}
        <p class="addr-shape-warn">Only one recipient can use "Send max" — only the last one you set will keep it.</p>
      {/if}

      {#if addrScanOpen}
        <QrScanModal
          title="Scan address"
          hint="Point your camera at a Bitcoin address or payment (BIP21) QR."
          validate={(v) => addressFromScan(v) !== null}
          invalidHint="That QR code doesn't look like a Bitcoin address. Keep scanning…"
          onResult={applyScannedAddress}
          onCancel={() => { addrScanOpen = false }}
        />
      {/if}
    </section>

    {#if hasSpRecipient}
      <section class="config-section sp-send-section">
        <h3 class="section-label">Silent Payments — sender authentication</h3>
        <p class="sp-help">
          BIP-352 needs your input private keys at build time to derive the recipient's one-time output key. Paste the mnemonic for <strong>{wallet.label}</strong>. It's used once and zeroized — never stored.
        </p>
        <div class="sp-seed-label-row">
          <label for="sp-send-mnemonic" class="field-label">Seed phrase</label>
          <button type="button" class="sp-seed-reveal" onclick={() => spSeedRevealed = !spSeedRevealed}>
            {spSeedRevealed ? 'Hide' : 'Show'}
          </button>
        </div>
        <input
          id="sp-send-mnemonic"
          class="sp-input"
          type={spSeedRevealed ? 'text' : 'password'}
          bind:value={spMnemonic}
          placeholder="twelve or twenty-four words"
          spellcheck="false"
          autocapitalize="off"
          autocomplete="off"
        />
        <label for="sp-send-pass" class="field-label">BIP39 passphrase <span class="optional">(optional)</span></label>
        <input
          id="sp-send-pass"
          class="sp-input"
          type="password"
          bind:value={spPassphrase}
          placeholder="Leave blank if unsure"
          autocomplete="new-password"
        />
        {#if recipients.some(r => r.sendMax)}
          <p class="sp-warn">Silent Payments send-max isn't supported — set explicit amounts for each recipient.</p>
        {/if}
        {#if wallet.kind === 'multisig'}
          <p class="sp-warn">Silent Payments sending isn't supported from multisig wallets.</p>
        {/if}
      </section>
    {/if}

    {#if payjoinActive}
      <section class="config-section pj-section">
        <h3 class="section-label">⚡ Payjoin <HelpLink anchor="payjoin-send" /></h3>
        {#if pjPhase === 'idle'}
          <p class="sp-help">
            This invoice supports <strong>Payjoin</strong> — Corvin coordinates privately with the recipient so the transaction also includes one of their inputs, breaking the common-input-ownership heuristic. Your seed for <strong>{wallet.label}</strong> signs here and once more to confirm; it's never stored.
            <button type="button" class="sp-link" onclick={() => payjoinOptOut = true}>Send normally instead</button>
          </p>
          <div class="sp-seed-label-row">
            <label for="pj-mnemonic" class="field-label">Seed phrase</label>
            <button type="button" class="sp-seed-reveal" onclick={() => spSeedRevealed = !spSeedRevealed}>{spSeedRevealed ? 'Hide' : 'Show'}</button>
          </div>
          <input id="pj-mnemonic" class="sp-input" type={spSeedRevealed ? 'text' : 'password'} bind:value={spMnemonic} placeholder="twelve or twenty-four words" spellcheck="false" autocapitalize="off" autocomplete="off" />
          <label for="pj-pass" class="field-label">BIP39 passphrase <span class="optional">(optional)</span></label>
          <input id="pj-pass" class="sp-input" type="password" bind:value={spPassphrase} placeholder="Leave blank if unsure" autocomplete="new-password" />
          {#if pjError}<p class="sp-warn">{pjError}</p>{/if}
          <button class="pj-btn" onclick={startPayjoin} disabled={!spSeedReady || pjBusy || previewLoading}>
            {pjBusy ? 'Starting…' : 'Send with Payjoin'}
          </button>
        {:else if pjPhase === 'negotiating'}
          <p class="sp-help"><span class="pj-spinner" aria-hidden="true"></span> Waiting for the recipient to respond…</p>
          <p class="pj-countdown">Sends the original automatically in {Math.floor(pjRemaining / 60)}:{(pjRemaining % 60).toString().padStart(2, '0')} if they don't.</p>
          {#if pjError}<p class="sp-warn">{pjError}</p>{/if}
          <button class="pj-btn pj-btn-ghost" onclick={abandonPayjoin} disabled={pjBusy}>Send original now</button>
        {:else if pjPhase === 'proposal'}
          <p class="sp-help">✓ The recipient cooperated. Review and confirm to send the payjoin.</p>
          {#if pjDiff}
            <ul class="pj-diff">
              <li>Recipient added <strong>{pjDiff.added_inputs}</strong> input{pjDiff.added_inputs === 1 ? '' : 's'}</li>
              <li>Network fee: <strong>{pjDiff.proposal_fee_sats.toLocaleString()}</strong> sat</li>
            </ul>
          {/if}
          {#if pjError}<p class="sp-warn">{pjError}</p>{/if}
          <button class="pj-btn" onclick={confirmPayjoin} disabled={pjBusy}>{pjBusy ? 'Signing…' : 'Confirm & send payjoin'}</button>
          <button class="pj-btn pj-btn-ghost" onclick={abandonPayjoin} disabled={pjBusy}>Send original instead</button>
        {:else if pjPhase === 'sent' || pjPhase === 'fellback'}
          <p class="pj-done">{pjPhase === 'sent' ? '✓ Payjoin sent' : '✓ Sent the original transaction'}</p>
          <code class="pj-txid">{pjTxid}</code>
          <div class="pj-done-actions">
            <button type="button" class="sp-link" onclick={() => navigator.clipboard?.writeText(pjTxid).catch(() => addToast('Copy failed — clipboard unavailable'))}>Copy txid</button>
            <button type="button" class="sp-link" onclick={onClose}>Done</button>
          </div>
        {/if}
      </section>
    {/if}

    <!-- Fee rate -->
    <FeePicker
      {feeRates}
      {mempoolBlocks}
      bind:preset={feePreset}
      bind:customFeeRate
      {effectiveFeeRate}
    />

    <!-- Coin control -->
    <CoinControlPanel
      {utxos}
      bind:enabled={coinControlEnabled}
      bind:selected
      {addressLabels}
      format={formatSats}
    />

    {:else}

    <!-- Step 2 — Review & sign. -->
    <button type="button" class="step-back" onclick={() => step = 'compose'}>← Edit transaction</button>

    <!-- Unified Transaction panel — one panel that walks compose → sign →
         verify → done, with chrome cues so the user always knows which
         phase they're in. The data source switches between `preview` (BDK
         build output) pre-signing and `verifyDecoded` (the actual signed
         tx) post-signing, but the row layout stays consistent so a tampered
         post-sign tx jumps out visually. -->
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
            {:else if txPhase === 'verify' && verifySummary?.mismatch}⚠ Address mismatch
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
          {:else if !preview}
            <p class="tx-placeholder">Enter a recipient address and amount to preview.</p>
          {:else}
            {@const flowOutputs = [
              { label: recipients.length > 1 ? 'Recipients' : 'Recipient', sats: preview.recipient_sats, kind: 'recipient' as const },
              ...(preview.change_sats > 0 ? [{ label: 'Change', sats: preview.change_sats, kind: 'change' as const }] : []),
              { label: 'Network fee', sats: preview.fee_sats, kind: 'fee' as const },
            ]}
            <div class="tx-body">
              <!-- Single recipient: the address already shows in the input + the
                   chunked echo above, so we skip a third copy here. Multiple
                   recipients: list them so each amount is attributable. -->
              {#if recipients.length > 1}
                {#each recipients as r, i (i)}
                  {#if r.address.trim()}
                    <div class="preview-recipient">
                      <span class="preview-recipient-label">To #{i + 1}{r.sendMax ? ' (max)' : ''}</span>
                      <span class="preview-recipient-addr mono">{r.address.trim()}</span>
                    </div>
                  {/if}
                {/each}
                <div class="preview-divider"></div>
              {/if}

              <TxFlowDiagram
                inputSats={preview.input_sats}
                inputCount={coinControlEnabled ? selected.size : null}
                outputs={flowOutputs}
                format={formatSats}
              />

              <div class="preview-divider"></div>

              <div class="preview-row leaving-row"><span>Total leaving wallet</span><span>− {formatSats(preview.recipient_sats + preview.fee_sats)}{#if $showFiatBalance && $currentBtcPrice}<span class="preview-fiat">≈ ${(((preview.recipient_sats + preview.fee_sats) / 1e8) * $currentBtcPrice).toLocaleString(undefined, { maximumFractionDigits: 2 })}</span>{/if}</span></div>
              {#if previewIsEstimate}
                <p class="preview-estimate-note">Estimate — enter your seed phrase below to build and sign this Silent Payments send.</p>
              {/if}
            </div>

            {#if txPhase === 'compose' && preview.warnings && preview.warnings.length > 0}
              <div class="warn-list" role="alert" aria-label="Privacy warnings">
                {#each preview.warnings as w, i (i)}
                  <div class="warn-item warn-{w.severity}" class:warn-lookalike={w.code === 'look_alike'}>
                    <span class="warn-icon" aria-hidden="true">{w.code === 'look_alike' ? '🛑' : w.severity === 'warning' ? '⚠' : 'ⓘ'}</span>
                    <div class="warn-body">
                      <div class="warn-msg">{w.message}</div>
                      {#if w.detail}<div class="warn-detail">{w.detail}</div>{/if}
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
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
                  <span class="verify-mismatch-label">You entered:</span>
                  {#each verifySummary.targets as t, i (i)}
                    <span class="mono verify-mismatch-addr">{t}</span>
                  {/each}
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
                {#each verifySummary.recipientOuts as o, i (i)}
                  <div class="verify-row">
                    <span class="verify-label">{verifySummary.recipientOuts.length > 1 ? `Sending #${i + 1}` : 'Sending'}</span>
                    <span class="verify-value">{formatSats(o.value_sat)}</span>
                  </div>
                  <div class="verify-row verify-row-addr">
                    <span class="verify-label">To</span>
                    <span class="mono verify-addr">{o.address}</span>
                  </div>
                {/each}
                {#each verifySummary.changeOuts as o, i (i)}
                  {#if o.address}
                    <div class="verify-row verify-change">
                      <span class="verify-label">Change</span>
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
                {#if sendSanityWarnings.length}
                  <div class="send-sanity" role="status">
                    {#each sendSanityWarnings as w (w)}
                      <p class="send-sanity-line"><span aria-hidden="true">⚠</span> {w}</p>
                    {/each}
                  </div>
                {/if}
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

    {/if}

    <!-- Footer — Step 1 shows the Preview gate; Step 2 shows the phase-driven
         signing actions (the Transaction panel above is the data source). -->
    <div class="modal-footer">
      {#if step === 'compose'}
        <button
          type="button"
          class="btn-primary preview-cta"
          disabled={!canPreview}
          onclick={() => step = 'review'}
        >Preview transaction →</button>
        {#if previewError}<p class="footer-hint hw-error">{previewError}</p>{/if}
      {:else if qrOpen}
        <QrSignFlow
          psbt={currentPsbt!}
          onSigned={handleQrSigned}
          onCancel={() => qrOpen = false}
        />
      {:else if txPhase === 'compose'}
        {#if payjoinActive}
          <p class="footer-hint">This invoice uses Payjoin — tap <strong>← Edit transaction</strong> and send from the Payjoin panel there.</p>
        {:else if hasSpRecipient}
          <!-- SP mode skips the HW/QR/Export buttons — the PSBT is built and
               signed server-side when the user provides their mnemonic.
               Just give them feedback while preview is in flight. -->
          <p class="footer-hint">
            {#if previewLoading}
              Deriving keys and building transaction…
            {:else if spMnemonic.trim().split(/\s+/).filter(Boolean).length < 12}
              Enter your seed phrase above to build the transaction.
            {:else if previewError}
              <span class="hw-error">{previewError}</span>
            {:else}
              Transaction will appear here once built.
            {/if}
          </p>
        {:else}
        {#if $hwEnabled}<button class="hw-sign-btn" onclick={startHwSign} disabled={!currentPsbt}>⚿ Hardware wallet</button>{/if}
        <button class="qr-sign-btn" onclick={() => qrOpen = true} disabled={!currentPsbt}>⬡ QR code</button>
        <button class="export-btn" onclick={exportPsbt} disabled={!currentPsbt}>↓ Export PSBT</button>
        <button class="copy-psbt-btn" onclick={copyPsbt} disabled={!currentPsbt}>
          {copied ? '✓ Copied' : '⎘ Copy PSBT'}
        </button>
        {#if hwStatus === 'error'}
          <p class="hw-error">{hwMessage}</p>
        {:else if isMultisig && currentPsbt}
          <p class="footer-hint">
            {sigsPresent} of {sigsRequired} signature{sigsRequired !== 1 ? 's' : ''} collected.
            {#if sigsPresent < sigsRequired}
              <strong>Next:</strong> sign with another device, or export this PSBT and send it to a co-signer who will sign and return it for you to combine below.
            {/if}
          </p>
        {:else}
          <p class="footer-hint">Sign this PSBT externally, then broadcast via ··· → Broadcast.</p>
        {/if}
        {/if}
      {:else if txPhase === 'sign'}
        <div class="hw-active">
          <div class="hw-status-msg">
            {#if hwStatus === 'connecting'}
              {#if hwBrand === 'ledger'}
                Connecting to Ledger…
                <span class="hw-stuck-hint">Make sure the Bitcoin app is open on the device.</span>
              {:else if hwBrand === 'trezor'}
                Connecting to Trezor… enter your PIN on the device if prompted.
              {:else}
                Connecting to device…
                {#if hwStuckHint}<span class="hw-stuck-hint">Make sure your device is plugged in via USB and unlocked.</span>{/if}
              {/if}
            {:else if hwStatus === 'pairing'}Verify pairing code: <code class="hw-code">{hwPairingCode}</code>
            {:else if hwStatus === 'verifying'}Checking {isPolicyWallet ? 'policy' : 'multisig'} registration on device…
            {:else if hwStatus === 'registering'}
              {#if isPolicyWallet}
                <strong>Confirm this wallet's spending policy on your device</strong> — it's asking you to register the policy (a one-time step). Verify the keys and timelock before accepting.
              {:else}
                <strong>Confirm the multisig wallet on your device</strong> — the device is asking you to register this wallet (a one-time step). Verify each cosigner's xpub before accepting.
              {/if}
            {:else if hwStatus === 'confirm' || hwStatus === 'signing'}
              {#if hwBrand === 'ledger'}
                Review the transaction on your Ledger and approve to sign.
              {:else if hwBrand === 'trezor'}
                Review the transaction on your Trezor and approve to sign.
              {:else}
                Confirm transaction on device…
              {/if}
            {:else if hwStatus === 'signed' && isMultisig && !combineReady}Signed — {sigsPresent} of {sigsRequired} signature{sigsRequired !== 1 ? 's' : ''} collected
            {/if}
          </div>
          {#if hwStatus === 'signed' && isMultisig && !combineReady}
            <button class="btn-cancel-hw" onclick={() => { hwStatus = 'idle'; hwMessage = ''; hwPairingCode = '' }}>
              Sign with another device
            </button>
          {:else if hwStatus !== 'signed'}
            <button class="btn-cancel-hw" onclick={cancelHwSign}>Cancel</button>
          {/if}
        </div>
        {#if broadcastError}<p class="hw-error">{broadcastError}</p>{/if}
      {:else if txPhase === 'verify' && !verifySummary?.mismatch && verifyDecoded}
        {#if $offline}
          <button class="btn-broadcast btn-broadcast-confirm" onclick={exportSignedTx}>
            ↓ Export signed transaction
          </button>
          <p class="footer-hint">Offline mode — move this signed PSBT to an online machine to broadcast it.</p>
        {:else}
          <button class="btn-broadcast btn-broadcast-confirm" onclick={broadcastSigned} disabled={broadcasting}>
            {broadcasting ? 'Broadcasting…' : 'Confirm and broadcast'}
          </button>
        {/if}
        {#if broadcastError}<p class="hw-error">{broadcastError}</p>{/if}
      {/if}

      {#if isMultisig && currentPsbt && !broadcastTxid && multisigInfo}
        <div class="ms-progress">
          <div class="ms-progress-header">
            <span class="ms-progress-title">Signature progress</span>
            <span class="ms-progress-count">{sigsPresent} of {sigsRequired} required</span>
          </div>
          <div class="ms-signer-list">
            {#each multisigInfo.signers as signer, i (i)}
              {@const signed = signedFingerprints.includes(signer.fingerprint.toLowerCase())}
              <div class="ms-signer-pill" class:ms-signed={signed}>
                <span class="ms-signer-icon">{signed ? '✓' : '○'}</span>
                <span class="ms-signer-name">Signer {i + 1}</span>
                <code class="ms-signer-fp">{signer.fingerprint}</code>
              </div>
            {/each}
          </div>
        </div>
      {/if}

      {#if isMultisig && currentPsbt && !broadcastTxid && !recipientReady}
        <div class="ms-import-section">
          <div class="ms-import-header">
            <p class="ms-import-label">Import signed PSBT from co-signer</p>
            <label class="ms-import-file-btn" title="Load a .psbt file from a co-signer">
              ↓ Load file
              <input type="file" accept=".psbt,.txt" onchange={handlePsbtFileImport} />
            </label>
          </div>
          <textarea
            class="ms-import-input"
            rows="3"
            placeholder="…or paste base64 PSBT here"
            spellcheck="false"
            bind:value={importPsbtText}
          ></textarea>
          {#if importPsbtError}<p class="hw-error">{importPsbtError}</p>{/if}
          <div class="ms-import-actions">
            <button
              class="ms-import-btn"
              onclick={importSignedPsbt}
              disabled={!importPsbtText.trim() || importingPsbt}
            >
              {importingPsbt ? 'Combining…' : 'Combine'}
            </button>
          </div>
        </div>
      {/if}
    </div>
  </div>
</dialog>

<style>
  /* Modal shell — mirrors the shared ui/Modal (centered card, blurred backdrop,
     mobile bottom-sheet). Send keeps its own bespoke shell rather than the shared
     component because of its phase-driven footer + Reset header chrome + edge-to-
     edge sectioned body. `max-height + overflow` on .inner (not a flex hack)
     avoids the WebKitGTK auto-height-dialog collapse. */
  .send-modal {
    position: fixed; inset: 0; margin: auto;
    border: none; background: transparent; padding: 0; color: inherit;
    max-height: min(92vh, 760px);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5); border-radius: 10px;
  }
  .send-modal::backdrop { background: rgba(0, 0, 0, 0.6); backdrop-filter: blur(2px); }
  .inner {
    width: min(600px, calc(100vw - 32px));
    max-height: inherit; overflow-y: auto;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 10px;
    display: flex; flex-direction: column;
  }

  /* Header */
  .modal-header {
    display: flex; align-items: flex-start; justify-content: space-between;
    gap: 12px;
    padding: 20px 20px 14px;
    border-bottom: 1px solid var(--border);
    position: sticky; top: 0;
    background: var(--surface-1);
    z-index: 1;
  }
  .modal-header h2 { margin: 0; font-size: 1rem; font-weight: 700; color: var(--text); }
  .subtitle { margin: 3px 0 0; font-size: 0.78rem; color: var(--text-muted); }
  .close-btn {
    background: none; border: none; cursor: pointer;
    color: var(--text-muted); font-size: 0.9rem; padding: 2px 6px;
    flex-shrink: 0; line-height: 1;
  }
  .close-btn:hover { color: var(--text); }
  .header-actions { display: flex; align-items: center; gap: 6px; flex-shrink: 0; }
  .reset-draft-btn {
    background: none; border: 1px solid var(--border); border-radius: 4px;
    color: var(--text-muted); cursor: pointer; font-size: 0.72rem; padding: 3px 8px;
  }
  .reset-draft-btn:hover { border-color: var(--text-muted); color: var(--text); }

  /* Sections */
  .config-section {
    padding: 16px 20px;
    border-bottom: 1px solid var(--border);
  }
  .section-label {
    margin: 0 0 10px;
    font-size: 0.72rem; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.07em; color: var(--text-muted);
  }

  /* Spend path (vault) */
  .spend-path-toggle { display: flex; gap: 8px; }
  .spend-path-toggle button {
    flex: 1; display: flex; flex-direction: column; gap: 2px; align-items: flex-start;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px;
    padding: 8px 11px; cursor: pointer; color: var(--text-muted);
  }
  .spend-path-toggle button.active { border-color: var(--accent); background: var(--surface-active); color: var(--text); }
  .spend-path-toggle button:disabled { opacity: 0.5; cursor: not-allowed; }
  .timelock-banner {
    margin: 0; font-size: 0.82rem; line-height: 1.5; padding: 10px 12px; border-radius: 6px;
    background: color-mix(in srgb, #e09c52 10%, transparent);
    border: 1px solid color-mix(in srgb, #e09c52 30%, transparent); color: var(--text);
  }
  .timelock-banner.ok {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .sp-path-title { font-size: 0.85rem; font-weight: 600; }
  .sp-path-sub { font-size: 0.72rem; color: var(--text-muted); }
  .spend-path-note {
    margin: 8px 0 0; font-size: 0.76rem; color: #e09c52; line-height: 1.5;
  }

  /* Recipients */
  .recipients-header {
    display: flex; align-items: center; justify-content: space-between;
    margin-bottom: 10px; gap: 12px;
  }
  .recipients-footer {
    display: flex; align-items: center; justify-content: space-between;
    margin-top: 4px; gap: 12px;
  }
  .add-recipient-btn {
    background: none; border: 1px dashed var(--border); border-radius: 5px;
    color: var(--text-muted); cursor: pointer; font-size: 0.78rem; padding: 6px 12px;
    transition: all 0.12s;
  }
  .add-recipient-btn:hover { border-color: var(--accent); color: var(--accent); border-style: solid; }
  .amount-fiat-inline {
    font-size: 0.75rem; color: var(--text-muted); font-variant-numeric: tabular-nums;
  }

  /* Address-shape warning is shared (recipient rows live in RecipientRow.svelte,
     but the SP/payjoin sections reuse this class). */
  .addr-shape-warn { margin: 6px 0 0; font-size: 0.74rem; color: #e09c52; line-height: 1.4; }

  /* Fat-finger soft warnings in the verify step (informational, non-blocking). */
  .send-sanity {
    margin-top: 10px; padding: 8px 10px;
    background: color-mix(in srgb, #e09c52 12%, var(--surface-2));
    border: 1px solid color-mix(in srgb, #e09c52 35%, var(--border));
    border-radius: 6px; display: flex; flex-direction: column; gap: 5px;
  }
  .send-sanity-line { margin: 0; font-size: 0.76rem; color: var(--text); line-height: 1.45; }

  .unit-toggle {
    display: flex; border: 1px solid var(--border); border-radius: 4px;
    overflow: hidden; flex-shrink: 0;
  }
  .unit-toggle button {
    background: var(--surface-2); border: none; color: var(--text-muted);
    padding: 5px 8px; cursor: pointer; font-size: 0.75rem; font-weight: 600;
  }
  .unit-toggle button + button { border-left: 1px solid var(--border); }
  .unit-toggle button.active { background: var(--accent); color: #000; }
  .unit-toggle button:hover:not(.active) { color: var(--text); }
  .leaving-row {
    color: var(--text); font-weight: 600;
    display: flex; justify-content: space-between; align-items: center;
  }
  .leaving-row > :last-child { display: flex; align-items: baseline; gap: 8px; }
  .preview-fiat { font-size: 0.72rem; color: var(--text-muted); font-weight: 400; }
  .hw-stuck-hint {
    display: block; font-size: 0.72rem; color: #e09c52; margin-top: 4px;
  }
  .ms-import-header { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .ms-import-file-btn {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 4px;
    color: var(--text-muted); cursor: pointer; font-size: 0.72rem; padding: 4px 8px;
    white-space: nowrap;
  }
  .ms-import-file-btn:hover { border-color: var(--accent); color: var(--accent); }
  .ms-import-file-btn input[type="file"] { display: none; }

  /* ── Unified Transaction panel ───────────────────────────────────────
     One panel, four phases, chrome cues differentiate. The row layout
     (.preview-row / .verify-row) is shared by compose and verify phases on
     purpose — visual continuity is the security signal that the signed tx
     matches what the user reviewed pre-sign. */
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
  .preview-divider { height: 1px; background: var(--border); }
  .preview-estimate-note { margin: 4px 0 0; font-size: 0.74rem; color: var(--text-muted); font-style: italic; }

  /* Verify-mode rows reuse the same layout primitive */
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
  .verify-change { color: var(--text-muted); font-style: italic; }
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

  /* Privacy / leak warnings — advisory, not blocking. Live inside the
     Transaction panel during compose phase only. */
  .warn-list {
    display: flex; flex-direction: column; gap: 6px;
    margin-top: 8px;
  }
  .warn-item {
    display: flex; align-items: flex-start; gap: 8px;
    padding: 8px 10px; border-radius: 5px;
    border: 1px solid var(--border);
    font-size: 0.78rem; line-height: 1.45;
  }
  .warn-item.warn-warning {
    background: color-mix(in srgb, #e09c52 8%, var(--surface-2));
    border-color: color-mix(in srgb, #e09c52 35%, var(--border));
  }
  .warn-item.warn-info {
    background: color-mix(in srgb, #52a8d4 6%, var(--surface-2));
    border-color: color-mix(in srgb, #52a8d4 25%, var(--border));
  }
  /* Look-alike / address poisoning is a fund-loss risk, not a privacy nudge —
     give it a stronger danger treatment so it can't be skimmed past. */
  .warn-item.warn-lookalike {
    background: color-mix(in srgb, #e05252 12%, var(--surface-2));
    border-color: #e05252;
    font-weight: 500;
  }
  .warn-icon { font-size: 0.95rem; line-height: 1.3; flex-shrink: 0; }
  .warn-item.warn-warning .warn-icon { color: #e09c52; }
  .warn-item.warn-info    .warn-icon { color: #52a8d4; }
  .warn-body { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .warn-msg { color: var(--text); }
  .warn-detail {
    font-size: 0.72rem; color: var(--text-muted);
    word-break: break-all;
  }

  /* Footer */
  .modal-footer {
    padding: 14px 20px 18px;
    display: flex; flex-wrap: wrap; align-items: center; gap: 8px;
    border-top: 1px solid var(--border);
    position: sticky; bottom: 0;
    background: var(--surface-1);
  }
  /* Hardware wallet is the primary signing action — accent fill. QR is the
     next-most-canonical (air-gapped HW). Export / Copy are escape hatches
     for sign-elsewhere workflows, styled as ghosts so they don't compete. */
  .hw-sign-btn {
    background: var(--accent); color: #000; border: none; border-radius: 5px;
    padding: 7px 12px; cursor: pointer; font-weight: 600; font-size: 0.82rem;
  }
  .hw-sign-btn:hover:not(:disabled) { opacity: 0.88; }
  .hw-sign-btn:disabled { opacity: 0.35; cursor: not-allowed; }
  .qr-sign-btn {
    background: none; border: 1px solid var(--accent);
    border-radius: 5px; padding: 7px 10px; font-size: 0.82rem; font-weight: 600;
    color: var(--accent); cursor: pointer; transition: all 0.12s;
  }
  .qr-sign-btn:hover:not(:disabled) { opacity: 0.78; }
  .qr-sign-btn:disabled { opacity: 0.35; cursor: not-allowed; }
  .export-btn {
    background: none; border: 1px solid var(--border); border-radius: 5px;
    color: var(--text-muted); cursor: pointer; font-size: 0.82rem; padding: 7px 10px;
  }
  .export-btn:hover:not(:disabled) { border-color: var(--text-muted); color: var(--text); }
  .export-btn:disabled { opacity: 0.35; cursor: not-allowed; }
  .copy-psbt-btn {
    background: none; border: 1px solid var(--border); border-radius: 5px;
    color: var(--text-muted); cursor: pointer; font-size: 0.82rem; padding: 7px 10px;
  }
  .copy-psbt-btn:hover:not(:disabled) { border-color: var(--text-muted); color: var(--text); }
  .copy-psbt-btn:disabled { opacity: 0.35; cursor: not-allowed; }
  .footer-hint { width: 100%; margin: 2px 0 0; font-size: 0.7rem; color: var(--text-muted); line-height: 1.4; }

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

  /* Silent Payments send-mode block. Visually distinct (accent-tinted) so
     users know they're in a different signing flow that needs the mnemonic. */
  .sp-send-section {
    background: color-mix(in srgb, var(--accent) 5%, var(--surface-1));
    border: 1px solid color-mix(in srgb, var(--accent) 25%, var(--border));
    border-radius: 6px;
    padding: 14px 16px;
  }
  .sp-help {
    font-size: 0.82rem; color: var(--text-muted); line-height: 1.5;
    margin: 0 0 8px;
  }
  .sp-help strong { color: var(--text); }
  .sp-input {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); padding: 8px 10px; font-size: 0.85rem;
    font-family: monospace; width: 100%; box-sizing: border-box;
    outline: none;
  }
  .sp-input:focus { border-color: var(--accent); }
  .sp-seed-label-row { display: flex; align-items: baseline; justify-content: space-between; }
  .sp-seed-reveal {
    background: none; border: none; color: var(--accent); cursor: pointer;
    font-size: 0.78rem; padding: 0;
  }
  .sp-seed-reveal:hover { text-decoration: underline; }
  .sp-warn {
    margin: 8px 0 0; font-size: 0.78rem; color: #e09c52; line-height: 1.5;
    padding: 6px 10px; border-left: 2px solid #e09c52;
  }
  .optional { color: var(--text-muted); font-weight: normal; margin-left: 3px; font-size: 0.78rem; }

  /* Payjoin */
  .pj-section {
    background: color-mix(in srgb, var(--accent) 7%, var(--surface-1));
    border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
    border-radius: 6px;
    padding: 14px 16px;
  }
  .pj-btn {
    margin-top: 12px; width: 100%;
    background: var(--accent); color: var(--accent-contrast, #fff);
    border: none; border-radius: 6px; padding: 10px 14px;
    font-size: 0.9rem; font-weight: 600; cursor: pointer;
  }
  .pj-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .pj-btn-ghost {
    background: none; color: var(--text-muted);
    border: 1px solid var(--border); margin-top: 8px;
  }
  .pj-countdown { font-size: 0.8rem; color: var(--text-muted); margin: 4px 0 0; font-variant-numeric: tabular-nums; }
  .pj-diff { margin: 8px 0 0; padding-left: 18px; font-size: 0.84rem; color: var(--text-muted); line-height: 1.6; }
  .pj-diff strong { color: var(--text); font-variant-numeric: tabular-nums; }
  .pj-done { font-size: 0.92rem; font-weight: 600; color: var(--text); margin: 0 0 6px; }
  .pj-txid { font-family: monospace; font-size: 0.8rem; color: var(--text-muted); word-break: break-all; }
  .pj-done-actions { display: flex; gap: 16px; margin-top: 10px; }
  .pj-spinner {
    display: inline-block; width: 11px; height: 11px; vertical-align: -1px;
    border: 2px solid color-mix(in srgb, var(--accent) 35%, transparent);
    border-top-color: var(--accent); border-radius: 50%;
    animation: pj-spin 0.7s linear infinite;
  }
  @keyframes pj-spin { to { transform: rotate(360deg); } }

  /* Multisig import */
  .ms-progress {
    display: flex; flex-direction: column; gap: 8px;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 5px; padding: 8px 12px;
    width: 100%;
  }
  .ms-progress-header { display: flex; justify-content: space-between; align-items: baseline; }
  .ms-progress-title { font-size: 0.75rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  .ms-progress-count { font-size: 0.78rem; color: var(--text); font-weight: 600; font-variant-numeric: tabular-nums; }
  .ms-signer-list { display: flex; flex-wrap: wrap; gap: 5px; }
  .ms-signer-pill {
    display: inline-flex; align-items: center; gap: 6px;
    background: var(--surface-1); border: 1px solid var(--border);
    border-radius: 999px; padding: 3px 9px;
    font-size: 0.72rem;
  }
  .ms-signer-pill.ms-signed {
    border-color: color-mix(in srgb, #52a875 50%, var(--border));
    background: color-mix(in srgb, #52a875 8%, var(--surface-1));
  }
  .ms-signer-icon { color: var(--text-muted); font-weight: bold; }
  .ms-signed .ms-signer-icon { color: #52a875; }
  .ms-signer-name { color: var(--text); }
  .ms-signer-fp { font-family: monospace; font-size: 0.68rem; color: var(--text-muted); }
  .ms-signed .ms-signer-fp { color: #52a875; }

  .ms-import-section {
    width: 100%; margin-top: 6px;
    border-top: 1px solid var(--border); padding-top: 10px;
    display: flex; flex-direction: column; gap: 6px;
  }
  .ms-import-label { font-size: 0.72rem; color: var(--text-muted); margin: 0; font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; }
  .ms-import-input {
    width: 100%; box-sizing: border-box; font-family: monospace; font-size: 0.72rem; resize: none;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px;
    color: var(--text); padding: 6px 8px; outline: none;
  }
  .ms-import-input:focus { border-color: var(--accent); }
  .ms-import-actions { display: flex; gap: 8px; align-items: center; }
  .ms-import-btn {
    background: none; border: 1px solid var(--border); border-radius: 5px;
    color: var(--text-muted); cursor: pointer; font-size: 0.82rem; padding: 7px 14px;
  }
  .ms-import-btn:hover:not(:disabled) { border-color: var(--accent); color: var(--accent); }
  .ms-import-btn:disabled { opacity: 0.35; cursor: not-allowed; }

  .mono { font-family: monospace; }

  /* Two-step controls */
  .step-back {
    align-self: flex-start;
    background: none; border: none; padding: 4px 20px 0; margin: 0;
    color: var(--text-muted); cursor: pointer; font-size: 0.82rem;
  }
  .step-back:hover { color: var(--text); }
  .preview-cta { width: 100%; }

  @media (max-width: 768px) {
    .send-modal { margin: auto 0 0; border-radius: 12px 12px 0 0; border-bottom: none; max-height: 94vh; }
    .inner { width: 100vw; border-radius: 12px 12px 0 0; }
  }
</style>
