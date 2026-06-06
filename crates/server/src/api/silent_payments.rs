//! BIP-352 Silent Payments handlers and persistence.
//!
//! Phase 1 of the SP rollout: enable on a wallet (consumes mnemonic, derives
//! scan/spend keys, stores them, returns the `sp1q…` receiving address),
//! list / disable for that wallet. Scanning and sending live in Phase 2/3.
//!
//! Storage: a single JSON file in the config directory keyed by wallet UUID.
//! The scan secret key is the only sensitive piece on disk — the spend
//! secret is intentionally re-derived from the mnemonic at send time (never
//! persisted), matching the rest of Corvin's "no signing keys on disk" model.

use crate::api::ApiError;
use crate::state::{AppState, SilentPaymentsCache, SubCommand, WalletInner};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use bdk_wallet::bitcoin::Network;
use chrono::Utc;
use corvin_core::silent_payments::address_from_stored;
use corvin_core::types::{InputKind, WalletEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::Mutex;
use uuid::Uuid;
use zeroize::Zeroizing;

// ── On-disk schema ────────────────────────────────────────────────────────────

/// A BIP-352 label (m >= 1). The address is cached for display; it's
/// derivable from the scan secret + m. The scanner registers every label so
/// payments to labeled addresses are detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpLabel {
    pub m: u32,
    pub name: String,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredKeys {
    pub scan_secret_hex: String,
    pub spend_pubkey_hex: String,
    /// Cached for display only — derivable from scan_secret + spend_pubkey,
    /// kept here so the UI never has to round-trip the key arithmetic.
    pub address: String,
    /// The Bitcoin network these keys are valid for. SP addresses differ by
    /// network prefix (sp / tsp / sprt), so we record this to refuse using
    /// keys derived under the wrong network after a config change.
    pub network: String,
    /// Block height to begin scanning from (the wallet's birthday) — passed as
    /// Frigate's `subscribe` `start` param so a fresh wallet doesn't rescan the
    /// whole chain. `None` for legacy entries: scan from the server's default.
    #[serde(default)]
    pub birthday_height: Option<u32>,
    /// BIP-352 labeled addresses (m >= 1). Empty = base address only.
    #[serde(default)]
    pub labels: Vec<SpLabel>,
    /// The BIP-352 account index this from-seed wallet was created at, so the
    /// spend secret can be re-derived directly at send time. `None` for
    /// watch-only wallets (no spend key) and legacy entries (fall back to
    /// guessing the account by matching the stored spend pubkey).
    #[serde(default)]
    pub account_index: Option<u32>,
}

/// Snapshot of all SP-enabled wallets — used by the scanner subscriber on
/// startup to know which wallets to open SP sessions for.
pub fn enabled_wallets() -> Vec<(Uuid, StoredKeys)> {
    load().into_iter().collect()
}

type Store = HashMap<Uuid, StoredKeys>;

fn store_path() -> PathBuf {
    crate::config::silent_payments_path()
}

fn load() -> Store {
    // Holds the SP scan secret (non-derivable secret material) — quarantine on
    // corruption rather than silently defaulting + overwriting on the next save.
    crate::state::load_or_quarantine(&store_path())
}

fn save(store: &Store) -> anyhow::Result<()> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Atomic + 0600 + at-rest-sealed via the shared writer (the scan secret is
    // sensitive, so it must be encrypted when the vault is unlocked).
    crate::state::write_private(&path, &serde_json::to_vec_pretty(store)?)?;
    Ok(())
}

/// Persist SP key material for `wallet_id`. Used by both the legacy
/// enable-on-existing flow and the new SP-as-wallet-kind creation endpoint.
pub fn store_keys(
    wallet_id: Uuid,
    scan_secret_hex: String,
    spend_pubkey_hex: String,
    address: String,
    network: String,
    birthday_height: Option<u32>,
    account_index: Option<u32>,
) -> anyhow::Result<()> {
    let mut store = load();
    store.insert(
        wallet_id,
        StoredKeys {
            scan_secret_hex,
            spend_pubkey_hex,
            address,
            network,
            birthday_height,
            labels: Vec::new(),
            account_index,
        },
    );
    save(&store)?;
    crate::state::restrict_to_owner(&store_path());
    Ok(())
}

/// The stored SP keys for one wallet (scan secret + spend pubkey + labels).
/// Used by the SP spend path to re-derive the spend secret and change address.
pub fn get_keys(wallet_id: Uuid) -> Option<StoredKeys> {
    load().remove(&wallet_id)
}

/// Full SP key store — used by the backup exporter. Contains `scan_secret_hex`,
/// so the resulting backup carries real secret material.
pub fn export_keys() -> Store {
    load()
}

/// Overwrite the SP key store from a restored backup. Replaces the whole store
/// (restore is a full replacement), then locks the file to the owner.
pub fn import_keys(store: Store) -> anyhow::Result<()> {
    save(&store)?;
    crate::state::restrict_to_owner(&store_path());
    Ok(())
}

/// Public helper used by the wallet-delete handler to clean up stale SP
/// keys when a wallet is removed.
pub fn forget_wallet(wallet_id: Uuid) -> anyhow::Result<()> {
    let mut store = load();
    if store.remove(&wallet_id).is_some() {
        save(&store)?;
    }
    Ok(())
}

// ── HTTP handlers ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SilentPaymentInfo {
    pub enabled: bool,
    /// Present when enabled — the `sp1q…` (mainnet) / `tsp1q…` / `sprt1q…`
    /// bech32m-encoded receiving address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Network the stored keys belong to. Lets the frontend warn if the
    /// configured network changed since keys were derived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
}

/// Watch-only key material for an SP wallet, returned by the export endpoint.
/// `scan_secret_hex` is the watch key — whoever holds it can see payments
/// received to this wallet (not spend them).
#[derive(Debug, Serialize)]
pub struct SpKeyExport {
    pub scan_secret_hex: String,
    pub spend_pubkey_hex: String,
    pub address: String,
    pub network: String,
}

/// `GET /api/wallets/{id}/silent-payments/export` — the scan secret + spend
/// pubkey for sharing this wallet as a watch-only view. Localhost-only API,
/// consistent with the no-auth threat model.
pub async fn export_silent_payment_keys(
    State(_state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SpKeyExport>, ApiError> {
    let store = load();
    let k = store
        .get(&id)
        .ok_or_else(|| anyhow::anyhow!("no Silent Payments keys for this wallet"))?;
    Ok(Json(SpKeyExport {
        scan_secret_hex: k.scan_secret_hex.clone(),
        spend_pubkey_hex: k.spend_pubkey_hex.clone(),
        address: k.address.clone(),
        network: k.network.clone(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct AddLabelRequest {
    pub name: String,
}

/// `GET /api/wallets/{id}/silent-payments/labels` — list this wallet's
/// BIP-352 labeled addresses.
pub async fn list_silent_payment_labels(
    State(_state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<SpLabel>>, ApiError> {
    let store = load();
    Ok(Json(
        store.get(&id).map(|k| k.labels.clone()).unwrap_or_default(),
    ))
}

/// `POST /api/wallets/{id}/silent-payments/labels` — create a named labeled
/// address. The label index `m` is assigned automatically (next free, m >= 1).
/// The scanner picks up the new label on the next restart.
pub async fn add_silent_payment_label(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<AddLabelRequest>,
) -> Result<Json<SpLabel>, ApiError> {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(anyhow::anyhow!("label name is required").into());
    }
    let network: Network = state.config.read().await.network.kind.to_bitcoin_network();

    let mut store = load();
    let keys = store
        .get_mut(&id)
        .ok_or_else(|| anyhow::anyhow!("not a Silent Payments wallet"))?;

    // m=0 is reserved for change; labels start at 1. Next = max existing + 1.
    let next_m = keys
        .labels
        .iter()
        .map(|l| l.m)
        .max()
        .map(|m| m + 1)
        .unwrap_or(1);

    let scan_bytes = parse_hex_32(&keys.scan_secret_hex)
        .ok_or_else(|| anyhow::anyhow!("stored scan secret is malformed"))?;
    let spend_bytes = parse_hex_33(&keys.spend_pubkey_hex)
        .ok_or_else(|| anyhow::anyhow!("stored spend pubkey is malformed"))?;
    let address = corvin_core::silent_payments::labeled_address_from_stored(
        &scan_bytes,
        &spend_bytes,
        network,
        next_m,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    let label = SpLabel {
        m: next_m,
        name,
        address,
    };
    keys.labels.push(label.clone());
    save(&store)?;
    crate::state::restrict_to_owner(&store_path());

    // Restart this wallet's scanner so it re-subscribes with the new label —
    // otherwise payments to the labeled address aren't detected until restart.
    if let Some(updated) = get_keys(id) {
        crate::sp_subscriber::respawn_one(&state, id, updated).await;
    }

    Ok(Json(label))
}

/// `GET /api/wallets/{id}/silent-payments` — current SP enable state.
pub async fn get_silent_payments(
    State(_state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SilentPaymentInfo>, ApiError> {
    let store = load();
    Ok(Json(match store.get(&id) {
        Some(k) => SilentPaymentInfo {
            enabled: true,
            address: Some(k.address.clone()),
            network: Some(k.network.clone()),
        },
        None => SilentPaymentInfo {
            enabled: false,
            address: None,
            network: None,
        },
    }))
}

// ── SP wallet creation (Sparrow-style; SP is its own wallet kind) ────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "source")]
#[serde(rename_all = "snake_case")]
pub enum SpWalletSource {
    /// Derive scan + spend keys from a BIP-39 mnemonic. The mnemonic is
    /// consumed once and never persisted.
    FromSeed {
        mnemonic: String,
        #[serde(default)]
        passphrase: String,
        #[serde(default)]
        account_index: u32,
    },
    /// Pre-existing keys, e.g. shared by someone else. We can scan for
    /// matches but can't sign. Spend pubkey is needed to compute the
    /// receiving address; scan secret is needed for matching.
    WatchOnly {
        scan_secret_hex: String,
        spend_pubkey_hex: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct CreateSpWalletRequest {
    pub label: String,
    /// Block height to begin scanning from. From-seed wallets pass the current
    /// chain tip (nothing received before now); watch-only imports pass the
    /// user-supplied birthday (or omit for a full scan).
    #[serde(default)]
    pub birthday_height: Option<u32>,
    /// Pinned scanner backend (a saved Frigate-capable server). None = the
    /// global SP-scanner config.
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(flatten)]
    pub source: SpWalletSource,
}

/// `POST /api/wallets/silent-payments` — create a new SP wallet (Sparrow-style:
/// SP is a wallet kind, not a feature attached to an existing wallet).
pub async fn create_silent_payments_wallet(
    State(state): State<AppState>,
    Json(mut req): Json<CreateSpWalletRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let label = req.label.trim().to_string();
    if label.is_empty() {
        return Err(anyhow::anyhow!("label is required").into());
    }

    let network: Network = state.config.read().await.network.kind.to_bitcoin_network();

    // Compute scan_secret + spend_pubkey + address for both source variants.
    // The output is a single canonical (scan_hex, spend_hex, address) tuple.
    let (scan_hex, spend_hex, address, account_index): (String, String, String, Option<u32>) =
        match &mut req.source {
            SpWalletSource::FromSeed {
                mnemonic,
                passphrase,
                account_index,
            } => {
                let m = Zeroizing::new(std::mem::take(mnemonic));
                let p = Zeroizing::new(std::mem::take(passphrase));
                let derived = corvin_core::silent_payments::derive_from_mnemonic(
                    m.as_str(),
                    p.as_str(),
                    network,
                    *account_index,
                )
                .map_err(|e| anyhow::anyhow!("{e}"))?;
                (
                    hex_encode(&derived.scan_secret),
                    hex_encode(&derived.spend_pubkey),
                    derived.address,
                    Some(*account_index),
                )
            }
            SpWalletSource::WatchOnly {
                scan_secret_hex,
                spend_pubkey_hex,
            } => {
                let scan_bytes = parse_hex_32(scan_secret_hex)
                    .ok_or_else(|| anyhow::anyhow!("scan_secret_hex must be 64 hex characters"))?;
                let spend_bytes = parse_hex_33(spend_pubkey_hex)
                    .ok_or_else(|| anyhow::anyhow!("spend_pubkey_hex must be 66 hex characters"))?;
                let addr = address_from_stored(&scan_bytes, &spend_bytes, network)
                    .map_err(|e| anyhow::anyhow!("computing address: {e}"))?;
                (
                    scan_secret_hex.to_lowercase(),
                    spend_pubkey_hex.to_lowercase(),
                    addr,
                    None,
                )
            }
        };

    // Build the WalletEntry. SP wallets have no BDK descriptor — `input`
    // carries the sp1q… address for display, descriptor fields stay empty.
    let id = Uuid::new_v4();
    let entry = WalletEntry {
        id,
        label,
        input: address.clone(),
        kind: InputKind::SilentPayments,
        external_descriptor: String::new(),
        internal_descriptor: None,
        threshold: None,
        backend: req.backend.clone(),
        created_at: Utc::now(),
    };

    // Persist keys to silent_payments.json before adding the wallet — that
    // way a load on next startup finds them and starts a scanner session.
    store_keys(
        id,
        scan_hex.clone(),
        spend_hex.clone(),
        address.clone(),
        format!("{network}"),
        req.birthday_height,
        account_index,
    )?;

    let _ = Mutex::new(()); // satisfy unused-import lint on stable when Mutex isn't reached
    let inner =
        WalletInner::SilentPayments(tokio::sync::Mutex::new(SilentPaymentsCache::default()));

    {
        let mut manager = state.manager.write().await;
        manager.add(entry.clone(), inner);
        crate::state::save_wallets(&manager).await?;
    }

    // SP wallets register zero scripts (the scanner handles discovery), so
    // an empty AddWallet command keeps the subscriber's bookkeeping
    // consistent without forcing it to subscribe to anything.
    let _ = state.sub_tx.send(SubCommand::AddWallet {
        id,
        scripts: Vec::new(),
    });

    // Start scanning immediately rather than only on the next restart — a
    // freshly-created SP wallet must discover incoming payments right away.
    if let Some(keys) = get_keys(id) {
        crate::sp_subscriber::respawn_one(&state, id, keys).await;
    }

    Ok((StatusCode::CREATED, Json(entry)))
}

fn parse_hex_32(s: &str) -> Option<[u8; 32]> {
    let bytes = hex_decode(s)?;
    if bytes.len() != 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Some(out)
}

fn parse_hex_33(s: &str) -> Option<[u8; 33]> {
    let bytes = hex_decode(s)?;
    if bytes.len() != 33 {
        return None;
    }
    let mut out = [0u8; 33];
    out.copy_from_slice(&bytes);
    Some(out)
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks_exact(2) {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ── hex helpers ──────────────────────────────────────────────────────────────

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[derive(Serialize)]
pub struct SpStatusEntry {
    pub wallet_id: Uuid,
    pub connected: bool,
    pub error: Option<String>,
}

/// Live Frigate-scanner connection state per SP wallet, for a per-wallet indicator.
/// SP wallets always talk to their SP (Frigate) backend, never the Electrum default,
/// so their status is tracked separately from `GET /backends/status`.
pub async fn list_sp_status(State(state): State<AppState>) -> Json<Vec<SpStatusEntry>> {
    let m = state.sp_status.read().unwrap_or_else(|e| e.into_inner());
    Json(
        m.iter()
            .map(|(id, v)| SpStatusEntry {
                wallet_id: *id,
                connected: v.connected,
                error: v.error.clone(),
            })
            .collect(),
    )
}
