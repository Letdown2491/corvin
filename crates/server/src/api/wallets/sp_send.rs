//! BIP-352 Silent Payments sending.
//!
//! Software-wallet-only: the sender's input private keys are needed at PSBT
//! build time to compute the BIP-352 `partial_secret`, so we can't take the
//! HW-wallet path. The mnemonic is re-supplied on each send (never stored)
//! and zeroized immediately after the partial secret is derived.
//!
//! The flow:
//!   1. Validate the wallet is a single-sig HD wallet (no multisig, no HW).
//!   2. Validate the user's mnemonic re-derives the persisted descriptor —
//!      catches wrong-seed cases up front instead of producing a corrupt tx.
//!   3. Use the persisted (xpub) wallet to build a PSBT with placeholder P2TR
//!      outputs of the same byte length as the real ones — that way BDK's
//!      coin selection, change calculation, and fee estimate are correct.
//!   4. Sum the input private keys (re-derived from mnemonic via each input's
//!      `bip32_derivation` entry in the PSBT) and run BIP-352's
//!      `calculate_partial_secret`.
//!   5. Call `generate_recipient_pubkeys` to derive the actual output x-only
//!      keys for every SP recipient. The crate handles multi-recipient
//!      automatically — recipients sharing a scan key get indexed n=0,1,2…
//!   6. Patch the placeholder outputs in the PSBT to use those derived keys.
//!   7. Sign every input by spinning up a transient xpriv-backed BDK wallet
//!      (no persistence). BDK matches keys via the bip32_derivation field
//!      and signs correctly regardless of script type.
//!   8. Return the signed PSBT — `/broadcast` handles the rest.

use axum::{
    extract::{Path, State},
    Json,
};
use bdk_wallet::bitcoin::{
    bip32::{DerivationPath, Xpriv},
    secp256k1::Secp256k1 as BdkSecp,
    Address, Amount, FeeRate, Network, OutPoint, ScriptBuf, Txid,
};
use bip39::{Language, Mnemonic};
use serde::Deserialize;
use silentpayments::secp256k1::SecretKey;
use silentpayments::sending::generate_recipient_pubkeys;
use silentpayments::utils::sending::calculate_partial_secret;
use silentpayments::SilentPaymentAddress;
use std::str::FromStr;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::api::ApiError;
use crate::state::{AppState, WalletInner};
use corvin_core::seed;
use corvin_core::types::InputKind;

use super::send::{OutputSpec, SendResult, SendWarning};
use super::{get_managed, network_from_config};

#[derive(Deserialize)]
pub struct SpSendRequest {
    pub outputs: Vec<OutputSpec>,
    pub fee_rate_sat_vb: f64,
    /// Coin control: specific outpoints ("txid:vout") to use. If absent, BDK
    /// auto-selects (same behavior as the regular send endpoint).
    pub utxos: Option<Vec<String>>,
    /// Required for a real send: SP needs input private keys at build time.
    /// May be empty when `estimate_only` is set.
    #[serde(default)]
    pub mnemonic: String,
    #[serde(default)]
    pub passphrase: String,
    /// Fee/amount preview only — build the placeholder PSBT (correct coin
    /// selection + fee, no seed needed) and return the summary without
    /// deriving keys, patching outputs, or signing. The returned `psbt` is
    /// empty and must not be broadcast.
    #[serde(default)]
    pub estimate_only: bool,
}

pub async fn build_sp_send_psbt(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<SpSendRequest>,
) -> Result<Json<SendResult>, ApiError> {
    if req.outputs.is_empty() {
        return Err(anyhow::anyhow!("at least one output is required").into());
    }
    let drain_count = req
        .outputs
        .iter()
        .filter(|o| o.amount_sats.is_none())
        .count();
    if drain_count > 1 {
        return Err(anyhow::anyhow!("only one output may be marked send-max (drain)").into());
    }

    let managed = get_managed(&state, &id).await?;
    if managed.entry.kind == InputKind::Multisig {
        return Err(anyhow::anyhow!(
            "Silent Payments sending isn't supported from multisig wallets. The sender's input \
             private keys must be available at build time, which multisig doesn't permit."
        )
        .into());
    }
    if managed.entry.kind == InputKind::Address {
        return Err(anyhow::anyhow!(
            "Silent Payments sending requires an HD wallet — watch-only address wallets can't sign."
        )
        .into());
    }
    if managed.entry.kind == InputKind::SilentPayments {
        return Err(anyhow::anyhow!(
            "Silent Payments sending FROM an SP wallet isn't supported yet — the spend secret \
             isn't persisted. Send from a regular HD wallet to the recipient's SP address instead."
        )
        .into());
    }
    let WalletInner::Hd(wm) = &managed.inner else {
        return Err(anyhow::anyhow!("send requires an HD wallet").into());
    };

    let network = network_from_config(&state).await;

    // Move secret material into zeroizing wrappers immediately.
    let mnemonic_z = Zeroizing::new(std::mem::take(&mut req.mnemonic));
    let passphrase_z = Zeroizing::new(std::mem::take(&mut req.passphrase));

    // Validate mnemonic syntax + checksum before any expensive work. Skipped
    // for estimate-only previews, which don't need (or take) a seed.
    if !req.estimate_only {
        Mnemonic::parse_in(Language::English, mnemonic_z.as_str())
            .map_err(|e| anyhow::anyhow!("invalid mnemonic: {e}"))?;
    }

    // Parse + classify each recipient. SP addresses get the BIP-352 path;
    // regular addresses pass through unchanged.
    let mut sp_recipients: Vec<(usize, SilentPaymentAddress, u64)> = Vec::new();
    let mut regular_outputs: Vec<(usize, ScriptBuf, Option<u64>)> = Vec::new();
    for (i, o) in req.outputs.iter().enumerate() {
        let trimmed = o.recipient.trim();
        let lower = trimmed.to_lowercase();
        let is_sp =
            lower.starts_with("sp1") || lower.starts_with("tsp1") || lower.starts_with("sprt1");
        if is_sp {
            let addr = SilentPaymentAddress::try_from(trimmed)
                .map_err(|e| anyhow::anyhow!("output #{}: invalid SP address: {e:?}", i + 1))?;
            // Network compatibility check.
            if !sp_address_matches_network(&addr, network) {
                return Err(anyhow::anyhow!(
                    "output #{}: SP address is for the wrong network",
                    i + 1
                )
                .into());
            }
            let amt = o.amount_sats.ok_or_else(|| {
                anyhow::anyhow!(
                    "output #{}: SP recipient can't be send-max in v1 — set an explicit amount",
                    i + 1
                )
            })?;
            if amt == 0 {
                return Err(anyhow::anyhow!("output #{} amount must be > 0", i + 1).into());
            }
            sp_recipients.push((i, addr, amt));
        } else {
            let addr = Address::from_str(trimmed)
                .map_err(|e| anyhow::anyhow!("output #{}: invalid address: {e}", i + 1))?
                .require_network(network)
                .map_err(|_| {
                    anyhow::anyhow!("output #{}: address is for the wrong network", i + 1)
                })?;
            regular_outputs.push((i, addr.script_pubkey(), o.amount_sats));
        }
    }

    if sp_recipients.is_empty() {
        return Err(anyhow::anyhow!(
            "no Silent Payments recipients in this send — use the regular send endpoint"
        )
        .into());
    }

    // Parse the wallet's descriptor origin to learn (a) the BIP32 base path,
    // and (b) the script type. Both are needed to re-derive descriptors from
    // the mnemonic.
    let origin = crate::api::descriptor_util::parse_descriptor_origin(
        &managed.entry.external_descriptor,
    ).ok_or_else(|| anyhow::anyhow!(
        "this wallet's descriptor doesn't include origin info — can't derive input keys from a mnemonic"
    ))?;
    let script_type_str = match origin.kind {
        crate::api::descriptor_util::SimpleScriptKind::P2Wpkh => "native_segwit",
        crate::api::descriptor_util::SimpleScriptKind::P2WpkhP2sh => "wrapped_segwit",
        crate::api::descriptor_util::SimpleScriptKind::P2Tr => "taproot",
    };
    let is_taproot_inputs = matches!(origin.kind, crate::api::descriptor_util::SimpleScriptKind::P2Tr);

    // Sanity-check that the mnemonic actually re-derives this wallet. Catches
    // typos and wrong-seed cases before we spend time building a tx that
    // would be unsignable. Skipped for estimate-only (no seed to check).
    if !req.estimate_only {
        let derived = seed::derive_descriptors(
            mnemonic_z.as_str(),
            passphrase_z.as_str(),
            &origin.base_path,
            script_type_str,
            network,
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        let strip_checksum = |s: &str| -> String {
            s.rfind('#')
                .map(|i| s[..i].to_string())
                .unwrap_or_else(|| s.to_string())
        };
        if strip_checksum(&derived.external) != strip_checksum(&managed.entry.external_descriptor) {
            return Err(ApiError::wrong_secret(
                "Mnemonic doesn't match this wallet's keys. The derived descriptor differs from the \
                 persisted one — wrong seed, wrong passphrase, or wrong account index.",
            ));
        }
    }

    let fee_rate_sats = super::validated_fee_rate_sats(req.fee_rate_sat_vb)?;
    let fee_rate = FeeRate::from_sat_per_vb(fee_rate_sats)
        .ok_or_else(|| anyhow::anyhow!("invalid fee rate"))?;

    let coin_control: Option<Vec<OutPoint>> = req
        .utxos
        .as_ref()
        .map(|strs| {
            strs.iter()
                .map(|s| {
                    let (t, v) = s
                        .split_once(':')
                        .ok_or_else(|| anyhow::anyhow!("malformed outpoint: {s}"))?;
                    let txid =
                        Txid::from_str(t).map_err(|_| anyhow::anyhow!("invalid txid: {t}"))?;
                    let vout: u32 = v
                        .parse()
                        .map_err(|_| anyhow::anyhow!("invalid vout: {v}"))?;
                    Ok(OutPoint { txid, vout })
                })
                .collect::<anyhow::Result<_>>()
        })
        .transpose()?;

    let frozen_set = state.annotations.frozen_utxos().await;

    // Build the unsigned PSBT via the persisted (xpub) wallet with placeholder
    // P2TR outputs for SP recipients. The placeholders have the right byte
    // length (34 bytes) so coin selection and fee come out correct.
    let placeholder_script = ScriptBuf::from_bytes({
        let mut v = vec![0u8; 34];
        v[0] = 0x51; // OP_1
        v[1] = 0x20; // OP_PUSHBYTES_32
                     // remaining 32 bytes are zeros — invalid as a point but the same
                     // length as the real key. We swap it for the real one after building.
        v
    });

    let mut psbt = {
        let mut hd = wm.lock().await;
        let tip = hd.wallet.latest_checkpoint().height();
        let coinbase_txids = super::coinbase_txids(&hd.wallet);

        // Exclude frozen / immature coins, then let BDK coin-select over the
        // rest. Collect the *unspendable* set (not the spendable one) so we don't
        // force-spend the whole wallet via `manually_selected_only` (same bug the
        // regular send path had).
        let (auto_unspendable, auto_spendable_count): (Option<Vec<OutPoint>>, usize) =
            if coin_control.is_none() {
                let mut unspendable: Vec<OutPoint> = Vec::new();
                let mut count: usize = 0;
                for u in hd.wallet.list_unspent() {
                    let key = format!("{}:{}", u.outpoint.txid, u.outpoint.vout);
                    if frozen_set.contains(&key) {
                        unspendable.push(u.outpoint);
                        continue;
                    }
                    if coinbase_txids.contains(&u.outpoint.txid)
                        && super::confs_at(&u.chain_position, tip) < 100
                    {
                        unspendable.push(u.outpoint);
                        continue;
                    }
                    count += 1;
                }
                (Some(unspendable), count)
            } else {
                (None, 0)
            };

        if coin_control.is_none() && auto_spendable_count == 0 {
            return Err(
                anyhow::anyhow!("No spendable UTXOs. Sync the wallet or unfreeze UTXOs.").into(),
            );
        }

        let mut builder = hd.wallet.build_tx();
        builder.fee_rate(fee_rate);
        match &coin_control {
            Some(ops) => {
                for op in ops {
                    builder
                        .add_utxo(*op)
                        .map_err(|e| anyhow::anyhow!("UTXO not available: {e}"))?;
                }
                builder.manually_selected_only();
            }
            None => {
                builder.unspendable(
                    auto_unspendable
                        .clone()
                        .expect("auto_unspendable is Some when coin_control is None"),
                );
            }
        }
        let has_drain = regular_outputs.iter().any(|(_, _, a)| a.is_none());
        for (_, script, amount) in &regular_outputs {
            if let Some(amt) = amount {
                builder.add_recipient(script.clone(), Amount::from_sat(*amt));
            } else {
                builder.drain_to(script.clone());
            }
        }
        for (_, _, amt) in &sp_recipients {
            builder.add_recipient(placeholder_script.clone(), Amount::from_sat(*amt));
        }
        // Send-max in auto mode: drain the whole spendable balance (respects the
        // `unspendable` filter). Coin-control's `manually_selected_only` already
        // forces the picked coins in.
        if has_drain && coin_control.is_none() {
            builder.drain_wallet();
        }

        builder
            .finish()
            .map_err(|e| anyhow::anyhow!("failed to build transaction: {e}"))?
    };

    // Fee + amount summary. The placeholder SP outputs are byte-identical P2TR
    // scripts of the correct length, so coin selection, change, and fee are
    // already final here — before any key derivation, patching, or signing.
    let (input_sats, recipient_sats, change_sats, fee_sats) =
        summarize_send(&psbt, &regular_outputs, &placeholder_script);

    // Estimate-only preview: return the fee/amount summary with no signed PSBT.
    if req.estimate_only {
        return Ok(Json(SendResult {
            psbt: String::new(),
            input_sats,
            recipient_sats,
            change_sats,
            fee_sats,
            warnings: Vec::new(),
        }));
    }

    // ── Real send only past this point (estimate returned above) ───────────────

    // Re-derive the master xpriv now so we can sign inputs after building.
    let mnemonic_parsed = Mnemonic::parse_in(Language::English, mnemonic_z.as_str())
        .map_err(|e| anyhow::anyhow!("invalid mnemonic: {e}"))?;
    let seed_bytes: Zeroizing<[u8; 64]> =
        Zeroizing::new(mnemonic_parsed.to_seed(passphrase_z.as_str()));
    let bdk_secp = BdkSecp::new();
    let master = Xpriv::new_master(network, &*seed_bytes)
        .map_err(|e| anyhow::anyhow!("master xpriv: {e}"))?;

    // Collect input keys (with is_taproot flag) and outpoints for the
    // BIP-352 partial_secret. Each PSBT input carries the bip32_derivation
    // we need — one entry for single-sig.
    let mut input_keys: Vec<(SecretKey, bool)> = Vec::new();
    let mut outpoints_data: Vec<(String, u32)> = Vec::new();
    for (i, input) in psbt.inputs.iter().enumerate() {
        // For taproot inputs BDK uses tap_key_origins; for non-taproot,
        // bip32_derivation. Try both.
        let path: DerivationPath = if let Some((_, (_, p))) = input.tap_key_origins.values().next()
        {
            p.clone()
        } else if let Some((_, (_, p))) = input.bip32_derivation.iter().next() {
            p.clone()
        } else {
            return Err(anyhow::anyhow!(
                "input #{} has no bip32 derivation info — can't derive privkey",
                i + 1
            )
            .into());
        };
        let derived_xpriv = master
            .derive_priv(&bdk_secp, &path)
            .map_err(|e| anyhow::anyhow!("derive privkey for input #{}: {e}", i + 1))?;
        // The bitcoin crate's SecretKey is API-compatible with secp256k1::SecretKey
        // bytes-for-bytes; convert via raw bytes through the silentpayments-bundled
        // secp256k1 to avoid type confusion across crate copies.
        let raw = derived_xpriv.private_key.secret_bytes();
        let sk = SecretKey::from_slice(&raw)
            .map_err(|e| anyhow::anyhow!("input #{}: invalid secret: {e}", i + 1))?;
        input_keys.push((sk, is_taproot_inputs));

        let txin = &psbt.unsigned_tx.input[i];
        outpoints_data.push((
            txin.previous_output.txid.to_string(),
            txin.previous_output.vout,
        ));
    }

    // Compute the BIP-352 partial_secret.
    let partial_secret = calculate_partial_secret(&input_keys, &outpoints_data)
        .map_err(|e| anyhow::anyhow!("BIP-352 partial_secret: {e:?}"))?;

    // Generate one x-only pubkey per SP recipient. The crate groups by scan
    // key — recipients sharing a scan key get distinct outputs (n=0, n=1, …).
    let addrs: Vec<SilentPaymentAddress> = sp_recipients.iter().map(|(_, a, _)| *a).collect();
    let derived_keys = generate_recipient_pubkeys(addrs.clone(), partial_secret)
        .map_err(|e| anyhow::anyhow!("BIP-352 generate_recipient_pubkeys: {e:?}"))?;

    // The crate hands back HashMap<addr, Vec<XOnlyPublicKey>>. Pop one key per
    // recipient, in the order we passed them in. Iterating in `sp_recipients`
    // order preserves the user-supplied ordering.
    let mut per_addr_keys: std::collections::HashMap<
        String,
        Vec<silentpayments::secp256k1::XOnlyPublicKey>,
    > = derived_keys
        .into_iter()
        .map(|(addr, keys)| (String::from(addr), keys))
        .collect();

    // Patch placeholder outputs. Walk tx.output in order; for each one whose
    // script matches our placeholder, swap in the next derived key (in
    // sp_recipients order).
    let mut sp_iter = sp_recipients.iter();
    let placeholder_bytes = placeholder_script.as_bytes().to_vec();
    for out in psbt.unsigned_tx.output.iter_mut() {
        if out.script_pubkey.as_bytes() == placeholder_bytes.as_slice() {
            let Some((_, sp_addr, _)) = sp_iter.next() else {
                break;
            };
            let key = per_addr_keys
                .get_mut(&String::from(*sp_addr))
                .and_then(|v| {
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.remove(0))
                    }
                })
                .ok_or_else(|| anyhow::anyhow!("ran out of derived keys for SP recipient"))?;
            let mut spk = vec![0u8; 34];
            spk[0] = 0x51; // OP_1
            spk[1] = 0x20; // OP_PUSHBYTES_32
            spk[2..].copy_from_slice(&key.serialize());
            out.script_pubkey = ScriptBuf::from_bytes(spk);
        }
    }

    // Sign the (now-patched) PSBT with a transient xpriv-backed BDK wallet
    // rebuilt from the mnemonic (no persistence).
    let finalized = super::seed_signer::sign_with_seed(
        &origin.base_path,
        script_type_str,
        network,
        mnemonic_z.as_str(),
        passphrase_z.as_str(),
        &mut psbt,
    )?;
    if !finalized {
        return Err(anyhow::anyhow!(
            "PSBT didn't finalize after signing — descriptor key path doesn't match the input outputs"
        ).into());
    }

    // Patching only swapped the placeholder x-only keys for real ones — amounts
    // and script lengths are unchanged, so the summary computed above still
    // holds. Reuse it rather than recompute.

    // Recipient-side heuristics (repeat-recipient, look-alike, round-amount)
    // don't apply: the SP recipient key is fresh per payment by construction.
    // But the *input-side* warnings do — spending HD coins together still links
    // their labels/categories on-chain regardless of where they're going — so we
    // surface those.
    let warnings: Vec<SendWarning> =
        super::send::compute_input_warnings(&state, &managed, &psbt).await;

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let psbt_b64 = STANDARD.encode(psbt.serialize());

    Ok(Json(SendResult {
        psbt: psbt_b64,
        input_sats,
        recipient_sats,
        change_sats,
        fee_sats,
        warnings,
    }))
}

/// Fee + amount summary for a built SP-send PSBT: `(input, recipient, change,
/// fee)` sats. Recipients are the explicit regular outputs plus the SP
/// placeholder outputs; everything else (including a taproot wallet's own P2TR
/// change) is change. Must be called on the **pre-patch** PSBT, where SP
/// outputs still carry the all-zeros `placeholder` script — that's how we tell
/// them apart from a real-key P2TR change output.
fn summarize_send(
    psbt: &bdk_wallet::bitcoin::Psbt,
    regular_outputs: &[(usize, ScriptBuf, Option<u64>)],
    placeholder: &ScriptBuf,
) -> (u64, u64, u64, u64) {
    let recipient_scripts: std::collections::HashSet<ScriptBuf> = {
        let mut s: std::collections::HashSet<ScriptBuf> = regular_outputs
            .iter()
            .map(|(_, scr, _)| scr.clone())
            .collect();
        for out in psbt.unsigned_tx.output.iter() {
            // Exact placeholder match — NOT "any P2TR", so a taproot source
            // wallet's own P2TR change isn't miscounted as a recipient.
            if &out.script_pubkey == placeholder {
                s.insert(out.script_pubkey.clone());
            }
        }
        s
    };

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

    let (recipient_sats, change_sats) =
        psbt.unsigned_tx
            .output
            .iter()
            .fold((0u64, 0u64), |(rec, chg), out| {
                if recipient_scripts.contains(&out.script_pubkey) {
                    (rec + out.value.to_sat(), chg)
                } else {
                    (rec, chg + out.value.to_sat())
                }
            });

    let fee_sats = input_sats.saturating_sub(recipient_sats + change_sats);
    (input_sats, recipient_sats, change_sats, fee_sats)
}

/// True if the SP address's network prefix matches our configured network.
fn sp_address_matches_network(addr: &SilentPaymentAddress, network: Network) -> bool {
    use silentpayments::Network as SpNet;
    let want = match network {
        Network::Bitcoin => SpNet::Mainnet,
        Network::Regtest => SpNet::Regtest,
        _ => SpNet::Testnet,
    };
    addr.get_network() == want
}
