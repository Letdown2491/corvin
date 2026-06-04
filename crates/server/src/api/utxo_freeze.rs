use crate::api::ApiError;
use crate::state::{utxo_key, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

pub async fn list_frozen(State(state): State<AppState>) -> Json<Vec<String>> {
    let mut list: Vec<String> = state.annotations.frozen_utxos().await.into_iter().collect();
    list.sort();
    Json(list)
}

fn validate_txid(txid: &str) -> Result<(), ApiError> {
    if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(anyhow::anyhow!("invalid txid").into());
    }
    Ok(())
}

pub async fn freeze_utxo(
    State(state): State<AppState>,
    Path((txid, vout)): Path<(String, u32)>,
) -> Result<StatusCode, ApiError> {
    validate_txid(&txid)?;
    state
        .annotations
        .set_frozen(utxo_key(&txid, vout), true)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unfreeze_utxo(
    State(state): State<AppState>,
    Path((txid, vout)): Path<(String, u32)>,
) -> Result<StatusCode, ApiError> {
    validate_txid(&txid)?;
    state
        .annotations
        .set_frozen(utxo_key(&txid, vout), false)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
