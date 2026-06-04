use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use corvin_core::{
    types::{InputKind, WalletEntry},
    wallet,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::{background_sync, get_managed, network_from_config};
use crate::api::ApiError;
use crate::state::{get_scripts, AppState, SubCommand, WalletInner};

#[derive(Deserialize)]
pub struct MultisigSignerSpec {
    pub fingerprint: String,
    pub path: String,
    pub xpub: String,
}

#[derive(Deserialize)]
pub struct CreateMultisigRequest {
    pub label: String,
    pub threshold: u32,
    pub signers: Vec<MultisigSignerSpec>,
    #[serde(default)]
    pub backend: Option<String>,
}

pub async fn create_multisig_wallet(
    State(state): State<AppState>,
    Json(req): Json<CreateMultisigRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.threshold < 1 {
        return Err(anyhow::anyhow!("threshold must be at least 1").into());
    }
    if req.signers.len() < 2 {
        return Err(anyhow::anyhow!("multisig requires at least 2 signers").into());
    }
    if req.threshold as usize > req.signers.len() {
        return Err(anyhow::anyhow!("threshold cannot exceed number of signers").into());
    }

    let network = network_from_config(&state).await;

    let signers: Vec<corvin_core::descriptor::MultisigSigner> = req
        .signers
        .iter()
        .map(|s| corvin_core::descriptor::MultisigSigner {
            fingerprint: s.fingerprint.clone(),
            path: s.path.clone(),
            xpub: s.xpub.clone(),
        })
        .collect();

    let parsed =
        corvin_core::descriptor::descriptor_from_multisig(req.threshold, &signers, network)
            .map_err(|e| anyhow::anyhow!("invalid multisig descriptor: {e}"))?;

    let id = Uuid::new_v4();
    let entry = WalletEntry {
        id,
        label: req.label.trim().to_string(),
        input: format!("{}-of-{}", req.threshold, req.signers.len()),
        kind: parsed.kind,
        external_descriptor: parsed.external,
        internal_descriptor: parsed.internal,
        threshold: Some(req.threshold),
        backend: req.backend.clone(),
        created_at: Utc::now(),
    };

    let db_path = crate::config::wallet_db_path(id);
    let hd = wallet::open_or_create_wallet(&entry, network, &db_path)?;
    crate::state::restrict_to_owner(&db_path);
    let inner = WalletInner::Hd(Mutex::new(hd));

    let managed = {
        let mut manager = state.manager.write().await;
        let m = manager.add(entry.clone(), inner);
        crate::state::save_wallets(&manager).await?;
        m
    };

    let scripts = get_scripts(&managed).await;
    let _ = state.sub_tx.send(SubCommand::AddWallet { id, scripts });

    let state2 = state.clone();
    tokio::spawn(async move { background_sync(state2, id).await });

    Ok((StatusCode::CREATED, Json(entry)))
}

#[derive(Deserialize)]
pub struct CombinePsbtRequest {
    pub psbt_a: String,
    pub psbt_b: String,
}

#[derive(Serialize)]
pub struct CombineResult {
    pub psbt: String,
    pub sigs_present: u32,
    pub sigs_required: u32,
    pub ready: bool,
    /// Master fingerprints (8 hex chars) of cosigners that contributed sigs.
    pub signed_fingerprints: Vec<String>,
}

pub async fn combine_psbt(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<CombinePsbtRequest>,
) -> Result<Json<CombineResult>, ApiError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use bdk_wallet::bitcoin::psbt::Psbt;
    use bdk_wallet::bitcoin::secp256k1::Secp256k1;
    use bdk_wallet::miniscript::psbt::PsbtExt;

    let managed = get_managed(&state, &id).await?;
    let sigs_required = managed.entry.threshold.unwrap_or(1);

    let bytes_a = STANDARD
        .decode(&req.psbt_a)
        .map_err(|e| anyhow::anyhow!("invalid PSBT A: {e}"))?;
    let bytes_b = STANDARD
        .decode(&req.psbt_b)
        .map_err(|e| anyhow::anyhow!("invalid PSBT B: {e}"))?;

    let mut psbt_a =
        Psbt::deserialize(&bytes_a).map_err(|e| anyhow::anyhow!("invalid PSBT A: {e}"))?;
    let psbt_b = Psbt::deserialize(&bytes_b).map_err(|e| anyhow::anyhow!("invalid PSBT B: {e}"))?;

    if psbt_a.unsigned_tx.compute_txid() != psbt_b.unsigned_tx.compute_txid() {
        return Err(anyhow::anyhow!("PSBTs refer to different transactions").into());
    }

    psbt_a
        .combine(psbt_b)
        .map_err(|e| anyhow::anyhow!("PSBT combination failed: {e}"))?;

    let sigs_present = psbt_a
        .inputs
        .iter()
        .map(|inp| {
            if inp.final_script_witness.is_some() || inp.final_script_sig.is_some() {
                sigs_required
            } else {
                inp.partial_sigs.len() as u32
            }
        })
        .min()
        .unwrap_or(0);

    let mut signed_fps: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for input in psbt_a.inputs.iter() {
        for pk in input.partial_sigs.keys() {
            if let Some((fp, _)) = input.bip32_derivation.get(&pk.inner) {
                signed_fps.insert(fp.to_string().to_lowercase());
            }
        }
    }
    let signed_fingerprints: Vec<String> = signed_fps.into_iter().collect();

    let secp = Secp256k1::verification_only();
    let mut check = psbt_a.clone();
    let ready = check.finalize_mut(&secp).is_ok();
    if ready {
        psbt_a = check;
    }

    let psbt_b64 = STANDARD.encode(psbt_a.serialize());

    Ok(Json(CombineResult {
        psbt: psbt_b64,
        sigs_present,
        sigs_required,
        ready,
        signed_fingerprints,
    }))
}

#[derive(Serialize)]
pub struct MultisigSignerInfo {
    pub fingerprint: String,
    pub path: String,
    pub xpub: String,
}

#[derive(Serialize)]
pub struct MultisigDetails {
    pub threshold: u32,
    pub signers: Vec<MultisigSignerInfo>,
}

/// Return the multisig wallet as a Coldcard-format setup file.
pub async fn export_multisig_config(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let manager = state.manager.read().await;
    let managed = manager
        .get(&id)
        .ok_or_else(|| anyhow::anyhow!("wallet not found"))?;
    if managed.entry.kind != InputKind::Multisig {
        return Err(anyhow::anyhow!("not a multisig wallet").into());
    }
    let details = parse_wsh_sortedmulti_for_api(&managed.entry.external_descriptor)
        .ok_or_else(|| anyhow::anyhow!("could not parse multisig descriptor"))?;

    let label = managed.label();
    let name = sanitize_coldcard_name(&label);
    let n = details.signers.len();
    let m = details.threshold;

    // Coldcard accepts one or many derivation paths. Emit a single global
    // `Derivation:` when all cosigners share the path; otherwise per-signer.
    let unique_paths: std::collections::BTreeSet<&String> =
        details.signers.iter().map(|s| &s.path).collect();

    let mut out = String::new();
    out.push_str("# Coldcard Multisig setup file (exported by Corvin)\n");
    out.push_str(&format!("Name: {name}\n"));
    out.push_str(&format!("Policy: {m} of {n}\n"));
    if unique_paths.len() == 1 {
        out.push_str(&format!("Derivation: {}\n", details.signers[0].path));
    }
    out.push_str("Format: P2WSH\n\n");

    for s in &details.signers {
        if unique_paths.len() > 1 {
            out.push_str(&format!("# derivation: {}\n", s.path));
        }
        out.push_str(&format!("{}: {}\n", s.fingerprint.to_uppercase(), s.xpub));
    }

    let filename = format!("{}-{}-of-{}-multisig.txt", slugify(&label), m, n,);

    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        out,
    ))
}

/// Coldcard wallet names: printable ASCII + spaces, max 20 chars.
fn sanitize_coldcard_name(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    let limited: String = trimmed.chars().take(20).collect();
    if limited.is_empty() {
        "Corvin Multisig".to_string()
    } else {
        limited
    }
}

fn slugify(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    cleaned.trim_matches('_').to_string()
}

pub async fn get_multisig_info(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<MultisigDetails>, ApiError> {
    let manager = state.manager.read().await;
    let managed = manager
        .get(&id)
        .ok_or_else(|| anyhow::anyhow!("wallet not found"))?;
    if managed.entry.kind != InputKind::Multisig {
        return Err(anyhow::anyhow!("not a multisig wallet").into());
    }
    let parsed = parse_wsh_sortedmulti_for_api(&managed.entry.external_descriptor)
        .ok_or_else(|| anyhow::anyhow!("could not parse multisig descriptor"))?;
    Ok(Json(parsed))
}

fn parse_wsh_sortedmulti_for_api(desc: &str) -> Option<MultisigDetails> {
    // Strip optional BDK/miniscript checksum suffix "#xxxxxxxx".
    let desc = if let Some(hash_pos) = desc.rfind('#') {
        let after = &desc[hash_pos + 1..];
        if after.len() == 8 && after.chars().all(|c| c.is_ascii_alphanumeric()) {
            &desc[..hash_pos]
        } else {
            desc
        }
    } else {
        desc
    };
    let after_open = desc
        .strip_prefix("wsh(sortedmulti(")
        .or_else(|| desc.strip_prefix("wsh(multi("))?;
    let inner = after_open.rsplit_once("))")?.0;
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() < 3 {
        return None;
    }
    let threshold: u32 = parts[0].trim().parse().ok()?;
    let mut signers = Vec::with_capacity(parts.len() - 1);
    for p in &parts[1..] {
        let s = p.trim();
        let open = s.find('[')?;
        let close_rel = s[open..].find(']')?;
        let close = open + close_rel;
        let origin = &s[open + 1..close];
        let slash = origin.find('/')?;
        let fp_hex = origin[..slash].to_lowercase();
        let path_str = &origin[slash + 1..];
        let after = &s[close + 1..];
        let xpub_str = after.trim_end_matches("/0/*").trim_end_matches("/1/*");
        signers.push(MultisigSignerInfo {
            fingerprint: fp_hex,
            path: format!("m/{path_str}"),
            xpub: xpub_str.to_string(),
        });
    }
    Some(MultisigDetails { threshold, signers })
}
