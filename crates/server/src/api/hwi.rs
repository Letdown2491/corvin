//! Hardware-wallet HTTP handlers + brand dispatcher.
//!
//! Each handler:
//!   1. Acquires the global HWI mutex (one operation at a time across all brands).
//!   2. Probes connected USB devices and identifies which brand is attached.
//!   3. Dispatches to the brand-specific module to run the actual flow.
//!
//! The frontend talks to a single SSE channel and doesn't know which brand
//! handled the request — events carry the same names across brands so the UI
//! can remain brand-agnostic. The optional `device_type` event lets the UI
//! customize hint text per brand when useful.

mod bitbox;
mod ledger;
mod trezor;

// `descriptor_util` + `ledger_hmac_store` are declared at the api level (always
// compiled); re-export them here so this module's `common::` and the submodules'
// `super::common` / `super::ledger_hmac_store` paths keep working unchanged.
pub(crate) use crate::api::descriptor_util as common;
pub(crate) use crate::api::ledger_hmac_store;

use axum::{
    extract::{Query, State},
    response::{
        sse::{Event, Sse},
        IntoResponse, Response,
    },
    Json,
};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, OwnedMutexGuard};
use tokio_stream::wrappers::ReceiverStream;

use crate::state::AppState;

/// Total budget for a single HWI operation (xpub export or PSBT sign). Beyond
/// this the device is considered hung — usually because the user walked away
/// without confirming. Releases the device lock so the next caller isn't stuck.
const HWI_TIMEOUT: Duration = Duration::from_secs(300);

/// How long a signing job stays valid before the SSE stream must consume it.
/// 60s is plenty for the browser round-trip; jobs older than this are GC'd.
const SIGN_JOB_TTL: Duration = Duration::from_secs(60);

/// Brands we have any kind of code path for, including stubs.
#[derive(Clone, Copy, Debug)]
enum HwBrand {
    Bitbox,
    Ledger,
    Trezor,
}

impl HwBrand {
    fn as_str(self) -> &'static str {
        match self {
            HwBrand::Bitbox => "bitbox",
            HwBrand::Ledger => "ledger",
            HwBrand::Trezor => "trezor",
        }
    }
}

/// Probe every supported brand and return the first one currently connected.
/// Runs detection on a blocking thread so USB I/O doesn't block the async
/// runtime. None = nothing recognised on the bus.
async fn detect_any_brand() -> Option<HwBrand> {
    tokio::task::spawn_blocking(|| {
        if bitbox::detect_sync() {
            return Some(HwBrand::Bitbox);
        }
        if ledger::detect_sync() {
            return Some(HwBrand::Ledger);
        }
        if trezor::detect_sync() {
            return Some(HwBrand::Trezor);
        }
        None
    })
    .await
    .unwrap_or(None)
}

/// Try to acquire the HWI mutex without blocking. Returns `Some(guard)` on
/// success, or `None` if another HWI operation is already running.
fn try_lock_hwi(lock: &Arc<Mutex<()>>) -> Option<OwnedMutexGuard<()>> {
    lock.clone().try_lock_owned().ok()
}

// ── shared SSE helpers ────────────────────────────────────────────────────────

/// Send a one-shot SSE error response to a fresh channel (used for early
/// rejections — wrong token, busy device, etc.). Returns a fully-converted
/// `Response` so handler functions can return `Response` and have all error /
/// success branches share the same concrete type.
async fn err_sse(message: &str) -> Response {
    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(1);
    let _ = tx
        .send(Ok(Event::default()
            .event("hw_error")
            .data(serde_json::json!({ "message": message }).to_string())))
        .await;
    let stream = ReceiverStream::new(rx);
    Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

/// Spawn the worker thread that runs a brand-specific operation under the
/// global HWI lock + timeout. Each brand module sends its own SSE events
/// through `tx`; we only emit a timeout error here when the budget is blown.
fn spawn_hw_worker<F>(
    tx: mpsc::Sender<Result<Event, Infallible>>,
    hwi_guard: OwnedMutexGuard<()>,
    brand: HwBrand,
    operation: F,
) where
    // The future returned here is intentionally NOT `Send` — bitbox-api's
    // session types carry `dyn ReadWrite` and `dyn NoiseConfig` which aren't
    // Sync, and we never move the future between threads (it runs on the
    // current_thread runtime we build inside the spawned OS thread).
    F: FnOnce(
            mpsc::Sender<Result<Event, Infallible>>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>
        + Send
        + 'static,
{
    std::thread::spawn(move || {
        let _hwi_guard = hwi_guard;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("hw worker runtime");
        let inner_tx = tx.clone();
        let result = rt.block_on(async move {
            // Tell the frontend which brand is handling this flow so the UI
            // can show brand-specific hints (pairing code visible? PIN keypad?
            // open-Bitcoin-app dialog?) without us having to translate every
            // event name.
            let _ = inner_tx
                .send(Ok(Event::default().event("device_type").data(
                    serde_json::json!({ "brand": brand.as_str() }).to_string(),
                )))
                .await;
            tokio::time::timeout(HWI_TIMEOUT, operation(inner_tx)).await
        });
        if result.is_err() {
            let _ = tx.blocking_send(Ok(Event::default()
                .event("hw_error")
                .data(serde_json::json!({
                    "message": format!("Hardware wallet timed out after {}s — did you confirm on the device?", HWI_TIMEOUT.as_secs())
                }).to_string())));
        }
    });
}

// ── detect ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct DetectResponse {
    found: bool,
}

pub async fn detect_hw(State(_state): State<AppState>) -> Json<DetectResponse> {
    Json(DetectResponse {
        found: detect_any_brand().await.is_some(),
    })
}

// ── xpub export ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct XpubQuery {
    account: Option<String>,
    account_index: Option<u32>,
}

pub async fn hw_xpub(State(state): State<AppState>, Query(query): Query<XpubQuery>) -> Response {
    let account_type = query.account.unwrap_or_else(|| "native_segwit".to_string());
    let account_index = query.account_index.unwrap_or(0);

    let Some(brand) = detect_any_brand().await else {
        return err_sse("No hardware wallet detected. Plug one in and try again.").await;
    };

    let hwi_guard = match try_lock_hwi(&state.hwi_lock) {
        Some(g) => g,
        None => {
            return err_sse("Another hardware-wallet operation is already running. Wait for it to finish or cancel it.").await;
        }
    };

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);
    let config_dir = crate::config::config_dir();
    let network_kind = state.config.read().await.network.kind;

    spawn_hw_worker(tx, hwi_guard, brand, move |inner_tx| {
        Box::pin(async move {
            match brand {
                HwBrand::Bitbox => {
                    bitbox::run_xpub_export(
                        inner_tx,
                        network_kind,
                        config_dir,
                        account_type,
                        account_index,
                    )
                    .await
                }
                HwBrand::Ledger => {
                    ledger::run_xpub_export(
                        inner_tx,
                        network_kind,
                        config_dir,
                        account_type,
                        account_index,
                    )
                    .await
                }
                HwBrand::Trezor => {
                    trezor::run_xpub_export(
                        inner_tx,
                        network_kind,
                        config_dir,
                        account_type,
                        account_index,
                    )
                    .await
                }
            }
        })
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

// ── Address verification on device ────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ShowAddressQuery {
    pub wallet_id: uuid::Uuid,
    pub address_index: u32,
    /// 0 = external (receive), 1 = internal (change)
    pub keychain: u32,
}

pub async fn hw_show_address(
    State(state): State<AppState>,
    Query(req): Query<ShowAddressQuery>,
) -> Response {
    // Resolve the wallet's descriptor first so we can fail fast if it's not
    // a shape any HW can verify (e.g. legacy multisig without registration).
    let descriptor = {
        let manager = state.manager.read().await;
        match manager.get(&req.wallet_id) {
            Some(m) => m.entry.external_descriptor.clone(),
            None => return err_sse("Wallet not found").await,
        }
    };

    let Some(brand) = detect_any_brand().await else {
        return err_sse("No hardware wallet detected. Plug one in and try again.").await;
    };

    let hwi_guard = match try_lock_hwi(&state.hwi_lock) {
        Some(g) => g,
        None => {
            return err_sse("Another hardware-wallet operation is already running.").await;
        }
    };

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);
    let config_dir = crate::config::config_dir();
    let network_kind = state.config.read().await.network.kind;
    let keychain = req.keychain;
    let address_index = req.address_index;

    spawn_hw_worker(tx, hwi_guard, brand, move |inner_tx| {
        Box::pin(async move {
            match brand {
                HwBrand::Bitbox => {
                    bitbox::run_show_address(
                        inner_tx,
                        network_kind,
                        config_dir,
                        descriptor,
                        keychain,
                        address_index,
                    )
                    .await
                }
                HwBrand::Ledger => {
                    ledger::run_show_address(
                        inner_tx,
                        network_kind,
                        config_dir,
                        descriptor,
                        keychain,
                        address_index,
                    )
                    .await
                }
                HwBrand::Trezor => {
                    trezor::run_show_address(
                        inner_tx,
                        network_kind,
                        config_dir,
                        descriptor,
                        keychain,
                        address_index,
                    )
                    .await
                }
            }
        })
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

// ── PSBT signing ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SignStartRequest {
    pub psbt: String,
    /// Optional — if supplied and the wallet is multisig, the signer will
    /// register the multisig script config on the device before signing.
    #[serde(default)]
    pub wallet_id: Option<uuid::Uuid>,
}

#[derive(Serialize)]
pub struct SignStartResponse {
    pub token: String,
}

/// Register a PSBT for signing and return a one-shot token. The client then
/// opens an EventSource to `GET /hwi/sign/{token}` which consumes the token
/// and runs the signing flow.
///
/// We don't put the PSBT in the SSE URL because multisig PSBTs can exceed
/// browser URL-length limits and end up in access logs / history.
pub async fn hw_sign_start(
    State(state): State<AppState>,
    Json(req): Json<SignStartRequest>,
) -> Json<SignStartResponse> {
    let token = uuid::Uuid::new_v4().simple().to_string();
    let now = std::time::Instant::now();
    let mut jobs = state.hwi_sign_jobs.lock().await;
    jobs.retain(|_, job| now.duration_since(job.registered_at) < SIGN_JOB_TTL);
    jobs.insert(
        token.clone(),
        crate::state::SignJob {
            psbt_b64: req.psbt,
            wallet_id: req.wallet_id,
            registered_at: now,
        },
    );
    Json(SignStartResponse { token })
}

#[derive(Deserialize)]
pub struct SignTokenPath {
    pub token: String,
}

pub async fn hw_sign(
    State(state): State<AppState>,
    axum::extract::Path(SignTokenPath { token }): axum::extract::Path<SignTokenPath>,
) -> Response {
    // Atomically take + validate the token. If it's missing or stale, reject.
    let (psbt_b64, wallet_id) = {
        let mut jobs = state.hwi_sign_jobs.lock().await;
        match jobs.remove(&token) {
            Some(job) if job.registered_at.elapsed() < SIGN_JOB_TTL => {
                (job.psbt_b64, job.wallet_id)
            }
            _ => {
                return err_sse("Sign token is invalid or expired — start the signing flow again.")
                    .await;
            }
        }
    };

    // If the wallet is multisig, resolve its descriptor + threshold now so the
    // brand module can register/check the script config on the device before
    // signing.
    let multisig_info: Option<common::MultisigInfo> = if let Some(wid) = wallet_id {
        let manager = state.manager.read().await;
        manager.get(&wid).and_then(|m| {
            if m.entry.kind == corvin_core::types::InputKind::Multisig {
                common::parse_wsh_sortedmulti(&m.entry.external_descriptor).map(
                    |(threshold, signers)| common::MultisigInfo {
                        threshold,
                        signers,
                        label: m.label(),
                    },
                )
            } else {
                None
            }
        })
    } else {
        None
    };

    // Policy/vault wallets (miniscript wsh + taproot): resolve the wallet policy
    // so the brand module can register it on the device before signing. Skipped
    // when the descriptor can't be a wallet policy (e.g. taproot-savings NUMS) —
    // those fall back to software signing.
    let policy_info: Option<common::PolicyInfo> = if multisig_info.is_none() {
        if let Some(wid) = wallet_id {
            let manager = state.manager.read().await;
            manager.get(&wid).and_then(|m| {
                if m.entry.kind == corvin_core::types::InputKind::Descriptor {
                    common::descriptor_to_wallet_policy(&m.entry.external_descriptor)
                        .ok()
                        .map(|(template, keys)| common::PolicyInfo {
                            template,
                            keys,
                            label: m.label(),
                        })
                } else {
                    None
                }
            })
        } else {
            None
        }
    } else {
        None
    };

    let Some(brand) = detect_any_brand().await else {
        return err_sse("No hardware wallet detected. Plug one in and try again.").await;
    };

    let hwi_guard = match try_lock_hwi(&state.hwi_lock) {
        Some(g) => g,
        None => {
            return err_sse("Another hardware-wallet operation is already running. Wait for it to finish or cancel it.").await;
        }
    };

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(16);
    let config_dir = crate::config::config_dir();
    let network_kind = state.config.read().await.network.kind;

    spawn_hw_worker(tx, hwi_guard, brand, move |inner_tx| {
        Box::pin(async move {
            match brand {
                HwBrand::Bitbox => {
                    bitbox::run_sign_psbt(
                        inner_tx,
                        network_kind,
                        config_dir,
                        psbt_b64,
                        multisig_info,
                        policy_info,
                        wallet_id,
                    )
                    .await
                }
                HwBrand::Ledger => {
                    ledger::run_sign_psbt(
                        inner_tx,
                        network_kind,
                        config_dir,
                        psbt_b64,
                        multisig_info,
                        policy_info,
                        wallet_id,
                    )
                    .await
                }
                HwBrand::Trezor => {
                    trezor::run_sign_psbt(
                        inner_tx,
                        network_kind,
                        config_dir,
                        psbt_b64,
                        multisig_info,
                        policy_info,
                        wallet_id,
                    )
                    .await
                }
            }
        })
    });

    let stream = ReceiverStream::new(rx);
    Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}
