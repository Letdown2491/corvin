mod api;
#[cfg(test)]
mod api_tests;
mod config;
mod harden;
mod payjoin_sessions;
mod payjoin_subscriber;
mod sp_outputs;
mod sp_subscriber;
mod state;
mod subscriber;

use api::{
    address_labels, backends, backup, backup_test, bip329, bip353, broadcast, categories,
    cost_basis, events, labels, messages, prices, proxy, security, settings, silent_payments,
    sweep, utxo_freeze, utxo_labels, wallets,
};
#[cfg(feature = "hw")]
use api::hwi;
use axum::{
    extract::Request,
    http::{header::HeaderName, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use rust_embed::Embed;
use std::net::SocketAddr;
use tower_http::cors::{AllowOrigin, CorsLayer};

#[derive(Embed)]
#[folder = "../../frontend/dist"]
struct FrontendAssets;

pub fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "corvin=info".into()),
        )
        .init();
}

/// Load config, start the background subscribers, and build the axum router +
/// the address it should bind to. Split out of `run` so the desktop shell can
/// bind the listener itself (before opening the window) and serve in-process.
pub async fn build_app() -> anyhow::Result<(Router, SocketAddr)> {
    // Before anything touches a secret: no core dump may carry a live seed/key.
    harden::suppress_core_dumps();

    // Bind the config-dir root so sealed files use a path-relative AAD. Must come
    // before any sealed read/write (migration recovery below included).
    corvin_core::at_rest::set_config_root(config::config_dir());

    // First: revert/finish any at-rest migration that was interrupted by a crash,
    // so the config dir is in a consistent (all-plaintext or all-encrypted) state.
    security::recover_interrupted_migration();

    // Boot mode: if the at-rest sentinel exists, the config dir is encrypted, so
    // boot LOCKED — don't read config/wallets or start any background task until
    // unlock. The listener still binds (on defaults/env) to serve the unlock gate.
    let locked = security::sentinel_exists();

    let cfg = if locked {
        corvin_core::at_rest::set_vault(corvin_core::at_rest::VaultState::Locked);
        tracing::info!("At-rest encryption is on — booting locked; unlock to start.");
        config::Config::default()
    } else {
        let c = config::load_config()?;
        tracing::info!("Config loaded from {}", config::config_path().display());
        c
    };

    let network = cfg.network.kind.to_bitcoin_network();
    let (app_state, sub_rx) = state::AppState::new(cfg.clone(), network);

    if locked {
        // Hold the subscriber receiver until unlock runs the deferred startup.
        *app_state.startup_rx.lock().await = Some(sub_rx);
    } else {
        start_services(&app_state, sub_rx).await;
    }

    // Env overrides win over config so a container can set bind/port without a
    // pre-seeded config.toml. The loopback fail-safe below still applies.
    let bind = config::resolve_bind(std::env::var("CORVIN_BIND").ok(), &cfg.server.bind);
    let port = config::resolve_port(std::env::var("CORVIN_PORT").ok(), cfg.server.port);
    let addr_str = format!("{bind}:{port}");
    let parsed_addr: SocketAddr = addr_str.parse()?;
    // Fail-safe: the API has no auth, so a non-loopback bind must be explicitly
    // opted into via CORVIN_ALLOW_LAN. The settings endpoint enforces this on
    // change, but a persisted (or hand-edited) config could carry a LAN bind —
    // re-check at startup and fall back to loopback rather than expose silently.
    let addr: SocketAddr =
        if !parsed_addr.ip().is_loopback() && std::env::var("CORVIN_ALLOW_LAN").is_err() {
            tracing::warn!(
                "configured bind {} is not loopback but CORVIN_ALLOW_LAN is unset — \
             binding to 127.0.0.1 instead (the API has no authentication). \
             Set CORVIN_ALLOW_LAN=1 to allow LAN exposure.",
                parsed_addr.ip()
            );
            SocketAddr::from(([127, 0, 0, 1], port))
        } else {
            parsed_addr
        };

    // CORS: only allow same-origin localhost requests
    let localhost_origins = [
        format!("http://localhost:{port}"),
        format!("http://127.0.0.1:{port}"),
    ]
    .into_iter()
    .filter_map(|s| s.parse::<HeaderValue>().ok())
    .collect::<Vec<_>>();

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(localhost_origins))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    let app = build_router(app_state).layer(cors);

    Ok((app, addr))
}

/// Load saved wallets and start the background services (subscriber, SP scanner,
/// payjoin poll tasks). Run at plaintext boot, or after unlock.
async fn start_services(
    app_state: &state::AppState,
    sub_rx: tokio::sync::mpsc::UnboundedReceiver<state::SubCommand>,
) {
    {
        let mut manager = app_state.manager.write().await;
        if let Err(e) = state::load_wallets(&mut manager).await {
            tracing::warn!("Failed to load saved wallets: {e}");
        }
        tracing::info!("Loaded {} wallet(s)", manager.wallets.len());
        let managed: Vec<_> = manager.wallets.values().cloned().collect();
        drop(manager);
        for m in managed {
            let id = m.entry.id;
            let scripts = state::get_scripts(&m).await;
            let _ = app_state
                .sub_tx
                .send(state::SubCommand::AddWallet { id, scripts });
        }
    }
    subscriber::spawn_subscriber(app_state.clone(), sub_rx);
    sp_subscriber::start(app_state.clone()).await;
    payjoin_subscriber::start(app_state).await;
}

/// After the vault is unlocked: load the now-decryptable config, apply it to the
/// running state (network + HTTP clients honouring the real proxy), then run the
/// deferred startup. Called by the unlock handler.
pub(crate) async fn run_startup_after_unlock(app_state: &state::AppState) -> anyhow::Result<()> {
    let cfg = config::load_config()?;
    let network = cfg.network.kind.to_bitcoin_network();
    let http = state::build_http_client(
        cfg.backend.socks5_proxy.as_deref(),
        cfg.backend.danger_accept_invalid_certs,
    )
    .unwrap_or_else(|_| reqwest::Client::new());
    let mempool_http = state::build_http_client(
        cfg.backend.socks5_proxy.as_deref(),
        cfg.backend.mempool_danger_accept_invalid_certs,
    )
    .unwrap_or_else(|_| reqwest::Client::new());
    *app_state.http.write().await = http;
    *app_state.mempool_http.write().await = mempool_http;
    app_state.manager.write().await.network = network;
    *app_state.config.write().await = cfg;

    let sub_rx = app_state
        .startup_rx
        .lock()
        .await
        .take()
        .ok_or_else(|| anyhow::anyhow!("services already started"))?;
    start_services(app_state, sub_rx).await;
    Ok(())
}

/// While the vault is locked, block every `/api/*` route except the few needed to
/// unlock. Frontend assets still serve so the unlock gate can load.
async fn lock_gate(req: Request, next: Next) -> Response {
    if corvin_core::at_rest::is_locked() {
        let path = req.uri().path();
        // NB: `/api/status` is intentionally NOT allowed while locked — it would
        // probe the (default, un-proxied) public backend before unlock. The
        // frontend only calls `/api/security/status` until the vault is unlocked.
        let allowed = matches!(
            path,
            "/api/security/status" | "/api/security/unlock" | "/api/version"
        );
        if path.starts_with("/api/") && !allowed {
            return (
                StatusCode::LOCKED,
                Json(serde_json::json!({ "error": "vault is locked", "code": "locked" })),
            )
                .into_response();
        }
    }
    next.run(req).await
}

/// Native-USB hardware-wallet routes, present only in the `hw` build. The Start9 /
/// headless build (`--no-default-features`) ships an empty set and signs via QR/PSBT.
#[cfg(feature = "hw")]
fn hwi_routes() -> Router<state::AppState> {
    Router::new()
        .route("/hwi/detect", get(hwi::detect_hw))
        .route("/hwi/xpub", get(hwi::hw_xpub))
        .route("/hwi/sign", post(hwi::hw_sign_start))
        .route("/hwi/sign/{token}", get(hwi::hw_sign))
        .route("/hwi/show-address", get(hwi::hw_show_address))
}
#[cfg(not(feature = "hw"))]
fn hwi_routes() -> Router<state::AppState> {
    Router::new()
}

/// Assemble the `/api`-nested router with state applied. Split from `build_app`
/// so tests can drive handlers without disk config, background tasks, or CORS.
fn build_router(app_state: state::AppState) -> Router {
    let api = Router::new()
        .route(
            "/wallets",
            get(wallets::list_wallets).post(wallets::add_wallet),
        )
        .route("/wallets/seed/generate", post(wallets::generate_seed))
        .route("/wallets/seed/import", post(wallets::import_seed_wallet))
        .route("/wallets/hwi/import", post(wallets::import_hw_wallet))
        .route(
            "/wallets/multisig/create",
            post(wallets::create_multisig_wallet),
        )
        .route("/wallets/create-vault", post(wallets::create_vault_wallet))
        .route(
            "/wallets/create-timelocked",
            post(wallets::create_timelocked_wallet),
        )
        .route(
            "/wallets/import-descriptor",
            post(wallets::import_descriptor),
        )
        .route(
            "/wallets/{id}",
            patch(wallets::rename_wallet).delete(wallets::delete_wallet),
        )
        .route(
            "/wallets/{id}/backend",
            axum::routing::put(wallets::set_wallet_backend),
        )
        .route("/wallets/{id}/balance", get(wallets::get_balance))
        .route("/wallets/{id}/txs", get(wallets::get_transactions))
        .route("/wallets/{id}/tx/{txid}", get(wallets::get_tx_breakdown))
        .route("/wallets/{id}/addresses", get(wallets::get_addresses))
        .route("/wallets/{id}/utxos", get(wallets::get_utxos))
        .route("/wallets/{id}/policy", get(wallets::get_policy))
        .route(
            "/wallets/{id}/export-descriptor",
            get(wallets::export_descriptor),
        )
        .route(
            "/wallets/{id}/balance-history",
            get(wallets::get_balance_history),
        )
        .route("/wallets/{id}/sync", post(wallets::sync_wallet))
        .route("/wallets/{id}/tax-report", get(wallets::get_tax_report))
        .route(
            "/wallets/{id}/consolidate-psbt",
            post(wallets::build_consolidate_psbt),
        )
        .route("/wallets/{id}/send-psbt", post(wallets::build_send_psbt))
        .route("/wallets/{id}/sp-send", post(wallets::build_sp_send_psbt))
        .route("/wallets/{id}/sp-spend", post(wallets::build_sp_spend))
        .route(
            "/wallets/{id}/payjoin-send",
            post(wallets::build_payjoin_send),
        )
        .route(
            "/wallets/{id}/payjoin-send/{sid}",
            axum::routing::get(wallets::payjoin_send_status).delete(wallets::abandon_payjoin_send),
        )
        .route(
            "/wallets/{id}/payjoin-send/{sid}/confirm",
            post(wallets::confirm_payjoin_send),
        )
        .route(
            "/wallets/{id}/payjoin-receive",
            post(wallets::provision_payjoin_receive).get(wallets::list_payjoin_receive),
        )
        .route(
            "/wallets/{id}/payjoin-receive/{sid}",
            axum::routing::get(wallets::payjoin_receive_status)
                .delete(wallets::cancel_payjoin_receive),
        )
        .route(
            "/wallets/{id}/payjoin-receive/{sid}/confirm",
            post(wallets::confirm_payjoin_receive),
        )
        .route("/wallets/{id}/rbf-psbt", post(wallets::build_rbf_psbt))
        .route("/wallets/{id}/cpfp-psbt", post(wallets::build_cpfp_psbt))
        .route("/wallets/{id}/combine-psbt", post(wallets::combine_psbt))
        .route(
            "/wallets/{id}/multisig-info",
            get(wallets::get_multisig_info),
        )
        .route(
            "/wallets/{id}/multisig-config",
            get(wallets::export_multisig_config),
        )
        .route("/messages/verify", post(messages::verify_message))
        .route("/wallets/{id}/sign-message", post(messages::sign_message))
        .route(
            "/wallets/{id}/silent-payments",
            get(silent_payments::get_silent_payments),
        )
        .route(
            "/wallets/{id}/silent-payments/export",
            get(silent_payments::export_silent_payment_keys),
        )
        .route(
            "/wallets/{id}/silent-payments/labels",
            get(silent_payments::list_silent_payment_labels)
                .post(silent_payments::add_silent_payment_label),
        )
        .route(
            "/wallets/silent-payments",
            post(silent_payments::create_silent_payments_wallet),
        )
        .route("/wallets/{id}/test-backup", post(backup_test::test_backup))
        .route("/labels", get(labels::list_labels))
        .route("/labels/{txid}", axum::routing::put(labels::set_label))
        .route("/labels/export-bip329", get(bip329::export_bip329))
        .route("/labels/import-bip329", post(bip329::import_bip329))
        .route("/address-labels", get(address_labels::list_address_labels))
        .route(
            "/address-labels/{address}",
            axum::routing::put(address_labels::set_address_label)
                .delete(address_labels::delete_address_label),
        )
        .route(
            "/categories",
            get(categories::list_categories).post(categories::create_category),
        )
        .route(
            "/categories/{id}",
            axum::routing::put(categories::update_category).delete(categories::delete_category),
        )
        .route(
            "/categories/address/{address}",
            axum::routing::put(categories::assign_address),
        )
        .route(
            "/categories/utxo/{outpoint}",
            axum::routing::put(categories::assign_utxo),
        )
        .route("/cost-basis", get(cost_basis::list_cost_basis))
        .route(
            "/cost-basis/{txid}",
            axum::routing::put(cost_basis::set_cost_basis).delete(cost_basis::delete_cost_basis),
        )
        .route("/price", get(prices::get_price))
        .route("/price/current", get(prices::get_current_price))
        .route("/utxo-labels", get(utxo_labels::list_utxo_labels))
        .route(
            "/utxo-labels/{txid}/{vout}",
            axum::routing::put(utxo_labels::set_utxo_label),
        )
        .route("/utxo-freeze", get(utxo_freeze::list_frozen))
        .route(
            "/utxo-freeze/{txid}/{vout}",
            axum::routing::put(utxo_freeze::freeze_utxo).delete(utxo_freeze::unfreeze_utxo),
        )
        .route("/resolve-name", post(bip353::resolve_name))
        .route("/decode", post(broadcast::decode_tx))
        .route("/broadcast", post(broadcast::broadcast_tx))
        .route("/sweep", post(sweep::preview_sweep))
        .route("/backup", get(backup::export_backup))
        .route("/restore", post(backup::import_backup))
        .route(
            "/backends",
            get(backends::list_backends).post(backends::create_backend),
        )
        .route("/backends/status", get(backends::list_backend_status))
        .route("/backends/test", post(backends::test_backend))
        .route("/backends/adopt-default", post(backends::adopt_default))
        .route(
            "/backends/{id}",
            axum::routing::put(backends::update_backend).delete(backends::delete_backend),
        )
        .route(
            "/settings",
            get(settings::get_settings).put(settings::put_settings),
        )
        .route("/onboarding/complete", post(settings::complete_onboarding))
        .route("/onboarding/reset", post(settings::reset_onboarding))
        .route("/version", get(settings::get_version))
        .route("/status", get(settings::get_status))
        .route("/backend/reconnect", post(settings::reconnect))
        .route("/status/test", post(settings::test_status))
        .route("/status/test-mempool", post(settings::test_mempool))
        .route("/proxy/tx/{txid}", get(proxy::proxy_tx))
        .route("/proxy/tx-rbf/{txid}", get(proxy::proxy_tx_rbf))
        .route("/proxy/tx-cpfp/{txid}", get(proxy::proxy_tx_cpfp))
        .route("/proxy/fees", get(proxy::proxy_fees))
        .route("/proxy/mempool-blocks", get(proxy::proxy_mempool_blocks))
        .route("/events", get(events::sse_handler))
        .merge(hwi_routes())
        .route("/security/status", get(security::get_security_status))
        .route("/security/unlock", post(security::unlock))
        .route("/security/enable", post(security::enable))
        .route("/security/disable", post(security::disable))
        .route("/security/change-password", post(security::change_password));

    Router::new()
        .nest("/api", api)
        .fallback(serve_frontend)
        // Block protected routes while the vault is locked (no-op when off/unlocked).
        .layer(middleware::from_fn(lock_gate))
        .with_state(app_state)
}

pub async fn run() -> anyhow::Result<()> {
    init_tracing();
    let (app, addr) = build_app().await?;
    tracing::info!("corvin listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Pull every `'sha256-…'` source expression out of the embedded `index.html`.
/// SvelteKit emits one per-build for its inline bootstrap script inside a
/// `<meta http-equiv="content-security-policy">` tag; if that tag ever carries
/// additional hashes (or future SvelteKit versions emit more inline scripts),
/// we pick them all up.
fn extract_script_hashes() -> Vec<String> {
    let Some(index) = FrontendAssets::get("index.html") else {
        return Vec::new();
    };
    let Ok(html) = std::str::from_utf8(&index.data) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel) = html[cursor..].find("'sha256-") {
        let start = cursor + rel;
        let after_open = start + 1;
        if let Some(rel_end) = html[after_open..].find('\'') {
            let end = after_open + rel_end + 1; // inclusive of closing '
            out.push(html[start..end].to_string());
            cursor = end;
        } else {
            break;
        }
    }
    out
}

static SECURITY_HEADERS: std::sync::LazyLock<Vec<(&'static str, String)>> =
    std::sync::LazyLock::new(|| {
        let hashes = extract_script_hashes();
        let script_src = if hashes.is_empty() {
            "script-src 'self'".to_string()
        } else {
            format!("script-src 'self' {}", hashes.join(" "))
        };
        let csp = format!(
            "default-src 'none'; \
             {script_src}; \
             object-src 'none'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob:; \
             connect-src 'self' ipc: http://ipc.localhost; \
             worker-src 'self' blob:; \
             manifest-src 'self'; \
             font-src 'self'; \
             base-uri 'self'; \
             form-action 'self'; \
             frame-ancestors 'none';"
        );
        vec![
            ("x-content-type-options", "nosniff".to_string()),
            ("x-frame-options", "DENY".to_string()),
            ("referrer-policy", "no-referrer".to_string()),
            ("content-security-policy", csp),
            // The frontend is served by the in-process (desktop) / local
            // (Start9) backend, so HTTP caching buys nothing and a heuristically
            // cached shell pointing at gone hashed chunks blanks the app after a
            // rebuild. Never cache; the service worker handles offline.
            ("cache-control", "no-store".to_string()),
        ]
    });

async fn serve_frontend(uri: axum::http::Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    let add_security_headers = |mut resp: axum::response::Response| {
        for (name, value) in SECURITY_HEADERS.iter() {
            if let (Ok(n), Ok(v)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                resp.headers_mut().insert(n, v);
            }
        }
        resp
    };

    match FrontendAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let resp = (
                [("content-type", mime.as_ref().to_string())],
                content.data.into_owned(),
            )
                .into_response();
            add_security_headers(resp)
        }
        // Missing *asset* requests must 404 — never the HTML shell. A hashed
        // build asset (`_app/…`) or any path with a file extension that isn't
        // present is a real miss; returning index.html (text/html) for a `.js`
        // import yields a cryptic "not a valid JavaScript MIME type" error and
        // a blank app (e.g. after a rebuild leaves a stale page referencing
        // gone chunks). Only extensionless paths are client routes → SPA shell.
        None => {
            let is_asset = path.starts_with("_app/")
                || path.rsplit('/').next().is_some_and(|seg| seg.contains('.'));
            if is_asset {
                return axum::http::StatusCode::NOT_FOUND.into_response();
            }
            // SPA fallback: serve index.html for unknown client routes.
            match FrontendAssets::get("index.html") {
                Some(index) => {
                    let resp =
                        Html(String::from_utf8_lossy(&index.data).into_owned()).into_response();
                    add_security_headers(resp)
                }
                None => axum::http::StatusCode::NOT_FOUND.into_response(),
            }
        }
    }
}
