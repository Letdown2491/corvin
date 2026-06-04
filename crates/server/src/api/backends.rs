use crate::api::ApiError;
use crate::config::{save_config, BackendEntry, BackendType};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use corvin_core::backends::{electrum, electrum::ElectrumConfig, rpc, rpc::RpcConfig};
use corvin_core::types::{BackendKind, NodeStatus};

// The registry of *additional* named backends a wallet can be pinned to. The
// default backend lives in settings (`/settings`), not here.
fn masked(mut e: BackendEntry) -> BackendEntry {
    e.rpc_pass = String::new();
    e
}

fn validate(e: &BackendEntry) -> Result<(), ApiError> {
    if e.id.trim().is_empty() {
        return Err(anyhow::anyhow!("backend id must not be empty").into());
    }
    if let Some(p) = &e.socks5_proxy {
        if !p.is_empty() && !p.contains(':') {
            return Err(anyhow::anyhow!("socks5_proxy must be host:port").into());
        }
    }
    if !e.rpc_url.is_empty() {
        let u = e.rpc_url.to_lowercase();
        if !u.starts_with("http://") && !u.starts_with("https://") {
            return Err(anyhow::anyhow!("rpc_url must start with http:// or https://").into());
        }
    }
    Ok(())
}

pub async fn list_backends(State(state): State<AppState>) -> Json<Vec<BackendEntry>> {
    let backends = state.config.read().await.backends.clone();
    Json(backends.into_iter().map(masked).collect())
}

pub async fn create_backend(
    State(state): State<AppState>,
    Json(entry): Json<BackendEntry>,
) -> Result<Json<BackendEntry>, ApiError> {
    validate(&entry)?;
    {
        let mut cfg = state.config.write().await;
        if cfg.backends.iter().any(|b| b.id == entry.id) {
            return Err(anyhow::anyhow!("a backend with id '{}' already exists", entry.id).into());
        }
        cfg.backends.push(entry.clone());
        save_config(&cfg)?;
    }
    // Nudge the subscriber so a wallet pinned to this backend can pick it up.
    state.wake_signal.notify_waiters();
    Ok(Json(masked(entry)))
}

pub async fn update_backend(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(mut entry): Json<BackendEntry>,
) -> Result<Json<BackendEntry>, ApiError> {
    validate(&entry)?;
    {
        let mut cfg = state.config.write().await;
        let Some(idx) = cfg.backends.iter().position(|b| b.id == id) else {
            return Err(anyhow::anyhow!("no backend with id '{id}'").into());
        };
        // Preserve the stored RPC password if the client sent an empty one, and
        // keep the path id authoritative (ignore any id change in the body).
        if entry.rpc_pass.is_empty() {
            entry.rpc_pass = cfg.backends[idx].rpc_pass.clone();
        }
        entry.id = id;
        cfg.backends[idx] = entry.clone();
        save_config(&cfg)?;
    }
    state.wake_signal.notify_waiters();
    Ok(Json(masked(entry)))
}

/// Probe a backend without saving it (the "Test" button in the add/edit form).
/// If the body's `rpc_pass` is empty but a stored backend with this id exists,
/// reuse the stored password (it's masked on read, so edits arrive blank).
pub async fn test_backend(
    State(state): State<AppState>,
    Json(mut entry): Json<BackendEntry>,
) -> Json<NodeStatus> {
    if entry.rpc_pass.is_empty() {
        if let Some(stored) = state
            .config
            .read()
            .await
            .backends
            .iter()
            .find(|b| b.id == entry.id)
        {
            entry.rpc_pass = stored.rpc_pass.clone();
        }
    }
    let status = tokio::task::spawn_blocking(move || match entry.kind {
        BackendType::Electrum => electrum::probe_status(&ElectrumConfig {
            url: entry.electrum_url(),
            validate_tls: entry.validate_tls,
            ca_cert_path: entry.ca_cert_path.clone(),
            danger_accept_invalid_certs: entry.danger_accept_invalid_certs,
            socks5_proxy: entry.socks5_proxy.clone(),
        }),
        BackendType::Rpc => rpc::probe_status(&RpcConfig {
            url: entry.rpc_url.clone(),
            user: entry.rpc_user.clone(),
            pass: entry.rpc_pass.clone(),
        }),
    })
    .await
    .unwrap_or_else(|_| NodeStatus {
        backend: BackendKind::Electrum,
        connected: false,
        network: "unknown".to_string(),
        tip_height: None,
        error: Some("probe task panicked".to_string()),
        offline: false,
    });
    Json(status)
}

/// Move the built-in default connection into the saved-backends registry and
/// point `default_backend` at it, so the default is always a *reference* (no
/// embedded ad-hoc "Current" connection). Idempotent: if a default is already
/// pinned, or an existing saved backend already matches the built-in connection,
/// we reuse it rather than creating a duplicate. Runs server-side so a custom RPC
/// default keeps its (masked-on-read) password. The frontend only calls this when
/// the built-in default isn't one of the public servers.
pub async fn adopt_default(State(state): State<AppState>) -> Result<Json<BackendEntry>, ApiError> {
    let mut cfg = state.config.write().await;
    if let Some(id) = cfg.default_backend.clone() {
        if let Some(e) = cfg.backend_entry(&id) {
            return Ok(Json(masked(e.clone())));
        }
    }
    let b = &cfg.backend;
    let matches = |e: &BackendEntry| -> bool {
        e.kind == b.kind
            && match b.kind {
                BackendType::Electrum => {
                    e.electrum_host == b.electrum_host
                        && e.electrum_port == b.electrum_port
                        && e.electrum_ssl == b.electrum_ssl
                }
                BackendType::Rpc => e.rpc_url == b.rpc_url && e.rpc_user == b.rpc_user,
            }
    };
    let entry = if let Some(existing) = cfg.backends.iter().find(|e| matches(e)).cloned() {
        existing
    } else {
        let label = match b.kind {
            BackendType::Electrum => b.electrum_host.clone(),
            BackendType::Rpc => "Bitcoin node".to_string(),
        };
        let new = BackendEntry {
            id: uuid::Uuid::new_v4().to_string(),
            label,
            kind: b.kind.clone(),
            frigate: false,
            electrum_host: b.electrum_host.clone(),
            electrum_port: b.electrum_port,
            electrum_ssl: b.electrum_ssl,
            validate_tls: b.validate_tls,
            ca_cert_path: b.ca_cert_path.clone(),
            danger_accept_invalid_certs: b.danger_accept_invalid_certs,
            socks5_proxy: b.socks5_proxy.clone(),
            rpc_url: b.rpc_url.clone(),
            rpc_user: b.rpc_user.clone(),
            rpc_pass: b.rpc_pass.clone(),
        };
        cfg.backends.push(new.clone());
        new
    };
    cfg.default_backend = Some(entry.id.clone());
    save_config(&cfg)?;
    state.wake_signal.notify_waiters();
    Ok(Json(masked(entry)))
}

#[derive(serde::Serialize)]
pub struct BackendStatusEntry {
    /// `None` = the default backend.
    pub backend: Option<String>,
    pub connected: bool,
    pub tip_height: Option<u32>,
    pub error: Option<String>,
}

/// Live connection state for every backend that has a running subscriber worker
/// (always includes the default if any wallet exists). The frontend maps each
/// wallet's `backend` to an entry here for a per-wallet connection indicator.
pub async fn list_backend_status(State(state): State<AppState>) -> Json<Vec<BackendStatusEntry>> {
    let mut out: Vec<BackendStatusEntry> = {
        let map = state.backend_status.read().unwrap_or_else(|e| e.into_inner());
        map.iter()
            .map(|(k, v)| BackendStatusEntry {
                backend: k.clone(),
                connected: v.connected,
                tip_height: v.tip_height,
                error: v.error.clone(),
            })
            .collect()
    };
    // When the default points at a saved backend, unpinned wallets group under
    // that id, so there's no `null`-keyed worker. Mirror its status under `null`
    // so the frontend's per-wallet lookups (which key default wallets as null)
    // still light up.
    let default_id = state.config.read().await.effective_backend_id(None);
    if let Some(did) = default_id {
        if !out.iter().any(|e| e.backend.is_none()) {
            if let Some(src) = out
                .iter()
                .find(|e| e.backend.as_deref() == Some(did.as_str()))
            {
                out.push(BackendStatusEntry {
                    backend: None,
                    connected: src.connected,
                    tip_height: src.tip_height,
                    error: src.error.clone(),
                });
            }
        }
    }
    Json(out)
}

pub async fn delete_backend(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    {
        let mut cfg = state.config.write().await;
        let before = cfg.backends.len();
        cfg.backends.retain(|b| b.id != id);
        // If this backend was the global default, fall back to the built-in one.
        let cleared_default = cfg.default_backend.as_deref() == Some(id.as_str());
        if cleared_default {
            cfg.default_backend = None;
        }
        if cfg.backends.len() != before || cleared_default {
            save_config(&cfg)?;
        }
    }
    state.wake_signal.notify_waiters();
    Ok(StatusCode::NO_CONTENT)
}
