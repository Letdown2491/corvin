//! Per-session payjoin poll tasks (BIP-77 sender side).
//!
//! Mirrors `sp_subscriber.rs`: one tokio task per active send session that
//! long-polls the directory (through the OHTTP relay) for the receiver's
//! proposal, emitting SSE on arrival and broadcasting the original fallback
//! transaction once `payjoin_fallback_secs` elapses. Sessions are event-sourced
//! to disk, so `start()` respawns any still-negotiating session after a restart.
//!
//! Unlike the SP scanner (a blocking socket in `spawn_blocking`), the payjoin
//! HTTP is async reqwest, so these are ordinary async tasks. The crate calls in
//! between are fast, synchronous crypto.

use std::time::Duration;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use payjoin::persist::{OptionalTransitionOutcome, SessionPersister};
use payjoin::receive::v2::{
    replay_event_log as replay_recv, ReceiveSession, SessionEvent as RecvEvent,
};
use payjoin::receive::InputPair;
use payjoin::send::v2::{
    replay_event_log, SendSession, SessionEvent, SessionHistory, SessionOutcome,
};

use crate::api::broadcast::broadcast_transaction;
use crate::payjoin_sessions::{self, FileSessionPersister, PjStatus, SessionKind};
use crate::state::{AppState, WalletInner};
use corvin_core::types::InputKind;

/// Concrete error for the payjoin receiver callbacks (`ImplementationError`
/// wants a `std::error::Error`).
#[derive(Debug)]
struct PjCallbackError(String);
impl std::fmt::Display for PjCallbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for PjCallbackError {}

/// POST an OHTTP-encapsulated payjoin request through the configured backend's
/// HTTP client (so the same socks5 proxy applies). Both the initial POST and
/// the polling GET are encapsulated POSTs to the relay.
pub(crate) async fn pj_post(
    state: &AppState,
    request: &payjoin::Request,
) -> anyhow::Result<Vec<u8>> {
    let client = state.http.read().await.clone();
    let resp = client
        .post(&request.url)
        .header(reqwest::header::CONTENT_TYPE, request.content_type)
        .body(request.body.clone())
        .send()
        .await?;
    let status = resp.status();
    let bytes = resp.bytes().await?;
    if !status.is_success() {
        anyhow::bail!("payjoin relay returned HTTP {status}");
    }
    Ok(bytes.to_vec())
}

/// Respawn poll tasks for every still-negotiating session at startup.
pub async fn start(state: &AppState) {
    // Offline mode: payjoin negotiation polls a relay over the network — skip it.
    if state.config.read().await.offline {
        return;
    }
    for (sid, meta) in payjoin_sessions::all_sessions() {
        if meta.status != PjStatus::Negotiating {
            continue;
        }
        match meta.kind {
            SessionKind::Send => respawn_one(state, sid).await,
            SessionKind::Receive => respawn_receive_one(state, sid).await,
        }
    }
}

/// (Re)spawn the poll task for one session, aborting any existing handle.
pub async fn respawn_one(state: &AppState, session_id: Uuid) {
    let mut tasks = state.payjoin_tasks.lock().await;
    // Reap finished handles so the map can't grow without bound over many sessions.
    tasks.retain(|_, h| !h.is_finished());
    if let Some(handle) = tasks.remove(&session_id) {
        handle.abort();
    }
    let state = state.clone();
    let handle = tokio::spawn(async move { run_one(state, session_id).await });
    tasks.insert(session_id, handle);
}

async fn run_one(state: AppState, session_id: Uuid) {
    let persister = FileSessionPersister::<SessionEvent>::new(session_id);
    let (session, history) = match replay_event_log(&persister) {
        Ok(x) => x,
        Err(e) => {
            tracing::warn!("payjoin session {session_id} replay failed: {e}");
            return;
        }
    };
    let mut sender = match session {
        SendSession::PollingForProposal(s) => s,
        // Created-but-not-posted (the build endpoint posts before spawning us)
        // or already closed — nothing to poll.
        _ => return,
    };

    let cfg = state.config.read().await.clone();
    let relay = cfg.backend.payjoin_ohttp_relay_url.clone();
    let fallback_secs = cfg.backend.payjoin_fallback_secs;

    let deadline = payjoin_sessions::get(session_id)
        .and_then(|m| DateTime::parse_from_rfc3339(&m.created_at).ok())
        .map(|c| c.with_timezone(&Utc) + chrono::Duration::seconds(fallback_secs as i64));

    loop {
        if let Some(dl) = deadline {
            if Utc::now() >= dl {
                fallback(&state, session_id, &persister, &history).await;
                return;
            }
        }

        let (request, ctx) = match sender.create_poll_request(relay.as_str()) {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!("payjoin poll request {session_id}: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let resp = match pj_post(&state, &request).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("payjoin poll {session_id}: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        match sender.process_response(&resp, ctx).save(&persister) {
            Ok(OptionalTransitionOutcome::Progress(_proposal)) => {
                let _ = payjoin_sessions::set_status(session_id, PjStatus::ProposalReady, None);
                let wallet_id = payjoin_sessions::get(session_id).map(|m| m.wallet_id);
                state.emit(
                    "payjoin_proposal_ready",
                    serde_json::json!({ "wallet_id": wallet_id, "session_id": session_id }),
                );
                return;
            }
            Ok(OptionalTransitionOutcome::Stasis(s)) => sender = s,
            Err(e) => {
                tracing::warn!("payjoin session {session_id} failed: {e}");
                let _ = payjoin_sessions::set_status(session_id, PjStatus::Failed, None);
                state.emit(
                    "error",
                    serde_json::json!({ "message": format!("payjoin failed: {e}") }),
                );
                return;
            }
        }

        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

async fn fallback(
    state: &AppState,
    session_id: Uuid,
    persister: &FileSessionPersister<SessionEvent>,
    history: &SessionHistory,
) {
    let tx = history.fallback_tx();
    let backend = match payjoin_sessions::get(session_id).map(|m| m.wallet_id) {
        Some(wid) => state
            .manager
            .read()
            .await
            .get(&wid)
            .and_then(|m| m.backend()),
        None => None,
    };
    match broadcast_transaction(state, &tx, backend.as_deref()).await {
        Ok(txid) => {
            let _ = persister.save_event(SessionEvent::Closed(SessionOutcome::Cancel));
            let _ =
                payjoin_sessions::set_status(session_id, PjStatus::FellBack, Some(txid.clone()));
            let wallet_id = payjoin_sessions::get(session_id).map(|m| m.wallet_id);
            state.emit(
                "payjoin_fell_back",
                serde_json::json!({ "wallet_id": wallet_id, "session_id": session_id, "txid": txid }),
            );
        }
        Err(e) => {
            tracing::error!("payjoin fallback broadcast {session_id}: {e}");
            let _ = payjoin_sessions::set_status(session_id, PjStatus::Failed, None);
            state.emit(
                "error",
                serde_json::json!({ "message": format!("payjoin fallback broadcast failed: {e}") }),
            );
        }
    }
}

// ── receiver side ─────────────────────────────────────────────────────────────

/// (Re)spawn the poll/process task for one receive session.
pub async fn respawn_receive_one(state: &AppState, session_id: Uuid) {
    let mut tasks = state.payjoin_tasks.lock().await;
    // Reap finished handles so the map can't grow without bound over many sessions.
    tasks.retain(|_, h| !h.is_finished());
    if let Some(handle) = tasks.remove(&session_id) {
        handle.abort();
    }
    let state = state.clone();
    let handle = tokio::spawn(async move { run_receive_one(state, session_id).await });
    tasks.insert(session_id, handle);
}

async fn run_receive_one(state: AppState, session_id: Uuid) {
    let persister = FileSessionPersister::<RecvEvent>::new(session_id);
    let (session, _history) = match replay_recv(&persister) {
        Ok(x) => x,
        Err(e) => {
            tracing::warn!("payjoin receive {session_id} replay failed: {e}");
            return;
        }
    };
    // Only a freshly-initialized session needs polling; anything past the
    // proposal is parked (awaiting the user's confirm) or already done.
    let mut receiver = match session {
        ReceiveSession::Initialized(r) => r,
        _ => return,
    };

    let relay = state
        .config
        .read()
        .await
        .backend
        .payjoin_ohttp_relay_url
        .clone();

    let unchecked = loop {
        let (request, ctx) = match receiver.create_poll_request(relay.as_str()) {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!("payjoin receive poll request {session_id}: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let resp = match pj_post(&state, &request).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("payjoin receive poll {session_id}: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        match receiver.process_response(&resp, ctx).save(&persister) {
            Ok(OptionalTransitionOutcome::Progress(next)) => break next,
            Ok(OptionalTransitionOutcome::Stasis(r)) => receiver = r,
            Err(e) => {
                tracing::warn!("payjoin receive {session_id} failed: {e}");
                let _ = payjoin_sessions::set_status(session_id, PjStatus::Failed, None);
                state.emit(
                    "error",
                    serde_json::json!({ "message": format!("payjoin receive failed: {e}") }),
                );
                return;
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    };

    if let Err(e) = process_received(&state, session_id, &persister, unchecked).await {
        tracing::warn!("payjoin receive {session_id} processing failed: {e:#}");
        let _ = payjoin_sessions::set_status(session_id, PjStatus::Failed, None);
        state.emit(
            "error",
            serde_json::json!({ "message": format!("payjoin receive: {e}") }),
        );
    }
}

/// Run the BIP-77 receiver checks, contribute one of our inputs, and persist up
/// to `ProvisionalProposal` — then park (the user confirms with their seed to
/// sign + post). No seed is needed here.
async fn process_received(
    state: &AppState,
    session_id: Uuid,
    persister: &FileSessionPersister<RecvEvent>,
    unchecked: payjoin::receive::v2::Receiver<payjoin::receive::v2::UncheckedOriginalPayload>,
) -> anyhow::Result<()> {
    let cfg = state.config.read().await.clone();
    let rpc_cfg = cfg.rpc_config();
    let wallet_id = payjoin_sessions::get(session_id)
        .map(|m| m.wallet_id)
        .ok_or_else(|| anyhow::anyhow!("session not found"))?;

    // 1. Broadcast suitability — the only check that needs the node, not the
    //    wallet (RPC testmempoolaccept). Blocks briefly; fine for one user.
    let maybe_owned = unchecked
        .check_broadcast_suitability(None, |tx| {
            corvin_core::backends::rpc::test_mempool_accept(tx, &rpc_cfg)
                .map_err(|e| payjoin::ImplementationError::new(PjCallbackError(e.to_string())))
        })
        .save(persister)
        .map_err(|e| anyhow::anyhow!("broadcast suitability: {e}"))?;

    let managed = state
        .manager
        .read()
        .await
        .get(&wallet_id)
        .ok_or_else(|| anyhow::anyhow!("wallet not found"))?;
    let WalletInner::Hd(wm) = &managed.inner else {
        anyhow::bail!("payjoin receive requires an HD wallet");
    };
    let kind = managed.entry.kind.clone();
    let frozen = state.annotations.frozen_utxos().await;
    let seen = payjoin_sessions::load_seen();
    let mut newly_seen: Vec<String> = Vec::new();

    // The whole typestate chain below is synchronous (checks + .save are sync
    // file I/O), so we hold the wallet lock without awaiting.
    {
        let hd = wm.lock().await;

        let maybe_seen = maybe_owned
            .check_inputs_not_owned(&mut |spk| Ok(hd.wallet.is_mine(spk.to_owned())))
            .save(persister)
            .map_err(|e| anyhow::anyhow!("inputs-not-owned: {e}"))?;

        let outputs_unknown = maybe_seen
            .check_no_inputs_seen_before(&mut |op| {
                let key = format!("{}:{}", op.txid, op.vout);
                let known = seen.contains(&key);
                if !known {
                    newly_seen.push(key);
                }
                Ok(known)
            })
            .save(persister)
            .map_err(|e| anyhow::anyhow!("inputs-seen-before: {e}"))?;

        let wants_outputs = outputs_unknown
            .identify_receiver_outputs(&mut |spk| Ok(hd.wallet.is_mine(spk.to_owned())))
            .save(persister)
            .map_err(|e| anyhow::anyhow!("identify-outputs: {e}"))?;

        let wants_inputs = wants_outputs
            .commit_outputs()
            .save(persister)
            .map_err(|e| anyhow::anyhow!("commit-outputs: {e}"))?;

        let candidates = build_input_pairs(&hd.wallet, &kind, &frozen)?;
        if candidates.is_empty() {
            anyhow::bail!("no spendable UTXOs to contribute to the payjoin");
        }
        let picked = wants_inputs
            .try_preserving_privacy(candidates)
            .map_err(|e| anyhow::anyhow!("input selection: {e}"))?;
        let contributed = wants_inputs
            .contribute_inputs([picked])
            .map_err(|e| anyhow::anyhow!("contribute input: {e}"))?;
        let wants_fee = contributed
            .commit_inputs()
            .save(persister)
            .map_err(|e| anyhow::anyhow!("commit-inputs: {e}"))?;
        let _provisional = wants_fee
            .apply_fee_range(None, None)
            .save(persister)
            .map_err(|e| anyhow::anyhow!("apply-fee-range: {e}"))?;
    }

    // Record the sender's inputs as seen (anti-replay) and park for confirm.
    payjoin_sessions::mark_seen(&newly_seen)?;
    payjoin_sessions::set_status(session_id, PjStatus::ProposalReady, None)?;
    state.emit(
        "payjoin_receive_proposal",
        serde_json::json!({ "wallet_id": wallet_id, "session_id": session_id }),
    );
    Ok(())
}

/// Build payjoin `InputPair`s from our spendable (non-frozen) UTXOs. Native
/// segwit + taproot only — the kinds payjoin receive is gated to.
fn build_input_pairs(
    wallet: &bdk_wallet::Wallet,
    kind: &InputKind,
    frozen: &std::collections::HashSet<String>,
) -> anyhow::Result<Vec<InputPair>> {
    let mut pairs = Vec::new();
    for u in wallet.list_unspent() {
        let key = format!("{}:{}", u.outpoint.txid, u.outpoint.vout);
        if frozen.contains(&key) {
            continue;
        }
        let pair = match kind {
            InputKind::Zpub => InputPair::new_p2wpkh(u.txout.clone(), u.outpoint),
            InputKind::Taproot => InputPair::new_p2tr_keyspend(u.txout.clone(), u.outpoint),
            _ => continue,
        }
        .map_err(|e| anyhow::anyhow!("input pair: {e}"))?;
        pairs.push(pair);
    }
    Ok(pairs)
}
