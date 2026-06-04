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
pub struct SetCostBasisRequest {
    pub usd: f64,
}

pub async fn list_cost_basis(State(state): State<AppState>) -> Json<HashMap<String, f64>> {
    Json(state.annotations.cost_basis().await)
}

fn validate_txid(txid: &str) -> Result<(), ApiError> {
    if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(anyhow::anyhow!("invalid txid").into());
    }
    Ok(())
}

pub async fn set_cost_basis(
    State(state): State<AppState>,
    Path(txid): Path<String>,
    Json(req): Json<SetCostBasisRequest>,
) -> Result<StatusCode, ApiError> {
    validate_txid(&txid)?;
    if !req.usd.is_finite() {
        return Err(anyhow::anyhow!("usd must be a finite number").into());
    }
    state.annotations.set_cost_basis(txid, req.usd).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_cost_basis(
    State(state): State<AppState>,
    Path(txid): Path<String>,
) -> Result<StatusCode, ApiError> {
    validate_txid(&txid)?;
    state.annotations.remove_cost_basis(&txid).await?;
    Ok(StatusCode::NO_CONTENT)
}
