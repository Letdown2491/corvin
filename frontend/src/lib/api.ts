import type { AddressInfo, BackendEntry, BackendStatusEntry, Balance, BalancePoint, Category, CategoryData, CombineResult, ConsolidateResult, DecodedTx, FeeBumpResult, FeeRates, MempoolBlock, MultisigDetails, NodeStatus, PayjoinBuildResult, PayjoinReceiveProvision, PayjoinStatus, PayjoinStatusKind, PolicyInfo, SecurityStatus, SendResult, Settings, SpSpendResult, SweepResult, SyncResult, TaxRecord, TxBreakdown, TxRecord, UtxoRecord, WalletEntry } from './types'
import { serverReachable } from '../stores/server'

const BASE = '/api'

/// Error thrown for a non-OK API response. Carries the server's stable machine
/// `code` (e.g. 'not_found', 'wrong_secret', 'insufficient_funds') and the HTTP
/// status alongside the human message, so callers can branch on `e.code` instead
/// of fragile message string-matching. Most call sites just show `e.message`.
export class ApiErr extends Error {
  code?: string
  status: number
  constructor(message: string, status: number, code?: string) {
    super(message)
    this.name = 'ApiErr'
    this.status = status
    this.code = code
  }
}

async function safeFetch(input: string, init?: RequestInit): Promise<Response> {
  try {
    const res = await fetch(input, init)
    serverReachable.set(true)
    return res
  } catch (e) {
    // An intentional abort (e.g. a superseded poll) isn't an unreachable
    // server — let it propagate without flipping the reachability flag.
    if (e instanceof DOMException && e.name === 'AbortError') throw e
    serverReachable.set(false)
    throw new Error('Cannot reach the server — is Corvin running?', { cause: e })
  }
}

async function req<T>(method: string, path: string, body?: unknown, opts?: { signal?: AbortSignal }): Promise<T> {
  const res = await safeFetch(`${BASE}${path}`, {
    method,
    headers: body ? { 'Content-Type': 'application/json' } : undefined,
    body: body ? JSON.stringify(body) : undefined,
    signal: opts?.signal,
  })
  if (!res.ok) {
    const err = await res.json().catch(() => ({}))
    throw new ApiErr(err.error ?? `Server error (${res.status})`, res.status, err.code)
  }
  if (res.status === 204) return undefined as unknown as T
  return res.json()
}

export const api = {
  wallets: {
    list: () => req<WalletEntry[]>('GET', '/wallets'),
    add: (label: string, input: string) => req<WalletEntry>('POST', '/wallets', { label, input }),
    setBackend: (id: string, backend: string | null) =>
      req<WalletEntry>('PUT', `/wallets/${id}/backend`, { backend }),
    seedGenerate: (words: 12 | 24) =>
      req<{ mnemonic: string }>('POST', '/wallets/seed/generate', { words }),
    seedImport: (body: {
      label: string; mnemonic: string; passphrase: string
      script_type: string; account_index: number; custom_path: string | null
      backend?: string | null
    }) => req<WalletEntry>('POST', '/wallets/seed/import', body),
    hwImport: (body: {
      label: string; xpub: string; fingerprint: string; path: string; account_type: string
      backend?: string | null
    }) => req<WalletEntry>('POST', '/wallets/hwi/import', body),
    multisigCreate: (body: {
      label: string
      threshold: number
      signers: Array<{ fingerprint: string; path: string; xpub: string }>
      backend?: string | null
    }) => req<WalletEntry>('POST', '/wallets/multisig/create', body),
    importDescriptor: (body: {
      label: string; descriptor: string; change_descriptor: string | null
      backend?: string | null
    }) => req<WalletEntry>('POST', '/wallets/import-descriptor', body),
    createVault: (body: {
      label: string
      threshold: number
      primary: Array<{ fingerprint: string; path: string; xpub: string }>
      recovery_threshold: number
      recovery: Array<{ fingerprint: string; path: string; xpub: string }>
      timelock_blocks?: number | null
      timelock_height?: number | null
      taproot?: boolean
      backend?: string | null
    }) => req<WalletEntry>('POST', '/wallets/create-vault', body),
    createTimelocked: (body: {
      label: string
      signer: { fingerprint: string; path: string; xpub: string }
      timelock_blocks?: number | null
      timelock_height?: number | null
      taproot?: boolean
      backend?: string | null
    }) => req<WalletEntry>('POST', '/wallets/create-timelocked', body),
    combinePsbt: (id: string, body: { psbt_a: string; psbt_b: string }) =>
      req<CombineResult>('POST', `/wallets/${id}/combine-psbt`, body),
    multisigInfo: (id: string) =>
      req<MultisigDetails>('GET', `/wallets/${id}/multisig-info`),
    remove: (id: string) => req<void>('DELETE', `/wallets/${id}`),
    rename: (id: string, label: string) => req<WalletEntry>('PATCH', `/wallets/${id}`, { label }),
    balance: (id: string) => req<Balance>('GET', `/wallets/${id}/balance`),
    txs: (id: string) => req<TxRecord[]>('GET', `/wallets/${id}/txs`),
    txBreakdown: (id: string, txid: string) => req<TxBreakdown>('GET', `/wallets/${id}/tx/${txid}`),
    addresses: (id: string) => req<AddressInfo[]>('GET', `/wallets/${id}/addresses`),
    utxos: (id: string) => req<UtxoRecord[]>('GET', `/wallets/${id}/utxos`),
    policy: (id: string) => req<PolicyInfo>('GET', `/wallets/${id}/policy`),
    exportDescriptor: (id: string) => req<{ descriptor: string }>('GET', `/wallets/${id}/export-descriptor`),
    balanceHistory: (id: string) => req<BalancePoint[]>('GET', `/wallets/${id}/balance-history`),
    sync: (id: string) => req<SyncResult>('POST', `/wallets/${id}/sync`),
    taxReport: (id: string, year: number, method: string) => {
      const p = new URLSearchParams({ year: String(year), method })
      return req<TaxRecord[]>('GET', `/wallets/${id}/tax-report?${p}`)
    },
    consolidatePsbt: (id: string, body: { utxos: string[]; fee_rate_sat_vb: number; destination: string }) =>
      req<ConsolidateResult>('POST', `/wallets/${id}/consolidate-psbt`, body),
    sendPsbt: (id: string, body: {
      outputs: { recipient: string; amount_sats: number | null }[]
      fee_rate_sat_vb: number
      utxos?: string[]
      spend_path?: 'primary' | 'recovery'
    }) => req<SendResult>('POST', `/wallets/${id}/send-psbt`, body),
    /// Silent Payments send. Same shape as sendPsbt + a mnemonic + passphrase,
    /// since BIP-352 needs the sender's input private keys at build time.
    spSend: (id: string, body: {
      outputs: { recipient: string; amount_sats: number | null }[]
      fee_rate_sat_vb: number
      utxos?: string[]
      mnemonic?: string
      passphrase?: string
      estimate_only?: boolean
    }) => req<SendResult>('POST', `/wallets/${id}/sp-send`, body),
    spSpend: (id: string, body: {
      outputs: { recipient: string; amount_sats: number | null }[]
      fee_rate_sat_vb: number
      mnemonic: string
      passphrase?: string
      utxos?: string[]
    }) => req<SpSpendResult>('POST', `/wallets/${id}/sp-spend`, body),
    rbfPsbt: (id: string, body: { txid: string; fee_rate_sat_vb: number }) =>
      req<FeeBumpResult>('POST', `/wallets/${id}/rbf-psbt`, body),
    cpfpPsbt: (id: string, body: { txid: string; fee_rate_sat_vb: number }) =>
      req<FeeBumpResult>('POST', `/wallets/${id}/cpfp-psbt`, body),
    /// Payjoin (BIP-77 / v2) send, software single-sig only. `build` signs the
    /// original and opens a session; the receiver's proposal arrives async
    /// (poll `status` or wait for the `payjoin_proposal_ready` SSE), then
    /// `confirm` re-signs + broadcasts. `abandon` broadcasts the original.
    payjoinSend: {
      build: (id: string, body: {
        uri: string
        fee_rate_sat_vb: number
        utxos?: string[]
        mnemonic: string
        passphrase?: string
      }) => req<PayjoinBuildResult>('POST', `/wallets/${id}/payjoin-send`, body),
      status: (id: string, sid: string) =>
        req<PayjoinStatus>('GET', `/wallets/${id}/payjoin-send/${sid}`),
      confirm: (id: string, sid: string, body: { mnemonic: string; passphrase?: string }) =>
        req<{ txid: string }>('POST', `/wallets/${id}/payjoin-send/${sid}/confirm`, body),
      abandon: (id: string, sid: string) =>
        req<{ txid: string }>('DELETE', `/wallets/${id}/payjoin-send/${sid}`),
    },
    /// Payjoin (BIP-77) receive. `provision` returns a pj= URI to share; a
    /// background task negotiates when a payer pays it. On the
    /// `payjoin_receive_proposal` SSE, `confirm` (with seed) signs our
    /// contributed input and posts the proposal back. Needs an RPC backend.
    payjoinReceive: {
      provision: (id: string, body: { amount_sats?: number }) =>
        req<PayjoinReceiveProvision>('POST', `/wallets/${id}/payjoin-receive`, body),
      list: (id: string) =>
        req<{ session_id: string; status: PayjoinStatusKind; created_at: string; uri?: string }[]>(
          'GET', `/wallets/${id}/payjoin-receive`),
      status: (id: string, sid: string) =>
        req<{ status: PayjoinStatusKind }>('GET', `/wallets/${id}/payjoin-receive/${sid}`),
      confirm: (id: string, sid: string, body: { mnemonic: string; passphrase?: string }) =>
        req<{ status: PayjoinStatusKind }>('POST', `/wallets/${id}/payjoin-receive/${sid}/confirm`, body),
      cancel: (id: string, sid: string) =>
        req<void>('DELETE', `/wallets/${id}/payjoin-receive/${sid}`),
    },
  },
  // Backend `set` with an empty note removes the label, so we don't need a
  // dedicated DELETE endpoint from the frontend's perspective.
  labels: {
    list: () => req<Record<string, string>>('GET', '/labels'),
    set: (txid: string, note: string) => req<void>('PUT', `/labels/${txid}`, { note }),
  },
  addressLabels: {
    list: () => req<Record<string, string>>('GET', '/address-labels'),
    set: (address: string, note: string) =>
      req<void>('PUT', `/address-labels/${encodeURIComponent(address)}`, { note }),
    delete: (address: string) =>
      req<void>('DELETE', `/address-labels/${encodeURIComponent(address)}`),
  },
  costBasis: {
    list: () => req<Record<string, number>>('GET', '/cost-basis'),
    set: (txid: string, usd: number) => req<void>('PUT', `/cost-basis/${txid}`, { usd }),
    delete: (txid: string) => req<void>('DELETE', `/cost-basis/${txid}`),
  },
  price: {
    historical: (timestamp: number) => req<{ usd: number | null }>('GET', `/price?timestamp=${timestamp}`),
    // Polled from the layout; pass an AbortSignal so a superseded poll cancels.
    current: (signal?: AbortSignal) => req<{ usd: number | null }>('GET', '/price/current', undefined, { signal }),
  },
  // mempool.space proxies. `fees` returns null when no mempool server is
  // configured (the backend answers 404), which the sidebar treats as "off".
  proxy: {
    fees: async (): Promise<FeeRates | null> => {
      const res = await safeFetch(`${BASE}/proxy/fees`)
      if (res.status === 404) return null
      if (!res.ok) throw new Error(`Server error (${res.status})`)
      return res.json()
    },
    mempoolBlocks: () => req<MempoolBlock[]>('GET', '/proxy/mempool-blocks'),
  },
  // Backend `set` with an empty note removes the label — DELETE is redundant.
  utxoLabels: {
    list: () => req<Record<string, string>>('GET', '/utxo-labels'),
    set: (txid: string, vout: number, note: string) =>
      req<void>('PUT', `/utxo-labels/${txid}/${vout}`, { note }),
  },
  categories: {
    list: () => req<CategoryData>('GET', '/categories'),
    create: (name: string, color: string) => req<Category>('POST', '/categories', { name, color }),
    update: (id: string, name: string, color: string) =>
      req<void>('PUT', `/categories/${encodeURIComponent(id)}`, { name, color }),
    delete: (id: string) => req<void>('DELETE', `/categories/${encodeURIComponent(id)}`),
    assignAddress: (address: string, categoryId: string | null) =>
      req<void>('PUT', `/categories/address/${encodeURIComponent(address)}`, { category_id: categoryId }),
    assignUtxo: (outpoint: string, categoryId: string | null) =>
      req<void>('PUT', `/categories/utxo/${encodeURIComponent(outpoint)}`, { category_id: categoryId }),
  },
  utxoFreeze: {
    list: () => req<string[]>('GET', '/utxo-freeze'),
    freeze: (txid: string, vout: number) =>
      req<void>('PUT', `/utxo-freeze/${txid}/${vout}`),
    unfreeze: (txid: string, vout: number) =>
      req<void>('DELETE', `/utxo-freeze/${txid}/${vout}`),
  },
  settings: {
    get: () => req<Settings>('GET', '/settings'),
    update: (s: Settings) => req<Settings>('PUT', '/settings', s),
  },
  /// At-rest encryption (the config dir). See docs/at-rest-encryption.md.
  security: {
    status: () => req<SecurityStatus>('GET', '/security/status'),
    unlock: (password: string) => req<SecurityStatus>('POST', '/security/unlock', { password }),
    enable: (password: string) => req<SecurityStatus>('POST', '/security/enable', { password }),
    disable: (password: string) => req<SecurityStatus>('POST', '/security/disable', { password }),
    changePassword: (current_password: string, new_password: string) =>
      req<SecurityStatus>('POST', '/security/change-password', { current_password, new_password }),
  },
  /// Saved backends a wallet can be pinned to (the default backend lives in settings).
  backends: {
    list: () => req<BackendEntry[]>('GET', '/backends'),
    create: (b: BackendEntry) => req<BackendEntry>('POST', '/backends', b),
    update: (id: string, b: BackendEntry) => req<BackendEntry>('PUT', `/backends/${id}`, b),
    delete: (id: string) => req<void>('DELETE', `/backends/${id}`),
    status: () => req<BackendStatusEntry[]>('GET', '/backends/status'),
    test: (b: BackendEntry) => req<NodeStatus>('POST', '/backends/test', b),
    /// Move the built-in default connection into the registry and pin it as the
    /// default (server-side, so a custom RPC default keeps its password).
    adoptDefault: () => req<BackendEntry>('POST', '/backends/adopt-default'),
  },
  status: () => req<NodeStatus>('GET', '/status'),
  version: () =>
    req<{ version: string; os: string; arch: string; hw_enabled: boolean }>('GET', '/version'),
  /// Mark first-run onboarding done (wizard finished or skipped).
  completeOnboarding: () => req<void>('POST', '/onboarding/complete'),
  /// Clear the onboarding flag so the wizard shows again.
  resetOnboarding: () => req<void>('POST', '/onboarding/reset'),
  /// Wake the subscriber from backoff sleep (called after laptop resume).
  reconnect: () => req<void>('POST', '/backend/reconnect'),
  testStatus: (s: Settings) => req<NodeStatus>('POST', '/status/test', s),
  testMempool: (url: string, socks5_proxy: string | null, danger_accept_invalid_certs: boolean) =>
    req<{ ok: boolean; msg: string }>('POST', '/status/test-mempool', { url, socks5_proxy, danger_accept_invalid_certs }),
  broadcast: {
    decode: (payload: { psbt?: string; raw_hex?: string }) => req<DecodedTx>('POST', '/decode', payload),
    broadcast: (payload: { psbt?: string; raw_hex?: string; wallet_id?: string }) => req<{ txid: string }>('POST', '/broadcast', payload),
  },
  hwi: {
    signStart: (psbt: string, wallet_id?: string) =>
      req<{ token: string }>('POST', '/hwi/sign', { psbt, wallet_id }),
  },
  backup: {
    export: async (): Promise<Blob> => {
      const res = await safeFetch('/api/backup')
      if (!res.ok) {
        const err = await res.json().catch(() => ({}))
        throw new ApiErr(err.error ?? `Server error (${res.status})`, res.status, err.code)
      }
      return res.blob()
    },
    restore: (data: unknown) => req<void>('POST', '/restore', data),
  },
  messages: {
    verify: (body: { address: string; message: string; signature: string }) =>
      req<{ valid: boolean; error?: string }>('POST', '/messages/verify', body),
    sign: (wallet_id: string, body: { address: string; message: string; mnemonic: string; passphrase?: string }) =>
      req<{ signature: string }>('POST', `/wallets/${wallet_id}/sign-message`, body),
  },
  silentPayments: {
    get: (wallet_id: string) =>
      req<{ enabled: boolean; address?: string; network?: string }>(
        'GET', `/wallets/${wallet_id}/silent-payments`,
      ),
    exportKeys: (wallet_id: string) =>
      req<{ scan_secret_hex: string; spend_pubkey_hex: string; address: string; network: string }>(
        'GET', `/wallets/${wallet_id}/silent-payments/export`,
      ),
    listLabels: (wallet_id: string) =>
      req<{ m: number; name: string; address: string }[]>(
        'GET', `/wallets/${wallet_id}/silent-payments/labels`,
      ),
    addLabel: (wallet_id: string, name: string) =>
      req<{ m: number; name: string; address: string }>(
        'POST', `/wallets/${wallet_id}/silent-payments/labels`, { name },
      ),
    /// Create a standalone Silent Payments wallet (Sparrow-style).
    createWallet: (body:
      | { label: string; source: 'from_seed'; mnemonic: string; passphrase?: string; account_index?: number; birthday_height?: number | null; backend?: string | null }
      | { label: string; source: 'watch_only'; scan_secret_hex: string; spend_pubkey_hex: string; birthday_height?: number | null; backend?: string | null }
    ) => req<WalletEntry>('POST', '/wallets/silent-payments', body),
  },
  resolveName: (name: string) =>
    req<{ uri: string; hrn: string }>('POST', '/resolve-name', { name }),
  sweep: (body: { wif: string; destination: string; fee_rate_sat_vb: number }) =>
    req<SweepResult>('POST', '/sweep', body),
  testBackup: (wallet_id: string, body: { mnemonic: string; passphrase?: string }) =>
    req<{ matches: boolean; message: string }>(
      'POST', `/wallets/${wallet_id}/test-backup`, body,
    ),
  bip329: {
    export: async (): Promise<Blob> => {
      const res = await safeFetch('/api/labels/export-bip329')
      if (!res.ok) {
        const err = await res.json().catch(() => ({}))
        throw new ApiErr(err.error ?? `Server error (${res.status})`, res.status, err.code)
      }
      return res.blob()
    },
    import: (body: { jsonl: string; replace?: boolean }) =>
      req<{ tx_labels: number; address_labels: number; utxo_labels: number; frozen_changes: number; skipped: number }>(
        'POST', '/labels/import-bip329', body
      ),
  },
}

/// Subscribe to server-sent events. Reconnects on error, visibility,
/// and online events so the stream survives laptop sleep / network drops.
export function subscribeEvents(handlers: {
  sync_started?: (payload: unknown) => void
  sync_complete?: (payload: unknown) => void
  /// Emitted by the SP scanner subscriber when a new BIP-352 output is
  /// discovered for an SP wallet. Frontend should refetch that wallet's
  /// balance/utxos/txs so the new payment appears without waiting for the
  /// next poll tick.
  sp_output_discovered?: (payload: unknown) => void
  /// Payjoin (BIP-77) sender-session events: the receiver's proposal is ready
  /// to confirm, the payjoin was broadcast, or it fell back to the original.
  payjoin_proposal_ready?: (payload: unknown) => void
  payjoin_sent?: (payload: unknown) => void
  payjoin_fell_back?: (payload: unknown) => void
  /// Payjoin receive: a payer paid our pj= URI and the proposal is built +
  /// awaiting our seed to sign; or it's been signed + posted.
  payjoin_receive_proposal?: (payload: unknown) => void
  payjoin_receive_sent?: (payload: unknown) => void
  error?: (payload: unknown) => void
}): () => void {
  let es: EventSource | null = null
  let closed = false
  let backoff = 1000 // ms; exponential up to 30s, reset on a healthy open.

  function connect() {
    if (closed) return
    // Already open or mid-connect (e.g. a focus/online event fired while the
    // stream is healthy) — don't tear down a working connection.
    if (es && es.readyState !== EventSource.CLOSED) return
    if (es) { es.close(); es = null }
    es = new EventSource('/api/events')
    es.onopen = () => { backoff = 1000 }
    for (const [event, handler] of Object.entries(handlers)) {
      if (handler) {
        es.addEventListener(event, (e: MessageEvent) => {
          try { handler(JSON.parse(e.data)) } catch { handler(e.data) }
        })
      }
    }
    es.onerror = () => {
      // Reopen if the browser gave up, backing off so we don't hammer a
      // down server every second.
      if (es && es.readyState === EventSource.CLOSED) {
        setTimeout(connect, backoff)
        backoff = Math.min(backoff * 2, 30_000)
      }
    }
  }

  function onVisible() {
    if (document.visibilityState === 'visible') connect()
  }
  function onOnline() { connect() }

  connect()
  document.addEventListener('visibilitychange', onVisible)
  window.addEventListener('online', onOnline)

  return () => {
    closed = true
    document.removeEventListener('visibilitychange', onVisible)
    window.removeEventListener('online', onOnline)
    if (es) { es.close(); es = null }
  }
}
