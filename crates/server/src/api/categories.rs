use crate::api::ApiError;
use crate::state::{CategoryData, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use corvin_core::types::Category;
use serde::Deserialize;

const MAX_NAME_LEN: usize = 60;
const MAX_ADDR_LEN: usize = 90;

#[derive(Deserialize)]
pub struct CategoryDef {
    pub name: String,
    pub color: String,
}

#[derive(Deserialize)]
pub struct AssignRequest {
    /// `None` (or null) clears the assignment.
    #[serde(default)]
    pub category_id: Option<String>,
}

fn validate_def(def: &CategoryDef) -> Result<(), ApiError> {
    if def.name.trim().is_empty() || def.name.len() > MAX_NAME_LEN {
        return Err(ApiError::bad_request("category name must be 1–60 characters"));
    }
    // Colors come from a fixed UI palette; keep it loose but bounded.
    if def.color.len() > 32 {
        return Err(ApiError::bad_request("invalid color"));
    }
    Ok(())
}

pub async fn list_categories(State(state): State<AppState>) -> Json<CategoryData> {
    Json(state.annotations.categories().await)
}

pub async fn create_category(
    State(state): State<AppState>,
    Json(def): Json<CategoryDef>,
) -> Result<Json<Category>, ApiError> {
    validate_def(&def)?;
    let cat = state.annotations.add_category(&def.name, &def.color).await?;
    Ok(Json(cat))
}

pub async fn update_category(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(def): Json<CategoryDef>,
) -> Result<StatusCode, ApiError> {
    validate_def(&def)?;
    state.annotations.update_category(&id, &def.name, &def.color).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_category(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.annotations.delete_category(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn assign_address(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Json(req): Json<AssignRequest>,
) -> Result<StatusCode, ApiError> {
    if address.is_empty() || address.len() > MAX_ADDR_LEN {
        return Err(ApiError::bad_request("invalid address"));
    }
    state
        .annotations
        .set_address_category(address, req.category_id.as_deref())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn assign_utxo(
    State(state): State<AppState>,
    Path(outpoint): Path<String>,
    Json(req): Json<AssignRequest>,
) -> Result<StatusCode, ApiError> {
    // Outpoint is "txid:vout"; keep validation loose but bounded.
    if !outpoint.contains(':') || outpoint.len() > 80 {
        return Err(ApiError::bad_request("invalid outpoint"));
    }
    state
        .annotations
        .set_utxo_category(outpoint, req.category_id.as_deref())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
