//! Backup-recovery testing.
//!
//! Lets the user verify their *written-down* mnemonic actually reproduces the
//! wallet they intend to back up, without modifying any state. The single
//! most common cause of lost funds in self-custody is "wrote the seed down
//! wrong, never noticed until restore time" — this catches that immediately.
//!
//! The check is a pure derivation comparison: we re-run the same descriptor
//! derivation the original `import_seed_wallet` did, then compare the
//! resulting external descriptor (sans checksum) against the wallet's stored
//! one. No persistence, no side effects. Mnemonic is zeroized on every
//! return path.

use crate::api::ApiError;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use corvin_core::seed;
use corvin_core::types::InputKind;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Debug, Deserialize)]
pub struct TestBackupRequest {
    pub mnemonic: String,
    #[serde(default)]
    pub passphrase: String,
}

#[derive(Debug, Serialize)]
pub struct TestBackupResponse {
    /// True if the supplied mnemonic + passphrase reproduces this wallet
    /// exactly (master fingerprint + derivation + script type all match).
    pub matches: bool,
    /// User-facing summary of the result. On success, confirms the match
    /// concisely; on failure, hints at the likely culprits (typo / missed
    /// word / wrong passphrase) without being preachy.
    pub message: String,
}

/// `POST /api/wallets/{id}/test-backup` — verify the supplied mnemonic
/// reproduces this wallet. No state is modified; mnemonic is wiped on every
/// return path.
pub async fn test_backup(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<TestBackupRequest>,
) -> Result<Json<TestBackupResponse>, ApiError> {
    // Move secrets into Zeroizing wrappers up front; clear the originals.
    let mnemonic = Zeroizing::new(std::mem::take(&mut req.mnemonic));
    let passphrase = Zeroizing::new(std::mem::take(&mut req.passphrase));

    let managed = {
        let manager = state.manager.read().await;
        manager
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("wallet not found"))?
    };

    // Eligibility: any HD singlesig wallet whose descriptor carries origin
    // info (i.e. has a `[fingerprint/path]xpub.../0/*` shape). Seed-imported
    // wallets always do; HW-imported wallets do too (we fetch fingerprint +
    // path from the device at import time); paste-xpub wallets do if the
    // user pasted a full descriptor rather than a bare xpub. Multisig and
    // watch-only-address are rejected up front.
    if managed.entry.kind == InputKind::Multisig {
        return Err(anyhow::anyhow!(
            "Backup testing isn't meaningful for multisig wallets — each cosigner has their own seed; verify them individually."
        )
        .into());
    }
    if managed.entry.kind == InputKind::Address {
        return Err(anyhow::anyhow!(
            "This is a watch-only address wallet — there's no seed to verify."
        )
        .into());
    }
    if managed.entry.kind == InputKind::SilentPayments {
        return Err(anyhow::anyhow!(
            "Backup testing isn't wired up for Silent Payments wallets yet — verify the underlying seed separately."
        )
        .into());
    }
    if managed.entry.kind == InputKind::Descriptor {
        return Err(anyhow::anyhow!(
            "This is an imported descriptor wallet — there's no seed to reproduce it from."
        )
        .into());
    }

    // Path source: prefer the wallet's `input` field if it looks like a BIP32
    // path (seed-imported case); otherwise pull it from the descriptor's
    // origin tag (HW-imported and descriptor-paste cases).
    let path = if managed.entry.input.starts_with("m/") {
        managed.entry.input.clone()
    } else {
        extract_path_from_descriptor(&managed.entry.external_descriptor)
            .ok_or_else(|| anyhow::anyhow!(
                "This wallet's descriptor doesn't include a derivation path, so we can't reproduce it from a mnemonic. Add it via 'Hardware wallet' or 'Seed' rather than a bare xpub paste."
            ))?
    };

    let network = state.config.read().await.network.kind.to_bitcoin_network();
    let script_type = match managed.entry.kind {
        InputKind::Taproot => "taproot",
        InputKind::Ypub => "wrapped_segwit",
        InputKind::Xpub => "legacy",
        InputKind::Zpub => "native_segwit",
        InputKind::Multisig
        | InputKind::Address
        | InputKind::SilentPayments
        | InputKind::Descriptor => {
            unreachable!("rejected above")
        }
    };

    let derived = seed::derive_descriptors(
        mnemonic.as_str(),
        passphrase.as_str(),
        &path,
        script_type,
        network,
    )
    .map_err(|e| anyhow::anyhow!("invalid mnemonic: {e}"))?;

    // Strip the optional `#xxxxxxxx` checksum from both sides before
    // comparing — BDK may append one on persist, our derive function
    // doesn't include it.
    let stored = strip_checksum(&managed.entry.external_descriptor);
    let derived_ext = strip_checksum(&derived.external);
    let matches = stored == derived_ext;

    let response = if matches {
        TestBackupResponse {
            matches: true,
            message: "Your backup reproduces this wallet exactly. Master fingerprint and derivation match.".into(),
        }
    } else {
        TestBackupResponse {
            matches: false,
            message: "Backup doesn't match this wallet. Most likely causes: a typo in one of the words, a missed word, the wrong word order, or a passphrase that differs from when the wallet was created. Re-check against your written-down copy.".into(),
        }
    };

    Ok(Json(response))
}

/// Strip the optional `#xxxxxxxx` BIP-380 checksum suffix from a descriptor
/// so comparisons aren't tripped by whether one side included it. Mirrors
/// the helper in `hwi::common`; duplicated locally to avoid pulling the HWI
/// module into the public surface.
fn strip_checksum(desc: &str) -> &str {
    if let Some(hash_pos) = desc.rfind('#') {
        let after = &desc[hash_pos + 1..];
        if after.len() == 8 && after.chars().all(|c| c.is_ascii_alphanumeric()) {
            return &desc[..hash_pos];
        }
    }
    desc
}

/// Extract the BIP32 derivation path from a singlesig descriptor's origin
/// tag. For `wpkh([deadbeef/84'/0'/0']xpub.../0/*)` returns `"m/84'/0'/0'"`.
/// Returns None if no `[fp/path]` origin is present.
fn extract_path_from_descriptor(desc: &str) -> Option<String> {
    let desc = strip_checksum(desc);
    let open = desc.find('[')?;
    let close = desc[open..].find(']')?;
    let inside = &desc[open + 1..open + close];
    // Origin tag is `<fingerprint>/<path>`; split on first slash.
    let slash = inside.find('/')?;
    let path = &inside[slash + 1..];
    if path.is_empty() {
        return None;
    }
    Some(format!("m/{path}"))
}
