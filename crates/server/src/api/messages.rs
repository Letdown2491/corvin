//! BIP-322 message signing and verification.
//!
//! Verification is keyless: `(address, message, signature) → valid?`. Anyone
//! can call it to validate a third party's claim.
//!
//! Signing requires the user to re-supply their mnemonic, because Corvin
//! deliberately never stores the seed on disk (only an xpub-based descriptor
//! is persisted). We re-derive the private key for the chosen address in
//! memory, sign, and zeroize. The mnemonic never touches storage.
//!
//! Address types supported by the underlying `bip322` crate: P2TR, P2WPKH,
//! and P2SH-P2WPKH. Legacy P2PKH addresses use the older "Bitcoin Signed
//! Message" format and are not handled here (BIP-322 doesn't cover them).

use crate::api::ApiError;
use crate::state::{AppState, ManagedWallet, WalletInner};
use axum::{
    extract::{Path, State},
    Json,
};
use bdk_wallet::bitcoin::{
    bip32::{DerivationPath, Xpriv},
    secp256k1::Secp256k1,
    Address, Network, PrivateKey,
};
use bip39::{Language, Mnemonic};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Deserialize)]
pub struct VerifyRequest {
    pub address: String,
    pub message: String,
    pub signature: String,
}

#[derive(Serialize)]
pub struct VerifyResponse {
    pub valid: bool,
    /// Populated with a short reason when `valid` is false (e.g. "Invalid
    /// signature", "Unsupported address"). `None` on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `POST /api/messages/verify` — keyless verification.
pub async fn verify_message(
    Json(req): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, ApiError> {
    match bip322::verify_simple_encoded(&req.address, &req.message, &req.signature) {
        Ok(()) => Ok(Json(VerifyResponse {
            valid: true,
            error: None,
        })),
        Err(e) => Ok(Json(VerifyResponse {
            valid: false,
            error: Some(e.to_string()),
        })),
    }
}

#[derive(Deserialize)]
pub struct SignRequest {
    /// Address belonging to the wallet for which we should sign.
    pub address: String,
    pub message: String,
    /// BIP39 mnemonic (12/15/18/21/24 words). Used once and zeroized.
    pub mnemonic: String,
    /// BIP39 passphrase ("25th word"). Optional; pass empty string if unused.
    #[serde(default)]
    pub passphrase: String,
}

#[derive(Serialize)]
pub struct SignResponse {
    /// Base64-encoded BIP-322 signature (Simple format).
    pub signature: String,
}

/// `POST /api/wallets/{id}/sign-message` — derive the address's private key
/// from the supplied mnemonic and produce a BIP-322 Simple signature.
pub async fn sign_message(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<SignRequest>,
) -> Result<Json<SignResponse>, ApiError> {
    // Move secrets into Zeroizing wrappers up front so they're wiped on every
    // return path. Replace the originals with empty strings.
    let mnemonic_str = Zeroizing::new(std::mem::take(&mut req.mnemonic));
    let passphrase = Zeroizing::new(std::mem::take(&mut req.passphrase));

    let network = network_from_config(&state).await;
    let managed = lookup_wallet(&state, &id).await?;

    if managed.entry.threshold.is_some() {
        return Err(
            anyhow::anyhow!("Multisig wallets can't sign single-key BIP-322 messages.").into(),
        );
    }

    let WalletInner::Hd(wm) = &managed.inner else {
        return Err(anyhow::anyhow!(
            "Message signing requires an HD wallet (watch-only address wallets have no keys)."
        )
        .into());
    };

    // Parse the user's selected address and verify it's on the right network.
    let target_addr = Address::from_str(&req.address)
        .map_err(|e| anyhow::anyhow!("invalid address: {e}"))?
        .require_network(network)
        .map_err(|_| anyhow::anyhow!("address is for the wrong network"))?;

    // Look up which keychain (external/internal) and index this address
    // corresponds to inside the wallet. If the address isn't tracked, refuse —
    // signing with an arbitrary key under the wrong path would be silently
    // wrong.
    let target_spk = target_addr.script_pubkey();
    let (keychain, index) = {
        let hd = wm.lock().await;
        hd.wallet
            .derivation_of_spk(target_spk)
            .ok_or_else(|| anyhow::anyhow!(
                "Address doesn't belong to this wallet. Reveal it under Addresses first, or pick one from the list."
            ))?
    };

    // Parse the BIP39 mnemonic. The crate validates the wordlist and checksum.
    let mnemonic = Mnemonic::parse_in(Language::English, mnemonic_str.as_str())
        .map_err(|e| anyhow::anyhow!("invalid mnemonic: {e}"))?;

    // Derive master xpriv from the mnemonic + passphrase.
    let seed: Zeroizing<[u8; 64]> = Zeroizing::new(mnemonic.to_seed(passphrase.as_str()));
    let secp = Secp256k1::new();
    let master = Xpriv::new_master(network, &*seed)
        .map_err(|e| anyhow::anyhow!("master key derivation failed: {e}"))?;

    // Extract the account-level derivation path from the wallet's descriptor.
    // For a descriptor like `wpkh([fp/84'/0'/0']xpub.../0/*)` the origin path
    // is `84'/0'/0'`. We append `/keychain/index` to reach the final key.
    let account_path = parse_account_path(&managed.entry.external_descriptor)?;
    let keychain_idx: u32 = match keychain {
        bdk_wallet::KeychainKind::External => 0,
        bdk_wallet::KeychainKind::Internal => 1,
    };
    let full_path_str = format!("m/{account_path}/{keychain_idx}/{index}");
    let full_path = DerivationPath::from_str(&full_path_str)
        .map_err(|e| anyhow::anyhow!("derivation path parse: {e}"))?;
    let child = master
        .derive_priv(&secp, &full_path)
        .map_err(|e| anyhow::anyhow!("child key derivation failed: {e}"))?;

    // Sanity check: the public key derived from this private key must produce
    // the address the user picked. If it doesn't, the user supplied the wrong
    // mnemonic / passphrase / wallet — bail rather than emit a misleading sig.
    let priv_key = PrivateKey::new(child.private_key, network);
    if !derived_matches_address(
        &priv_key,
        &target_addr,
        &managed.entry.external_descriptor,
        network,
    )? {
        return Err(anyhow::anyhow!(
            "The mnemonic doesn't match this wallet's descriptor. Double-check the words and passphrase."
        )
        .into());
    }

    // Hand off to bip322. The crate handles P2TR, P2WPKH, P2SH-P2WPKH; it
    // returns an error for legacy P2PKH which we surface to the user.
    let wif = Zeroizing::new(priv_key.to_wif());
    let signature = bip322::sign_simple_encoded(&req.address, &req.message, &wif)
        .map_err(|e| anyhow::anyhow!("signing failed: {e}"))?;

    Ok(Json(SignResponse { signature }))
}

/// Pull the `account` derivation path out of a single-sig descriptor's origin
/// tag. Returns the path *without* the leading `m/` (e.g. `"84'/0'/0'"`).
///
/// We accept any of the three single-sig forms Corvin emits: `wpkh(...)`,
/// `sh(wpkh(...))`, `tr(...)`, `pkh(...)`. Multisig descriptors are rejected
/// upstream so we don't need to parse them.
fn parse_account_path(descriptor: &str) -> anyhow::Result<String> {
    // Find the `[fp/path]` origin tag. There should be exactly one for a
    // single-sig descriptor.
    let open = descriptor
        .find('[')
        .ok_or_else(|| anyhow::anyhow!("descriptor has no key origin tag"))?;
    let close = descriptor[open..]
        .find(']')
        .ok_or_else(|| anyhow::anyhow!("descriptor origin tag is unterminated"))?
        + open;
    let inside = &descriptor[open + 1..close]; // e.g. "deadbeef/84'/0'/0'"
    let (_fp, path) = inside
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("descriptor origin tag has no path"))?;
    Ok(path.to_string())
}

/// True when the public key derived from `priv_key` would produce `address`
/// under the script type implied by the descriptor. Used as a paranoia check
/// before signing — if the user's mnemonic is wrong, the derived key won't
/// match the address they picked.
fn derived_matches_address(
    priv_key: &PrivateKey,
    address: &Address,
    descriptor: &str,
    network: Network,
) -> anyhow::Result<bool> {
    use bdk_wallet::bitcoin::{secp256k1::Secp256k1, CompressedPublicKey, PublicKey};
    let secp = Secp256k1::new();
    let pubkey = PublicKey::from_private_key(&secp, priv_key);

    // Build the address that would come from this pubkey under each
    // single-sig script type Corvin supports, then compare.
    let derived = if descriptor.starts_with("tr(") {
        let xonly = bdk_wallet::bitcoin::XOnlyPublicKey::from(pubkey.inner);
        Address::p2tr(&secp, xonly, None, network)
    } else if descriptor.starts_with("wpkh(") {
        let cpk = CompressedPublicKey::from_private_key(&secp, priv_key)
            .map_err(|e| anyhow::anyhow!("compressed pubkey: {e}"))?;
        Address::p2wpkh(&cpk, network)
    } else if descriptor.starts_with("sh(wpkh(") {
        let cpk = CompressedPublicKey::from_private_key(&secp, priv_key)
            .map_err(|e| anyhow::anyhow!("compressed pubkey: {e}"))?;
        Address::p2shwpkh(&cpk, network)
    } else if descriptor.starts_with("pkh(") {
        Address::p2pkh(pubkey, network)
    } else {
        return Err(anyhow::anyhow!("unsupported descriptor type for signing"));
    };

    Ok(&derived == address)
}

async fn network_from_config(state: &AppState) -> Network {
    state.config.read().await.network.kind.to_bitcoin_network()
}

async fn lookup_wallet(state: &AppState, id: &Uuid) -> Result<Arc<ManagedWallet>, ApiError> {
    state
        .manager
        .read()
        .await
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("wallet not found").into())
}
