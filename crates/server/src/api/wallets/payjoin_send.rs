//! Payjoin (BIP-77 / v2) sending — software single-sig wallets only.
//!
//! Async by design: we build + sign the original ("fallback") transaction, POST
//! it to the payjoin directory through an OHTTP relay, then a background task
//! (`payjoin_subscriber`) long-polls for the receiver's proposal. Because the
//! receiver may be offline for a while, the session is event-sourced to disk
//! (`payjoin_sessions`) and survives restarts; if no proposal arrives before
//! `payjoin_fallback_secs`, the original is broadcast instead.
//!
//! Two seed entries per send: once here to sign the original, once at
//! `/confirm` to re-sign the receiver-modified proposal. The mnemonic is never
//! persisted (re-supplied each step, zeroized on return).

use axum::{
    extract::{Path, State},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use bdk_wallet::bitcoin::{FeeRate, Psbt};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use payjoin::persist::SessionPersister;
use payjoin::send::v2::{
    replay_event_log, SendSession, SenderBuilder, SessionEvent, SessionOutcome,
};
use payjoin::{PjParam, Uri, UriExt};

use crate::api::ApiError;
use crate::payjoin_sessions::{self, FileSessionPersister, PjStatus, SessionKind, SessionMeta};
use crate::state::AppState;
use corvin_core::types::InputKind;

use super::seed_signer;
use super::send::{OutputSpec, SendRequest};
use super::{get_managed, network_from_config};

#[derive(Deserialize)]
pub struct BuildPayjoinRequest {
    /// A BIP-21 `bitcoin:` URI carrying a `pj=` payjoin endpoint.
    pub uri: String,
    pub fee_rate_sat_vb: f64,
    /// Coin-control passthrough to the regular send builder.
    #[serde(default)]
    pub utxos: Option<Vec<String>>,
    pub mnemonic: String,
    #[serde(default)]
    pub passphrase: String,
}

#[derive(Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum BuildPayjoinResponse {
    /// v2 session opened; the poll task is now negotiating with the receiver.
    Negotiating {
        session_id: Uuid,
        fallback_txid: String,
    },
    /// The URI is v1-only (no async support). The frontend should fall back to
    /// a normal send to the on-chain address.
    V1Unsupported {
        recipient: String,
        amount_sats: Option<u64>,
    },
}

#[derive(Deserialize)]
pub struct ConfirmPayjoinRequest {
    pub mnemonic: String,
    #[serde(default)]
    pub passphrase: String,
}

#[derive(Serialize)]
pub struct ConfirmPayjoinResponse {
    pub txid: String,
}

#[derive(Serialize)]
pub struct ProposalDiff {
    /// Inputs the receiver contributed (proposal inputs minus original inputs).
    pub added_inputs: usize,
    pub total_inputs: usize,
    /// Network fee of the payjoin transaction.
    pub proposal_fee_sats: u64,
}

#[derive(Serialize)]
pub struct PayjoinStatusResponse {
    pub status: PjStatus,
    pub result_txid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<ProposalDiff>,
}

fn supported_kind(kind: &InputKind) -> bool {
    matches!(
        kind,
        InputKind::Xpub | InputKind::Ypub | InputKind::Zpub | InputKind::Taproot
    )
}

/// `POST /wallets/{id}/payjoin-send` — build + sign the original, open a v2
/// session, POST it, and spawn the poll task.
pub async fn build_payjoin_send(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<BuildPayjoinRequest>,
) -> Result<Json<BuildPayjoinResponse>, ApiError> {
    let mnemonic = Zeroizing::new(std::mem::take(&mut req.mnemonic));
    let passphrase = Zeroizing::new(std::mem::take(&mut req.passphrase));

    let cfg = state.config.read().await.clone();
    if !cfg.backend.payjoin_enabled {
        return Err(
            anyhow::anyhow!("Payjoin is disabled — enable it in Settings → Backend").into(),
        );
    }
    let network = network_from_config(&state).await;

    let managed = get_managed(&state, &id).await?;
    if !supported_kind(&managed.entry.kind) {
        return Err(anyhow::anyhow!(
            "Payjoin send is only supported for single-signature software wallets"
        )
        .into());
    }
    let (base_path, script_type) = seed_signer::signing_params(&managed.entry)?;

    // Parse + network-check the URI, then confirm it's payjoin-capable.
    let uri = Uri::try_from(req.uri.as_str())
        .map_err(|e| anyhow::anyhow!("invalid bitcoin URI: {e}"))?
        .require_network(network)
        .map_err(|_| anyhow::anyhow!("this URI is for a different Bitcoin network"))?;
    let pjuri = uri
        .check_pj_supported()
        .map_err(|_| anyhow::anyhow!("this URI has no payjoin (pj=) endpoint"))?;

    let recipient = pjuri.address.to_string();
    let amount = pjuri.amount;

    // v2 only. A v1 URI would panic the v2 builder, so route it back to the UI
    // for a normal send.
    if matches!(pjuri.extras.pj_param(), PjParam::V1(_)) {
        return Ok(Json(BuildPayjoinResponse::V1Unsupported {
            recipient,
            amount_sats: amount.map(|a| a.to_sat()),
        }));
    }
    let amount = amount.ok_or_else(|| anyhow::anyhow!("payjoin URI is missing an amount"))?;

    // Build the original via the regular send builder (carries coin control,
    // fee logic and privacy warnings), then sign it — it doubles as the
    // broadcastable fallback transaction.
    let send_req = SendRequest {
        outputs: vec![OutputSpec {
            recipient: recipient.clone(),
            amount_sats: Some(amount.to_sat()),
        }],
        fee_rate_sat_vb: req.fee_rate_sat_vb,
        utxos: req.utxos.clone(),
        spend_path: None,
    };
    let built =
        super::send::build_send_psbt(State(state.clone()), Path(id), Json(send_req)).await?;
    let mut original = Psbt::deserialize(
        &STANDARD
            .decode(&built.0.psbt)
            .map_err(|e| anyhow::anyhow!("decode original psbt: {e}"))?,
    )
    .map_err(|e| anyhow::anyhow!("parse original psbt: {e}"))?;

    let finalized = seed_signer::sign_with_seed(
        &base_path,
        script_type,
        network,
        mnemonic.as_str(),
        passphrase.as_str(),
        &mut original,
    )?;
    if !finalized {
        return Err(ApiError::wrong_secret(
            "couldn't fully sign the original transaction — wrong seed/passphrase for this wallet?",
        ));
    }
    let fallback_txid = original
        .clone()
        .extract_tx()
        .map(|t| t.compute_txid().to_string())
        .map_err(|e| anyhow::anyhow!("original not broadcastable: {e}"))?;

    // Payjoin minimum fee rate (a floor for the builder; 0 = no minimum, which is
    // legitimate). Guard against non-finite/absurd input so Infinity can't wrap to
    // u64::MAX and then silently collapse to ZERO.
    let min_fee_sats = if req.fee_rate_sat_vb.is_finite() && req.fee_rate_sat_vb > 0.0 {
        (req.fee_rate_sat_vb.ceil() as u64).min(10_000)
    } else {
        0
    };
    let min_fee_rate = FeeRate::from_sat_per_vb(min_fee_sats).unwrap_or(FeeRate::ZERO);

    // Open + persist the v2 session.
    let session_id = Uuid::new_v4();
    let persister = FileSessionPersister::<SessionEvent>::new(session_id);
    let sender = SenderBuilder::new(original, pjuri)
        .build_recommended(min_fee_rate)
        .map_err(|e| anyhow::anyhow!("payjoin build: {e}"))?
        .save(&persister)
        .map_err(|e| anyhow::anyhow!("persist payjoin session: {e}"))?;
    payjoin_sessions::register(
        session_id,
        SessionMeta {
            wallet_id: id,
            kind: SessionKind::Send,
            created_at: Utc::now().to_rfc3339(),
            status: PjStatus::Negotiating,
            result_txid: None,
        },
    )?;

    // POST the original to the directory through the OHTTP relay.
    let (request, post_ctx) = sender
        .create_v2_post_request(cfg.backend.payjoin_ohttp_relay_url.as_str())
        .map_err(|e| anyhow::anyhow!("build payjoin request: {e}"))?;
    let resp = match crate::payjoin_subscriber::pj_post(&state, &request).await {
        Ok(r) => r,
        Err(e) => {
            let _ = payjoin_sessions::forget(session_id);
            return Err(anyhow::anyhow!("posting to payjoin directory: {e}").into());
        }
    };
    sender
        .process_response(&resp, post_ctx)
        .save(&persister)
        .map_err(|e| anyhow::anyhow!("payjoin post rejected: {e}"))?;

    crate::payjoin_subscriber::respawn_one(&state, session_id).await;

    Ok(Json(BuildPayjoinResponse::Negotiating {
        session_id,
        fallback_txid,
    }))
}

/// `GET /wallets/{id}/payjoin-send/{sid}` — current status (+ proposal diff once
/// the receiver has responded).
pub async fn payjoin_send_status(
    State(_state): State<AppState>,
    Path((_id, sid)): Path<(Uuid, Uuid)>,
) -> Result<Json<PayjoinStatusResponse>, ApiError> {
    let meta =
        payjoin_sessions::get(sid).ok_or_else(|| anyhow::anyhow!("payjoin session not found"))?;

    let diff = if meta.status == PjStatus::ProposalReady {
        proposal_diff(sid)
    } else {
        None
    };

    Ok(Json(PayjoinStatusResponse {
        status: meta.status,
        result_txid: meta.result_txid,
        diff,
    }))
}

/// Compute the receiver's contribution from the persisted session: original
/// input count vs the validated proposal, plus the proposal's fee.
fn proposal_diff(sid: Uuid) -> Option<ProposalDiff> {
    let persister = FileSessionPersister::<SessionEvent>::new(sid);
    let (session, history) = replay_event_log(&persister).ok()?;
    let proposal = match session {
        SendSession::Closed(SessionOutcome::Success(psbt)) => psbt,
        _ => return None,
    };
    let original_inputs = history.fallback_tx().input.len();
    let total_inputs = proposal.unsigned_tx.input.len();
    let proposal_fee_sats = proposal.fee().ok()?.to_sat();
    Some(ProposalDiff {
        added_inputs: total_inputs.saturating_sub(original_inputs),
        total_inputs,
        proposal_fee_sats,
    })
}

/// `POST /wallets/{id}/payjoin-send/{sid}/confirm` — re-sign the receiver's
/// proposal and broadcast the payjoin.
pub async fn confirm_payjoin_send(
    State(state): State<AppState>,
    Path((id, sid)): Path<(Uuid, Uuid)>,
    Json(mut req): Json<ConfirmPayjoinRequest>,
) -> Result<Json<ConfirmPayjoinResponse>, ApiError> {
    let mnemonic = Zeroizing::new(std::mem::take(&mut req.mnemonic));
    let passphrase = Zeroizing::new(std::mem::take(&mut req.passphrase));
    let network = network_from_config(&state).await;

    let managed = get_managed(&state, &id).await?;
    let (base_path, script_type) = seed_signer::signing_params(&managed.entry)?;

    let persister = FileSessionPersister::<SessionEvent>::new(sid);
    let (session, _history) =
        replay_event_log(&persister).map_err(|e| anyhow::anyhow!("replay session: {e}"))?;
    let mut proposal = match session {
        SendSession::Closed(SessionOutcome::Success(psbt)) => psbt,
        _ => {
            return Err(anyhow::anyhow!("no payjoin proposal is ready for this session yet").into())
        }
    };

    // Re-sign our inputs over the receiver-modified transaction. The crate
    // already validated the proposal (outputs preserved, fee not decreased) and
    // restored our input metadata, so the transient wallet can sign.
    let finalized = seed_signer::sign_with_seed(
        &base_path,
        script_type,
        network,
        mnemonic.as_str(),
        passphrase.as_str(),
        &mut proposal,
    )?;
    if !finalized {
        return Err(ApiError::wrong_secret(
            "payjoin proposal didn't finalize after signing — wrong seed/passphrase?",
        ));
    }
    let tx = proposal
        .extract_tx()
        .map_err(|e| anyhow::anyhow!("payjoin proposal not broadcastable: {e}"))?;
    let backend = state
        .manager
        .read()
        .await
        .get(&id)
        .and_then(|m| m.backend());
    let txid =
        crate::api::broadcast::broadcast_transaction(&state, &tx, backend.as_deref()).await?;

    payjoin_sessions::set_status(sid, PjStatus::Sent, Some(txid.clone()))?;
    state.emit(
        "payjoin_sent",
        serde_json::json!({ "wallet_id": id, "session_id": sid, "txid": txid }),
    );

    Ok(Json(ConfirmPayjoinResponse { txid }))
}

/// `DELETE /wallets/{id}/payjoin-send/{sid}` — abandon negotiation and broadcast
/// the original fallback transaction now.
pub async fn abandon_payjoin_send(
    State(state): State<AppState>,
    Path((id, sid)): Path<(Uuid, Uuid)>,
) -> Result<Json<ConfirmPayjoinResponse>, ApiError> {
    // Stop the poll task first so it can't also broadcast.
    if let Some(handle) = state.payjoin_tasks.lock().await.remove(&sid) {
        handle.abort();
    }
    let persister = FileSessionPersister::<SessionEvent>::new(sid);
    let (_session, history) =
        replay_event_log(&persister).map_err(|e| anyhow::anyhow!("replay session: {e}"))?;
    let tx = history.fallback_tx();
    let backend = state
        .manager
        .read()
        .await
        .get(&id)
        .and_then(|m| m.backend());
    let txid =
        crate::api::broadcast::broadcast_transaction(&state, &tx, backend.as_deref()).await?;
    let _ = persister.save_event(SessionEvent::Closed(SessionOutcome::Cancel));
    payjoin_sessions::set_status(sid, PjStatus::FellBack, Some(txid.clone()))?;
    state.emit(
        "payjoin_fell_back",
        serde_json::json!({ "wallet_id": id, "session_id": sid, "txid": txid }),
    );
    Ok(Json(ConfirmPayjoinResponse { txid }))
}
