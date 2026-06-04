use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::{get_managed, network_from_config};
use crate::api::ApiError;
use crate::state::{AppState, WalletInner};

/// Validate a user-supplied fee rate and round up to whole sat/vB. Rejects
/// non-finite (NaN/Infinity) and absurd values up front so we return a clean
/// error instead of relying on a downstream overflow (Infinity → u64::MAX).
/// 10_000 sat/vB is already ~100x any real network spike.
pub(crate) fn validated_fee_rate_sats(fee_rate_sat_vb: f64) -> Result<u64, ApiError> {
    if !fee_rate_sat_vb.is_finite() || fee_rate_sat_vb > 10_000.0 {
        return Err(ApiError::bad_request("fee rate must be between 1 and 10000 sat/vB"));
    }
    Ok(fee_rate_sat_vb.max(1.0).ceil() as u64)
}

#[derive(Debug, Deserialize)]
pub struct OutputSpec {
    pub recipient: String,
    /// Amount to send to this recipient in satoshis. `None` marks this
    /// recipient as the "drain" target — they receive everything left over
    /// after fees and the other recipients' fixed amounts. At most one
    /// `OutputSpec` per request may have `amount_sats = None`.
    pub amount_sats: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct SendRequest {
    pub outputs: Vec<OutputSpec>,
    pub fee_rate_sat_vb: f64,
    /// Coin control: specific outpoints ("txid:vout") to use. If absent, auto-select.
    pub utxos: Option<Vec<String>>,
    /// For policy/vault wallets with more than one spending branch: which to
    /// satisfy. "recovery" selects the timelocked branch; anything else (or
    /// absent) uses the primary branch. Ignored for single-path wallets.
    #[serde(default)]
    pub spend_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SendResult {
    pub psbt: String,
    pub input_sats: u64,
    /// Sum of all recipient outputs (excluding change).
    pub recipient_sats: u64,
    pub change_sats: u64,
    pub fee_sats: u64,
    /// Privacy / leak heuristics computed against the *built* PSBT — i.e. the
    /// inputs BDK actually selected and the outputs we'd broadcast. Empty for
    /// a clean send. The frontend renders them in the preview card; they're
    /// all non-blocking (advisory).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<SendWarning>,
}

#[derive(Debug, Serialize)]
pub struct SendWarning {
    /// Machine-readable identifier so the frontend can localize / restyle.
    pub code: SendWarningCode,
    pub severity: WarningSeverity,
    /// One-sentence human-readable summary.
    pub message: String,
    /// Optional extra detail — e.g. the list of labels involved, the date of
    /// the prior payment. Rendered as a secondary line under the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SendWarningCode {
    /// Inputs being spent together carry different user-supplied labels, which
    /// means the labels' identities will be publicly linked on-chain.
    MixedLabels,
    /// Inputs being spent together belong to different coin categories (privacy
    /// compartments), merging those compartments on-chain.
    MixedCategories,
    /// A recipient address shares the same visible prefix + suffix as a different
    /// address in the wallet's history — a hallmark of address poisoning.
    LookAlike,
    /// The recipient address has already received from this wallet at least
    /// once. The recipient can connect this payment to prior payments.
    RepeatRecipient,
    /// Payment amount is round (whole BTC / whole 100k sats / etc.) while a
    /// change output is present. Chain analysis trivially identifies the
    /// non-round output as change.
    RoundAmountReveal,
    /// The change output's script type differs from the recipient's, so chain
    /// analysis can pick out the change (it's the output matching the wallet's
    /// own input type). Informational — the change type follows the wallet.
    ChangeScriptMismatch,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningSeverity {
    Info,
    Warning,
}

pub async fn build_send_psbt(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<SendRequest>,
) -> Result<Json<SendResult>, ApiError> {
    if req.outputs.is_empty() {
        return Err(ApiError::bad_request("at least one output is required"));
    }
    let drain_count = req
        .outputs
        .iter()
        .filter(|o| o.amount_sats.is_none())
        .count();
    if drain_count > 1 {
        return Err(ApiError::bad_request("only one output may be marked send-max (drain)"));
    }
    for (i, o) in req.outputs.iter().enumerate() {
        if let Some(amt) = o.amount_sats {
            if amt == 0 {
                return Err(ApiError::bad_request(format!("output #{} amount must be > 0", i + 1)));
            }
        }
    }

    let network = network_from_config(&state).await;
    let managed = get_managed(&state, &id).await?;

    let WalletInner::Hd(wm) = &managed.inner else {
        return Err(anyhow::anyhow!("send requires an HD wallet").into());
    };

    use bdk_wallet::bitcoin::{Address, Amount, FeeRate, OutPoint, ScriptBuf, Txid};
    use bdk_wallet::KeychainKind;
    use corvin_core::types::InputKind;
    use std::collections::BTreeMap;
    use std::str::FromStr;

    // Timelock gate: when the chosen spend path is timelocked, only matured
    // UTXOs are eligible. A vault's primary path (branch 0) is never gated; the
    // recovery branch (1) and single-path timelocked savings are. `None` = no
    // gate (normal wallet, or vault primary). Computed from the descriptor's
    // policy — no wallet lock needed.
    let spend_branch = if req.spend_path.as_deref() == Some("recovery") {
        1usize
    } else {
        0
    };
    let timelock_gate: Option<corvin_core::descriptor::PolicyTimelock> =
        if managed.entry.kind == InputKind::Descriptor {
            match corvin_core::descriptor::describe_policy(&managed.entry.external_descriptor) {
                Ok(s) if !s.timelocks.is_empty() => {
                    // Vault: gate only the recovery branch. Savings: always gated.
                    let gated = if s.requires_path {
                        spend_branch == 1
                    } else {
                        true
                    };
                    gated.then(|| s.timelocks.into_iter().next()).flatten()
                }
                _ => None,
            }
        } else {
            None
        };

    let parsed_outputs: Vec<(ScriptBuf, Option<u64>)> = req
        .outputs
        .iter()
        .enumerate()
        .map(|(i, o)| {
            // Silent Payments use a different build path (`/sp-send`) that
            // requires the sender's input private keys at build time. The
            // frontend should route sp1q… recipients there; this is a
            // defensive guard if something slips through.
            let lower = o.recipient.to_lowercase();
            if lower.starts_with("sp1")
                || lower.starts_with("tsp1")
                || lower.starts_with("sprt1")
            {
                return Err(anyhow::anyhow!(
                    "output #{}: Silent Payments addresses use a different send flow — call /sp-send instead.",
                    i + 1
                ));
            }
            let addr = Address::from_str(&o.recipient)
                .map_err(|e| anyhow::anyhow!("output #{}: invalid address: {e}", i + 1))?
                .require_network(network)
                .map_err(|_| anyhow::anyhow!("output #{}: address is for the wrong network", i + 1))?;
            Ok::<_, anyhow::Error>((addr.script_pubkey(), o.amount_sats))
        })
        .collect::<anyhow::Result<_>>()?;
    let recipient_scripts: std::collections::HashSet<ScriptBuf> =
        parsed_outputs.iter().map(|(s, _)| s.clone()).collect();

    let fee_rate_sats = validated_fee_rate_sats(req.fee_rate_sat_vb)?;
    let fee_rate = FeeRate::from_sat_per_vb(fee_rate_sats)
        .ok_or_else(|| anyhow::anyhow!("invalid fee rate"))?;

    let frozen_set = state.annotations.frozen_utxos().await;

    let coin_control: Option<Vec<OutPoint>> = req
        .utxos
        .as_ref()
        .map(|strs| {
            strs.iter()
                .map(|s| {
                    let (txid_s, vout_s) = s
                        .split_once(':')
                        .ok_or_else(|| anyhow::anyhow!("malformed outpoint: {s}"))?;
                    let txid = Txid::from_str(txid_s)
                        .map_err(|_| anyhow::anyhow!("invalid txid: {txid_s}"))?;
                    let vout: u32 = vout_s
                        .parse()
                        .map_err(|_| anyhow::anyhow!("invalid vout: {vout_s}"))?;
                    Ok(OutPoint { txid, vout })
                })
                .collect::<anyhow::Result<_>>()
        })
        .transpose()?;

    // Auto-select: exclude frozen / immature / timelocked coins, then let BDK
    // run its normal coin selection over the rest. We collect the *unspendable*
    // set (not the spendable one) and hand it to `builder.unspendable()` — using
    // `add_utxo` + `manually_selected_only` for the whole eligible set would
    // force-spend the entire wallet on every send (a privacy + fee disaster).
    // Done before the mutable builder borrow so `list_unspent` and `build_tx`
    // don't overlap.
    let auto_unspendable: Option<Vec<OutPoint>> = if coin_control.is_none() {
        let hd = wm.lock().await;
        let tip = hd.wallet.latest_checkpoint().height();
        let coinbase_txids = super::coinbase_txids(&hd.wallet);
        let mut unspendable: Vec<OutPoint> = Vec::new();
        let mut spendable_count: usize = 0;
        // Soonest unlock (in blocks) among UTXOs excluded by a relative timelock,
        // for a precise "spendable in ~N blocks" message.
        let mut soonest_unlock_blocks: Option<u32> = None;
        let mut any_timelocked = false;
        for u in hd.wallet.list_unspent() {
            let key = format!("{}:{}", u.outpoint.txid, u.outpoint.vout);
            if frozen_set.contains(&key) {
                unspendable.push(u.outpoint);
                continue;
            }
            let confs = super::confs_at(&u.chain_position, tip);
            if coinbase_txids.contains(&u.outpoint.txid) && confs < 100 {
                unspendable.push(u.outpoint);
                continue;
            }
            if let Some(tl) = &timelock_gate {
                if corvin_core::descriptor::timelock_spendable(tl, confs, tip) == Some(false) {
                    any_timelocked = true;
                    if tl.kind == "relative" && tl.blocks {
                        let remaining = tl.value.saturating_sub(confs);
                        soonest_unlock_blocks =
                            Some(soonest_unlock_blocks.map_or(remaining, |m| m.min(remaining)));
                    }
                    unspendable.push(u.outpoint);
                    continue;
                }
            }
            spendable_count += 1;
        }
        if spendable_count == 0 {
            if any_timelocked {
                let tl = timelock_gate
                    .as_ref()
                    .expect("any_timelocked is only set when timelock_gate is Some");
                let msg = match (tl.kind.as_str(), tl.blocks) {
                    ("relative", true) => match soonest_unlock_blocks {
                        Some(rem) => format!("These coins are timelocked — the earliest unlocks in ~{rem} more block(s). Wait for confirmations."),
                        None => format!("These coins are timelocked ({}).", tl.label),
                    },
                    ("absolute", true) => format!(
                        "These coins are timelocked until block {} (chain tip is {tip}).",
                        tl.value
                    ),
                    _ => format!("These coins are timelocked ({}).", tl.label),
                };
                return Err(anyhow::anyhow!(msg).into());
            }
            return Err(anyhow::anyhow!(
                "No spendable UTXOs — sync your wallet, or unfreeze UTXOs if all are frozen. \
                 (Immature mining rewards are excluded until they have 100 confirmations.)"
            )
            .into());
        }
        Some(unspendable)
    } else {
        None
    };

    // Coin-control: reject explicitly-picked outpoints that are immature
    // coinbase. BDK's own error here is less helpful.
    if let Some(ops) = &coin_control {
        let hd = wm.lock().await;
        let tip = hd.wallet.latest_checkpoint().height();
        let coinbase_txids = super::coinbase_txids(&hd.wallet);
        let utxo_by_op: std::collections::HashMap<_, _> = hd
            .wallet
            .list_unspent()
            .map(|u| (u.outpoint, u.chain_position))
            .collect();
        for op in ops {
            // Timelock gate applies to every explicitly-picked UTXO.
            if let Some(tl) = &timelock_gate {
                let confs = utxo_by_op
                    .get(op)
                    .map(|cp| super::confs_at(cp, tip))
                    .unwrap_or(0);
                if corvin_core::descriptor::timelock_spendable(tl, confs, tip) == Some(false) {
                    return Err(anyhow::anyhow!(
                        "UTXO {}:{} is timelocked ({}) — not spendable yet on this path.",
                        op.txid,
                        op.vout,
                        tl.label
                    )
                    .into());
                }
            }
            if !coinbase_txids.contains(&op.txid) {
                continue;
            }
            let confs = utxo_by_op
                .get(op)
                .map(|cp| super::confs_at(cp, tip))
                .unwrap_or(0);
            if confs < 100 {
                return Err(anyhow::anyhow!(
                    "UTXO {}:{} is an immature coinbase output ({} of 100 confirmations). \
                     Wait until it matures before spending it.",
                    op.txid,
                    op.vout,
                    confs
                )
                .into());
            }
        }
    }

    let psbt = {
        let mut hd = wm.lock().await;
        let wallet = &mut hd.wallet;

        // Policy/vault wallets expose more than one spending branch; BDK needs
        // to be told which to satisfy or finish() errors SpendingPolicyRequired.
        // Default to branch 0 (primary, spendable now); "recovery" picks the
        // timelocked branch. Computed before the build_tx mutable borrow.
        let branch: usize = if req.spend_path.as_deref() == Some("recovery") {
            1
        } else {
            0
        };
        let mut path_selections: Vec<(KeychainKind, String)> = Vec::new();
        for keychain in [KeychainKind::External, KeychainKind::Internal] {
            if let Ok(Some(policy)) = wallet.policies(keychain) {
                if policy.requires_path() {
                    path_selections.push((keychain, policy.id.clone()));
                }
            }
        }
        // Anchor locktime to the chain tip for policy wallets so their
        // absolute/relative timelocks resolve; normal wallets keep BDK's
        // default tip-based anti-fee-sniping locktime.
        let policy_tip = (managed.entry.kind == InputKind::Descriptor)
            .then(|| wallet.latest_checkpoint().height());

        let mut builder = wallet.build_tx();
        builder.fee_rate(fee_rate);
        for (keychain, id) in &path_selections {
            builder.policy_path(BTreeMap::from([(id.clone(), vec![branch])]), *keychain);
        }
        if let Some(h) = policy_tip {
            builder.current_height(h);
        }

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
                // Exclude frozen / immature / timelocked coins and let BDK run
                // its own coin selection over the rest — do NOT force-spend the
                // whole wallet.
                builder.unspendable(
                    auto_unspendable
                        .clone()
                        .expect("auto_unspendable is Some when coin_control is None"),
                );
            }
        }

        for (script, amount) in &parsed_outputs {
            if let Some(amt) = amount {
                builder.add_recipient(script.clone(), Amount::from_sat(*amt));
            }
        }
        if let Some((drain_script, _)) = parsed_outputs.iter().find(|(_, a)| a.is_none()) {
            builder.drain_to(drain_script.clone());
            // Send-max: in auto mode we must spend the whole spendable balance,
            // so explicitly drain the wallet (this still respects `unspendable`).
            // In coin-control mode `manually_selected_only` already forces every
            // picked coin in, so `drain_to` alone is enough there.
            if coin_control.is_none() {
                builder.drain_wallet();
            }
        }

        builder.finish().map_err(|e| {
            // Classify the common "not enough coins" case so the UI can react to
            // it specifically; everything else stays a generic build error.
            if matches!(e, bdk_wallet::error::CreateTxError::CoinSelection(_)) {
                ApiError::insufficient_funds(format!("not enough funds to cover the amount + fee: {e}"))
            } else {
                ApiError::from(anyhow::anyhow!("failed to build transaction: {e}"))
            }
        })?
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

    let warnings =
        compute_send_warnings(&state, &managed, &psbt, &recipient_scripts, change_sats).await;

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

/// Two addresses are "look-alikes" if they share their first 6 and last 6
/// characters but differ overall — the visible-collision attackers grind for.
fn looks_alike(a: &str, b: &str) -> bool {
    a != b
        && a.len() >= 12
        && b.len() >= 12
        && a.as_bytes()[..6] == b.as_bytes()[..6]
        && a.as_bytes()[a.len() - 6..] == b.as_bytes()[b.len() - 6..]
}

/// `bc1q…wxyz` style abbreviation for warning detail text.
fn abbrev_addr(s: &str) -> String {
    if s.len() > 18 {
        format!("{}…{}", &s[..8], &s[s.len() - 8..])
    } else {
        s.to_string()
    }
}

/// Classify a scriptPubKey into a coarse address-type label for the
/// change-script-mismatch heuristic.
fn script_kind(s: &bdk_wallet::bitcoin::Script) -> &'static str {
    if s.is_p2tr() {
        "p2tr"
    } else if s.is_p2wpkh() {
        "p2wpkh"
    } else if s.is_p2wsh() {
        "p2wsh"
    } else if s.is_p2sh() {
        "p2sh"
    } else if s.is_p2pkh() {
        "p2pkh"
    } else {
        "other"
    }
}

/// Input-side privacy advisories — mixed labels + mixed categories. These depend
/// only on the coins being spent (not the recipients), so the regular send and
/// the SP-send path (which also spends HD inputs) both reuse them.
pub(crate) async fn compute_input_warnings(
    state: &AppState,
    managed: &Arc<crate::state::ManagedWallet>,
    psbt: &bdk_wallet::bitcoin::psbt::Psbt,
) -> Vec<SendWarning> {
    use bdk_wallet::bitcoin::{Address, Network};

    let mut warnings = Vec::new();
    let network: Network = state.config.read().await.network.kind.to_bitcoin_network();

    // Mixed labels: two+ distinct user labels among inputs publicly link
    // those identities on-chain. Unlabeled inputs don't count.
    {
        let utxo_labels = state.annotations.utxo_labels().await;
        let mut labels_seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for input in &psbt.unsigned_tx.input {
            let key = format!(
                "{}:{}",
                input.previous_output.txid, input.previous_output.vout
            );
            if let Some(label) = utxo_labels.get(&key) {
                let trimmed = label.trim();
                if !trimmed.is_empty() {
                    labels_seen.insert(trimmed.to_string());
                }
            }
        }
        if labels_seen.len() >= 2 {
            let labels: Vec<String> = labels_seen.iter().take(4).cloned().collect();
            let mut detail = labels
                .iter()
                .map(|l| format!("\"{l}\""))
                .collect::<Vec<_>>()
                .join(", ");
            if labels_seen.len() > 4 {
                detail.push_str(&format!(", +{}", labels_seen.len() - 4));
            }
            warnings.push(SendWarning {
                code: SendWarningCode::MixedLabels,
                severity: WarningSeverity::Warning,
                message: format!(
                    "Spending {} differently-labeled inputs together publicly links those identities on-chain.",
                    labels_seen.len()
                ),
                detail: Some(format!("Labels combined: {detail}")),
            });
        }
    }

    // Mixed categories: inputs from 2+ coin categories (privacy compartments)
    // merge those compartments on-chain. Effective category of an input is its
    // own UTXO override, else its receiving address's category. Needs the HD
    // wallet to resolve the receiving address.
    if let crate::state::WalletInner::Hd(wm) = &managed.inner {
        let hd = wm.lock().await;
        let cats = state.annotations.categories().await;
        if !cats.definitions.is_empty() {
            let mut ids_seen: std::collections::BTreeSet<String> =
                std::collections::BTreeSet::new();
            for input in &psbt.unsigned_tx.input {
                let op = input.previous_output;
                let key = format!("{}:{}", op.txid, op.vout);
                let id = cats.utxos.get(&key).cloned().or_else(|| {
                    hd.wallet.get_utxo(op).and_then(|u| {
                        Address::from_script(&u.txout.script_pubkey, network)
                            .ok()
                            .and_then(|a| cats.addresses.get(&a.to_string()).cloned())
                    })
                });
                if let Some(id) = id {
                    ids_seen.insert(id);
                }
            }
            if ids_seen.len() >= 2 {
                let names: Vec<String> = ids_seen
                    .iter()
                    .filter_map(|id| {
                        cats.definitions.iter().find(|d| &d.id == id).map(|d| d.name.clone())
                    })
                    .collect();
                warnings.push(SendWarning {
                    code: SendWarningCode::MixedCategories,
                    severity: WarningSeverity::Warning,
                    message: format!(
                        "Spending coins from {} different categories together merges those privacy compartments on-chain.",
                        ids_seen.len()
                    ),
                    detail: Some(format!("Categories combined: {}", names.join(", "))),
                });
            }
        }
    }

    warnings
}

/// Non-blocking privacy advisories for the built send PSBT. One warning per
/// detected concern; broadcasting isn't gated.
async fn compute_send_warnings(
    state: &AppState,
    managed: &Arc<crate::state::ManagedWallet>,
    psbt: &bdk_wallet::bitcoin::psbt::Psbt,
    recipient_scripts: &std::collections::HashSet<bdk_wallet::bitcoin::ScriptBuf>,
    change_sats: u64,
) -> Vec<SendWarning> {
    use bdk_wallet::bitcoin::{Address, Network};

    // Input-side warnings (mixed labels + categories) are shared with SP-send.
    let mut warnings = compute_input_warnings(state, managed, psbt).await;
    let network: Network = state.config.read().await.network.kind.to_bitcoin_network();

    // Recipient-side heuristics need the wallet's history.
    {
        let crate::state::WalletInner::Hd(wm) = &managed.inner else {
            return warnings;
        };
        let hd = wm.lock().await;

        // One pass over the wallet's history feeds both heuristics below: the
        // per-recipient prior-output counts (repeat-recipient) and the set of
        // every address ever seen (look-alike). Avoids scanning the full history
        // twice on each send preview.
        let mut script_counts: std::collections::HashMap<bdk_wallet::bitcoin::ScriptBuf, u32> =
            std::collections::HashMap::new();
        let mut known: std::collections::HashSet<String> = std::collections::HashSet::new();
        for ctx in hd.wallet.transactions() {
            for out in &ctx.tx_node.tx.output {
                if recipient_scripts.contains(&out.script_pubkey) {
                    *script_counts.entry(out.script_pubkey.clone()).or_insert(0) += 1;
                }
                if let Ok(a) = Address::from_script(&out.script_pubkey, network) {
                    known.insert(a.to_string());
                }
            }
        }

        let mut repeats: Vec<(Address, u32)> = Vec::new();
        for recipient_script in recipient_scripts {
            let count = script_counts.get(recipient_script).copied().unwrap_or(0);
            if count > 0 {
                if let Ok(addr) = Address::from_script(recipient_script, network) {
                    repeats.push((addr, count));
                }
            }
        }
        if !repeats.is_empty() {
            let summary = repeats
                .iter()
                .map(|(a, n)| {
                    let s = a.to_string();
                    let abbrev = if s.len() > 18 {
                        format!("{}…{}", &s[..8], &s[s.len() - 8..])
                    } else {
                        s
                    };
                    if *n == 1 {
                        format!("{abbrev} (1 prior)")
                    } else {
                        format!("{abbrev} ({n} prior)")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            warnings.push(SendWarning {
                code: SendWarningCode::RepeatRecipient,
                severity: WarningSeverity::Warning,
                message: "This wallet has paid one of these recipients before — they can chain-analyze this payment together with the earlier ones.".into(),
                detail: Some(summary),
            });
        }

        // Look-alike (address poisoning): a recipient that shares the same first
        // 6 and last 6 characters as a *different* address from the wallet's
        // history is the signature of a ground vanity address used to trick a
        // copy-paste. Strong signal — the suffix collision alone is ~1-in-a-billion
        // by chance, so this almost never fires unless someone ground it.
        {
            let recipient_addrs: Vec<String> = recipient_scripts
                .iter()
                .filter_map(|s| Address::from_script(s, network).ok().map(|a| a.to_string()))
                .collect();
            // `known` was built in the single history pass above.
            let mut hits: Vec<(String, String)> = Vec::new();
            for r in &recipient_addrs {
                if let Some(k) = known.iter().find(|k| looks_alike(r, k)) {
                    hits.push((r.clone(), k.clone()));
                }
            }
            if !hits.is_empty() {
                let detail = hits
                    .iter()
                    .map(|(r, k)| format!("{} resembles {}", abbrev_addr(r), abbrev_addr(k)))
                    .collect::<Vec<_>>()
                    .join("; ");
                warnings.push(SendWarning {
                    code: SendWarningCode::LookAlike,
                    severity: WarningSeverity::Warning,
                    message: "A recipient looks almost identical to a different address in your history — same start and end, different middle. This is how address-poisoning attacks steal funds. Verify every character before sending.".into(),
                    detail: Some(detail),
                });
            }
        }
    }

    // Round-amount reveal: a round payment + non-round change exposes which
    // output is change. Only fires when change exists; threshold is a multiple
    // of 100k sats (0.001 BTC).
    if change_sats > 0 {
        let round_paid_outputs: Vec<u64> = psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|o| recipient_scripts.contains(&o.script_pubkey))
            .map(|o| o.value.to_sat())
            .filter(|v| *v >= 100_000 && *v % 100_000 == 0)
            .collect();
        if !round_paid_outputs.is_empty() {
            warnings.push(SendWarning {
                code: SendWarningCode::RoundAmountReveal,
                severity: WarningSeverity::Info,
                message: "Round payment amount makes it trivial to spot which output is change — adding even a few sats of noise hides which is which.".into(),
                detail: None,
            });
        }
    }

    // Change-script mismatch: when the change output's script type differs from
    // the recipient's, chain analysis can pick the change out (it's the output
    // whose type matches the wallet's own inputs). Informational — the change
    // type follows the wallet's descriptor, so the user can't change it per-send.
    if change_sats > 0 {
        let recipient_kinds: std::collections::HashSet<&'static str> = psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|o| recipient_scripts.contains(&o.script_pubkey))
            .map(|o| script_kind(&o.script_pubkey))
            .collect();
        let mismatched = psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|o| !recipient_scripts.contains(&o.script_pubkey))
            .map(|o| script_kind(&o.script_pubkey))
            .find(|k| !recipient_kinds.contains(k));
        if let (false, Some(change_kind)) = (recipient_kinds.is_empty(), mismatched) {
            let recipient_kind = recipient_kinds.iter().next().copied().unwrap_or("the recipient's");
            warnings.push(SendWarning {
                code: SendWarningCode::ChangeScriptMismatch,
                severity: WarningSeverity::Info,
                message: "Your change is a different address type than the recipient, which makes it easy to tell which output is change. Paying a recipient of the same address type avoids this.".into(),
                detail: Some(format!("change is {change_kind}, recipient is {recipient_kind}")),
            });
        }
    }

    warnings
}

#[derive(Debug, Deserialize)]
pub struct FeeBumpRequest {
    pub txid: String,
    pub fee_rate_sat_vb: f64,
}

#[derive(Debug, Serialize)]
pub struct FeeBumpResult {
    pub psbt: String,
    pub fee_sats: u64,
}

fn sum_psbt_inputs(psbt: &bdk_wallet::bitcoin::psbt::Psbt) -> u64 {
    psbt.inputs
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
        .sum()
}

pub async fn build_rbf_psbt(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<FeeBumpRequest>,
) -> Result<Json<FeeBumpResult>, ApiError> {
    let managed = get_managed(&state, &id).await?;
    let WalletInner::Hd(wm) = &managed.inner else {
        return Err(anyhow::anyhow!("fee bump requires an HD wallet").into());
    };

    use bdk_wallet::bitcoin::{FeeRate, Txid};
    use std::str::FromStr;

    let target_txid =
        Txid::from_str(&req.txid).map_err(|e| anyhow::anyhow!("invalid txid: {e}"))?;
    let fee_rate_sats = validated_fee_rate_sats(req.fee_rate_sat_vb)?;
    let fee_rate = FeeRate::from_sat_per_vb(fee_rate_sats)
        .ok_or_else(|| anyhow::anyhow!("invalid fee rate"))?;

    let psbt = {
        let mut hd = wm.lock().await;
        let wallet = &mut hd.wallet;

        let mut builder = wallet
            .build_fee_bump(target_txid)
            .map_err(|e| anyhow::anyhow!("cannot bump fee: {e}"))?;
        builder.fee_rate(fee_rate);

        builder
            .finish()
            .map_err(|e| anyhow::anyhow!("failed to build RBF transaction: {e}"))?
    };

    let input_sats = sum_psbt_inputs(&psbt);
    let output_sats: u64 = psbt
        .unsigned_tx
        .output
        .iter()
        .map(|o| o.value.to_sat())
        .sum();
    let fee_sats = input_sats.saturating_sub(output_sats);

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let psbt_b64 = STANDARD.encode(psbt.serialize());

    Ok(Json(FeeBumpResult {
        psbt: psbt_b64,
        fee_sats,
    }))
}

pub async fn build_cpfp_psbt(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<FeeBumpRequest>,
) -> Result<Json<FeeBumpResult>, ApiError> {
    let managed = get_managed(&state, &id).await?;
    let WalletInner::Hd(wm) = &managed.inner else {
        return Err(anyhow::anyhow!("fee bump requires an HD wallet").into());
    };

    use bdk_wallet::bitcoin::{Amount, FeeRate, OutPoint, Txid};
    use bdk_wallet::KeychainKind;
    use std::str::FromStr;

    let target_txid =
        Txid::from_str(&req.txid).map_err(|e| anyhow::anyhow!("invalid txid: {e}"))?;
    let fee_rate_sats = validated_fee_rate_sats(req.fee_rate_sat_vb)?;
    let fee_rate = FeeRate::from_sat_per_vb(fee_rate_sats)
        .ok_or_else(|| anyhow::anyhow!("invalid fee rate"))?;

    let psbt = {
        let mut hd = wm.lock().await;

        let child_utxos: Vec<OutPoint> = hd
            .wallet
            .list_unspent()
            .filter(|u| u.outpoint.txid == target_txid)
            .map(|u| u.outpoint)
            .collect();

        if child_utxos.is_empty() {
            return Err(anyhow::anyhow!(
                "no spendable outputs from that transaction found in this wallet"
            )
            .into());
        }

        // CPFP must hit the requested rate for the *package* (parent + child),
        // not just the child — otherwise the parent's low fee drags the combined
        // rate below target. Need the parent's vsize + fee to size the child.
        // The parent is a signed, broadcast (unconfirmed) tx the wallet knows
        // (we just took its outputs). `calculate_fee` can fail when the wallet
        // doesn't know all the parent's inputs (a received tx) — fall back to a
        // conservative 0 (child then covers the whole package; overpays slightly
        // rather than undershooting the target).
        let (parent_vsize, parent_fee) = match hd.wallet.get_tx(target_txid) {
            Some(c) => {
                let tx = c.tx_node.tx.clone();
                let vsize = tx.vsize() as u64;
                let fee = hd
                    .wallet
                    .calculate_fee(&tx)
                    .map(|f| f.to_sat())
                    .unwrap_or(0);
                (vsize, fee)
            }
            None => (0, 0),
        };

        let change_spk = hd
            .wallet
            .reveal_next_address(KeychainKind::Internal)
            .address
            .script_pubkey();

        // Probe build at the target rate to learn the child's BDK-estimated fee
        // (= rate × child vsize, witnesses included). The child's structure is
        // fixed (these inputs → one drain output), so its vsize doesn't depend
        // on the final absolute fee.
        let probe_fee = {
            let mut b = hd.wallet.build_tx();
            b.fee_rate(fee_rate);
            for op in &child_utxos {
                b.add_utxo(*op)
                    .map_err(|e| anyhow::anyhow!("UTXO not available: {e}"))?;
            }
            b.manually_selected_only();
            b.drain_to(change_spk.clone());
            let probe = b
                .finish()
                .map_err(|e| anyhow::anyhow!("failed to build CPFP transaction: {e}"))?;
            let out: u64 = probe
                .unsigned_tx
                .output
                .iter()
                .map(|o| o.value.to_sat())
                .sum();
            sum_psbt_inputs(&probe).saturating_sub(out)
        };

        // Child fee = its own rate-fee + the parent's shortfall at the target
        // rate. If the parent already paid ≥ target, the deficit is 0 and the
        // child just pays its own rate.
        let parent_deficit = (fee_rate_sats * parent_vsize).saturating_sub(parent_fee);
        let target_fee = probe_fee + parent_deficit;

        let mut builder = hd.wallet.build_tx();
        builder.fee_absolute(Amount::from_sat(target_fee));
        for op in &child_utxos {
            builder
                .add_utxo(*op)
                .map_err(|e| anyhow::anyhow!("UTXO not available: {e}"))?;
        }
        builder.manually_selected_only();
        builder.drain_to(change_spk);
        let psbt = builder
            .finish()
            .map_err(|e| anyhow::anyhow!(
                "failed to build CPFP transaction (the output may be too small to bump the parent to {fee_rate_sats} sat/vB): {e}"
            ))?;
        hd.persist_staged()?;
        psbt
    };

    let input_sats = sum_psbt_inputs(&psbt);
    let output_sats: u64 = psbt
        .unsigned_tx
        .output
        .iter()
        .map(|o| o.value.to_sat())
        .sum();
    let fee_sats = input_sats.saturating_sub(output_sats);

    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let psbt_b64 = STANDARD.encode(psbt.serialize());

    Ok(Json(FeeBumpResult {
        psbt: psbt_b64,
        fee_sats,
    }))
}

#[cfg(test)]
mod tests {
    use super::{abbrev_addr, looks_alike};

    #[test]
    fn looks_alike_flags_shared_ends_only() {
        // Same first-6 AND last-6, different middle → the address-poisoning shape.
        assert!(looks_alike("bc1qAB1111UVWXYZ", "bc1qAB2222UVWXYZ"));
        // Identical strings are not look-alikes (it's the same address).
        assert!(!looks_alike("bc1qAB1111UVWXYZ", "bc1qAB1111UVWXYZ"));
        // Differing suffix → not flagged (attackers grind the suffix).
        assert!(!looks_alike("bc1qAB1111UVWXYZ", "bc1qAB2222UVWXYQ"));
        // Differing prefix → not flagged.
        assert!(!looks_alike("bc1qXX1111UVWXYZ", "bc1qAB1111UVWXYZ"));
    }

    #[test]
    fn looks_alike_ignores_too_short() {
        // Under 12 chars there isn't a distinct 6-prefix + 6-suffix to compare.
        assert!(!looks_alike("abc", "abd"));
        assert!(!looks_alike("short1", "short2"));
    }

    #[test]
    fn abbrev_addr_shortens_only_long_strings() {
        let long = "bc1qsomethingreallylonghere0000";
        let a = abbrev_addr(long);
        assert!(a.contains('…'));
        assert!(a.starts_with("bc1qsome"));
        assert!(a.ends_with("here0000"));
        // Short strings (<= 18 chars) pass through untouched.
        assert_eq!(abbrev_addr("bc1qshort"), "bc1qshort");
    }
}
