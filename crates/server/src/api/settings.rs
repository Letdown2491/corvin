use crate::api::ApiError;
use crate::config::{save_config, Config};
use crate::state::{build_http_client, AppState};
use axum::{extract::State, Json};
use corvin_core::backends::{electrum, rpc};
use corvin_core::types::NodeStatus;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct TestMempoolRequest {
    pub url: String,
    pub socks5_proxy: Option<String>,
    #[serde(default)]
    pub danger_accept_invalid_certs: bool,
}

#[derive(Serialize)]
pub struct TestMempoolResult {
    pub ok: bool,
    pub msg: String,
}

fn masked(mut cfg: Config) -> Config {
    cfg.backend.rpc_pass = String::new();
    cfg
}

pub async fn get_settings(State(state): State<AppState>) -> Json<Config> {
    Json(masked(state.config.read().await.clone()))
}

pub async fn put_settings(
    State(state): State<AppState>,
    Json(mut new_cfg): Json<Config>,
) -> Result<Json<Config>, ApiError> {
    validate_config(&new_cfg)?;

    // Preserve the stored password if the client sent an empty string.
    if new_cfg.backend.rpc_pass.is_empty() {
        new_cfg.backend.rpc_pass = state.config.read().await.backend.rpc_pass.clone();
    }
    let old_default = state.config.read().await.default_backend.clone();
    let old_offline = state.config.read().await.offline;
    // The settings form doesn't carry the backend registry, so don't let a save
    // wipe it — keep whatever's already stored.
    new_cfg.backends = state.config.read().await.backends.clone();
    // Same for the onboarding flag — not carried by the settings form.
    new_cfg.onboarding_complete = state.config.read().await.onboarding_complete;
    // A default that points at a non-existent backend would silently fall back;
    // normalize it to None so what's stored matches what's used.
    if let Some(id) = &new_cfg.default_backend {
        if new_cfg.backend_entry(id).is_none() {
            new_cfg.default_backend = None;
        }
    }
    // Build the HTTP client first so a bad proxy / TLS config is rejected before
    // we persist anything — otherwise the user thinks Tor is on while we silently
    // drop the proxy.
    let new_http = build_http_client(
        new_cfg.backend.socks5_proxy.as_deref(),
        new_cfg.backend.danger_accept_invalid_certs,
    )?;
    let new_mempool_http = build_http_client(
        new_cfg.backend.socks5_proxy.as_deref(),
        new_cfg.backend.mempool_danger_accept_invalid_certs,
    )?;
    let default_changed = old_default != new_cfg.default_backend;
    let offline_changed = old_offline != new_cfg.offline;
    let now_offline = new_cfg.offline;
    save_config(&new_cfg)?;
    *state.config.write().await = new_cfg.clone();
    *state.http.write().await = new_http;
    *state.mempool_http.write().await = new_mempool_http;
    // Changing the default re-homes every wallet that rides it onto the new
    // backend's worker — otherwise they'd keep syncing through the old one until
    // their next periodic sync.
    if default_changed {
        regroup_default_wallets(&state).await;
    }
    // Toggling offline: wake the subscriber workers so they re-evaluate (park
    // when going offline, reconnect when coming back), and stop or (re)start the
    // SP scanners, which hold their own Frigate connections.
    if offline_changed {
        if now_offline {
            let mut scanners = state.sp_scanners.lock().await;
            for (_, handle) in scanners.drain() {
                handle.abort();
            }
            drop(scanners);
            // Payjoin poll tasks also hit the network (OHTTP relay) — stop them too,
            // so "offline" actually means offline. Resumable sessions respawn below.
            let mut pj = state.payjoin_tasks.lock().await;
            for (_, handle) in pj.drain() {
                handle.abort();
            }
        } else {
            crate::sp_subscriber::start(state.clone()).await;
            crate::payjoin_subscriber::start(&state).await;
        }
        state.wake_signal.notify_waiters();
    }
    Ok(Json(masked(new_cfg)))
}

/// Re-route wallets with no pinned backend (so they use the default) to whatever
/// the default now resolves to, by re-issuing `AddWallet` (the supervisor
/// re-groups on it). SP wallets are unaffected — their scanner config is separate.
async fn regroup_default_wallets(state: &AppState) {
    use crate::state::{get_scripts, SubCommand, WalletInner};
    let managed: Vec<_> = {
        let mgr = state.manager.read().await;
        mgr.wallets
            .values()
            .filter(|m| m.backend().is_none() && !matches!(m.inner, WalletInner::SilentPayments(_)))
            .cloned()
            .collect()
    };
    for m in managed {
        let scripts = get_scripts(&m).await;
        let _ = state.sub_tx.send(SubCommand::AddWallet {
            id: m.entry.id,
            scripts,
        });
    }
    state.wake_signal.notify_waiters();
}

fn validate_config(cfg: &Config) -> Result<(), ApiError> {
    // Bind: must parse as an IP. Refuse non-loopback unless the user has set
    // CORVIN_ALLOW_LAN — the assumed deployment model is local-only or behind
    // a platform reverse-proxy (Start9, etc.) on 127.0.0.1.
    let bind: std::net::IpAddr = cfg
        .server
        .bind
        .parse()
        .map_err(|_| anyhow::anyhow!("server.bind must be a valid IP address"))?;
    if !bind.is_loopback() && std::env::var("CORVIN_ALLOW_LAN").is_err() {
        return Err(anyhow::anyhow!(
            "server.bind is not a loopback address. Corvin is meant for local-only / Start9 use; \
             set CORVIN_ALLOW_LAN=1 in the environment if you really want to expose it."
        )
        .into());
    }
    if cfg.server.port < 1 {
        return Err(anyhow::anyhow!("server.port must be >= 1").into());
    }

    // Network kind: typed enum on Config — serde rejects unknown variants
    // at deserialize time, so anything reaching us here is already valid.
    let _ = cfg.network.kind;

    // Backend URLs: reject file:// or javascript: schemes. Empty mempool_url is OK
    // (just disables mempool features).
    if !cfg.backend.mempool_url.is_empty() {
        let u = cfg.backend.mempool_url.to_lowercase();
        if !u.starts_with("http://") && !u.starts_with("https://") {
            return Err(
                anyhow::anyhow!("backend.mempool_url must start with http:// or https://").into(),
            );
        }
    }
    if !cfg.backend.rpc_url.is_empty() {
        let u = cfg.backend.rpc_url.to_lowercase();
        if !u.starts_with("http://") && !u.starts_with("https://") {
            return Err(
                anyhow::anyhow!("backend.rpc_url must start with http:// or https://").into(),
            );
        }
    }
    if !cfg.backend.bip353_doh_url.is_empty() {
        let u = cfg.backend.bip353_doh_url.to_lowercase();
        if !u.starts_with("http://") && !u.starts_with("https://") {
            return Err(anyhow::anyhow!(
                "backend.bip353_doh_url must start with http:// or https://"
            )
            .into());
        }
    }

    // SOCKS5 proxy: if set, must be host:port.
    if let Some(p) = &cfg.backend.socks5_proxy {
        if !p.is_empty() && !p.contains(':') {
            return Err(
                anyhow::anyhow!("backend.socks5_proxy must be in the form host:port").into(),
            );
        }
    }

    if cfg.backend.poll_interval_secs < 5 {
        return Err(anyhow::anyhow!("backend.poll_interval_secs must be at least 5").into());
    }

    Ok(())
}

/// Mark first-run onboarding as done (wizard finished or skipped).
pub async fn complete_onboarding(
    State(state): State<AppState>,
) -> Result<axum::http::StatusCode, ApiError> {
    set_onboarding(&state, true).await
}

/// Clear the onboarding flag so the wizard shows again (from Display settings).
pub async fn reset_onboarding(
    State(state): State<AppState>,
) -> Result<axum::http::StatusCode, ApiError> {
    set_onboarding(&state, false).await
}

async fn set_onboarding(state: &AppState, value: bool) -> Result<axum::http::StatusCode, ApiError> {
    let mut cfg = state.config.write().await;
    if cfg.onboarding_complete != value {
        cfg.onboarding_complete = value;
        save_config(&cfg)?;
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct VersionInfo {
    version: &'static str,
    os: &'static str,
    arch: &'static str,
    /// Native-USB hardware-wallet support compiled in. False on the Start9/headless
    /// build, where the frontend hides the USB-connect UI and signs via QR/PSBT.
    hw_enabled: bool,
}

pub async fn get_version() -> Json<VersionInfo> {
    Json(VersionInfo {
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        hw_enabled: cfg!(feature = "hw"),
    })
}

pub async fn get_status(State(state): State<AppState>) -> Json<NodeStatus> {
    let cfg = state.config.read().await.clone();
    // Offline mode: report it without probing any backend.
    if cfg.offline {
        use corvin_core::types::BackendKind;
        let backend = match cfg.backend_kind_for(None) {
            crate::config::BackendType::Electrum => BackendKind::Electrum,
            crate::config::BackendType::Rpc => BackendKind::Rpc,
        };
        return Json(NodeStatus {
            backend,
            connected: false,
            network: cfg.network.kind.to_bitcoin_network().to_string(),
            tip_height: None,
            error: None,
            offline: true,
        });
    }
    // Probe the connection unpinned wallets actually use — which may now be a
    // saved backend selected as the default, not `cfg.backend`.
    let kind = cfg.backend_kind_for(None);
    let ecfg = cfg.electrum_config_for(None);
    let rcfg = cfg.rpc_config_for(None);
    let status = tokio::task::spawn_blocking(move || match kind {
        crate::config::BackendType::Electrum => electrum::probe_status(&ecfg),
        crate::config::BackendType::Rpc => rpc::probe_status(&rcfg),
    })
    .await
    .unwrap_or_else(|_| NodeStatus {
        backend: corvin_core::types::BackendKind::Electrum,
        connected: false,
        network: "unknown".to_string(),
        tip_height: None,
        error: Some("status task panicked".to_string()),
        offline: false,
    });
    Json(status)
}

/// Kick the subscriber to retry its electrum connection immediately. Wakes every
/// per-backend worker currently waiting in backoff, not just one.
pub async fn reconnect(State(state): State<AppState>) -> axum::http::StatusCode {
    state.wake_signal.notify_waiters();
    axum::http::StatusCode::NO_CONTENT
}

pub async fn test_status(
    State(_state): State<AppState>,
    Json(cfg): Json<Config>,
) -> Json<NodeStatus> {
    Json(probe_config(cfg).await)
}

pub async fn test_mempool(
    State(_state): State<AppState>,
    Json(req): Json<TestMempoolRequest>,
) -> Json<TestMempoolResult> {
    if req.url.is_empty() {
        return Json(TestMempoolResult {
            ok: false,
            msg: "URL is empty".into(),
        });
    }
    let client =
        match build_http_client(req.socks5_proxy.as_deref(), req.danger_accept_invalid_certs) {
            Ok(c) => c,
            Err(e) => {
                return Json(TestMempoolResult {
                    ok: false,
                    msg: format!("HTTP client config error: {e:#}"),
                })
            }
        };
    let url = format!("{}/api/v1/prices", req.url.trim_end_matches('/'));
    let result = client.get(&url).send().await;
    let response = match result {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(json) if json.get("USD").is_some() => TestMempoolResult {
                ok: true,
                msg: format!("Connected · {}", req.url),
            },
            _ => TestMempoolResult {
                ok: false,
                msg:
                    "Reached server but response was unexpected — is this a mempool.space instance?"
                        .into(),
            },
        },
        Ok(r) => TestMempoolResult {
            ok: false,
            msg: format!("Server returned HTTP {}", r.status()),
        },
        Err(e) => TestMempoolResult {
            ok: false,
            msg: e.to_string(),
        },
    };
    Json(response)
}

async fn probe_config(cfg: Config) -> NodeStatus {
    tokio::task::spawn_blocking(move || match cfg.backend.kind {
        crate::config::BackendType::Electrum => electrum::probe_status(&cfg.electrum_config()),
        crate::config::BackendType::Rpc => rpc::probe_status(&cfg.rpc_config()),
    })
    .await
    .unwrap_or_else(|_| NodeStatus {
        backend: corvin_core::types::BackendKind::Electrum,
        connected: false,
        network: "unknown".to_string(),
        tip_height: None,
        error: Some("status task panicked".to_string()),
        offline: false,
    })
}
