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

pub async fn list_address_labels(State(state): State<AppState>) -> Json<HashMap<String, String>> {
    Json(state.annotations.address_labels().await)
}

const MAX_NOTE_LEN: usize = 500;
/// Longest legitimate Bitcoin address is a P2WSH bech32 (62 chars). Round up
/// to keep room for future formats but reject anything obviously garbage.
const MAX_ADDR_LEN: usize = 90;

fn validate_address(addr: &str) -> Result<(), ApiError> {
    if addr.is_empty() || addr.len() > MAX_ADDR_LEN {
        return Err(anyhow::anyhow!("invalid address").into());
    }
    Ok(())
}

pub async fn set_address_label(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Json(req): Json<SetLabelRequest>,
) -> Result<StatusCode, ApiError> {
    validate_address(&address)?;
    if req.note.len() > MAX_NOTE_LEN {
        return Err(
            anyhow::anyhow!("note exceeds maximum length of {MAX_NOTE_LEN} characters").into(),
        );
    }
    state
        .annotations
        .set_address_label(address, &req.note)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_address_label(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Result<StatusCode, ApiError> {
    validate_address(&address)?;
    state.annotations.remove_address_label(&address).await?;
    Ok(StatusCode::NO_CONTENT)
}
