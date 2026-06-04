// Core wallet/chain types are generated from the Rust definitions in
// crates/core/src/types.rs by ts-rs. Regenerate with `just gen-types`.
// Don't hand-edit anything under ./generated.
export type { InputKind } from './generated/InputKind'
export type { WalletEntry } from './generated/WalletEntry'
export type { Balance } from './generated/Balance'
export type { TxRecord } from './generated/TxRecord'
export type { AddressInfo } from './generated/AddressInfo'
export type { AddressKind } from './generated/AddressKind'
export type { UtxoRecord } from './generated/UtxoRecord'
export type { BalancePoint } from './generated/BalancePoint'
export type { SyncResult } from './generated/SyncResult'
export type { NodeStatus } from './generated/NodeStatus'
export type { BackendKind } from './generated/BackendKind'

/// A saved backend in the registry (mirrors `BackendEntry` in
/// crates/server/src/config.rs). The default backend lives in settings, not
/// here; these are additional servers a wallet can be pinned to. `rpc_pass` is
/// blanked on read and preserved if sent empty on update.
export interface BackendEntry {
  id: string
  label: string
  type: 'electrum' | 'rpc'
  /// Electrum server that also speaks BIP-352 (Frigate) — selectable as a Silent
  /// Payments scanner. Connects like Electrum; the flag only gates which wallet
  /// kinds may pick it.
  frigate?: boolean
  electrum_host: string
  electrum_port: number
  electrum_ssl: boolean
  validate_tls: boolean
  ca_cert_path?: string | null
  danger_accept_invalid_certs: boolean
  socks5_proxy?: string | null
  rpc_url: string
  rpc_user: string
  rpc_pass: string
}

/// Live connection state for one backend (from GET /backends/status). `backend`
/// is null for the default backend; match a wallet's `backend` field against it.
export interface BackendStatusEntry {
  backend: string | null
  connected: boolean
  tip_height?: number | null
  error?: string | null
}

export interface FeeBumpResult {
  psbt: string
  fee_sats: number
}

export interface TaxRecord {
  txid: string
  date: string
  type: 'sent' | 'received'
  btc: number
  usd_price: number | null
  usd_value: number | null
  cost_basis: number | null
  gain_loss: number | null
}

export interface DecodedTxInput {
  txid: string
  vout: number
  value_sat: number | null
}

export interface DecodedTxOutput {
  address: string | null
  value_sat: number
}

export interface DecodedTx {
  txid: string
  inputs: DecodedTxInput[]
  outputs: DecodedTxOutput[]
  fee_sat: number | null
  fee_rate_sat_vb: number | null
  vsize: number
  is_rbf: boolean
  vsize_approximate: boolean
}

export interface ConsolidateResult {
  psbt: string
  input_sats: number
  output_sats: number
  fee_sats: number
}

export interface CombineResult {
  psbt: string
  sigs_present: number
  sigs_required: number
  ready: boolean
  /** 8-hex master fingerprints (lowercase) that have contributed at least one signature. */
  signed_fingerprints: string[]
}

export interface MultisigSignerInfo {
  fingerprint: string
  path: string
  xpub: string
}

export interface MultisigDetails {
  threshold: number
  signers: MultisigSignerInfo[]
}

export interface SendResult {
  psbt: string
  input_sats: number
  recipient_sats: number
  change_sats: number
  fee_sats: number
  warnings?: SendWarning[]
}

export type PayjoinStatusKind = 'negotiating' | 'proposal_ready' | 'sent' | 'fell_back' | 'failed'

export type PayjoinBuildResult =
  | { result: 'negotiating'; session_id: string; fallback_txid: string }
  | { result: 'v1_unsupported'; recipient: string; amount_sats: number | null }

export interface PayjoinProposalDiff {
  added_inputs: number
  total_inputs: number
  proposal_fee_sats: number
}

export interface PayjoinStatus {
  status: PayjoinStatusKind
  result_txid: string | null
  diff?: PayjoinProposalDiff
}

export interface PayjoinReceiveProvision {
  session_id: string
  uri: string
  address: string
}

export type SendWarningCode = 'mixed_labels' | 'mixed_categories' | 'repeat_recipient' | 'round_amount_reveal' | 'look_alike' | 'change_script_mismatch'

/** At-rest encryption state. 'off' = no encryption; 'locked' = on, not unlocked;
 *  'unlocked' = on, key in memory. */
export type SecurityState = 'off' | 'locked' | 'unlocked'
export interface SecurityStatus {
  state: SecurityState
}

export interface SendWarning {
  code: SendWarningCode
  severity: 'info' | 'warning'
  message: string
  detail?: string
}

export interface Settings {
  server: {
    port: number
    bind: string
  }
  backend: {
    type: 'electrum' | 'rpc'
    electrum_host: string
    electrum_port: number
    electrum_ssl: boolean
    validate_tls: boolean
    ca_cert_path: string | null
    danger_accept_invalid_certs: boolean
    rpc_url: string
    rpc_user: string
    rpc_pass: string
    mempool_url: string
    poll_interval_secs: number
    show_price_data: boolean
    show_current_price: boolean
    show_fiat_balance: boolean
    socks5_proxy: string | null
    // Accept a self-signed/invalid TLS cert for the mempool server (fees + price).
    mempool_danger_accept_invalid_certs: boolean
    // DoH resolver for BIP-353 name resolution. The resolver sees which name you
    // resolve (= who you're paying); DNSSEC stops forgery, not observation.
    bip353_doh_url: string
    // Default Silent Payments scanner (frigate.2140.dev by default). No longer
    // edited via a settings section — SP wallets pin a Frigate backend, or fall
    // back to this. Kept here so it round-trips untouched through settings saves.
    sp_electrum_host: string
    sp_electrum_port: number
    sp_electrum_ssl: boolean
    sp_validate_tls: boolean
    sp_ca_cert_path: string | null
    sp_danger_accept_invalid_certs: boolean
    sp_socks5_proxy: string | null
    // Payjoin (BIP-77 / v2) send. Off by default; uses the same socks5_proxy.
    payjoin_enabled: boolean
    payjoin_directory_url: string
    payjoin_ohttp_relay_url: string
    payjoin_fallback_secs: number
  }
  network: {
    type: string
  }
  /// Which backend unpinned wallets use: null = the built-in `backend` connection
  /// (the selected public server); otherwise the id of a saved backend.
  default_backend?: string | null
  /// True once the first-run onboarding wizard has been completed or skipped.
  onboarding_complete?: boolean
  /// Air-gapped/offline mode: never open a backend connection. Sync + broadcast
  /// are unavailable; signing + PSBT export still work.
  offline?: boolean
}

/// Projected next-block templates from the mempool (mempool.space shape).
export interface MempoolBlock { blockVSize: number; nTx: number; medianFee: number; feeRange: number[] }

/// Recommended fee rates from the mempool server, in sat/vB.
export interface FeeRates { fastestFee: number; halfHourFee: number; hourFee: number }

export interface SweepFound {
  script_type: string
  address: string
  utxos_found: number
  input_sats: number
  output_sats: number
  fee_sats: number
  signed_tx_hex: string
  txid: string
}
export interface SweepResult { found: SweepFound[]; empty: string[] }

export interface PolicyInfo {
  policy: string
  timelocks: { kind: string; value: number; blocks: boolean; label: string }[]
  key_fingerprints: string[]
  requires_path: boolean
}

export interface SpSpendResult {
  txid: string
  input_sats: number
  recipient_sats: number
  change_sats: number
  fee_sats: number
}

/// A user-defined coin category (privacy compartment) — mirrors corvin_core::types::Category.
export interface Category {
  id: string
  name: string
  color: string
}

/// Category definitions plus assignment maps (address → id, outpoint → id).
export interface CategoryData {
  definitions: Category[]
  addresses: Record<string, string>
  utxos: Record<string, string>
}

/// One classified side of a transaction (output), for the tx-detail flow.
export interface TxIo {
  address: string | null
  value_sats: number
  is_mine: boolean
}

/// Wallet-aware breakdown of a transaction (#31). Outputs classified is_mine;
/// fee present only when inputs' prevouts are known (sent txs).
export interface TxBreakdown {
  outputs: TxIo[]
  fee_sats: number | null
  input_count: number
}
