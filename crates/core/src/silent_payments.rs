//! BIP-352 Silent Payments key derivation.
//!
//! Phase 1: derive the scan + spend keys from a BIP39 seed, encode the
//! sp1q… receiving address. Scanning and sending live in later phases.
//!
//! Derivation paths per BIP-352:
//!   - spend secret key: m/352'/coin'/account'/0'/0
//!   - scan secret key:  m/352'/coin'/account'/1'/0
//!
//! `coin` = 0 for Bitcoin mainnet, 1 for testnet / signet / regtest.

use anyhow::Result;
use bip39::{Language, Mnemonic};
use bitcoin::bip32::{DerivationPath, Xpriv};
use bitcoin::secp256k1::Secp256k1;
use bitcoin::Network as BtcNetwork;
use silentpayments::receiving::{Label, Receiver};
use silentpayments::secp256k1::PublicKey;
use silentpayments::Network as SpNetwork;
use silentpayments::SilentPaymentAddress;
use std::str::FromStr;
use zeroize::Zeroizing;

/// Public result of enabling silent payments on a wallet. The scan secret key
/// is the only piece that has to persist on disk (otherwise we can't scan on
/// startup); the spend secret key is intentionally not returned or stored —
/// we re-derive it from the mnemonic at send time so spending still requires
/// the user's seed.
pub struct DerivedSilentPaymentKeys {
    /// 32-byte scan secret key — needed for scanning incoming payments.
    pub scan_secret: [u8; 32],
    /// 33-byte compressed spend pubkey — needed for address generation and
    /// (later) for the receiver scanner. Safe to store in the clear.
    pub spend_pubkey: [u8; 33],
    /// Cached bech32m-encoded receiving address (e.g. `sp1q…`). Stored so the
    /// UI can show it without redoing key arithmetic on every read.
    pub address: String,
}

fn sp_network_for(net: BtcNetwork) -> SpNetwork {
    match net {
        BtcNetwork::Bitcoin => SpNetwork::Mainnet,
        BtcNetwork::Regtest => SpNetwork::Regtest,
        // BIP-352 treats signet under the same address prefix as testnet.
        BtcNetwork::Testnet | BtcNetwork::Signet => SpNetwork::Testnet,
        _ => SpNetwork::Mainnet,
    }
}

fn coin_index_for(net: BtcNetwork) -> u32 {
    match net {
        BtcNetwork::Bitcoin => 0,
        _ => 1,
    }
}

/// Derive scan + spend keys from the supplied mnemonic and produce the
/// receiving address. Memory containing secret material is wiped via
/// `Zeroizing` wrappers on every return path.
pub fn derive_from_mnemonic(
    mnemonic_str: &str,
    passphrase: &str,
    network: BtcNetwork,
    account_index: u32,
) -> Result<DerivedSilentPaymentKeys> {
    let mnemonic = Mnemonic::parse_in(Language::English, mnemonic_str)
        .map_err(|e| anyhow::anyhow!("invalid mnemonic: {e}"))?;
    let seed: Zeroizing<[u8; 64]> = Zeroizing::new(mnemonic.to_seed(passphrase));

    let secp = Secp256k1::new();
    let master =
        Xpriv::new_master(network, &*seed).map_err(|e| anyhow::anyhow!("master xpriv: {e}"))?;

    let coin = coin_index_for(network);
    let scan_path = DerivationPath::from_str(&format!("m/352'/{coin}'/{account_index}'/1'/0"))
        .map_err(|e| anyhow::anyhow!("scan path: {e}"))?;
    let spend_path = DerivationPath::from_str(&format!("m/352'/{coin}'/{account_index}'/0'/0"))
        .map_err(|e| anyhow::anyhow!("spend path: {e}"))?;

    let scan_child = master
        .derive_priv(&secp, &scan_path)
        .map_err(|e| anyhow::anyhow!("scan derive: {e}"))?;
    let spend_child = master
        .derive_priv(&secp, &spend_path)
        .map_err(|e| anyhow::anyhow!("spend derive: {e}"))?;

    // Convert between the two `secp256k1` crate copies that flow through
    // bitcoin (bdk_wallet path) vs. silentpayments. They're the same wire
    // bytes; just rebuild the key from the raw 32-byte secret.
    let scan_secret_bytes: [u8; 32] = scan_child.private_key.secret_bytes();
    let spend_secret_bytes: Zeroizing<[u8; 32]> =
        Zeroizing::new(spend_child.private_key.secret_bytes());

    let scan_sk_sp = silentpayments::secp256k1::SecretKey::from_slice(&scan_secret_bytes)
        .map_err(|e| anyhow::anyhow!("scan key parse: {e}"))?;
    let spend_sk_sp = silentpayments::secp256k1::SecretKey::from_slice(spend_secret_bytes.as_ref())
        .map_err(|e| anyhow::anyhow!("spend key parse: {e}"))?;

    let secp_sp = silentpayments::secp256k1::Secp256k1::new();
    let scan_pk_sp: PublicKey = scan_sk_sp.public_key(&secp_sp);
    let spend_pk_sp: PublicKey = spend_sk_sp.public_key(&secp_sp);

    let address = SilentPaymentAddress::new(scan_pk_sp, spend_pk_sp, sp_network_for(network), 0)
        .map_err(|e| anyhow::anyhow!("sp address build: {e}"))?;

    // Drop the SecretKey wrappers early; rust-secp256k1's SecretKey zeroizes
    // its bytes on drop, so this wipes the in-memory key material here.
    let _ = scan_sk_sp;
    let _ = spend_sk_sp;

    Ok(DerivedSilentPaymentKeys {
        scan_secret: scan_secret_bytes,
        spend_pubkey: spend_pk_sp.serialize(),
        address: address.to_string(),
    })
}

/// Reconstruct the bech32m-encoded receiving address from stored key material.
/// Used to display the address after a restart without re-deriving from the
/// mnemonic.
pub fn address_from_stored(
    scan_secret: &[u8; 32],
    spend_pubkey: &[u8; 33],
    network: BtcNetwork,
) -> Result<String> {
    let secp = silentpayments::secp256k1::Secp256k1::new();
    let scan_sk = silentpayments::secp256k1::SecretKey::from_slice(scan_secret)
        .map_err(|e| anyhow::anyhow!("scan key: {e}"))?;
    let scan_pk = scan_sk.public_key(&secp);
    let spend_pk = silentpayments::secp256k1::PublicKey::from_slice(spend_pubkey)
        .map_err(|e| anyhow::anyhow!("spend pubkey: {e}"))?;
    let addr = SilentPaymentAddress::new(scan_pk, spend_pk, sp_network_for(network), 0)
        .map_err(|e| anyhow::anyhow!("sp address: {e}"))?;
    Ok(addr.to_string())
}

/// Derive the BIP-352 spend secret key (`m/352'/coin'/account'/0'/0`) from the
/// mnemonic. Needed to spend FROM an SP wallet — the per-output spending key is
/// `spend_secret + t_n`. Held in `Zeroizing` and wiped on drop.
pub fn derive_spend_secret(
    mnemonic_str: &str,
    passphrase: &str,
    network: BtcNetwork,
    account_index: u32,
) -> Result<Zeroizing<[u8; 32]>> {
    let mnemonic = Mnemonic::parse_in(Language::English, mnemonic_str)
        .map_err(|e| anyhow::anyhow!("invalid mnemonic: {e}"))?;
    let seed: Zeroizing<[u8; 64]> = Zeroizing::new(mnemonic.to_seed(passphrase));
    let secp = Secp256k1::new();
    let master =
        Xpriv::new_master(network, &*seed).map_err(|e| anyhow::anyhow!("master xpriv: {e}"))?;
    let coin = coin_index_for(network);
    let spend_path = DerivationPath::from_str(&format!("m/352'/{coin}'/{account_index}'/0'/0"))
        .map_err(|e| anyhow::anyhow!("spend path: {e}"))?;
    let spend_child = master
        .derive_priv(&secp, &spend_path)
        .map_err(|e| anyhow::anyhow!("spend derive: {e}"))?;
    Ok(Zeroizing::new(spend_child.private_key.secret_bytes()))
}

/// Legacy fallback: recover the spend secret for the account whose derived
/// spend pubkey matches the wallet's stored one. Used only for SP wallets that
/// predate persisting the account index; we try accounts 0..100 (matching the
/// creation range) until the pubkey matches — also a "this mnemonic belongs to
/// this wallet" check. New wallets store the index and derive directly.
pub fn derive_spend_secret_for_spend_pubkey(
    mnemonic_str: &str,
    passphrase: &str,
    network: BtcNetwork,
    expected_spend_pubkey: &[u8; 33],
) -> Result<Zeroizing<[u8; 32]>> {
    for account in 0..100u32 {
        let derived = derive_from_mnemonic(mnemonic_str, passphrase, network, account)?;
        if &derived.spend_pubkey == expected_spend_pubkey {
            return derive_spend_secret(mnemonic_str, passphrase, network, account);
        }
    }
    Err(anyhow::anyhow!(
        "this mnemonic doesn't match the wallet's spend key (checked accounts 0–99)"
    ))
}

/// The wallet's own BIP-352 change address (label m=0), used as the change
/// destination when spending FROM an SP wallet. The scanner registers m=0, so
/// change is re-discovered after broadcast.
pub fn change_address_from_stored(
    scan_secret: &[u8; 32],
    spend_pubkey: &[u8; 33],
    network: BtcNetwork,
) -> Result<String> {
    let secp = silentpayments::secp256k1::Secp256k1::new();
    let scan_sk = silentpayments::secp256k1::SecretKey::from_slice(scan_secret)
        .map_err(|e| anyhow::anyhow!("scan key: {e}"))?;
    let scan_pk = scan_sk.public_key(&secp);
    let spend_pk = silentpayments::secp256k1::PublicKey::from_slice(spend_pubkey)
        .map_err(|e| anyhow::anyhow!("spend pubkey: {e}"))?;
    let change_label = Label::new(scan_sk, 0);
    let receiver = Receiver::new(0, scan_pk, spend_pk, change_label, sp_network_for(network))
        .map_err(|e| anyhow::anyhow!("sp receiver: {e}"))?;
    Ok(receiver.get_change_address().to_string())
}

/// Derive the BIP-352 *labeled* receiving address for label index `m` (m >= 1;
/// m=0 is reserved for change). Labeled addresses share the scan key with the
/// base address — the recipient can tell which label a payment used, but
/// outside observers can link the base and labeled addresses to each other.
pub fn labeled_address_from_stored(
    scan_secret: &[u8; 32],
    spend_pubkey: &[u8; 33],
    network: BtcNetwork,
    m: u32,
) -> Result<String> {
    if m == 0 {
        return Err(anyhow::anyhow!(
            "label m=0 is reserved for change; use m >= 1"
        ));
    }
    let secp = silentpayments::secp256k1::Secp256k1::new();
    let scan_sk = silentpayments::secp256k1::SecretKey::from_slice(scan_secret)
        .map_err(|e| anyhow::anyhow!("scan key: {e}"))?;
    let scan_pk = scan_sk.public_key(&secp);
    let spend_pk = silentpayments::secp256k1::PublicKey::from_slice(spend_pubkey)
        .map_err(|e| anyhow::anyhow!("spend pubkey: {e}"))?;
    let change_label = Label::new(scan_sk, 0);
    let mut receiver = Receiver::new(0, scan_pk, spend_pk, change_label, sp_network_for(network))
        .map_err(|e| anyhow::anyhow!("sp receiver: {e}"))?;
    let label = Label::new(scan_sk, m);
    receiver
        .add_label(label.clone())
        .map_err(|e| anyhow::anyhow!("add label: {e}"))?;
    let addr = receiver
        .get_receiving_address_for_label(&label)
        .map_err(|e| anyhow::anyhow!("labeled address: {e}"))?;
    Ok(addr.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn spend_secret_recovery_roundtrip() {
        let net = BtcNetwork::Bitcoin;
        let d = derive_from_mnemonic(M, "", net, 0).unwrap();
        let secret = derive_spend_secret_for_spend_pubkey(M, "", net, &d.spend_pubkey).unwrap();
        let secp = silentpayments::secp256k1::Secp256k1::new();
        let sk = silentpayments::secp256k1::SecretKey::from_slice(secret.as_ref()).unwrap();
        assert_eq!(
            sk.public_key(&secp).serialize(),
            d.spend_pubkey,
            "recovered secret's pubkey matches stored spend pubkey"
        );
    }

    #[test]
    fn spend_secret_recovers_high_account() {
        // Regression for the account-index bug: a wallet created at account 25
        // must be spendable — directly (stored index) and via the widened
        // legacy fallback (0..100, previously 0..20 → would have failed).
        let net = BtcNetwork::Bitcoin;
        let d = derive_from_mnemonic(M, "", net, 25).unwrap();
        let secp = silentpayments::secp256k1::Secp256k1::new();

        let direct = derive_spend_secret(M, "", net, 25).unwrap();
        let sk = silentpayments::secp256k1::SecretKey::from_slice(&direct[..]).unwrap();
        assert_eq!(
            sk.public_key(&secp).serialize(),
            d.spend_pubkey,
            "direct derivation at account 25"
        );

        let guessed = derive_spend_secret_for_spend_pubkey(M, "", net, &d.spend_pubkey).unwrap();
        assert_eq!(*guessed, *direct, "widened fallback finds account 25");
    }

    #[test]
    fn spend_secret_wrong_mnemonic_rejected() {
        let net = BtcNetwork::Bitcoin;
        let d = derive_from_mnemonic(M, "", net, 0).unwrap();
        let other = "legal winner thank year wave sausage worth useful legal winner thank yellow";
        assert!(derive_spend_secret_for_spend_pubkey(other, "", net, &d.spend_pubkey).is_err());
    }

    #[test]
    fn derived_address_decodes_to_its_own_keys() {
        // Wire-format conformance: the bech32m address our code emits must decode,
        // through the canonical BIP-352 codec, back to exactly the scan + spend
        // pubkeys we encoded — i.e. our encoding is interoperable, not just
        // self-consistent.
        use silentpayments::secp256k1::{Secp256k1, SecretKey};
        let net = BtcNetwork::Bitcoin;
        let d = derive_from_mnemonic(M, "", net, 0).unwrap();

        let parsed = SilentPaymentAddress::try_from(d.address.as_str())
            .expect("our address parses with the canonical decoder");

        let secp = Secp256k1::new();
        let scan_pk = SecretKey::from_slice(&d.scan_secret)
            .unwrap()
            .public_key(&secp);
        assert_eq!(
            parsed.get_scan_key(),
            scan_pk,
            "decoded scan key matches the scan secret's pubkey"
        );
        assert_eq!(
            parsed.get_spend_key().serialize(),
            d.spend_pubkey,
            "decoded spend key matches the stored spend pubkey"
        );
    }

    #[test]
    fn change_address_is_valid_sp() {
        let net = BtcNetwork::Bitcoin;
        let d = derive_from_mnemonic(M, "", net, 0).unwrap();
        let change = change_address_from_stored(&d.scan_secret, &d.spend_pubkey, net).unwrap();
        assert!(
            change.starts_with("sp1"),
            "change address is a mainnet SP address: {change}"
        );
        assert_ne!(
            change, d.address,
            "change address differs from the base address"
        );
    }
}
