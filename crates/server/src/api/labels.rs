use crate::api::ApiError;
use crate::state::AppState;
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

pub async fn list_labels(State(state): State<AppState>) -> Json<HashMap<String, String>> {
    Json(state.annotations.tx_labels().await)
}

const MAX_NOTE_LEN: usize = 500;

/// Bitcoin txids are 64 hex chars. Validate before using the path param as a
/// key so callers can't store arbitrary keys in our label store.
fn validate_txid(txid: &str) -> Result<(), ApiError> {
    if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(anyhow::anyhow!("invalid txid").into());
    }
    Ok(())
}

/// Set a label, or clear it when `note` is empty (callers don't need a
/// separate DELETE — empty string is the documented "remove" signal).
pub async fn set_label(
    State(state): State<AppState>,
    Path(txid): Path<String>,
    Json(req): Json<SetLabelRequest>,
) -> Result<StatusCode, ApiError> {
    validate_txid(&txid)?;
    if req.note.len() > MAX_NOTE_LEN {
        return Err(
            anyhow::anyhow!("note exceeds maximum length of {MAX_NOTE_LEN} characters").into(),
        );
    }
    state.annotations.set_tx_label(txid, &req.note).await?;
    Ok(StatusCode::NO_CONTENT)
}
