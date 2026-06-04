//! Spending FROM a Silent Payments wallet (BIP-352 receive-side coins → out).
//!
//! BDK is no help here: the inputs are SP-discovered P2TR outputs whose keys
//! are `spend_secret + t_n`, not described by any descriptor. So we hand-build
//! and sign the transaction:
//!   1. Re-derive the spend secret from the mnemonic (account recovered by
//!      matching the stored spend pubkey).
//!   2. Source UTXOs from the wallet's `SilentPaymentsCache` (unspent, unfrozen).
//!   3. Per input, the spending key is `d_n = spend_secret + t_n`; sanity-check
//!      its x-only pubkey equals the recorded output key.
//!   4. Coin-select (largest-first), estimate the fee, and — for change — make
//!      a BIP-352 self-payment to our own m=0 change address (the scanner
//!      registers m=0, so change is re-discovered).
//!   5. Sign each input with a key-path BIP-340 Schnorr signature. The SP output
//!      key is used directly (no taproot tweak), so the signing key is d_n.
//!   6. Broadcast and mark the consumed outputs spent.
//!
//! v1 limits: recipients must be plain addresses (no SP→SP yet).

use axum::{
    extract::{Path, State},
    Json,
};
use bdk_wallet::bitcoin::hashes::Hash;
use bdk_wallet::bitcoin::{
    absolute::LockTime,
    secp256k1::{Keypair, Message, Scalar, Secp256k1, SecretKey},
    sighash::{Prevouts, SighashCache, TapSighashType},
    taproot,
    transaction::Version,
    Address, Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid,
    Witness,
};
use serde::{Deserialize, Serialize};
use silentpayments::sending::generate_recipient_pubkeys;
use silentpayments::utils::sending::calculate_partial_secret;
use silentpayments::SilentPaymentAddress;
use std::str::FromStr;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::api::ApiError;
use crate::state::{AppState, WalletInner};
use corvin_core::types::{InputKind, SpOutputRecord};

use super::send::OutputSpec;
use super::{get_managed, network_from_config};

/// P2TR dust floor — change below this is dropped into the fee instead.
const DUST_SATS: u64 = 330;
/// Approx vbytes for one P2TR key-path input (57.5, rounded up).
const INPUT_VB: u64 = 58;
/// Fixed per-tx overhead (version, locktime, segwit marker, counts).
const TX_OVERHEAD_VB: u64 = 11;

#[derive(Deserialize)]
pub struct SpSpendRequest {
    pub outputs: Vec<OutputSpec>,
    pub fee_rate_sat_vb: f64,
    pub mnemonic: String,
    #[serde(default)]
    pub passphrase: String,
    /// Coin control: specific `txid:vout` outputs to spend. Absent = all
    /// unspent, unfrozen SP outputs.
    #[serde(default)]
    pub utxos: Option<Vec<String>>,
}

#[derive(Serialize)]
pub struct SpSpendResult {
    pub txid: String,
    pub input_sats: u64,
    pub recipient_sats: u64,
    pub change_sats: u64,
    pub fee_sats: u64,
}

fn is_sp_address(s: &str) -> bool {
    let l = s.to_lowercase();
    l.starts_with("sp1") || l.starts_with("tsp1") || l.starts_with("sprt1")
}

fn parse_hex_32(s: &str) -> Option<[u8; 32]> {
    let b = hex::decode(s).ok()?;
    b.try_into().ok()
}
fn parse_hex_33(s: &str) -> Option<[u8; 33]> {
    let b = hex::decode(s).ok()?;
    b.try_into().ok()
}

/// Estimated vbytes for `n_inputs` P2TR inputs plus the given output scripts.
fn est_vsize(n_inputs: u64, output_spks: &[ScriptBuf]) -> u64 {
    let outs: u64 = output_spks.iter().map(|s| 9 + s.len() as u64).sum();
    TX_OVERHEAD_VB + INPUT_VB * n_inputs + outs
}

pub async fn build_sp_spend(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<SpSpendRequest>,
) -> Result<Json<SpSpendResult>, ApiError> {
    if req.outputs.is_empty() {
        return Err(anyhow::anyhow!("at least one output is required").into());
    }
    if req
        .outputs
        .iter()
        .filter(|o| o.amount_sats.is_none())
        .count()
        > 1
    {
        return Err(anyhow::anyhow!("only one output may be send-max (drain)").into());
    }

    let managed = get_managed(&state, &id).await?;
    if managed.entry.kind != InputKind::SilentPayments {
        return Err(anyhow::anyhow!("this endpoint spends from a Silent Payments wallet").into());
    }
    let WalletInner::SilentPayments(cache_mutex) = &managed.inner else {
        return Err(anyhow::anyhow!("not a Silent Payments wallet").into());
    };

    let network = network_from_config(&state).await;
    let mnemonic_z = Zeroizing::new(std::mem::take(&mut req.mnemonic));
    let passphrase_z = Zeroizing::new(std::mem::take(&mut req.passphrase));

    // Stored SP keys → recover the spend secret (also a wrong-seed check).
    let keys = crate::api::silent_payments::get_keys(id)
        .ok_or_else(|| anyhow::anyhow!("no Silent Payments keys for this wallet"))?;
    let scan_secret = parse_hex_32(&keys.scan_secret_hex)
        .ok_or_else(|| anyhow::anyhow!("stored scan secret is malformed"))?;
    let spend_pubkey = parse_hex_33(&keys.spend_pubkey_hex)
        .ok_or_else(|| anyhow::anyhow!("stored spend pubkey is malformed"))?;
    // Prefer the stored account index (direct derivation); fall back to guessing
    // by spend-pubkey match for legacy entries that predate persisting it.
    let spend_secret = match keys.account_index {
        Some(account) => corvin_core::silent_payments::derive_spend_secret(
            mnemonic_z.as_str(),
            passphrase_z.as_str(),
            network,
            account,
        )?,
        None => corvin_core::silent_payments::derive_spend_secret_for_spend_pubkey(
            mnemonic_z.as_str(),
            passphrase_z.as_str(),
            network,
            &spend_pubkey,
        )?,
    };

    // Candidate UTXOs from the SP cache (unspent, unfrozen), plus the chain tip
    // for anti-fee-sniping locktime.
    let frozen = state.annotations.frozen_utxos().await;
    let (mut candidates, tip): (Vec<SpOutputRecord>, u32) = {
        let cache = cache_mutex.lock().await;
        let outs = cache
            .outputs
            .iter()
            .filter(|o| !o.spent && !frozen.contains(&format!("{}:{}", o.txid, o.vout)))
            .cloned()
            .collect();
        (outs, cache.tip_height)
    };
    if let Some(cc) = &req.utxos {
        let want: std::collections::HashSet<String> = cc.iter().cloned().collect();
        candidates.retain(|o| want.contains(&format!("{}:{}", o.txid, o.vout)));
    }

    // Build + sign the transaction (network-free; broadcast happens below).
    let built = build_sp_spend_tx(
        &req.outputs,
        req.fee_rate_sat_vb,
        &spend_secret,
        &scan_secret,
        &spend_pubkey,
        candidates,
        tip,
        network,
    )?;
    let tx = built.tx;

    // ── Broadcast + mark spent ──────────────────────────────────────────────
    let txid = tx.compute_txid().to_string();
    {
        use crate::config::BackendType;
        use corvin_core::backends::{electrum, rpc};
        let backend_owned = managed.backend();
        let backend = backend_owned.as_deref();
        let cfg = state.config.read().await;
        match cfg.backend_kind_for(backend) {
            BackendType::Rpc => {
                let c = cfg.rpc_config_for(backend);
                drop(cfg);
                rpc::broadcast_tx(&tx, &c).map_err(|e| anyhow::anyhow!("broadcast: {e}"))?;
            }
            BackendType::Electrum => {
                let c = cfg.electrum_config_for(backend);
                drop(cfg);
                electrum::broadcast_tx(&tx, &c).map_err(|e| anyhow::anyhow!("broadcast: {e}"))?;
            }
        }
    }

    // Record the spend. The scanner only ever *adds* outputs (it doesn't detect
    // spends), so this is the sole place SP UTXOs are marked spent. Update the
    // in-memory cache FIRST (infallible) so the running session is correct even
    // if the disk write fails; then persist. The tx is already broadcast, so a
    // persist failure is a warning, not an error returned to the caller.
    let spent_outpoints = built.spent_outpoints;
    {
        let mut cache = cache_mutex.lock().await;
        for o in cache.outputs.iter_mut() {
            if spent_outpoints
                .iter()
                .any(|(t, v)| *t == o.txid && *v == o.vout)
            {
                o.spent = true;
            }
        }
    }
    if let Err(e) = crate::sp_outputs::mark_spent(id, &spent_outpoints) {
        tracing::warn!(
            "sp-spend {txid}: broadcast succeeded but persisting spent state failed: {e}"
        );
    }

    Ok(Json(SpSpendResult {
        txid,
        input_sats: built.input_sats,
        recipient_sats: built.recipient_sats,
        change_sats: built.change_sats,
        fee_sats: built.fee_sats,
    }))
}

/// The result of building + signing an SP-spend transaction, before broadcast.
struct SpSpendTx {
    tx: Transaction,
    input_sats: u64,
    recipient_sats: u64,
    change_sats: u64,
    fee_sats: u64,
    spent_outpoints: Vec<(String, u32)>,
}

/// Build + key-path-Schnorr-sign the SP-spend transaction — everything except
/// broadcast and marking outputs spent. Pure + network-free so it's unit-testable:
/// given the wallet's keys and its candidate SP outputs it coin-selects, derives
/// each input's spending key `d_n = spend_secret + t_n`, builds the outputs (incl.
/// a BIP-352 self-payment change), and signs every input.
#[allow(clippy::too_many_arguments)]
fn build_sp_spend_tx(
    outputs: &[OutputSpec],
    fee_rate_sat_vb: f64,
    spend_secret: &[u8; 32],
    scan_secret: &[u8; 32],
    spend_pubkey: &[u8; 33],
    mut candidates: Vec<SpOutputRecord>,
    tip: u32,
    network: Network,
) -> Result<SpSpendTx, ApiError> {
    // Parse recipients — plain addresses only in v1.
    let mut recipients: Vec<(ScriptBuf, Option<u64>)> = Vec::new();
    for (i, o) in outputs.iter().enumerate() {
        let t = o.recipient.trim();
        if is_sp_address(t) {
            return Err(anyhow::anyhow!(
                "output #{}: paying a Silent Payment address from an SP wallet isn't supported yet — send to a regular address.",
                i + 1
            ).into());
        }
        let addr = Address::from_str(t)
            .map_err(|e| anyhow::anyhow!("output #{}: invalid address: {e}", i + 1))?
            .require_network(network)
            .map_err(|_| anyhow::anyhow!("output #{}: address is for the wrong network", i + 1))?;
        if let Some(a) = o.amount_sats {
            if a == 0 {
                return Err(anyhow::anyhow!("output #{}: amount must be > 0", i + 1).into());
            }
        }
        recipients.push((addr.script_pubkey(), o.amount_sats));
    }

    if candidates.is_empty() {
        return Err(anyhow::anyhow!("no spendable Silent Payments UTXOs").into());
    }
    candidates.sort_by_key(|c| std::cmp::Reverse(c.value_sats));

    // Validate + clamp the fee rate (rejects NaN/Infinity, caps at 10k sat/vB)
    // through the same gate the regular send path uses, instead of raw float math
    // that could overflow to u64::MAX on a non-finite input.
    let fee_rate_sats = super::validated_fee_rate_sats(fee_rate_sat_vb)?;
    let fee_for = |vb: u64| -> u64 { vb.saturating_mul(fee_rate_sats) };
    let drain = outputs.iter().any(|o| o.amount_sats.is_none());
    let explicit_total: u64 = recipients.iter().filter_map(|(_, a)| *a).sum();
    let recipient_spks: Vec<ScriptBuf> = recipients.iter().map(|(s, _)| s.clone()).collect();

    // ── Coin selection ──────────────────────────────────────────────────────
    let (selected, fee, change_sats): (Vec<SpOutputRecord>, u64, u64) = if drain {
        let total: u64 = candidates.iter().map(|o| o.value_sats).sum();
        let fee = fee_for(est_vsize(candidates.len() as u64, &recipient_spks));
        // The drain output gets `total - explicit_total - fee`; require it to be
        // at least the dust floor so we never build a sub-dust output the network
        // would reject.
        if total < explicit_total + fee + DUST_SATS {
            return Err(anyhow::anyhow!("insufficient funds to cover recipients + fee").into());
        }
        (candidates, fee, 0)
    } else {
        // Largest-first until inputs cover recipients + fee (with a change out).
        let mut change_spk_probe = recipient_spks.clone();
        change_spk_probe.push(p2tr_spk(&[0u8; 32])); // placeholder change for sizing
        let mut sel: Vec<SpOutputRecord> = Vec::new();
        let mut sum = 0u64;
        let mut chosen: Option<(u64, u64)> = None; // (fee, change)
        for o in &candidates {
            sel.push(o.clone());
            sum += o.value_sats;
            let fee = fee_for(est_vsize(sel.len() as u64, &change_spk_probe));
            if sum >= explicit_total + fee {
                chosen = Some((fee, sum - explicit_total - fee));
                break;
            }
        }
        let (mut fee, mut change) = chosen
            .ok_or_else(|| anyhow::anyhow!("insufficient funds to cover recipients + fee"))?;
        if change <= DUST_SATS {
            // No worthwhile change — drop the change output and fold the excess
            // into the fee. Report the *actual* fee (everything not paid to a
            // recipient), not just the nominal rate-based fee, so the UI doesn't
            // understate what the user pays.
            let nominal = fee_for(est_vsize(sel.len() as u64, &recipient_spks));
            if sum < explicit_total + nominal {
                return Err(anyhow::anyhow!("insufficient funds to cover recipients + fee").into());
            }
            fee = sum - explicit_total;
            change = 0;
        }
        (sel, fee, change)
    };

    let input_sats: u64 = selected.iter().map(|o| o.value_sats).sum();
    let secp = Secp256k1::new();

    // ── Per-input spending keys d_n = spend_secret + t_n ─────────────────────
    let base_sk = SecretKey::from_slice(spend_secret.as_ref())
        .map_err(|e| anyhow::anyhow!("spend secret: {e}"))?;
    let mut input_keys: Vec<SecretKey> = Vec::with_capacity(selected.len());
    let mut prevouts: Vec<TxOut> = Vec::with_capacity(selected.len());
    for o in &selected {
        let t_n = parse_hex_32(&o.tweak_t_n_hex)
            .ok_or_else(|| anyhow::anyhow!("malformed tweak for {}:{}", o.txid, o.vout))?;
        let scalar = Scalar::from_be_bytes(t_n)
            .map_err(|_| anyhow::anyhow!("tweak out of range for {}:{}", o.txid, o.vout))?;
        let d_n = base_sk
            .add_tweak(&scalar)
            .map_err(|e| anyhow::anyhow!("derive input key: {e}"))?;
        // Sanity: the derived key must control the recorded output key.
        let (xonly, _parity) = d_n.x_only_public_key(&secp);
        let expected = parse_hex_32(&o.output_xonly_hex)
            .ok_or_else(|| anyhow::anyhow!("malformed output key for {}:{}", o.txid, o.vout))?;
        if xonly.serialize() != expected {
            return Err(anyhow::anyhow!(
                "derived spending key doesn't match UTXO {}:{} — wrong seed for this wallet",
                o.txid,
                o.vout
            )
            .into());
        }
        input_keys.push(d_n);
        let spk = ScriptBuf::from_hex(&o.script_pubkey_hex)
            .map_err(|e| anyhow::anyhow!("bad script for {}:{}: {e}", o.txid, o.vout))?;
        prevouts.push(TxOut {
            value: Amount::from_sat(o.value_sats),
            script_pubkey: spk,
        });
    }

    // ── Build outputs ─────────────────────────────────────────────────────────
    let mut tx_outs: Vec<TxOut> = Vec::new();
    let mut recipient_sats = 0u64;
    for (spk, amount) in &recipients {
        let value = match amount {
            Some(a) => *a,
            None => input_sats.saturating_sub(explicit_total + fee), // drain remainder
        };
        recipient_sats += value;
        tx_outs.push(TxOut {
            value: Amount::from_sat(value),
            script_pubkey: spk.clone(),
        });
    }

    // Change → BIP-352 self-payment to our own m=0 change address.
    if change_sats > 0 {
        let sp_input_keys: Vec<(silentpayments::secp256k1::SecretKey, bool)> = input_keys
            .iter()
            .map(|k| {
                silentpayments::secp256k1::SecretKey::from_slice(&k.secret_bytes())
                    .map(|sk| (sk, true))
            })
            .collect::<Result<_, _>>()
            .map_err(|e| anyhow::anyhow!("input key convert: {e}"))?;
        let outpoints: Vec<(String, u32)> =
            selected.iter().map(|o| (o.txid.clone(), o.vout)).collect();
        let partial = calculate_partial_secret(&sp_input_keys, &outpoints)
            .map_err(|e| anyhow::anyhow!("partial_secret: {e:?}"))?;
        let change_addr = corvin_core::silent_payments::change_address_from_stored(
            scan_secret,
            spend_pubkey,
            network,
        )?;
        let change_sp = SilentPaymentAddress::try_from(change_addr.as_str())
            .map_err(|e| anyhow::anyhow!("change address: {e:?}"))?;
        let derived = generate_recipient_pubkeys(vec![change_sp], partial)
            .map_err(|e| anyhow::anyhow!("change pubkey: {e:?}"))?;
        let xonly = derived
            .get(&change_sp)
            .and_then(|v| v.first())
            .ok_or_else(|| anyhow::anyhow!("no change output key derived"))?;
        tx_outs.push(TxOut {
            value: Amount::from_sat(change_sats),
            script_pubkey: p2tr_spk(&xonly.serialize()),
        });
    }

    // ── Assemble + sign ───────────────────────────────────────────────────────
    let mut tx = Transaction {
        version: Version::TWO,
        lock_time: LockTime::from_height(tip).unwrap_or(LockTime::ZERO),
        input: selected
            .iter()
            .map(|o| -> Result<TxIn, ApiError> {
                Ok(TxIn {
                    previous_output: OutPoint {
                        txid: Txid::from_str(&o.txid)
                            .map_err(|e| anyhow::anyhow!("bad txid {}: {e}", o.txid))?,
                        vout: o.vout,
                    },
                    script_sig: ScriptBuf::new(),
                    sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                    witness: Witness::new(),
                })
            })
            .collect::<Result<_, _>>()?,
        output: tx_outs,
    };

    // Compute all key-spend sighashes first (immutable borrow), then sign.
    let sighashes: Vec<_> = {
        let mut cache = SighashCache::new(&tx);
        (0..tx.input.len())
            .map(|i| {
                cache
                    .taproot_key_spend_signature_hash(
                        i,
                        &Prevouts::All(&prevouts),
                        TapSighashType::Default,
                    )
                    .map_err(|e| anyhow::anyhow!("sighash #{i}: {e}"))
            })
            .collect::<Result<_, _>>()?
    };
    for (i, sighash) in sighashes.into_iter().enumerate() {
        // SP output key is used directly (no taproot tweak); sign with d_n.
        // sign_schnorr handles BIP-340 even-Y internally.
        let keypair = Keypair::from_secret_key(&secp, &input_keys[i]);
        let msg = Message::from_digest(sighash.to_byte_array());
        let sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
        let tap_sig = taproot::Signature {
            signature: sig,
            sighash_type: TapSighashType::Default,
        };
        let mut w = Witness::new();
        w.push(tap_sig.to_vec());
        tx.input[i].witness = w;
    }

    let spent_outpoints: Vec<(String, u32)> =
        selected.iter().map(|o| (o.txid.clone(), o.vout)).collect();

    Ok(SpSpendTx {
        tx,
        input_sats,
        recipient_sats,
        change_sats,
        fee_sats: fee,
        spent_outpoints,
    })
}

/// Build a P2TR scriptPubKey from a 32-byte x-only key used directly (BIP-352
/// outputs are not taproot-tweaked): `OP_1 OP_PUSHBYTES_32 <key>`.
fn p2tr_spk(xonly: &[u8; 32]) -> ScriptBuf {
    let mut v = Vec::with_capacity(34);
    v.push(0x51);
    v.push(0x20);
    v.extend_from_slice(xonly);
    ScriptBuf::from_bytes(v)
}

#[cfg(test)]
mod tests {
    use super::{build_sp_spend_tx, p2tr_spk};
    use crate::api::wallets::send::OutputSpec;
    use bdk_wallet::bitcoin::hashes::Hash;
    use bdk_wallet::bitcoin::secp256k1::{schnorr, Message, Scalar, Secp256k1, SecretKey};
    use bdk_wallet::bitcoin::sighash::{Prevouts, SighashCache, TapSighashType};
    use bdk_wallet::bitcoin::{
        Address, Amount, CompressedPublicKey, Network, ScriptBuf, TxOut, XOnlyPublicKey,
    };
    use corvin_core::types::SpOutputRecord;

    const MNEMONIC: &str =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    /// Fabricate an SP output the wallet's spend secret controls: choose a tweak
    /// `t_n`, derive `d_n = spend_secret + t_n`, take its x-only pubkey as the
    /// output key, and build the matching P2TR scriptPubKey — exactly the shape
    /// the scanner records. Returns (record, output x-only key, scriptPubKey).
    fn fabricate_output(
        secp: &Secp256k1<bdk_wallet::bitcoin::secp256k1::All>,
        base_sk: &SecretKey,
        idx: u8,
        value_sats: u64,
    ) -> (SpOutputRecord, XOnlyPublicKey, ScriptBuf) {
        let t_n = [idx + 1; 32];
        let scalar = Scalar::from_be_bytes(t_n).unwrap();
        let d_n = base_sk.add_tweak(&scalar).unwrap();
        let (xonly, _parity) = d_n.x_only_public_key(secp);
        let spk = p2tr_spk(&xonly.serialize());
        let rec = SpOutputRecord {
            txid: format!("{:064x}", idx as u128 + 1),
            vout: 0,
            height: 100,
            value_sats,
            script_pubkey_hex: hex::encode(spk.as_bytes()),
            output_xonly_hex: hex::encode(xonly.serialize()),
            tweak_t_n_hex: hex::encode(t_n),
            frigate_tweak_hex: String::new(),
            label_m: None,
            found_at: chrono::Utc::now(),
            spent: false,
        };
        (rec, xonly, spk)
    }

    fn regtest_recipient(secp: &Secp256k1<bdk_wallet::bitcoin::secp256k1::All>) -> String {
        let sk = SecretKey::from_slice(&[0x42u8; 32]).unwrap();
        let pk = CompressedPublicKey(sk.public_key(secp));
        Address::p2wpkh(&pk, Network::Regtest).to_string()
    }

    #[test]
    fn sp_spend_produces_valid_keyspend_signatures_with_change() {
        // The real correctness check: the hand-rolled key-path Schnorr signatures
        // must actually validate against each input's output key. Two inputs +
        // a non-drain amount also exercises change (a BIP-352 self-payment).
        let spend_secret =
            corvin_core::silent_payments::derive_spend_secret(MNEMONIC, "", Network::Regtest, 0)
                .unwrap();
        let keys =
            corvin_core::silent_payments::derive_from_mnemonic(MNEMONIC, "", Network::Regtest, 0)
                .unwrap();
        let secp = Secp256k1::new();
        let base_sk = SecretKey::from_slice(&spend_secret[..]).unwrap();

        let (rec0, xonly0, spk0) = fabricate_output(&secp, &base_sk, 0, 100_000);
        let (rec1, xonly1, spk1) = fabricate_output(&secp, &base_sk, 1, 100_000);

        let built = build_sp_spend_tx(
            &[OutputSpec {
                recipient: regtest_recipient(&secp),
                amount_sats: Some(120_000),
            }],
            2.0,
            &spend_secret,
            &keys.scan_secret,
            &keys.spend_pubkey,
            vec![rec0, rec1],
            200,
            Network::Regtest,
        )
        .ok()
        .expect("sp-spend tx should build");

        assert_eq!(built.tx.input.len(), 2, "both inputs spent");
        assert_eq!(built.tx.output.len(), 2, "recipient + change");
        assert_eq!(built.input_sats, 200_000);
        assert_eq!(built.recipient_sats, 120_000);
        assert!(built.change_sats > 0, "change should be present");
        assert_eq!(
            built.input_sats,
            built.recipient_sats + built.change_sats + built.fee_sats,
            "value must balance: inputs = recipient + change + fee"
        );

        // Verify every input's signature against its prevout's output key.
        let prevouts = vec![
            TxOut {
                value: Amount::from_sat(100_000),
                script_pubkey: spk0,
            },
            TxOut {
                value: Amount::from_sat(100_000),
                script_pubkey: spk1,
            },
        ];
        let xonlys = [xonly0, xonly1];
        let mut cache = SighashCache::new(&built.tx);
        for (i, xonly) in xonlys.iter().enumerate() {
            let sighash = cache
                .taproot_key_spend_signature_hash(i, &Prevouts::All(&prevouts), TapSighashType::Default)
                .unwrap();
            let msg = Message::from_digest(sighash.to_byte_array());
            let sig_item = built.tx.input[i]
                .witness
                .iter()
                .next()
                .expect("input must carry a witness signature");
            let sig = schnorr::Signature::from_slice(sig_item).expect("64-byte Schnorr signature");
            secp.verify_schnorr(&sig, &msg, xonly)
                .expect("key-path signature must validate against the output key");
        }
    }

    #[test]
    fn sp_spend_rejects_subdust_drain() {
        // A drain whose only input can't cover fee + the dust floor for the drain
        // output must be rejected, not produce a sub-dust output.
        let spend_secret =
            corvin_core::silent_payments::derive_spend_secret(MNEMONIC, "", Network::Regtest, 0)
                .unwrap();
        let keys =
            corvin_core::silent_payments::derive_from_mnemonic(MNEMONIC, "", Network::Regtest, 0)
                .unwrap();
        let secp = Secp256k1::new();
        let base_sk = SecretKey::from_slice(&spend_secret[..]).unwrap();
        // ~200 sat fee at 2 sat/vB for 1 input + 1 output; 400 < fee + DUST_SATS.
        let (rec, _x, _s) = fabricate_output(&secp, &base_sk, 0, 400);

        let res = build_sp_spend_tx(
            &[OutputSpec {
                recipient: regtest_recipient(&secp),
                amount_sats: None, // drain
            }],
            2.0,
            &spend_secret,
            &keys.scan_secret,
            &keys.spend_pubkey,
            vec![rec],
            200,
            Network::Regtest,
        );
        assert!(res.is_err(), "sub-dust drain must be rejected");
    }
}
