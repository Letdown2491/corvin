//! Encrypted wallet persistence.
//!
//! A [`bdk_wallet::WalletPersister`] that stores a wallet's aggregated
//! [`ChangeSet`] as a single CBOR file, written through the shared at-rest
//! sealed writer (plaintext when the vault is off, XChaCha20-Poly1305 when
//! unlocked). This replaces BDK's SQLite store so wallet data uses the exact
//! same encryption as every other file, with no SQLite/C dependency.
//!
//! BDK owns the hard part (changeset semantics + merge). We only hold the
//! aggregate in memory, merge each delta, and serialize/seal it.

use std::path::PathBuf;

use bdk_wallet::chain::Merge;
use bdk_wallet::{ChangeSet, WalletPersister};

/// File-backed, at-rest-sealed persister for one wallet's `ChangeSet`.
pub struct EncryptedChangeSetStore {
    path: PathBuf,
    /// The full aggregated changeset, kept in memory so each `persist` can
    /// rewrite the whole (small) blob atomically.
    aggregated: ChangeSet,
}

impl EncryptedChangeSetStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            aggregated: ChangeSet::default(),
        }
    }
}

impl WalletPersister for EncryptedChangeSetStore {
    type Error = anyhow::Error;

    fn initialize(persister: &mut Self) -> Result<ChangeSet, Self::Error> {
        if let Some(bytes) = crate::at_rest::read_sealed(&persister.path)? {
            persister.aggregated = ciborium::from_reader(&bytes[..])
                .map_err(|e| anyhow::anyhow!("decoding wallet changeset: {e}"))?;
        }
        Ok(persister.aggregated.clone())
    }

    fn persist(persister: &mut Self, changeset: &ChangeSet) -> Result<(), Self::Error> {
        persister.aggregated.merge(changeset.clone());
        let mut buf = Vec::new();
        ciborium::into_writer(&persister.aggregated, &mut buf)
            .map_err(|e| anyhow::anyhow!("encoding wallet changeset: {e}"))?;
        crate::at_rest::write_sealed(&persister.path, &buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdk_wallet::bitcoin::Network;
    use bdk_wallet::{KeychainKind, Wallet};

    // A descriptor pair derived from a fixed BIP-32 test key.
    fn descriptors() -> (String, String) {
        use bdk_wallet::bitcoin::bip32::{Xpriv, Xpub};
        use bdk_wallet::bitcoin::secp256k1::Secp256k1;
        let secp = Secp256k1::new();
        let xpriv = Xpriv::new_master(Network::Regtest, &[42u8; 32]).unwrap();
        let xpub = Xpub::from_priv(&secp, &xpriv);
        (
            format!("wpkh({xpub}/0/*)"),
            format!("wpkh({xpub}/1/*)"),
        )
    }

    #[test]
    fn changeset_persists_and_reloads() {
        // Round-trips through the global VAULT (write_sealed/read_sealed) — serialize
        // against the other vault-touching tests so a concurrent unlock can't interfere.
        let _serial = crate::at_rest::VAULT_TEST_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("corvin-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.changeset");
        let (ext, int) = descriptors();

        // Create + reveal an address + persist.
        let revealed = {
            let mut store = EncryptedChangeSetStore::new(path.clone());
            let mut wallet = Wallet::create(ext.clone(), int.clone())
                .network(Network::Regtest)
                .create_wallet(&mut store)
                .unwrap();
            let addr = wallet.reveal_next_address(KeychainKind::External).address;
            wallet.persist(&mut store).unwrap();
            addr.to_string()
        };

        // The file exists and is not the raw plaintext descriptor (it's CBOR).
        assert!(path.exists());

        // Reload from disk: same wallet, same revealed address.
        let mut store2 = EncryptedChangeSetStore::new(path.clone());
        let wallet2 = Wallet::load()
            .check_network(Network::Regtest)
            .load_wallet(&mut store2)
            .unwrap()
            .expect("wallet should load from the persisted changeset");
        assert_eq!(
            wallet2.peek_address(KeychainKind::External, 0).address.to_string(),
            revealed,
            "reloaded wallet has the same first address"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
