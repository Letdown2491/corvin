use crate::api::ApiError;
use crate::state::{utxo_key, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct SetLabelRequest {
    pub note: String,
}

pub async fn list_utxo_labels(State(state): State<AppState>) -> Json<HashMap<String, String>> {
    Json(state.annotations.utxo_labels().await)
}

const MAX_NOTE_LEN: usize = 500;

fn validate_txid(txid: &str) -> Result<(), ApiError> {
    if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(anyhow::anyhow!("invalid txid").into());
    }
    Ok(())
}

pub async fn set_utxo_label(
    State(state): State<AppState>,
    Path((txid, vout)): Path<(String, u32)>,
    Json(req): Json<SetLabelRequest>,
) -> Result<StatusCode, ApiError> {
    validate_txid(&txid)?;
    if req.note.len() > MAX_NOTE_LEN {
        return Err(
            anyhow::anyhow!("note exceeds maximum length of {MAX_NOTE_LEN} characters").into(),
        );
    }
    state
        .annotations
        .set_utxo_label(utxo_key(&txid, vout), &req.note)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
