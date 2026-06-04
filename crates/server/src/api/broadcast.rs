use axum::{extract::State, http::StatusCode, Json};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use bdk_wallet::bitcoin::{consensus::deserialize, psbt::Psbt, Address, Transaction};
use bdk_wallet::SignOptions;
use corvin_core::backends::{electrum, rpc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    config::BackendType,
    state::{AppState, WalletInner},
};

type Err = (StatusCode, Json<serde_json::Value>);

fn bad_req(msg: impl std::fmt::Display) -> Err {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": msg.to_string() })),
    )
}

fn gateway_err(msg: impl std::fmt::Display) -> Err {
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({ "error": msg.to_string() })),
    )
}

#[derive(Deserialize)]
pub struct TxPayload {
    pub psbt: Option<String>,
    pub raw_hex: Option<String>,
    /// When the PSBT belongs to one of our wallets, lets the server finalize it
    /// via the wallet's descriptor before extracting the tx — needed for
    /// miniscript/taproot script-path spends the device-side finalizer can't
    /// assemble. Optional (pasted-PSBT broadcasts won't have it).
    #[serde(default)]
    pub wallet_id: Option<uuid::Uuid>,
}

#[derive(Serialize)]
pub struct DecodedInput {
    txid: String,
    vout: u32,
    value_sat: Option<u64>,
}

#[derive(Serialize)]
pub struct DecodedOutput {
    address: Option<String>,
    value_sat: u64,
}

#[derive(Serialize)]
pub struct DecodedTx {
    txid: String,
    inputs: Vec<DecodedInput>,
    outputs: Vec<DecodedOutput>,
    fee_sat: Option<u64>,
    fee_rate_sat_vb: Option<f64>,
    vsize: u64,
    is_rbf: bool,
    /// true when vsize comes from the unsigned tx (PSBT not yet finalized)
    vsize_approximate: bool,
}

#[derive(Serialize)]
pub struct BroadcastResult {
    txid: String,
}

fn parse_psbt(b64: &str) -> Result<Psbt, Err> {
    let bytes = STANDARD
        .decode(b64)
        .map_err(|e| bad_req(format!("Invalid base64: {e}")))?;
    Psbt::deserialize(&bytes).map_err(|e| bad_req(format!("Invalid PSBT: {e}")))
}

fn parse_raw_hex(hex_str: &str) -> Result<Transaction, Err> {
    let bytes = hex::decode(hex_str).map_err(|e| bad_req(format!("Invalid hex: {e}")))?;
    deserialize::<Transaction>(&bytes).map_err(|e| bad_req(format!("Invalid transaction: {e}")))
}

pub async fn decode_tx(
    State(state): State<AppState>,
    Json(body): Json<TxPayload>,
) -> Result<Json<DecodedTx>, Err> {
    let network = state.manager.read().await.network;

    let (tx, psbt, vsize_approximate) = if let Some(b64) = &body.psbt {
        let psbt = parse_psbt(b64)?;
        // Try finalized tx for accurate vsize; fall back to unsigned_tx for decode-only
        let (tx, approx) = psbt
            .clone()
            .extract_tx()
            .map(|t| (t, false))
            .unwrap_or_else(|_| (psbt.unsigned_tx.clone(), true));
        (tx, Some(psbt), approx)
    } else if let Some(hex_str) = &body.raw_hex {
        (parse_raw_hex(hex_str)?, None, false)
    } else {
        return Err(bad_req("Provide 'psbt' (base64) or 'raw_hex' (hex)"));
    };

    let outputs: Vec<DecodedOutput> = tx
        .output
        .iter()
        .map(|o| DecodedOutput {
            address: Address::from_script(&o.script_pubkey, network)
                .ok()
                .map(|a| a.to_string()),
            value_sat: o.value.to_sat(),
        })
        .collect();

    let inputs: Vec<DecodedInput> = tx
        .input
        .iter()
        .enumerate()
        .map(|(i, inp)| {
            let value_sat = psbt.as_ref().and_then(|p| p.inputs.get(i)).and_then(|pi| {
                pi.witness_utxo
                    .as_ref()
                    .map(|u| u.value.to_sat())
                    .or_else(|| {
                        pi.non_witness_utxo.as_ref().and_then(|prev| {
                            prev.output
                                .get(inp.previous_output.vout as usize)
                                .map(|o| o.value.to_sat())
                        })
                    })
            });
            DecodedInput {
                txid: inp.previous_output.txid.to_string(),
                vout: inp.previous_output.vout,
                value_sat,
            }
        })
        .collect();

    let total_out: u64 = outputs.iter().map(|o| o.value_sat).sum();
    let fee_sat = inputs
        .iter()
        .map(|i| i.value_sat)
        .collect::<Option<Vec<_>>>()
        .map(|vs| vs.iter().sum::<u64>().saturating_sub(total_out));

    let vsize = tx.vsize() as u64;
    let fee_rate_sat_vb = fee_sat.map(|f| f as f64 / vsize as f64);
    let is_rbf = tx.input.iter().any(|i| i.sequence.is_rbf());

    Ok(Json(DecodedTx {
        txid: tx.compute_txid().to_string(),
        inputs,
        outputs,
        fee_sat,
        fee_rate_sat_vb,
        vsize,
        is_rbf,
        vsize_approximate,
    }))
}

pub async fn broadcast_tx(
    State(state): State<AppState>,
    Json(body): Json<TxPayload>,
) -> Result<Json<BroadcastResult>, Err> {
    if state.config.read().await.offline {
        return Err(bad_req(
            "Corvin is in offline mode — broadcasting is unavailable.",
        ));
    }
    let tx: Transaction = if let Some(b64) = &body.psbt {
        let mut psbt = parse_psbt(b64)?;
        // Finalize via the owning wallet's descriptor when known — assembles
        // miniscript/taproot script-path witnesses from partial sigs that the
        // device-side finalizer leaves untouched. No-op for already-final PSBTs.
        if let Some(wid) = body.wallet_id {
            let managed = state.manager.read().await.get(&wid);
            if let Some(m) = managed {
                if let WalletInner::Hd(wm) = &m.inner {
                    let hd = wm.lock().await;
                    let _ = hd.wallet.finalize_psbt(&mut psbt, SignOptions::default());
                }
            }
        }
        psbt.extract_tx()
            .map_err(|e| bad_req(format!("PSBT is not fully signed: {e}")))?
    } else if let Some(hex_str) = &body.raw_hex {
        parse_raw_hex(hex_str)?
    } else {
        return Err(bad_req("Provide 'psbt' (base64) or 'raw_hex' (hex)"));
    };

    let txid = tx.compute_txid().to_string();
    // Broadcast through the owning wallet's backend when known, so a private
    // wallet's transaction isn't relayed via the default (public) server.
    let backend: Option<String> = match body.wallet_id {
        Some(wid) => state
            .manager
            .read()
            .await
            .get(&wid)
            .and_then(|m| m.backend()),
        None => None,
    };
    let cfg = state.config.read().await;

    match cfg.backend_kind_for(backend.as_deref()) {
        BackendType::Rpc => {
            let rpc_cfg = cfg.rpc_config_for(backend.as_deref());
            drop(cfg);
            rpc::broadcast_tx(&tx, &rpc_cfg).map_err(gateway_err)?;
        }
        BackendType::Electrum => {
            let ecfg = cfg.electrum_config_for(backend.as_deref());
            drop(cfg);
            electrum::broadcast_tx(&tx, &ecfg).map_err(gateway_err)?;
        }
    }

    Ok(Json(BroadcastResult { txid }))
}

/// Broadcast an already-assembled transaction via a wallet's backend (`None` =
/// the default backend). Shared by the payjoin sender (fallback + final payjoin
/// tx) where there's no HTTP request/response to thread through `broadcast_tx`.
pub(crate) async fn broadcast_transaction(
    state: &AppState,
    tx: &Transaction,
    backend: Option<&str>,
) -> anyhow::Result<String> {
    let txid = tx.compute_txid().to_string();
    let cfg = state.config.read().await;
    match cfg.backend_kind_for(backend) {
        BackendType::Rpc => {
            let rpc_cfg = cfg.rpc_config_for(backend);
            drop(cfg);
            rpc::broadcast_tx(tx, &rpc_cfg).map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        BackendType::Electrum => {
            let ecfg = cfg.electrum_config_for(backend);
            drop(cfg);
            electrum::broadcast_tx(tx, &ecfg).map_err(|e| anyhow::anyhow!("{e}"))?;
        }
    }
    Ok(txid)
}
