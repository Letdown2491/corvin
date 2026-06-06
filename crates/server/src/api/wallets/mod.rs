mod crud;
mod multisig;
mod payjoin_receive;
mod payjoin_send;
mod reads;
pub(crate) mod seed_signer;
pub(crate) mod send;
mod sp_send;
mod sp_spend;
mod sync;
mod tax;
mod vault;
pub use crud::*;
pub use multisig::*;
pub use payjoin_receive::*;
pub use payjoin_send::*;
pub use reads::*;
pub use send::*;
pub use sp_send::*;
pub use sp_spend::*;
pub use sync::*;
pub use tax::*;
pub use vault::*;

use crate::api::ApiError;
use crate::state::{AppState, WalletInner};
use axum::{
    extract::{Path, State},
    Json,
};
use bdk_wallet::bitcoin::Network;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// Confirmations of a UTXO at chain `tip` (0 if unconfirmed). Shared by the
/// send paths so the maturity math isn't copy-pasted (and can't drift).
pub(super) fn confs_at(
    pos: &bdk_wallet::chain::ChainPosition<bdk_wallet::chain::ConfirmationBlockTime>,
    tip: u32,
) -> u32 {
    match pos {
        bdk_wallet::chain::ChainPosition::Confirmed { anchor, .. } => {
            tip.saturating_sub(anchor.block_id.height).saturating_add(1)
        }
        _ => 0,
    }
}

/// Txids of the wallet's coinbase transactions (their outputs need 100 confs
/// before they're spendable).
pub(super) fn coinbase_txids(
    wallet: &bdk_wallet::Wallet,
) -> std::collections::HashSet<bdk_wallet::bitcoin::Txid> {
    wallet
        .transactions()
        .filter(|c| c.tx_node.tx.is_coinbase())
        .map(|c| c.tx_node.txid)
        .collect()
}

// ── consolidation PSBT ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ConsolidateRequest {
    pub utxos: Vec<String>,
    pub fee_rate_sat_vb: f64,
    pub destination: String,
}

#[derive(Debug, Serialize)]
pub struct ConsolidateResult {
    pub psbt: String,
    pub input_sats: u64,
    pub output_sats: u64,
    pub fee_sats: u64,
}

pub async fn build_consolidate_psbt(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<ConsolidateRequest>,
) -> Result<Json<ConsolidateResult>, ApiError> {
    if req.utxos.len() < 2 {
        return Err(anyhow::anyhow!("select at least 2 UTXOs to consolidate").into());
    }

    // Freeze is authoritative: a frozen coin is never a spend candidate, including here
    // (the UI hides them, but reject server-side so the API can't bypass the freeze).
    let frozen_set = state.annotations.frozen_utxos().await;
    if let Some(f) = req.utxos.iter().find(|s| frozen_set.contains(*s)) {
        return Err(anyhow::anyhow!("UTXO {f} is frozen — unfreeze it before consolidating").into());
    }

    let network = network_from_config(&state).await;
    let managed = get_managed(&state, &id).await?;

    let WalletInner::Hd(wm) = &managed.inner else {
        return Err(anyhow::anyhow!("consolidation requires an HD wallet").into());
    };

    use bdk_wallet::bitcoin::{Address, FeeRate, OutPoint, Txid};
    use std::str::FromStr;

    let dest_addr = Address::from_str(&req.destination)
        .map_err(|e| anyhow::anyhow!("invalid destination address: {e}"))?
        .require_network(network)
        .map_err(|_| anyhow::anyhow!("destination address is for the wrong network"))?;
    let dest_script = dest_addr.script_pubkey();

    let outpoints: Vec<OutPoint> = req
        .utxos
        .iter()
        .map(|s| {
            let (txid_s, vout_s) = s
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("malformed outpoint: {s}"))?;
            let txid =
                Txid::from_str(txid_s).map_err(|_| anyhow::anyhow!("invalid txid: {txid_s}"))?;
            let vout: u32 = vout_s
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid vout: {vout_s}"))?;
            Ok(OutPoint { txid, vout })
        })
        .collect::<anyhow::Result<_>>()?;

    let fee_rate_sats = validated_fee_rate_sats(req.fee_rate_sat_vb)?;
    let fee_rate = FeeRate::from_sat_per_vb(fee_rate_sats)
        .ok_or_else(|| anyhow::anyhow!("invalid fee rate"))?;

    let psbt = {
        let mut hd = wm.lock().await;
        let tip = hd.wallet.latest_checkpoint().height();
        let coinbase_txids: std::collections::HashSet<_> = hd
            .wallet
            .transactions()
            .filter(|c| c.tx_node.tx.is_coinbase())
            .map(|c| c.tx_node.txid)
            .collect();
        let utxo_by_op: std::collections::HashMap<_, _> = hd
            .wallet
            .list_unspent()
            .map(|u| (u.outpoint, u.chain_position))
            .collect();
        for op in &outpoints {
            if !coinbase_txids.contains(&op.txid) {
                continue;
            }
            let confs = match utxo_by_op.get(op) {
                Some(bdk_wallet::chain::ChainPosition::Confirmed { anchor, .. }) => {
                    tip.saturating_sub(anchor.block_id.height).saturating_add(1)
                }
                _ => 0,
            };
            if confs < 100 {
                return Err(anyhow::anyhow!(
                    "UTXO {}:{} is an immature coinbase output ({} of 100 confirmations). \
                     Wait until it matures before consolidating.",
                    op.txid,
                    op.vout,
                    confs
                )
                .into());
            }
        }

        let wallet = &mut hd.wallet;
        let mut builder = wallet.build_tx();
        for op in &outpoints {
            builder
                .add_utxo(*op)
                .map_err(|e| anyhow::anyhow!("UTXO not available: {e}"))?;
        }
        builder.manually_selected_only();
        builder.drain_to(dest_script);
        builder.fee_rate(fee_rate);
        builder
            .finish()
            .map_err(|e| anyhow::anyhow!("failed to build transaction: {e}"))?
    };

    let output_sats = psbt
        .unsigned_tx
        .output
        .first()
        .map(|o| o.value.to_sat())
        .unwrap_or(0);

    let input_sats: u64 = psbt
        .inputs
        .iter()
        .enumerate()
        .map(|(i, inp)| {
            if let Some(txout) = &inp.witness_utxo {
                txout.value.to_sat()
            } else if let Some(tx) = &inp.non_witness_utxo {
                let vout = psbt.unsigned_tx.input[i].previous_output.vout as usize;
                tx.output.get(vout).map(|o| o.value.to_sat()).unwrap_or(0)
            } else {
                0
            }
        })
        .sum();

    let fee_sats = input_sats.saturating_sub(output_sats);

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let psbt_b64 = STANDARD.encode(psbt.serialize());

    Ok(Json(ConsolidateResult {
        psbt: psbt_b64,
        input_sats,
        output_sats,
        fee_sats,
    }))
}

// ── helpers ───────────────────────────────────────────────────────────────────

pub(super) async fn network_from_config(state: &AppState) -> Network {
    state.config.read().await.network.kind.to_bitcoin_network()
}

pub(super) async fn get_managed(
    state: &AppState,
    id: &Uuid,
) -> Result<Arc<crate::state::ManagedWallet>, ApiError> {
    let manager = state.manager.read().await;
    manager
        .get(id)
        .ok_or_else(|| ApiError::not_found("wallet not found"))
}
