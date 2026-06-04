//! Payjoin (BIP-77 / v2) receiving — software single-sig wallets, RPC backend.
//!
//! `provision` returns a `pj=` URI to share; a background task
//! (`payjoin_subscriber::run_receive_one`) long-polls the directory and, when
//! the payer's original arrives, runs the BIP-77 receiver checks, contributes
//! one of our inputs, and parks at `ProvisionalProposal` (no seed needed). The
//! user then `confirm`s with their seed, which signs our contributed input and
//! posts the proposal back. Receive needs an RPC backend (testmempoolaccept)
//! and a native-segwit or taproot wallet (the input kinds we can contribute).

use axum::{
    extract::{Path, State},
    Json,
};
use bdk_wallet::bitcoin::{Amount, Psbt};
use bdk_wallet::KeychainKind;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use payjoin::receive::v2::{replay_event_log, ReceiveSession, ReceiverBuilder, SessionEvent};
use payjoin::OhttpKeys;

use crate::api::ApiError;
use crate::config::{BackendType, Config};
use crate::payjoin_sessions::{self, FileSessionPersister, PjStatus, SessionKind, SessionMeta};
use crate::state::{AppState, WalletInner};
use corvin_core::types::InputKind;

use super::seed_signer;
use super::{get_managed, network_from_config};

#[derive(Debug)]
struct PjSignError(String);
impl std::fmt::Display for PjSignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for PjSignError {}

#[derive(Deserialize)]
pub struct ProvisionReceiveRequest {
    /// Optional amount to embed in the pj= URI (sats).
    #[serde(default)]
    pub amount_sats: Option<u64>,
}

#[derive(Serialize)]
pub struct ProvisionReceiveResponse {
    pub session_id: Uuid,
    /// BIP-21 `bitcoin:…?pj=…` URI to hand to the payer.
    pub uri: String,
    pub address: String,
}

#[derive(Deserialize)]
pub struct ConfirmReceiveRequest {
    pub mnemonic: String,
    #[serde(default)]
    pub passphrase: String,
}

#[derive(Serialize)]
pub struct PayjoinReceiveStatus {
    pub status: PjStatus,
}

fn supported_receive_kind(kind: &InputKind) -> bool {
    matches!(kind, InputKind::Zpub | InputKind::Taproot)
}

/// Fetch the directory's OHTTP gateway keys. When a SOCKS5 proxy is configured
/// we fetch the directory's `/.well-known/ohttp-gateway` directly through our
/// proxied client (Tor provides the IP privacy the relay hop otherwise would),
/// honoring the proxy. Without a proxy we use the crate's relay-proxied fetch
/// so the directory still doesn't see our IP.
async fn fetch_ohttp_keys(state: &AppState, cfg: &Config) -> anyhow::Result<OhttpKeys> {
    let directory = cfg.backend.payjoin_directory_url.trim_end_matches('/');
    if cfg.backend.socks5_proxy.is_some() {
        let client = state.http.read().await.clone();
        let url = format!("{directory}/.well-known/ohttp-gateway");
        let resp = client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("ohttp-gateway returned HTTP {}", resp.status());
        }
        let body = resp.bytes().await?;
        OhttpKeys::decode(&body).map_err(|e| anyhow::anyhow!("decode ohttp keys: {e}"))
    } else {
        payjoin::io::fetch_ohttp_keys(cfg.backend.payjoin_ohttp_relay_url.as_str(), directory)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

/// `POST /wallets/{id}/payjoin-receive` — provision a receive session + URI.
pub async fn provision_payjoin_receive(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<ProvisionReceiveRequest>,
) -> Result<Json<ProvisionReceiveResponse>, ApiError> {
    let cfg = state.config.read().await.clone();
    if !cfg.backend.payjoin_enabled {
        return Err(
            anyhow::anyhow!("Payjoin is disabled — enable it in Settings → Backend").into(),
        );
    }
    if !matches!(cfg.backend.kind, BackendType::Rpc) {
        return Err(anyhow::anyhow!(
            "Payjoin receive needs a Bitcoin node (RPC) backend (to validate the sender's transaction)"
        )
        .into());
    }

    let managed = get_managed(&state, &id).await?;
    if !supported_receive_kind(&managed.entry.kind) {
        return Err(anyhow::anyhow!(
            "Payjoin receive currently supports native-segwit and taproot wallets"
        )
        .into());
    }

    // Reveal + persist a fresh receive address (so it isn't reused).
    let address = {
        let WalletInner::Hd(wm) = &managed.inner else {
            return Err(anyhow::anyhow!("not an HD wallet").into());
        };
        let mut hd = wm.lock().await;
        let info = hd.wallet.reveal_next_address(KeychainKind::External);
        let addr = info.address.clone();
        if let Err(e) = hd.persist_staged() {
            tracing::warn!("couldn't persist revealed payjoin receive address: {e}");
        }
        addr
    };

    // Fetch the directory's OHTTP gateway keys to publish in our URI.
    let ohttp_keys = fetch_ohttp_keys(&state, &cfg)
        .await
        .map_err(|e| anyhow::anyhow!("fetching payjoin OHTTP keys: {e}"))?;

    let session_id = Uuid::new_v4();
    let persister = FileSessionPersister::<SessionEvent>::new(session_id);
    let mut builder = ReceiverBuilder::new(
        address.clone(),
        cfg.backend.payjoin_directory_url.as_str(),
        ohttp_keys,
    )
    .map_err(|e| anyhow::anyhow!("payjoin receiver: {e}"))?;
    if let Some(sats) = req.amount_sats {
        builder = builder.with_amount(Amount::from_sat(sats));
    }
    let receiver = builder
        .build()
        .save(&persister)
        .map_err(|e| anyhow::anyhow!("persist payjoin session: {e}"))?;
    let uri = receiver.pj_uri().to_string();

    payjoin_sessions::register(
        session_id,
        SessionMeta {
            wallet_id: id,
            kind: SessionKind::Receive,
            created_at: Utc::now().to_rfc3339(),
            status: PjStatus::Negotiating,
            result_txid: None,
        },
    )?;
    crate::payjoin_subscriber::respawn_receive_one(&state, session_id).await;

    Ok(Json(ProvisionReceiveResponse {
        session_id,
        uri,
        address: address.to_string(),
    }))
}

#[derive(Serialize)]
pub struct PayjoinReceiveSessionInfo {
    pub session_id: Uuid,
    pub status: PjStatus,
    pub created_at: String,
    /// The `pj=` invoice to re-display, for a session still awaiting payment.
    /// `None` once a proposal has arrived (confirm doesn't need it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// Reconstruct the `pj=` URI for a session still in the Initialized (waiting)
/// state by replaying its event log.
fn session_pj_uri(sid: Uuid) -> Option<String> {
    let persister = FileSessionPersister::<SessionEvent>::new(sid);
    let (session, _history) = replay_event_log(&persister).ok()?;
    match session {
        ReceiveSession::Initialized(r) => Some(r.pj_uri().to_string()),
        _ => None,
    }
}

/// `GET /wallets/{id}/payjoin-receive` — list this wallet's receive sessions
/// (newest first). Lets the Receive modal resume either a still-waiting invoice
/// (status `negotiating`, with its `uri`) or a proposal that arrived while the
/// modal was closed (status `proposal_ready`).
pub async fn list_payjoin_receive(
    State(_state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<PayjoinReceiveSessionInfo>>, ApiError> {
    let mut out: Vec<PayjoinReceiveSessionInfo> = payjoin_sessions::list_for_wallet(id)
        .into_iter()
        .filter(|(_, m)| m.kind == SessionKind::Receive)
        .map(|(sid, m)| {
            let uri = if m.status == PjStatus::Negotiating {
                session_pj_uri(sid)
            } else {
                None
            };
            PayjoinReceiveSessionInfo {
                session_id: sid,
                status: m.status,
                created_at: m.created_at,
                uri,
            }
        })
        .collect();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(Json(out))
}

/// `GET /wallets/{id}/payjoin-receive/{sid}` — current status.
pub async fn payjoin_receive_status(
    State(_state): State<AppState>,
    Path((_id, sid)): Path<(Uuid, Uuid)>,
) -> Result<Json<PayjoinReceiveStatus>, ApiError> {
    let meta =
        payjoin_sessions::get(sid).ok_or_else(|| anyhow::anyhow!("payjoin session not found"))?;
    Ok(Json(PayjoinReceiveStatus {
        status: meta.status,
    }))
}

/// `POST /wallets/{id}/payjoin-receive/{sid}/confirm` — sign our contributed
/// input and post the proposal back to the payer.
pub async fn confirm_payjoin_receive(
    State(state): State<AppState>,
    Path((id, sid)): Path<(Uuid, Uuid)>,
    Json(mut req): Json<ConfirmReceiveRequest>,
) -> Result<Json<PayjoinReceiveStatus>, ApiError> {
    let mnemonic = Zeroizing::new(std::mem::take(&mut req.mnemonic));
    let passphrase = Zeroizing::new(std::mem::take(&mut req.passphrase));
    let network = network_from_config(&state).await;
    let cfg = state.config.read().await.clone();

    let managed = get_managed(&state, &id).await?;
    let (base_path, script_type) = seed_signer::signing_params(&managed.entry)?;

    let persister = FileSessionPersister::<SessionEvent>::new(sid);
    let (session, _history) =
        replay_event_log(&persister).map_err(|e| anyhow::anyhow!("replay session: {e}"))?;
    let provisional = match session {
        ReceiveSession::ProvisionalProposal(r) => r,
        _ => {
            return Err(anyhow::anyhow!("no payjoin proposal is ready for this session yet").into())
        }
    };

    // Sign only our contributed input (the sender re-signs theirs).
    let sign = |psbt: &Psbt| -> Result<Psbt, payjoin::ImplementationError> {
        let mut p = psbt.clone();
        seed_signer::sign_with_seed(
            &base_path,
            script_type,
            network,
            mnemonic.as_str(),
            passphrase.as_str(),
            &mut p,
        )
        .map_err(|e| payjoin::ImplementationError::new(PjSignError(e.to_string())))?;
        Ok(p)
    };

    let proposal = provisional
        .finalize_proposal(sign)
        .save(&persister)
        .map_err(|e| anyhow::anyhow!("finalize payjoin proposal: {e}"))?;

    let (request, ctx) = proposal
        .create_post_request(cfg.backend.payjoin_ohttp_relay_url.as_str())
        .map_err(|e| anyhow::anyhow!("build payjoin response: {e}"))?;
    let resp = crate::payjoin_subscriber::pj_post(&state, &request).await?;
    proposal
        .process_response(&resp, ctx)
        .save(&persister)
        .map_err(|e| anyhow::anyhow!("post payjoin response: {e}"))?;

    payjoin_sessions::set_status(sid, PjStatus::Sent, None)?;
    state.emit(
        "payjoin_receive_sent",
        serde_json::json!({ "wallet_id": id, "session_id": sid }),
    );

    Ok(Json(PayjoinReceiveStatus {
        status: PjStatus::Sent,
    }))
}

/// `DELETE /wallets/{id}/payjoin-receive/{sid}` — cancel the session.
pub async fn cancel_payjoin_receive(
    State(state): State<AppState>,
    Path((_id, sid)): Path<(Uuid, Uuid)>,
) -> Result<axum::http::StatusCode, ApiError> {
    if let Some(handle) = state.payjoin_tasks.lock().await.remove(&sid) {
        handle.abort();
    }
    payjoin_sessions::forget(sid)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
