//! Wallet data reads — served from cached snapshots (HD) or the per-kind caches,
//! so they never touch the live backend. Split from mod.rs (concern: reads).
use super::{get_managed, network_from_config};
use crate::api::ApiError;
use crate::state::{AppState, WalletInner};
use axum::{
    extract::{Path, State},
    Json,
};
use bdk_wallet::bitcoin::{Address, Txid};
use corvin_core::{
    types::{AddressInfo, Balance, BalancePoint, UtxoRecord},
    wallet,
};
use serde::Serialize;
use std::str::FromStr;
use uuid::Uuid;

/// One side of a transaction (input or output), classified against the wallet.
#[derive(Serialize)]
pub struct TxIo {
    pub address: Option<String>,
    pub value_sats: u64,
    /// True when this output's script belongs to this wallet (change / received).
    pub is_mine: bool,
}

/// Wallet-aware breakdown of one transaction, for the tx-detail flow diagram.
/// Outputs are classified is_mine; fee is present only when the inputs' prevouts
/// are known to the wallet (i.e. transactions this wallet sent).
#[derive(Serialize)]
pub struct TxBreakdown {
    pub outputs: Vec<TxIo>,
    pub fee_sats: Option<u64>,
    pub input_count: usize,
}

pub async fn get_tx_breakdown(
    State(state): State<AppState>,
    Path((id, txid_str)): Path<(Uuid, String)>,
) -> Result<Json<TxBreakdown>, ApiError> {
    let txid = Txid::from_str(&txid_str).map_err(|_| ApiError::bad_request("invalid txid"))?;
    let managed = get_managed(&state, &id).await?;
    let network = network_from_config(&state).await;
    let WalletInner::Hd(wm) = &managed.inner else {
        return Err(ApiError::bad_request(
            "transaction breakdown is only available for HD wallets",
        ));
    };
    let hd = wm.lock().await;
    let Some(ctx) = hd.wallet.transactions().find(|c| c.tx_node.txid == txid) else {
        return Err(ApiError::not_found("transaction not found in this wallet"));
    };
    let tx = ctx.tx_node.tx.clone();
    let outputs = tx
        .output
        .iter()
        .map(|o| TxIo {
            address: Address::from_script(&o.script_pubkey, network)
                .ok()
                .map(|a| a.to_string()),
            value_sats: o.value.to_sat(),
            is_mine: hd.wallet.is_mine(o.script_pubkey.clone()),
        })
        .collect();
    let fee_sats = hd.wallet.calculate_fee(&tx).ok().map(|a| a.to_sat());
    Ok(Json(TxBreakdown {
        outputs,
        fee_sats,
        input_count: tx.input.len(),
    }))
}

pub async fn get_balance(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Balance>, ApiError> {
    let managed = get_managed(&state, &id).await?;
    let mut balance = match &managed.inner {
        WalletInner::Hd(_) => managed
            .balance_snapshot
            .lock()
            .await
            .clone()
            .unwrap_or_default(),
        WalletInner::Address(cache) => cache
            .lock()
            .await
            .as_ref()
            .map(|c| c.balance.clone())
            .unwrap_or_default(),
        WalletInner::SilentPayments(cache) => cache.lock().await.balance(),
    };
    balance.last_synced = *managed.last_synced.lock().await;
    Ok(Json(balance))
}

pub async fn get_transactions(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<std::sync::Arc<Vec<corvin_core::types::TxRecord>>>, ApiError> {
    let managed = get_managed(&state, &id).await?;
    // Return Arc<Vec<T>> so serde serializes the shared buffer without cloning.
    let txs = match &managed.inner {
        WalletInner::Hd(_) => managed.txs_snapshot.lock().await.clone(),
        WalletInner::Address(cache) => std::sync::Arc::new(
            cache
                .lock()
                .await
                .as_ref()
                .map(|c| c.txs.clone())
                .unwrap_or_default(),
        ),
        WalletInner::SilentPayments(cache) => std::sync::Arc::new(cache.lock().await.txs()),
    };
    Ok(Json(txs))
}

pub async fn get_utxos(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<std::sync::Arc<Vec<UtxoRecord>>>, ApiError> {
    let managed = get_managed(&state, &id).await?;
    let utxos = match &managed.inner {
        WalletInner::Hd(_) => managed.utxos_snapshot.lock().await.clone(),
        WalletInner::Address(_) => std::sync::Arc::new(vec![]),
        WalletInner::SilentPayments(cache) => {
            let threshold = state
                .config
                .read()
                .await
                .sp_dust_threshold_sats
                .unwrap_or(crate::config::DEFAULT_SP_DUST_THRESHOLD_SATS);
            std::sync::Arc::new(cache.lock().await.utxos(threshold))
        }
    };
    Ok(Json(utxos))
}

pub async fn get_balance_history(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<BalancePoint>>, ApiError> {
    let managed = get_managed(&state, &id).await?;
    // Computed from txs on demand — only the Charts tab asks for it.
    let points = match &managed.inner {
        WalletInner::Hd(_) => {
            let txs = managed.txs_snapshot.lock().await.clone();
            wallet::balance_history_from_records(&txs)
        }
        WalletInner::Address(cache) => {
            let txs = cache
                .lock()
                .await
                .as_ref()
                .map(|c| c.txs.clone())
                .unwrap_or_default();
            wallet::balance_history_from_records(&txs)
        }
        WalletInner::SilentPayments(cache) => {
            let txs = cache.lock().await.txs();
            wallet::balance_history_from_records(&txs)
        }
    };
    Ok(Json(points))
}

pub async fn get_addresses(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<std::sync::Arc<Vec<AddressInfo>>>, ApiError> {
    let managed = get_managed(&state, &id).await?;
    let addrs = match &managed.inner {
        WalletInner::Hd(_) => managed.addresses_snapshot.lock().await.clone(),
        WalletInner::Address(_) => std::sync::Arc::new(wallet::address_info(&managed.entry.input)),
        // SP wallets have one synthetic address (the sp1q… itself). Reusing
        // address_info gives it the same shape as the Address kind.
        WalletInner::SilentPayments(_) => {
            std::sync::Arc::new(wallet::address_info(&managed.entry.input))
        }
    };
    Ok(Json(addrs))
}
