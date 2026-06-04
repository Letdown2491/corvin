//! Wallet sync: the explicit `sync_wallet` endpoint and the fire-and-forget
//! `background_sync` driven by the subscriber. Split from mod.rs (concern: sync).
use super::get_managed;
use crate::api::ApiError;
use crate::state::{get_scripts, AddressCache, AppState, SubCommand, WalletInner};
use axum::{
    extract::{Path, State},
    Json,
};
use chrono::Utc;
use corvin_core::{types::SyncResult, wallet};
use std::sync::Arc;
use uuid::Uuid;

pub async fn sync_wallet(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SyncResult>, ApiError> {
    let managed = get_managed(&state, &id).await?;

    if state.config.read().await.offline {
        return Err(ApiError::bad_request(
            "Corvin is in offline mode — syncing is unavailable.",
        ));
    }

    state.emit("sync_started", serde_json::json!({ "wallet_id": id }));

    let electrum_cfg = {
        let cfg = state.config.read().await;
        cfg.electrum_config_for(managed.backend().as_deref())
    };

    let result = match &managed.inner {
        WalletInner::Address(cache_mutex) => {
            let address = managed.entry.input.clone();
            let (balance, txs) = tokio::task::spawn_blocking(move || {
                wallet::sync_address_electrum(&address, &electrum_cfg)
            })
            .await??;

            let prev_len = cache_mutex
                .lock()
                .await
                .as_ref()
                .map(|c| c.txs.len())
                .unwrap_or(0);
            let new_txs = txs.len().saturating_sub(prev_len);

            *cache_mutex.lock().await = Some(AddressCache { balance, txs });

            SyncResult {
                wallet_id: id,
                new_txs,
                synced_at: Utc::now(),
            }
        }

        // SP wallets receive via the dedicated scanner subscription (which only
        // ever *adds* outputs). Sync is where we reconcile the other direction:
        // check on-chain whether any unspent SP outputs have been spent (e.g. by
        // another wallet/device, or after a failed post-broadcast persist) so the
        // balance can't overstate.
        WalletInner::SilentPayments(cache_mutex) => {
            let to_check: Vec<(String, u32, String)> = {
                let cache = cache_mutex.lock().await;
                cache
                    .outputs
                    .iter()
                    .filter(|o| !o.spent)
                    .map(|o| (o.txid.clone(), o.vout, o.script_pubkey_hex.clone()))
                    .collect()
            };

            let mut newly_spent: Vec<(String, u32)> = Vec::new();
            if !to_check.is_empty() {
                let sp_cfg = {
                    let cfg = state.config.read().await;
                    cfg.sp_electrum_config_for(managed.backend().as_deref())
                };
                match tokio::task::spawn_blocking(move || {
                    corvin_core::backends::electrum::find_spent_sp_outputs(&to_check, &sp_cfg)
                })
                .await
                {
                    Ok(Ok(spent)) => newly_spent = spent,
                    Ok(Err(e)) => tracing::warn!("SP reconcile {id}: {e:#}"),
                    Err(e) => tracing::warn!("SP reconcile {id}: join error: {e}"),
                }
            }

            if !newly_spent.is_empty() {
                {
                    let mut cache = cache_mutex.lock().await;
                    for o in cache.outputs.iter_mut() {
                        if newly_spent
                            .iter()
                            .any(|(t, v)| *t == o.txid && *v == o.vout)
                        {
                            o.spent = true;
                        }
                    }
                }
                if let Err(e) = crate::sp_outputs::mark_spent(id, &newly_spent) {
                    tracing::warn!("SP reconcile {id}: persisting spent state failed: {e}");
                }
            }

            SyncResult {
                wallet_id: id,
                new_txs: 0,
                synced_at: Utc::now(),
            }
        }

        WalletInner::Hd(_) => {
            let mc = Arc::clone(&managed);
            let mut result = tokio::task::spawn_blocking(move || {
                let WalletInner::Hd(ref wm) = mc.inner else {
                    unreachable!()
                };
                let mut hd = wm.blocking_lock();
                // One fee_cache acquisition for the whole sync: merge new fees,
                // prune entries whose txid no longer appears in the wallet (RBF
                // replacements / dropped txs) so the cache can't grow unbounded,
                // then take the snapshot view.
                let fees_for_snap = {
                    let mut fee_cache = mc.fee_cache.blocking_lock();
                    let (sync_result_inner, new_fees) =
                        wallet::sync_electrum_with_fees(&mut hd.wallet, &electrum_cfg, &fee_cache)?;
                    fee_cache.extend(new_fees);
                    let live: std::collections::HashSet<String> = hd
                        .wallet
                        .transactions()
                        .map(|t| t.tx_node.txid.to_string())
                        .collect();
                    fee_cache.retain(|txid, _| live.contains(txid));
                    let snap_fees = fee_cache.clone();
                    (sync_result_inner, snap_fees)
                };
                let (sync_result, fees_for_snap) = fees_for_snap;
                hd.persist_staged()?;

                let snap = crate::state::compute_snapshot(&hd.wallet, &fees_for_snap);
                *mc.txs_snapshot.blocking_lock() = std::sync::Arc::new(snap.txs);
                *mc.utxos_snapshot.blocking_lock() = std::sync::Arc::new(snap.utxos);
                *mc.balance_snapshot.blocking_lock() = Some(snap.balance);
                *mc.addresses_snapshot.blocking_lock() = std::sync::Arc::new(snap.addresses);

                Ok::<_, anyhow::Error>(sync_result)
            })
            .await??;

            result.wallet_id = id;
            result
        }
    };

    *managed.last_synced.lock().await = Some(Utc::now());

    state.emit(
        "sync_complete",
        serde_json::json!({ "wallet_id": id, "new_txs": result.new_txs }),
    );

    Ok(Json(result))
}

pub async fn background_sync(state: AppState, id: Uuid) {
    let managed = {
        let manager = state.manager.read().await;
        manager.get(&id)
    };
    let Some(managed) = managed else { return };

    let electrum_cfg = {
        let cfg = state.config.read().await;
        cfg.electrum_config_for(managed.backend().as_deref())
    };

    state.emit("sync_started", serde_json::json!({ "wallet_id": id }));

    let result: anyhow::Result<usize> = match &managed.inner {
        // Background sync for SP wallets is the scanner subscription — no
        // on-demand path here.
        WalletInner::SilentPayments(_) => Ok(0),
        WalletInner::Address(cache_mutex) => {
            let address = managed.entry.input.clone();
            match tokio::task::spawn_blocking(move || {
                wallet::sync_address_electrum(&address, &electrum_cfg)
            })
            .await
            {
                Ok(Ok((balance, txs))) => {
                    let n = txs.len();
                    *cache_mutex.lock().await = Some(AddressCache { balance, txs });
                    Ok(n)
                }
                Ok(Err(e)) => Err(e),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }
        WalletInner::Hd(_) => {
            let mc = Arc::clone(&managed);
            match tokio::task::spawn_blocking(move || {
                let WalletInner::Hd(ref wm) = mc.inner else {
                    unreachable!()
                };
                let mut hd = wm.blocking_lock();
                // One fee_cache acquisition: merge, prune stale txids (see the
                // background_sync arm above), then snapshot.
                let (sync_result, fees_for_snap) = {
                    let mut fee_cache = mc.fee_cache.blocking_lock();
                    let (sync_result_inner, new_fees) =
                        wallet::sync_electrum_with_fees(&mut hd.wallet, &electrum_cfg, &fee_cache)?;
                    fee_cache.extend(new_fees);
                    let live: std::collections::HashSet<String> = hd
                        .wallet
                        .transactions()
                        .map(|t| t.tx_node.txid.to_string())
                        .collect();
                    fee_cache.retain(|txid, _| live.contains(txid));
                    (sync_result_inner, fee_cache.clone())
                };
                hd.persist_staged()?;

                // Refresh snapshots under the BDK lock for consistency.
                let snap = crate::state::compute_snapshot(&hd.wallet, &fees_for_snap);
                *mc.txs_snapshot.blocking_lock() = std::sync::Arc::new(snap.txs);
                *mc.utxos_snapshot.blocking_lock() = std::sync::Arc::new(snap.utxos);
                *mc.balance_snapshot.blocking_lock() = Some(snap.balance);
                *mc.addresses_snapshot.blocking_lock() = std::sync::Arc::new(snap.addresses);

                Ok::<_, anyhow::Error>(sync_result.new_txs)
            })
            .await
            {
                Ok(Ok(n)) => Ok(n),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }
    };

    match result {
        Ok(new_txs) => {
            *managed.last_synced.lock().await = Some(Utc::now());
            state.emit(
                "sync_complete",
                serde_json::json!({ "wallet_id": id, "new_txs": new_txs }),
            );
            // After an HD sync, reveal new addresses and update the subscriber's
            // lookahead so fresh scripts are subscribed to immediately.
            if matches!(managed.inner, WalletInner::Hd(_)) {
                let scripts = get_scripts(&managed).await;
                let _ = state.sub_tx.send(SubCommand::UpdateScripts { id, scripts });
            }
        }
        Err(e) => {
            tracing::warn!("Auto-sync {id} failed: {e:#}");
            state.emit(
                "error",
                serde_json::json!({ "wallet_id": id, "message": format!("{e:#}") }),
            );
        }
    }
}
