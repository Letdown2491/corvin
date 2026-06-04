use bip39::{Language, Mnemonic};
use bitcoin::bip32::{DerivationPath, Xpriv, Xpub};
use bitcoin::secp256k1::Secp256k1;
use bitcoin::Network;
use std::str::FromStr;
use thiserror::Error;
use zeroize::Zeroizing;

#[derive(Debug, Error)]
pub enum SeedError {
    #[error("invalid mnemonic: {0}")]
    Mnemonic(String),
    #[error("key derivation failed: {0}")]
    Derivation(#[from] bitcoin::bip32::Error),
    #[error("invalid derivation path: {0}")]
    Path(String),
}

pub fn generate_mnemonic(word_count: usize) -> Result<String, SeedError> {
    Mnemonic::generate_in(Language::English, word_count)
        .map(|m| m.to_string())
        .map_err(|e| SeedError::Mnemonic(e.to_string()))
}

pub fn default_derivation_path(script_type: &str, network: Network, account: u32) -> String {
    let coin = if network == Network::Bitcoin { 0 } else { 1 };
    let purpose: u32 = match script_type {
        "legacy" => 44,
        "wrapped_segwit" => 49,
        "taproot" => 86,
        _ => 84,
    };
    format!("m/{purpose}'/{coin}'/{account}'")
}

pub struct DerivedDescriptors {
    pub external: String,
    pub internal: String,
}

pub fn derive_descriptors(
    mnemonic_str: &str,
    passphrase: &str,
    path_str: &str,
    script_type: &str,
    network: Network,
) -> Result<DerivedDescriptors, SeedError> {
    let mnemonic = Mnemonic::parse_in(Language::English, mnemonic_str)
        .map_err(|e| SeedError::Mnemonic(e.to_string()))?;
    // Wrap the 64-byte seed so it's wiped from memory when this function returns,
    // even if the caller mishandles error paths. Xpriv's SecretKey already zeros
    // on drop (via secp256k1); this protects the raw seed bytes too.
    let seed: Zeroizing<[u8; 64]> = Zeroizing::new(mnemonic.to_seed(passphrase));

    let secp = Secp256k1::new();
    let master = Xpriv::new_master(network, &*seed)?;
    let path = DerivationPath::from_str(path_str).map_err(|e| SeedError::Path(e.to_string()))?;
    let child_priv = master.derive_priv(&secp, &path)?;
    let child_pub = Xpub::from_priv(&secp, &child_priv);

    let master_fp = master.fingerprint(&secp);
    let path_no_m = path_str.trim_start_matches("m/");
    let origin = format!("[{master_fp}/{path_no_m}]");
    let xpub = child_pub.to_string();

    let (ext, int) = match script_type {
        "taproot" => (
            format!("tr({origin}{xpub}/0/*)"),
            format!("tr({origin}{xpub}/1/*)"),
        ),
        "wrapped_segwit" => (
            format!("sh(wpkh({origin}{xpub}/0/*))"),
            format!("sh(wpkh({origin}{xpub}/1/*))"),
        ),
        "legacy" => (
            format!("pkh({origin}{xpub}/0/*)"),
            format!("pkh({origin}{xpub}/1/*)"),
        ),
        _ => (
            format!("wpkh({origin}{xpub}/0/*)"),
            format!("wpkh({origin}{xpub}/1/*)"),
        ),
    };

    Ok(DerivedDescriptors {
        external: ext,
        internal: int,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdk_wallet::{KeychainKind, Wallet};

    // BIP-39 all-zeros test-vector mnemonic.
    const M: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    // Crypto conformance: the full pipeline (seed → xpub descriptor → BDK wallet → first
    // receive address) must reproduce the OFFICIAL published first-address vectors for this
    // mnemonic. Proves our derivation matches BIP-84/49/86 to the byte.
    #[test]
    fn first_address_matches_official_bip_vectors() {
        let cases = [
            // BIP-84 m/84'/0'/0'/0/0
            ("native_segwit", "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"),
            // BIP-49 m/49'/0'/0'/0/0
            ("wrapped_segwit", "37VucYSaXLCAsxYyAPfbSi9eh4iEcbShgf"),
            // BIP-86 m/86'/0'/0'/0/0
            ("taproot", "bc1p5cyxnuxmeuwuvkwfem96lqzszd02n6xdcjrs20cac6yqjjwudpxqkedrcr"),
        ];
        for (script_type, expected) in cases {
            let path = default_derivation_path(script_type, Network::Bitcoin, 0);
            let d = derive_descriptors(M, "", &path, script_type, Network::Bitcoin).unwrap();
            let wallet = Wallet::create(d.external, d.internal)
                .network(Network::Bitcoin)
                .create_wallet_no_persist()
                .unwrap();
            let addr = wallet
                .peek_address(KeychainKind::External, 0)
                .address
                .to_string();
            assert_eq!(addr, expected, "{script_type}: first address must match the published BIP vector");
        }
    }
}
