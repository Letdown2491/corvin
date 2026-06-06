//! Wallet lifecycle: list/add, the import variants (descriptor / seed / HW),
//! delete, rename, set-backend, plus descriptor/policy info reads. Split from
//! mod.rs (concern: CRUD). Shared helpers (get_managed, network_from_config) and
//! the background sync it kicks off stay in mod.rs / sync.rs and are reached via
//! `super::`.
use super::{background_sync, get_managed, network_from_config};
use crate::api::ApiError;
use crate::state::{get_scripts, AppState, SubCommand, WalletInner};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use corvin_core::{
    descriptor::parse_input,
    seed,
    types::{InputKind, WalletEntry},
    wallet,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct AddWalletRequest {
    pub label: String,
    pub input: String,
    #[serde(default)]
    pub backend: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RenameWalletRequest {
    pub label: String,
}

pub async fn list_wallets(
    State(state): State<AppState>,
) -> Result<Json<Vec<WalletEntry>>, ApiError> {
    let manager = state.manager.read().await;
    Ok(Json(manager.list_entries()))
}

pub async fn add_wallet(
    State(state): State<AppState>,
    Json(req): Json<AddWalletRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let network = network_from_config(&state).await;
    let parsed = parse_input(&req.input, network)
        .map_err(|e| ApiError::bad_request(format!("could not parse wallet input: {e}")))?;
    let id = uuid::Uuid::new_v4();

    let entry = WalletEntry {
        id,
        label: req.label.trim().to_string(),
        input: req.input.clone(),
        kind: parsed.kind.clone(),
        external_descriptor: parsed.external,
        internal_descriptor: parsed.internal,
        threshold: None,
        backend: req.backend.clone(),
        created_at: Utc::now(),
    };

    let inner = if parsed.kind == InputKind::Address {
        WalletInner::Address(Mutex::new(None))
    } else {
        let db_path = crate::config::wallet_db_path(id);
        let hd = wallet::open_or_create_wallet(&entry, network, &db_path)?;
        crate::state::restrict_to_owner(&db_path);
        WalletInner::Hd(Mutex::new(hd))
    };

    let managed = {
        let mut manager = state.manager.write().await;
        let m = manager.add(entry.clone(), inner);
        crate::state::save_wallets(&manager).await?;
        m
    };

    // Tell the subscriber about this wallet's scripts.
    let scripts = get_scripts(&managed).await;
    let _ = state.sub_tx.send(SubCommand::AddWallet { id, scripts });

    // Kick off an immediate sync so data appears without waiting for a notification.
    // Skipped in offline mode — there's no backend to reach.
    if !state.config.read().await.offline {
        let state2 = state.clone();
        tokio::spawn(async move { background_sync(state2, id).await });
    }

    Ok((StatusCode::CREATED, Json(entry)))
}

/// `GET /wallets/{id}/policy` — lift the wallet's descriptor to a human-readable
/// spending-policy summary (keys, thresholds, timelocks). Makes imported
/// miniscript/policy wallets and created vaults legible.
pub async fn get_policy(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<corvin_core::descriptor::PolicySummary>, ApiError> {
    let managed = get_managed(&state, &id).await?;
    if managed.entry.external_descriptor.trim().is_empty() {
        return Err(anyhow::anyhow!("this wallet has no descriptor policy").into());
    }
    let summary = corvin_core::descriptor::describe_policy(&managed.entry.external_descriptor)?;
    Ok(Json(summary))
}

/// `GET /wallets/{id}/export-descriptor` — the wallet's policy as a portable
/// multipath descriptor (`/<0;1>/*` + checksum) for importing into Coldcard,
/// Sparrow, Bitcoin Core, etc. (the air-gapped signing path).
pub async fn export_descriptor(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let managed = get_managed(&state, &id).await?;
    if managed.entry.external_descriptor.trim().is_empty() {
        return Err(anyhow::anyhow!("this wallet has no descriptor to export").into());
    }
    let descriptor =
        corvin_core::descriptor::descriptor_to_multipath(&managed.entry.external_descriptor)?;
    Ok(Json(serde_json::json!({ "descriptor": descriptor })))
}

#[derive(Debug, Deserialize)]
pub struct ImportDescriptorRequest {
    pub label: String,
    pub descriptor: String,
    #[serde(default)]
    pub change_descriptor: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
}

pub async fn import_descriptor(
    State(state): State<AppState>,
    Json(req): Json<ImportDescriptorRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let network = network_from_config(&state).await;
    let (parsed, threshold) = corvin_core::descriptor::parse_descriptor_import(
        &req.descriptor,
        req.change_descriptor.as_deref(),
    )?;

    let id = Uuid::new_v4();
    let entry = WalletEntry {
        id,
        label: req.label.trim().to_string(),
        input: req.descriptor.trim().to_string(),
        kind: parsed.kind.clone(),
        external_descriptor: parsed.external,
        internal_descriptor: parsed.internal,
        threshold,
        backend: req.backend.clone(),
        created_at: Utc::now(),
    };

    let db_path = crate::config::wallet_db_path(id);
    // open_or_create_wallet is also our network/validity backstop: BDK rejects
    // descriptors whose keys don't match the configured network.
    let hd = wallet::open_or_create_wallet(&entry, network, &db_path).map_err(|e| {
        anyhow::anyhow!("descriptor rejected (does it match the {network} network?): {e}")
    })?;
    crate::state::restrict_to_owner(&db_path);
    let inner = WalletInner::Hd(Mutex::new(hd));

    let managed = {
        let mut manager = state.manager.write().await;
        let m = manager.add(entry.clone(), inner);
        crate::state::save_wallets(&manager).await?;
        m
    };

    let scripts = get_scripts(&managed).await;
    let _ = state.sub_tx.send(SubCommand::AddWallet { id, scripts });

    let state2 = state.clone();
    tokio::spawn(async move { background_sync(state2, id).await });

    Ok((StatusCode::CREATED, Json(entry)))
}

#[derive(Deserialize)]
pub struct GenerateSeedRequest {
    pub words: usize,
}

#[derive(Serialize)]
pub struct GenerateSeedResponse {
    pub mnemonic: String,
}

pub async fn generate_seed(
    Json(req): Json<GenerateSeedRequest>,
) -> Result<Json<GenerateSeedResponse>, ApiError> {
    if req.words != 12 && req.words != 24 {
        return Err(anyhow::anyhow!("word count must be 12 or 24").into());
    }
    let mnemonic = seed::generate_mnemonic(req.words).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(Json(GenerateSeedResponse { mnemonic }))
}

#[derive(Deserialize)]
pub struct SeedImportRequest {
    pub label: String,
    pub mnemonic: String,
    pub passphrase: String,
    pub script_type: String,
    pub account_index: u32,
    pub custom_path: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
}

pub async fn import_seed_wallet(
    State(state): State<AppState>,
    Json(mut req): Json<SeedImportRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let network = network_from_config(&state).await;

    let path = req.custom_path.clone().unwrap_or_else(|| {
        seed::default_derivation_path(&req.script_type, network, req.account_index)
    });

    // Move sensitive fields into Zeroizing wrappers so their memory is wiped
    // when the handler returns (whether via Ok or error). Replace the originals
    // with empty strings so the SeedImportRequest's Drop doesn't leave the
    // seed bytes around.
    let mnemonic = zeroize::Zeroizing::new(std::mem::take(&mut req.mnemonic));
    let passphrase = zeroize::Zeroizing::new(std::mem::take(&mut req.passphrase));

    let descriptors =
        seed::derive_descriptors(&mnemonic, &passphrase, &path, &req.script_type, network)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

    let kind = match req.script_type.as_str() {
        "taproot" => InputKind::Taproot,
        "wrapped_segwit" => InputKind::Ypub,
        "legacy" => InputKind::Xpub,
        _ => InputKind::Zpub,
    };

    let id = Uuid::new_v4();
    let entry = WalletEntry {
        id,
        label: req.label.trim().to_string(),
        input: path,
        kind,
        external_descriptor: descriptors.external,
        internal_descriptor: Some(descriptors.internal),
        threshold: None,
        backend: req.backend.clone(),
        created_at: Utc::now(),
    };

    let db_path = crate::config::wallet_db_path(id);
    let hd = wallet::open_or_create_wallet(&entry, network, &db_path)?;
    crate::state::restrict_to_owner(&db_path);
    let inner = WalletInner::Hd(Mutex::new(hd));

    let managed = {
        let mut manager = state.manager.write().await;
        let m = manager.add(entry.clone(), inner);
        crate::state::save_wallets(&manager).await?;
        m
    };

    let scripts = get_scripts(&managed).await;
    let _ = state.sub_tx.send(SubCommand::AddWallet { id, scripts });

    let state2 = state.clone();
    tokio::spawn(async move { background_sync(state2, id).await });

    Ok((StatusCode::CREATED, Json(entry)))
}

// ── Hardware wallet import ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct HwImportRequest {
    pub label: String,
    pub xpub: String,
    pub fingerprint: String,
    pub path: String,
    pub account_type: String,
    #[serde(default)]
    pub backend: Option<String>,
}

pub async fn import_hw_wallet(
    State(state): State<AppState>,
    Json(req): Json<HwImportRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let network = network_from_config(&state).await;

    let parsed = corvin_core::descriptor::descriptor_from_hw_xpub(
        &req.xpub,
        &req.fingerprint,
        &req.path,
        &req.account_type,
        network,
    )
    .map_err(|e| anyhow::anyhow!("invalid xpub: {e}"))?;

    let id = Uuid::new_v4();
    let entry = WalletEntry {
        id,
        label: req.label.trim().to_string(),
        input: req.xpub.trim().to_string(),
        kind: parsed.kind,
        external_descriptor: parsed.external,
        internal_descriptor: parsed.internal,
        threshold: None,
        backend: req.backend.clone(),
        created_at: Utc::now(),
    };

    let db_path = crate::config::wallet_db_path(id);
    let hd = wallet::open_or_create_wallet(&entry, network, &db_path)?;
    crate::state::restrict_to_owner(&db_path);
    let inner = WalletInner::Hd(Mutex::new(hd));

    let managed = {
        let mut manager = state.manager.write().await;
        let m = manager.add(entry.clone(), inner);
        crate::state::save_wallets(&manager).await?;
        m
    };

    let scripts = get_scripts(&managed).await;
    let _ = state.sub_tx.send(SubCommand::AddWallet { id, scripts });

    let state2 = state.clone();
    tokio::spawn(async move { background_sync(state2, id).await });

    Ok((StatusCode::CREATED, Json(entry)))
}

pub async fn delete_wallet(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let removed = {
        let mut manager = state.manager.write().await;
        if manager.remove(&id) {
            crate::state::save_wallets(&manager).await?;
            true
        } else {
            false
        }
    };
    if removed {
        let _ = state.sub_tx.send(SubCommand::RemoveWallet(id));
        let db_path = crate::config::wallet_db_path(id);
        if db_path.exists() {
            let _ = std::fs::remove_file(&db_path);
        }
        // Clean up any Ledger multisig registration HMACs associated with this
        // wallet so they don't accumulate on disk after the wallet is gone.
        if let Err(e) = crate::api::ledger_hmac_store::forget_wallet(id) {
            tracing::warn!("Couldn't clear Ledger HMACs for {id}: {e}");
        }
        // Stop the SP scanner task FIRST, before clearing keys/outputs — otherwise
        // a still-running scanner holds the scan secret in memory, stays subscribed
        // to the third-party SP server, and can re-create the sp_outputs.json we're
        // about to delete on its next match.
        if let Some(handle) = state.sp_scanners.lock().await.remove(&id) {
            handle.abort();
        }
        state.sp_status_remove(id);
        if let Err(e) = crate::api::silent_payments::forget_wallet(id) {
            tracing::warn!("Couldn't clear silent-payment keys for {id}: {e}");
        }
        if let Err(e) = crate::sp_outputs::forget_wallet(id) {
            tracing::warn!("Couldn't clear silent-payment outputs for {id}: {e}");
        }
        // Abort any running payjoin poll tasks for this wallet, then drop the
        // sessions from disk.
        for (sid, _) in crate::payjoin_sessions::list_for_wallet(id) {
            if let Some(handle) = state.payjoin_tasks.lock().await.remove(&sid) {
                handle.abort();
            }
        }
        if let Err(e) = crate::payjoin_sessions::forget_wallet(id) {
            tracing::warn!("Couldn't clear payjoin sessions for {id}: {e}");
        }
        Ok(StatusCode::NO_CONTENT)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}

pub async fn rename_wallet(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<RenameWalletRequest>,
) -> Result<Json<WalletEntry>, ApiError> {
    let label = req.label.trim().to_string();
    if label.is_empty() {
        return Err(anyhow::anyhow!("label cannot be empty").into());
    }
    let manager = state.manager.write().await;
    if !manager.rename(&id, label) {
        return Err(ApiError::not_found("wallet not found"));
    }
    crate::state::save_wallets(&manager).await?;
    let entry = manager
        .get(&id)
        .map(|m| {
            let mut e = m.entry.clone();
            e.label = m.label();
            e
        })
        .ok_or_else(|| ApiError::not_found("wallet not found"))?;
    Ok(Json(entry))
}

#[derive(Debug, Deserialize)]
pub struct SetBackendRequest {
    /// Backend id to pin the wallet to, or null for the default backend.
    #[serde(default)]
    pub backend: Option<String>,
}

/// `PUT /wallets/{id}/backend` — change which backend a wallet syncs/broadcasts
/// through (or scans, for SP). Re-routes the wallet to the right subscriber and
/// re-syncs against the new server.
pub async fn set_wallet_backend(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<SetBackendRequest>,
) -> Result<Json<WalletEntry>, ApiError> {
    if let Some(b) = &req.backend {
        if state.config.read().await.backend_entry(b).is_none() {
            return Err(anyhow::anyhow!("unknown backend '{b}'").into());
        }
    }

    let entry = {
        let manager = state.manager.write().await;
        if !manager.set_backend(&id, req.backend.clone()) {
            return Err(ApiError::not_found("wallet not found"));
        }
        crate::state::save_wallets(&manager).await?;
        manager
            .get(&id)
            .map(|m| {
                let mut e = m.entry.clone();
                e.label = m.label();
                e.backend = m.backend();
                e
            })
            .ok_or_else(|| ApiError::not_found("wallet not found"))?
    };

    // Re-route to the right subscriber for the new backend. SP wallets use the
    // dedicated scanner (respawn it); everything else the electrum subscriber
    // (a fresh AddWallet makes the supervisor move it to the new group).
    let managed = get_managed(&state, &id).await?;
    if matches!(managed.inner, WalletInner::SilentPayments(_)) {
        if let Some((_, keys)) = crate::api::silent_payments::enabled_wallets()
            .into_iter()
            .find(|(wid, _)| *wid == id)
        {
            crate::sp_subscriber::respawn_one(&state, id, keys).await;
        }
    } else {
        let scripts = crate::state::get_scripts(&managed).await;
        let _ = state
            .sub_tx
            .send(crate::state::SubCommand::AddWallet { id, scripts });
    }

    // Refresh against the new backend.
    let s = state.clone();
    tokio::spawn(async move { background_sync(s, id).await });

    Ok(Json(entry))
}
