use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};

use crate::state::AppState;

/// Proxy a mempool transaction lookup through the backend's HTTP client (which
/// may be configured to use a SOCKS5/Tor proxy). This prevents the browser from
/// making direct requests to the mempool explorer and leaking the user's IP.
pub async fn proxy_tx(
    State(state): State<AppState>,
    Path(txid): Path<String>,
) -> impl IntoResponse {
    if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return axum::http::StatusCode::BAD_REQUEST.into_response();
    }

    let mempool_url = {
        let cfg = state.config.read().await;
        cfg.backend.mempool_url.clone()
    };
    if mempool_url.is_empty() {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }

    let url = format!("{}/api/tx/{}", mempool_url.trim_end_matches('/'), txid);
    match state.mempool_http.read().await.get(&url).send().await {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(json) => Json(json).into_response(),
            Err(_) => axum::http::StatusCode::BAD_GATEWAY.into_response(),
        },
        _ => axum::http::StatusCode::BAD_GATEWAY.into_response(),
    }
}

/// Proxy the mempool-blocks endpoint (per-block fee histograms / queue depth).
pub async fn proxy_mempool_blocks(State(state): State<AppState>) -> impl IntoResponse {
    let mempool_url = {
        let cfg = state.config.read().await;
        cfg.backend.mempool_url.clone()
    };
    if mempool_url.is_empty() {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }

    let url = format!(
        "{}/api/v1/fees/mempool-blocks",
        mempool_url.trim_end_matches('/')
    );
    match state.mempool_http.read().await.get(&url).send().await {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(json) => Json(json).into_response(),
            Err(_) => axum::http::StatusCode::BAD_GATEWAY.into_response(),
        },
        _ => axum::http::StatusCode::BAD_GATEWAY.into_response(),
    }
}

/// Proxy the CPFP package info for a transaction (effective fee rate across ancestor chain).
pub async fn proxy_tx_cpfp(
    State(state): State<AppState>,
    Path(txid): Path<String>,
) -> impl IntoResponse {
    if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return axum::http::StatusCode::BAD_REQUEST.into_response();
    }

    let mempool_url = {
        let cfg = state.config.read().await;
        cfg.backend.mempool_url.clone()
    };
    if mempool_url.is_empty() {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }

    let url = format!("{}/api/v1/cpfp/{}", mempool_url.trim_end_matches('/'), txid);
    match state.mempool_http.read().await.get(&url).send().await {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(json) => Json(json).into_response(),
            Err(_) => axum::http::StatusCode::BAD_GATEWAY.into_response(),
        },
        _ => axum::http::StatusCode::BAD_GATEWAY.into_response(),
    }
}

/// Proxy the RBF replacement history for a transaction.
pub async fn proxy_tx_rbf(
    State(state): State<AppState>,
    Path(txid): Path<String>,
) -> impl IntoResponse {
    if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return axum::http::StatusCode::BAD_REQUEST.into_response();
    }

    let mempool_url = {
        let cfg = state.config.read().await;
        cfg.backend.mempool_url.clone()
    };
    if mempool_url.is_empty() {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }

    let url = format!(
        "{}/api/v1/tx/{}/rbf",
        mempool_url.trim_end_matches('/'),
        txid
    );
    match state.mempool_http.read().await.get(&url).send().await {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(json) => Json(json).into_response(),
            Err(_) => axum::http::StatusCode::BAD_GATEWAY.into_response(),
        },
        _ => axum::http::StatusCode::BAD_GATEWAY.into_response(),
    }
}

/// Proxy the recommended fee rates endpoint.
pub async fn proxy_fees(State(state): State<AppState>) -> impl IntoResponse {
    let mempool_url = {
        let cfg = state.config.read().await;
        cfg.backend.mempool_url.clone()
    };
    if mempool_url.is_empty() {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }

    let url = format!(
        "{}/api/v1/fees/recommended",
        mempool_url.trim_end_matches('/')
    );
    match state.mempool_http.read().await.get(&url).send().await {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(json) => Json(json).into_response(),
            Err(_) => axum::http::StatusCode::BAD_GATEWAY.into_response(),
        },
        _ => axum::http::StatusCode::BAD_GATEWAY.into_response(),
    }
}
