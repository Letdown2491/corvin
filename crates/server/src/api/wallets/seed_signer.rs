//! Shared software-wallet signing.
//!
//! Rebuilds xpriv-bearing descriptors from a mnemonic and signs a PSBT with a
//! transient (never-persisted) BDK wallet. This is the one signing path used by
//! both SP-send and payjoin-send — the only flows that sign on the server.
//! Regular send hands an unsigned PSBT to a hardware/external signer instead.
//!
//! The mnemonic/passphrase are caller-owned `Zeroizing` strings; the xpriv
//! descriptor strings we build are likewise `Zeroizing`. The transient BDK
//! wallet copies the key un-zeroized for its (short) lifetime — a BDK
//! limitation outside our control; it's dropped immediately after signing.

use bdk_wallet::{
    bitcoin::{
        bip32::{DerivationPath, Xpriv},
        secp256k1::Secp256k1 as BdkSecp,
        Network, Psbt,
    },
    SignOptions, Wallet,
};
use bip39::{Language, Mnemonic};
use std::str::FromStr;
use zeroize::Zeroizing;

use crate::api::descriptor_util::{parse_descriptor_origin, SimpleScriptKind};
use corvin_core::types::WalletEntry;

pub(crate) struct XprivDescriptors {
    pub external: Zeroizing<String>,
    pub internal: Zeroizing<String>,
}

pub(crate) fn script_type_str(kind: SimpleScriptKind) -> &'static str {
    match kind {
        SimpleScriptKind::P2Wpkh => "native_segwit",
        SimpleScriptKind::P2WpkhP2sh => "wrapped_segwit",
        SimpleScriptKind::P2Tr => "taproot",
    }
}

/// `(base_path, script_type)` for a single-sig HD wallet, parsed from its
/// descriptor origin. Errors if the descriptor lacks origin info (paste-xpub
/// without `[fp/path]`), since we then can't re-derive keys from a mnemonic.
pub(crate) fn signing_params(entry: &WalletEntry) -> anyhow::Result<(String, &'static str)> {
    let origin = parse_descriptor_origin(&entry.external_descriptor).ok_or_else(|| {
        anyhow::anyhow!(
            "this wallet's descriptor doesn't include origin info — can't derive keys from a mnemonic"
        )
    })?;
    Ok((origin.base_path, script_type_str(origin.kind)))
}

/// Build descriptors with xpriv key material from the mnemonic. Same shape as
/// `seed::derive_descriptors` but emits xpriv (signing) rather than xpub
/// (watch-only).
pub(crate) fn build_xpriv_descriptors(
    mnemonic_str: &str,
    passphrase: &str,
    base_path_str: &str,
    script_type: &str,
    network: Network,
) -> anyhow::Result<XprivDescriptors> {
    let mnemonic = Mnemonic::parse_in(Language::English, mnemonic_str)
        .map_err(|e| anyhow::anyhow!("invalid mnemonic: {e}"))?;
    let seed_bytes: Zeroizing<[u8; 64]> = Zeroizing::new(mnemonic.to_seed(passphrase));
    let secp = BdkSecp::new();
    let master = Xpriv::new_master(network, &*seed_bytes)
        .map_err(|e| anyhow::anyhow!("master xpriv: {e}"))?;
    let path = DerivationPath::from_str(base_path_str)
        .map_err(|e| anyhow::anyhow!("invalid base path: {e}"))?;
    let child = master
        .derive_priv(&secp, &path)
        .map_err(|e| anyhow::anyhow!("derive child xpriv: {e}"))?;

    let master_fp = master.fingerprint(&secp);
    let path_no_m = base_path_str.trim_start_matches("m/");
    let origin = format!("[{master_fp}/{path_no_m}]");
    let xpriv_str = Zeroizing::new(child.to_string());
    let k = xpriv_str.as_str();

    let (ext, int) = match script_type {
        "taproot" => (
            format!("tr({origin}{k}/0/*)"),
            format!("tr({origin}{k}/1/*)"),
        ),
        "wrapped_segwit" => (
            format!("sh(wpkh({origin}{k}/0/*))"),
            format!("sh(wpkh({origin}{k}/1/*))"),
        ),
        "legacy" => (
            format!("pkh({origin}{k}/0/*)"),
            format!("pkh({origin}{k}/1/*)"),
        ),
        _ => (
            format!("wpkh({origin}{k}/0/*)"),
            format!("wpkh({origin}{k}/1/*)"),
        ),
    };

    Ok(XprivDescriptors {
        external: Zeroizing::new(ext),
        internal: Zeroizing::new(int),
    })
}

/// Sign `psbt` in place with a transient xpriv-backed wallet rebuilt from the
/// mnemonic. Returns whether BDK finalized it (all inputs satisfied).
pub(crate) fn sign_with_seed(
    base_path: &str,
    script_type: &str,
    network: Network,
    mnemonic: &str,
    passphrase: &str,
    psbt: &mut Psbt,
) -> anyhow::Result<bool> {
    let d = build_xpriv_descriptors(mnemonic, passphrase, base_path, script_type, network)?;
    let wallet = Wallet::create(d.external.to_string(), d.internal.to_string())
        .network(network)
        .create_wallet_no_persist()
        .map_err(|e| anyhow::anyhow!("transient signing wallet: {e}"))?;
    wallet
        .sign(psbt, SignOptions::default())
        .map_err(|e| anyhow::anyhow!("sign: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdk_wallet::bitcoin::Network;
    use bdk_wallet::KeychainKind;

    // BIP-39 all-zeros test vector.
    const M: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    // The critical signing-path invariant: the xpriv (signing) descriptors must derive
    // the SAME addresses as the xpub (watch-only) descriptors for the same seed — i.e.
    // `sign_with_seed` signs for the exact wallet you're watching. Checked per script type.
    #[test]
    fn xpriv_descriptors_match_watch_only_addresses() {
        for st in ["native_segwit", "wrapped_segwit", "taproot"] {
            let path = corvin_core::seed::default_derivation_path(st, Network::Regtest, 0);
            let watch =
                corvin_core::seed::derive_descriptors(M, "", &path, st, Network::Regtest).unwrap();
            let signing = build_xpriv_descriptors(M, "", &path, st, Network::Regtest).unwrap();

            let w_watch = Wallet::create(watch.external.clone(), watch.internal.clone())
                .network(Network::Regtest)
                .create_wallet_no_persist()
                .unwrap();
            let w_sign = Wallet::create(signing.external.to_string(), signing.internal.to_string())
                .network(Network::Regtest)
                .create_wallet_no_persist()
                .unwrap();

            for kc in [KeychainKind::External, KeychainKind::Internal] {
                assert_eq!(
                    w_watch.peek_address(kc, 0).address,
                    w_sign.peek_address(kc, 0).address,
                    "{st} {kc:?}: signing descriptor derives a different address than watch-only"
                );
            }
        }
    }
}
