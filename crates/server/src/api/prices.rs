use crate::api::ApiError;
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PriceQuery {
    pub timestamp: i64,
}

fn day_start(ts: i64) -> i64 {
    ts - (ts % 86_400)
}

/// Fetch the USD price for a given Unix timestamp, caching by UTC day.
/// Pass `force = true` to bypass the `show_price_data` setting (used by tax report).
/// Returns None if the configured mempool URL lacks price data or the request fails.
pub async fn fetch_price_cached(state: &AppState, timestamp: i64, force: bool) -> Option<f64> {
    let day = day_start(timestamp);

    if let Some(price) = state.prices.historical(day).await {
        return Some(price);
    }

    let (mempool_url, show_price_data) = {
        let cfg = state.config.read().await;
        (cfg.backend.mempool_url.clone(), cfg.backend.show_price_data)
    };
    if (!force && !show_price_data) || mempool_url.is_empty() {
        return None;
    }

    let url = format!("{mempool_url}/api/v1/historical-price?currency=USD&timestamp={timestamp}");

    let resp = state
        .mempool_http
        .read()
        .await
        .clone()
        .get(&url)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let json: serde_json::Value = resp.json().await.ok()?;

    // Response: {"prices":[{"time":...,"USD":...}]}
    let price = json["prices"].as_array()?.first()?.get("USD")?.as_f64()?;

    let _ = state.prices.store_historical(day, price).await;

    Some(price)
}

pub async fn get_price(
    State(state): State<AppState>,
    Query(q): Query<PriceQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let price = fetch_price_cached(&state, q.timestamp, false).await;
    Ok(Json(serde_json::json!({ "usd": price })))
}

pub async fn get_current_price(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // 60-second in-memory cache so rapid polls don't hammer mempool.space.
    if let Some(price) = state.prices.current_if_fresh().await {
        return Ok(Json(serde_json::json!({ "usd": price })));
    }

    let mempool_url = {
        let cfg = state.config.read().await;
        cfg.backend.mempool_url.clone()
    };
    if mempool_url.is_empty() {
        return Ok(Json(serde_json::json!({ "usd": null })));
    }

    let resp = state
        .mempool_http
        .read()
        .await
        .clone()
        .get(format!("{mempool_url}/api/v1/prices"))
        .send()
        .await;

    let price = match resp {
        Ok(r) if r.status().is_success() => r
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|j| j.get("USD").and_then(|v| v.as_f64())),
        _ => None,
    };

    if let Some(p) = price {
        state.prices.store_current(p).await;
    }

    Ok(Json(serde_json::json!({ "usd": price })))
}
