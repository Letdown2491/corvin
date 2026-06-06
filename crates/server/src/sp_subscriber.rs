//! Background Silent Payments scanner sessions.
//!
//! On startup, one task per SP-enabled wallet. Each task opens a dedicated
//! socket to the configured Frigate server, sends `silentpayments.subscribe`
//! with the stored scan secret + spend pubkey, and drains notifications.
//!
//! On each `history` notification it fetches the tx, runs the BIP-352 receiver
//! math (`scan_transaction`), and for any match persists an `SpOutputRecord`
//! (`sp_outputs.json`), updates the wallet's in-memory `SilentPaymentsCache`
//! (which balance/utxos/txs derive from), and emits an `sp_output_discovered`
//! SSE event so the UI refreshes. See `process_history_entry`.

use bdk_wallet::bitcoin::{consensus::Decodable, Transaction};
use corvin_core::sp_scanner::{HistoryEntry, SpScanConfig, SpScanner};
use corvin_core::types::SpOutputRecord;
use silentpayments::receiving::{Label, Receiver};
use silentpayments::secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey, XOnlyPublicKey};
use silentpayments::Network as SpNetwork;
use std::time::Duration;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::state::{AppState, WalletInner};

/// Spawn one async task per SP-enabled wallet. Each task runs the
/// blocking scanner client in `spawn_blocking` and loops with exponential
/// backoff on connection errors.
/// Start scanner tasks for every SP-enabled wallet at startup, registering each
/// handle so it can be restarted at runtime.
pub async fn start(state: AppState) {
    // Offline mode: the SP scanner connects to a Frigate server, so don't start it.
    if state.config.read().await.offline {
        return;
    }
    for (wallet_id, keys) in crate::api::silent_payments::enabled_wallets() {
        respawn_one(&state, wallet_id, keys).await;
    }
}

/// Spawn (or restart) the scanner for one SP wallet: abort any existing task
/// for it, spawn a fresh one, and store the handle. Used on wallet creation and
/// on label add (so the new label is subscribed) — no app restart needed.
///
/// Aborting cancels the outer task at its next await; the inner `spawn_blocking`
/// socket op finishes on its own, so the old session drains briefly alongside
/// the new one — harmless (same wallet).
pub async fn respawn_one(
    state: &AppState,
    wallet_id: Uuid,
    keys: crate::api::silent_payments::StoredKeys,
) {
    let mut scanners = state.sp_scanners.lock().await;
    if let Some(old) = scanners.remove(&wallet_id) {
        old.abort();
    }
    let s = state.clone();
    let handle: JoinHandle<()> = tokio::spawn(async move { run_one(s, wallet_id, keys).await });
    scanners.insert(wallet_id, handle);
}

async fn run_one(state: AppState, wallet_id: Uuid, keys: crate::api::silent_payments::StoredKeys) {
    let scan_hex = keys.scan_secret_hex;
    let spend_hex = keys.spend_pubkey_hex;
    let key_network = keys.network;
    let birthday = keys.birthday_height;
    // Label indices (m >= 1) the scanner must register so payments to labeled
    // addresses are detected, not just the base/change addresses.
    let label_ms: Vec<u32> = keys.labels.iter().map(|l| l.m).collect();
    // Verify the keys belong to the network the wallet is currently configured
    // for. SP addresses differ per network; running a session with the wrong
    // keys would either fail or (worse) succeed against unrelated chain data.
    let configured_network = format!(
        "{}",
        state.config.read().await.network.kind.to_bitcoin_network()
    );
    if configured_network != key_network {
        state.sp_disconnected(
            wallet_id,
            Some(format!(
                "wallet keys are for {key_network}, but Corvin is on {configured_network}"
            )),
        );
        tracing::warn!(
            wallet = %wallet_id,
            stored = %key_network,
            configured = %configured_network,
            "SP scanner: stored keys are for a different network — skipping",
        );
        return;
    }

    let mut backoff = Duration::from_secs(5);
    let max_backoff = Duration::from_secs(300);

    loop {
        let sp_cfg = {
            let backend = state
                .manager
                .read()
                .await
                .get(&wallet_id)
                .and_then(|m| m.backend());
            let cfg = state.config.read().await;
            let e = cfg.sp_electrum_config_for(backend.as_deref());
            SpScanConfig {
                url: e.url,
                validate_tls: e.validate_tls,
                ca_cert_path: e.ca_cert_path,
                danger_accept_invalid_certs: e.danger_accept_invalid_certs,
                socks5_proxy: e.socks5_proxy,
            }
        };

        let scan_hex_clone = scan_hex.clone();
        let spend_hex_clone = spend_hex.clone();
        let configured_network_owned = configured_network.clone();
        let label_ms_clone = label_ms.clone();
        let state_clone = state.clone();
        let blocking = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            // Parse the persisted hex keys into secp256k1 types ONCE — these
            // get reused for every history entry the scanner reports.
            let scan_bytes = hex_to_bytes(&scan_hex_clone)
                .map_err(|e| anyhow::anyhow!("scan_secret_hex: {e}"))?;
            let spend_bytes = hex_to_bytes(&spend_hex_clone)
                .map_err(|e| anyhow::anyhow!("spend_pubkey_hex: {e}"))?;
            let scan_secret = SecretKey::from_slice(&scan_bytes)
                .map_err(|e| anyhow::anyhow!("scan secret: {e}"))?;
            let spend_pubkey = PublicKey::from_slice(&spend_bytes)
                .map_err(|e| anyhow::anyhow!("spend pubkey: {e}"))?;
            let secp = Secp256k1::new();
            let scan_pubkey = scan_secret.public_key(&secp);
            let sp_net = sp_network_from_string(&configured_network_owned);
            // Receiver needs a change label — Label::new(scan_secret, 0).
            let change_label = Label::new(scan_secret, 0);
            let mut receiver = Receiver::new(0, scan_pubkey, spend_pubkey, change_label, sp_net)
                .map_err(|e| anyhow::anyhow!("building receiver: {e:?}"))?;
            // Register every labeled address so scan_transaction matches them.
            for m in &label_ms_clone {
                receiver
                    .add_label(Label::new(scan_secret, *m))
                    .map_err(|e| anyhow::anyhow!("add label {m}: {e:?}"))?;
            }

            let mut scanner = SpScanner::connect(&sp_cfg)?;
            let initial =
                scanner.subscribe(&scan_hex_clone, &spend_hex_clone, birthday.map(u64::from))?;
            // Receive address is wallet-identifying; keep it out of default logs.
            tracing::debug!(wallet = %wallet_id, address = %initial.address, "SP scanner: address");
            tracing::info!(
                wallet = %wallet_id,
                labels = ?initial.labels,
                start_height = initial.start_height,
                "SP scanner: subscribed",
            );
            state_clone.sp_connected(wallet_id);
            loop {
                match scanner.next_notification()? {
                    Some(n) => {
                        if let Some(progress) = n.progress {
                            tracing::debug!(
                                wallet = %wallet_id,
                                progress = progress,
                                "SP scanner: progress update",
                            );
                        }
                        if let Some(hist) = n.history {
                            if !hist.is_empty() {
                                tracing::info!(
                                    wallet = %wallet_id,
                                    count = hist.len(),
                                    "SP scanner: history update with {} entries",
                                    hist.len(),
                                );
                                for entry in &hist {
                                    if let Err(e) = process_history_entry(
                                        &mut scanner,
                                        &state_clone,
                                        wallet_id,
                                        &scan_secret,
                                        &receiver,
                                        entry,
                                    ) {
                                        tracing::warn!(
                                            wallet = %wallet_id,
                                            error = %format!("{e:#}"),
                                            "SP scanner: failed to process history entry",
                                        );
                                        // txid is wallet-identifying; debug-only.
                                        tracing::debug!(wallet = %wallet_id, txid = %entry.tx_hash, "SP scanner: failing entry");
                                    }
                                }
                            }
                        }
                    }
                    None => {
                        anyhow::bail!("server closed connection");
                    }
                }
            }
        })
        .await;

        match blocking {
            Ok(Ok(())) => {
                // Should never reach here — the inner loop never returns Ok.
                tracing::warn!(wallet = %wallet_id, "SP scanner: session ended cleanly");
            }
            Ok(Err(e)) => {
                let msg = format!("{e:#}");
                // JSON-RPC error -32601 ("method not found") means the server
                // doesn't speak BIP-352 at all. No amount of retrying will
                // change that — bail out of the loop entirely so we stop
                // spamming logs. User has to switch SP server in Settings
                // and restart Corvin to retry.
                if msg.contains("-32601") || msg.contains("Unsupported request") {
                    state.sp_disconnected(
                        wallet_id,
                        Some("server doesn't support Silent Payments (BIP-352)".to_string()),
                    );
                    tracing::warn!(
                        wallet = %wallet_id,
                        "SP scanner: configured Electrum server doesn't support BIP-352. \
                         Set a dedicated SP server in Settings → Silent Payments scanner \
                         (e.g. frigate.2140.dev:50002), then restart Corvin.",
                    );
                    return;
                }
                if msg.contains("idle timeout") {
                    // Expected: the connection went quiet and we'll reconnect. Show a
                    // friendly status and log it softly, not as an alarming error.
                    state.sp_disconnected(wallet_id, Some("connection idle, reconnecting".to_string()));
                    tracing::info!(wallet = %wallet_id, "SP scanner: connection idle, reconnecting");
                } else {
                    state.sp_disconnected(wallet_id, Some(msg.clone()));
                    tracing::warn!(
                        wallet = %wallet_id,
                        error = %msg,
                        "SP scanner: session error",
                    );
                }
            }
            Err(join_err) => {
                state.sp_disconnected(wallet_id, Some("scanner task crashed".to_string()));
                tracing::error!(
                    wallet = %wallet_id,
                    error = %join_err,
                    "SP scanner: blocking task panicked",
                );
            }
        }

        tracing::info!(
            wallet = %wallet_id,
            backoff_secs = backoff.as_secs(),
            "SP scanner: reconnecting after backoff",
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

/// Process one `{height, tx_hash, tweak_key}` entry from a scanner history
/// notification: fetch the tx, run BIP-352 receiver math, persist any matches,
/// update the wallet's in-memory cache, and emit an SSE event.
fn process_history_entry(
    scanner: &mut SpScanner,
    state: &AppState,
    wallet_id: Uuid,
    scan_secret: &SecretKey,
    receiver: &Receiver,
    entry: &HistoryEntry,
) -> anyhow::Result<()> {
    // Fetch the matched tx so we can find which output(s) belong to us.
    let tx_bytes = scanner.tx_get(&entry.tx_hash)?;
    let tx = Transaction::consensus_decode(&mut tx_bytes.as_slice())
        .map_err(|e| anyhow::anyhow!("decoding tx: {e}"))?;

    // Extract all Taproot output x-only keys — these are the candidates the
    // receiver math will check against.
    let p2tr_outputs: Vec<(u32, XOnlyPublicKey, u64, Vec<u8>)> = tx
        .output
        .iter()
        .enumerate()
        .filter_map(|(i, o)| {
            let spk = o.script_pubkey.as_bytes();
            // P2TR: OP_1 (0x51) + OP_PUSHBYTES_32 (0x20) + 32 bytes
            if spk.len() != 34 || spk[0] != 0x51 || spk[1] != 0x20 {
                return None;
            }
            let xonly = XOnlyPublicKey::from_slice(&spk[2..]).ok()?;
            Some((i as u32, xonly, o.value.to_sat(), spk.to_vec()))
        })
        .collect();

    if p2tr_outputs.is_empty() {
        return Ok(());
    }

    // Compute ECDH shared secret: scan_secret × tweak_key (the latter is
    // input_hash * Σ input_pubkeys, pre-multiplied by the server).
    let tweak_bytes =
        hex_to_bytes(&entry.tweak_key).map_err(|e| anyhow::anyhow!("tweak_key hex: {e}"))?;
    let tweak_pk =
        PublicKey::from_slice(&tweak_bytes).map_err(|e| anyhow::anyhow!("tweak_key parse: {e}"))?;
    let secp = Secp256k1::new();
    let shared_secret = tweak_pk
        .mul_tweak(&secp, &Scalar::from(*scan_secret))
        .map_err(|e| anyhow::anyhow!("ECDH: {e}"))?;

    // Run the BIP-352 receiver routine against this tx's Taproot outputs.
    let xonly_candidates: Vec<XOnlyPublicKey> =
        p2tr_outputs.iter().map(|(_, x, _, _)| *x).collect();
    let found = receiver
        .scan_transaction(&shared_secret, xonly_candidates)
        .map_err(|e| anyhow::anyhow!("scan_transaction: {e:?}"))?;

    if found.is_empty() {
        // Frigate flagged this tx but our math doesn't agree. Could happen if
        // labels are involved that we didn't register, or if the server is
        // overreporting — log and move on.
        tracing::debug!(
            wallet = %wallet_id,
            txid = %entry.tx_hash,
            "SP scanner: server reported match but local scan found none",
        );
        return Ok(());
    }

    // Each `found` entry: HashMap<XOnlyPublicKey, Scalar (t_n)>. Build an
    // SpOutputRecord per match.
    let now = chrono::Utc::now();
    let mut new_records: Vec<SpOutputRecord> = Vec::new();
    for (label, matches_for_label) in &found {
        let label_m = label.as_ref().map(|l| {
            // Label::as_string returns hex. We don't currently expose the
            // numeric m on the public API; only the bare-address label (m=0
            // / change) is in use today. Mark labelled matches with `m=0` if
            // labelled, None otherwise — refine later when we register more.
            let _ = l;
            0u32
        });
        for (xonly, t_n) in matches_for_label {
            // Find the matching output in the tx by x-only key.
            let Some((vout, _, value_sats, spk_bytes)) =
                p2tr_outputs.iter().find(|(_, x, _, _)| x == xonly)
            else {
                continue;
            };
            new_records.push(SpOutputRecord {
                txid: entry.tx_hash.clone(),
                vout: *vout,
                height: entry.height as u32,
                value_sats: *value_sats,
                script_pubkey_hex: bytes_to_hex(spk_bytes),
                output_xonly_hex: bytes_to_hex(&xonly.serialize()),
                tweak_t_n_hex: bytes_to_hex(&t_n.to_be_bytes()),
                frigate_tweak_hex: entry.tweak_key.clone(),
                label_m,
                found_at: now,
                spent: false,
            });
        }
    }

    // Persist + update the in-memory cache. We hold the cache mutex with
    // `blocking_lock` since we're in a spawn_blocking thread.
    for rec in new_records.iter() {
        if let Err(e) = crate::sp_outputs::append(wallet_id, rec.clone()) {
            tracing::warn!(error = %e, "SP scanner: persist failed");
        }
    }

    {
        let manager = state.manager.blocking_read();
        if let Some(managed) = manager.get(&wallet_id) {
            if let WalletInner::SilentPayments(cache_mutex) = &managed.inner {
                let mut cache = cache_mutex.blocking_lock();
                if entry.height as u32 > cache.tip_height {
                    cache.tip_height = entry.height as u32;
                }
                for rec in new_records.iter() {
                    cache
                        .outputs
                        .retain(|r| !(r.txid == rec.txid && r.vout == rec.vout));
                    cache.outputs.push(rec.clone());
                }
            }
        }
    }

    if !new_records.is_empty() {
        tracing::info!(
            wallet = %wallet_id,
            txid = %entry.tx_hash,
            height = entry.height,
            count = new_records.len(),
            "SP scanner: discovered {} matching output(s)",
            new_records.len(),
        );
        state.emit(
            "sp_output_discovered",
            serde_json::json!({
                "wallet_id": wallet_id,
                "txid": entry.tx_hash,
                "height": entry.height,
                "count": new_records.len(),
            }),
        );
    }

    Ok(())
}

fn sp_network_from_string(s: &str) -> SpNetwork {
    // The state.config's `network.kind.to_bitcoin_network()` formats as
    // "bitcoin", "testnet", "signet", or "regtest" via Debug.
    match s.to_lowercase().as_str() {
        "bitcoin" => SpNetwork::Mainnet,
        "regtest" => SpNetwork::Regtest,
        _ => SpNetwork::Testnet, // testnet + signet share the SP prefix
    }
}

fn hex_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("odd hex length".to_string());
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks_exact(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    Ok(match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => return Err(format!("invalid hex byte: {b}")),
    })
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
