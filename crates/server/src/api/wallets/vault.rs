//! Policy-template wallet creation (miniscript).
//!
//! Two templates, both watch-only `wsh(...)` descriptor wallets (signing via
//! the PSBT/HW/airgap path):
//!   - **Vault**: `wsh(or_d(<primary>, and_v(v:<recovery>, <timelock>)))` —
//!     primary keys (N-of-M) spend anytime; a disjoint recovery group (k-of-j)
//!     spends only after a timelock.
//!   - **Timelocked savings**: `wsh(and_v(v:pk(K), <timelock>))` — a single
//!     key whose funds are locked until the timelock elapses.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::Utc;
use corvin_core::descriptor::{
    descriptor_from_inheritance_vault, descriptor_from_taproot_savings,
    descriptor_from_taproot_vault, descriptor_from_timelocked_savings, MultisigSigner,
    ParsedDescriptor, VaultTimelock,
};
use corvin_core::{types::WalletEntry, wallet};
use serde::Deserialize;
use tokio::sync::Mutex;
use uuid::Uuid;

use super::{background_sync, network_from_config, MultisigSignerSpec};
use crate::api::ApiError;
use crate::state::{get_scripts, AppState, SubCommand, WalletInner};

fn to_signer(s: &MultisigSignerSpec) -> MultisigSigner {
    MultisigSigner {
        fingerprint: s.fingerprint.clone(),
        path: s.path.clone(),
        xpub: s.xpub.clone(),
    }
}

/// Exactly one of `blocks` (relative CSV) or `height` (absolute CLTV).
fn timelock_from(blocks: Option<u32>, height: Option<u32>) -> Result<VaultTimelock, ApiError> {
    match (blocks, height) {
        (Some(b), None) => Ok(VaultTimelock::RelativeBlocks(b)),
        (None, Some(h)) => Ok(VaultTimelock::AbsoluteHeight(h)),
        _ => Err(anyhow::anyhow!(
            "specify exactly one of timelock_blocks (relative) or timelock_height (absolute)"
        )
        .into()),
    }
}

/// Shared tail: create the BDK wallet, register it, subscribe, kick a sync.
async fn finalize_descriptor_wallet(
    state: AppState,
    label: String,
    input: String,
    threshold: Option<u32>,
    parsed: ParsedDescriptor,
    backend: Option<String>,
) -> Result<WalletEntry, ApiError> {
    let network = network_from_config(&state).await;
    let id = Uuid::new_v4();
    let entry = WalletEntry {
        id,
        label: label.trim().to_string(),
        input,
        kind: parsed.kind,
        external_descriptor: parsed.external,
        internal_descriptor: parsed.internal,
        threshold,
        backend,
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
    Ok(entry)
}

#[derive(Deserialize)]
pub struct CreateVaultRequest {
    pub label: String,
    /// Threshold for the primary signers (N of M).
    pub threshold: u32,
    pub primary: Vec<MultisigSignerSpec>,
    /// Threshold for the recovery group (k of j). 1 for a single recovery key.
    pub recovery_threshold: u32,
    pub recovery: Vec<MultisigSignerSpec>,
    #[serde(default)]
    pub timelock_blocks: Option<u32>,
    #[serde(default)]
    pub timelock_height: Option<u32>,
    /// Taproot variant: primary on the key-path (single key only), recovery in
    /// a hidden tapleaf. Defaults to the wsh form.
    #[serde(default)]
    pub taproot: bool,
    #[serde(default)]
    pub backend: Option<String>,
}

pub async fn create_vault_wallet(
    State(state): State<AppState>,
    Json(req): Json<CreateVaultRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if req.primary.is_empty() {
        return Err(anyhow::anyhow!("a vault needs at least one primary signer").into());
    }
    if req.recovery.is_empty() {
        return Err(anyhow::anyhow!("a vault needs at least one recovery key").into());
    }
    let timelock = timelock_from(req.timelock_blocks, req.timelock_height)?;

    let network = network_from_config(&state).await;
    let primary: Vec<MultisigSigner> = req.primary.iter().map(to_signer).collect();
    let recovery: Vec<MultisigSigner> = req.recovery.iter().map(to_signer).collect();

    let parsed = if req.taproot {
        if primary.len() != 1 {
            return Err(anyhow::anyhow!(
                "a taproot vault uses a single key-path key; use the SegWit form for multisig primaries"
            )
            .into());
        }
        descriptor_from_taproot_vault(
            &primary[0],
            req.recovery_threshold,
            &recovery,
            timelock,
            network,
        )
        .map_err(|e| anyhow::anyhow!("invalid taproot vault: {e}"))?
    } else {
        descriptor_from_inheritance_vault(
            req.threshold,
            &primary,
            req.recovery_threshold,
            &recovery,
            timelock,
            network,
        )
        .map_err(|e| anyhow::anyhow!("invalid vault: {e}"))?
    };

    let recovery_desc = if req.recovery.len() == 1 {
        "timelocked recovery key".to_string()
    } else {
        format!(
            "{}-of-{} timelocked recovery",
            req.recovery_threshold,
            req.recovery.len()
        )
    };
    let input = format!(
        "{}-of-{} vault + {recovery_desc}",
        req.threshold,
        req.primary.len()
    );

    let entry = finalize_descriptor_wallet(
        state,
        req.label,
        input,
        Some(req.threshold),
        parsed,
        req.backend,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(entry)))
}

#[derive(Deserialize)]
pub struct CreateTimelockedRequest {
    pub label: String,
    pub signer: MultisigSignerSpec,
    #[serde(default)]
    pub timelock_blocks: Option<u32>,
    #[serde(default)]
    pub timelock_height: Option<u32>,
    /// Taproot variant: NUMS internal key, timelocked tapleaf (hidden until
    /// spent). Defaults to the wsh form.
    #[serde(default)]
    pub taproot: bool,
    #[serde(default)]
    pub backend: Option<String>,
}

pub async fn create_timelocked_wallet(
    State(state): State<AppState>,
    Json(req): Json<CreateTimelockedRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let timelock = timelock_from(req.timelock_blocks, req.timelock_height)?;
    let network = network_from_config(&state).await;
    let signer = to_signer(&req.signer);

    let parsed = if req.taproot {
        descriptor_from_taproot_savings(&signer, timelock, network)
            .map_err(|e| anyhow::anyhow!("invalid taproot timelocked-savings policy: {e}"))?
    } else {
        descriptor_from_timelocked_savings(&signer, timelock, network)
            .map_err(|e| anyhow::anyhow!("invalid timelocked-savings policy: {e}"))?
    };

    let input = match (req.timelock_blocks, req.timelock_height) {
        (Some(b), _) => format!("timelocked savings (after {b} blocks)"),
        (_, Some(h)) => format!("timelocked savings (at height {h})"),
        _ => "timelocked savings".to_string(),
    };

    let entry =
        finalize_descriptor_wallet(state, req.label, input, None, parsed, req.backend).await?;
    Ok((StatusCode::CREATED, Json(entry)))
}
