use crate::api::silent_payments::StoredKeys;
use crate::api::ApiError;
use crate::config::{save_config, wallet_db_path, Config};
use crate::state::{
    get_scripts, restrict_to_owner, save_wallets, AnnotationData, AppState, CategoryData,
    SilentPaymentsCache, SubCommand, WalletInner,
};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::Utc;
use corvin_core::types::{InputKind, WalletEntry};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct Backup {
    pub version: u32,
    pub exported_at: String,
    pub wallets: Vec<WalletEntry>,
    pub labels: HashMap<String, String>,
    pub cost_basis: HashMap<String, f64>,
    pub settings: Config,
    #[serde(default)]
    pub address_labels: HashMap<String, String>,
    // v2 additions. `#[serde(default)]` keeps v1 backups (which lack these)
    // restorable. `silent_payments` carries `scan_secret_hex` — a real secret,
    // which is why the UI requires a passphrase when it's non-empty.
    #[serde(default)]
    pub utxo_labels: HashMap<String, String>,
    #[serde(default)]
    pub frozen_utxos: HashSet<String>,
    #[serde(default)]
    pub silent_payments: HashMap<Uuid, StoredKeys>,
    /// v3 addition: coin categories (definitions + address/UTXO assignments).
    #[serde(default)]
    pub categories: CategoryData,
}

pub async fn export_backup(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let wallets = state.manager.read().await.list_entries();
    let labels = state.annotations.tx_labels().await;
    let cost_basis = state.annotations.cost_basis().await;
    let mut settings = state.config.read().await.clone();
    settings.backend.rpc_pass = String::new();
    let address_labels = state.annotations.address_labels().await;
    let utxo_labels = state.annotations.utxo_labels().await;
    let frozen_utxos = state.annotations.frozen_utxos().await;
    let silent_payments = crate::api::silent_payments::export_keys();
    let categories = state.annotations.categories().await;

    let now = Utc::now();
    let backup = Backup {
        version: 3,
        exported_at: now.to_rfc3339(),
        wallets,
        labels,
        cost_basis,
        settings,
        address_labels,
        utxo_labels,
        frozen_utxos,
        silent_payments,
        categories,
    };

    let json = serde_json::to_string_pretty(&backup)?;
    let filename = format!("corvin-backup-{}.json", now.format("%Y-%m-%d"));

    Ok((
        [
            ("content-type", "application/json".to_string()),
            (
                "content-disposition",
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        json,
    ))
}

pub async fn import_backup(
    State(state): State<AppState>,
    Json(backup): Json<Backup>,
) -> Result<StatusCode, ApiError> {
    // Accept v1–v3. v3 added `categories`; every newer field has a serde default,
    // so older backups restore fine and a current (v3) backup round-trips. Keep
    // this in lockstep with the `version` written by `export_backup`.
    if backup.version == 0 || backup.version > 3 {
        return Err(anyhow::anyhow!("Unsupported backup version {}", backup.version).into());
    }

    // We restore onto the BACKUP's network, not the running instance's — they
    // can differ, and opening wallets on the wrong network would reject every
    // descriptor and silently drop them. Capture it before `settings` is moved.
    let backup_network = backup.settings.network.kind.to_bitcoin_network();

    // Pre-flight: validate every HD wallet's descriptor parses + matches the
    // backup's network BEFORE mutating any on-disk state. Restore overwrites
    // several stores in sequence and clears the wallet set, so a failure partway
    // would leave a half-restored, inconsistent state. Reject up front instead —
    // nothing below runs unless all wallets are sound.
    {
        use bdk_wallet::Wallet;
        let mut bad: Vec<String> = Vec::new();
        for entry in &backup.wallets {
            if matches!(entry.kind, InputKind::Address | InputKind::SilentPayments) {
                continue; // no BDK descriptor to validate
            }
            let ext = entry.external_descriptor.clone();
            let res = match entry.internal_descriptor.clone() {
                Some(int) => Wallet::create(ext, int)
                    .network(backup_network)
                    .create_wallet_no_persist(),
                None => Wallet::create_single(ext)
                    .network(backup_network)
                    .create_wallet_no_persist(),
            };
            if let Err(e) = res {
                bad.push(format!("'{}' ({}): {e}", entry.label, entry.id));
            }
        }
        if !bad.is_empty() {
            return Err(anyhow::anyhow!(
                "Backup not restored (no changes made) — these wallets failed validation against the \
                 backup's network ({backup_network}): {}",
                bad.join("; ")
            )
            .into());
        }
    }

    // Restore all annotations (utxo labels + freezes are v2; empty for v1 backups).
    state
        .annotations
        .replace_all(AnnotationData {
            tx_labels: backup.labels,
            cost_basis: backup.cost_basis,
            utxo_labels: backup.utxo_labels,
            address_labels: backup.address_labels,
            frozen_utxos: backup.frozen_utxos,
        })
        .await?;
    // Categories are a v3 addition; empty for older backups.
    state.annotations.replace_categories(backup.categories).await?;

    // Restore Silent Payments key material. The background scanner picks these
    // up on the next startup (same contract as creating an SP wallet).
    let sp_keys_present: HashSet<Uuid> = backup.silent_payments.keys().copied().collect();
    crate::api::silent_payments::import_keys(backup.silent_payments)?;

    // Restore settings
    save_config(&backup.settings)?;
    *state.config.write().await = backup.settings;

    // Restore wallets — drop manager lock before calling get_scripts (async)
    let managed_wallets = {
        let mut manager = state.manager.write().await;

        // Open the restored wallets on the backup's network (validated above).
        manager.network = backup_network;

        for id in manager.wallets.keys().copied().collect::<Vec<_>>() {
            let _ = state.sub_tx.send(SubCommand::RemoveWallet(id));
        }
        manager.wallets.clear();

        for entry in backup.wallets {
            let inner = match entry.kind {
                InputKind::Address => WalletInner::Address(Mutex::new(None)),
                // SP wallets have no BDK descriptor — restore the in-memory cache
                // directly (keys were written above; scanner repopulates on
                // restart). Without this arm SP wallets fall into the HD branch,
                // fail `open_or_create_wallet`, and get silently dropped. Skip if
                // the backup carried no keys for it (e.g. a pre-v2 backup) — a
                // keyless SP wallet can never scan, so don't resurrect a broken one.
                InputKind::SilentPayments => {
                    if !sp_keys_present.contains(&entry.id) {
                        tracing::warn!(
                            "Skipping SP wallet {} on restore — backup has no keys for it",
                            entry.id
                        );
                        continue;
                    }
                    WalletInner::SilentPayments(Mutex::new(SilentPaymentsCache::default()))
                }
                // Explicit so adding an InputKind variant fails to compile here
                // instead of silently routing through the HD path.
                InputKind::Xpub
                | InputKind::Ypub
                | InputKind::Zpub
                | InputKind::Taproot
                | InputKind::Multisig
                | InputKind::Descriptor => {
                    let db_path = wallet_db_path(entry.id);
                    match corvin_core::wallet::open_or_create_wallet(
                        &entry,
                        manager.network,
                        &db_path,
                    ) {
                        Ok(hd) => {
                            restrict_to_owner(&db_path);
                            WalletInner::Hd(Mutex::new(hd))
                        }
                        Err(e) => {
                            tracing::warn!("Failed to restore wallet {}: {e:#}", entry.id);
                            continue;
                        }
                    }
                }
            };
            manager.add(entry, inner);
        }

        save_wallets(&manager).await?;
        manager.wallets.values().cloned().collect::<Vec<_>>()
    };

    for m in managed_wallets {
        let scripts = get_scripts(&m).await;
        let _ = state.sub_tx.send(SubCommand::AddWallet {
            id: m.entry.id,
            scripts,
        });
        // Restored SP wallets need their scanner started now, not on next
        // restart — same contract as creating one.
        if m.entry.kind == InputKind::SilentPayments {
            if let Some(keys) = crate::api::silent_payments::get_keys(m.entry.id) {
                crate::sp_subscriber::respawn_one(&state, m.entry.id, keys).await;
            }
        }
    }

    Ok(StatusCode::NO_CONTENT)
}
