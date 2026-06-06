use crate::backends::electrum::ElectrumConfig;
use crate::types::{
    AddressInfo, AddressKind, Balance, BalancePoint, SyncResult, TxRecord, UtxoRecord, WalletEntry,
};
use anyhow::{Context, Result};
use bdk_wallet::bitcoin::{Address, Network};
use bdk_wallet::chain::ChainPosition;
use bdk_wallet::{KeychainKind, PersistedWallet, Wallet};
use chrono::DateTime;
use std::collections::HashMap;

use crate::wallet_store::EncryptedChangeSetStore;

/// A wallet + its persistence store, kept together so callers can write staged
/// changes. The store is an at-rest-sealed CBOR `ChangeSet` blob (no SQLite).
pub struct HdWalletState {
    pub wallet: PersistedWallet<EncryptedChangeSetStore>,
    pub store: EncryptedChangeSetStore,
}

impl HdWalletState {
    /// Write any staged changeset from the wallet to its (sealed) store.
    pub fn persist_staged(&mut self) -> Result<()> {
        self.wallet
            .persist(&mut self.store)
            .map_err(|e| anyhow::anyhow!("persist wallet: {e}"))?;
        Ok(())
    }
}

fn is_hd(descriptor_str: &str) -> bool {
    descriptor_str.contains("/*")
}

/// Open an existing BDK wallet from SQLite, or create it fresh if the DB is empty.
///
/// All subsequent syncs should call `HdWalletState::persist_staged()` so that
/// revealed keychain indices and transaction data survive restarts.
pub fn open_or_create_wallet(
    entry: &WalletEntry,
    network: Network,
    store_path: &std::path::Path,
) -> Result<HdWalletState> {
    if let Some(parent) = store_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating wallet store dir {}", parent.display()))?;
    }

    let mut store = EncryptedChangeSetStore::new(store_path.to_path_buf());

    let ext = entry.external_descriptor.clone();
    let int_opt = entry.internal_descriptor.clone();

    // Try to load an existing wallet from the persisted changeset.
    let maybe_wallet = Wallet::load()
        .check_network(network)
        .load_wallet(&mut store)
        .map_err(|e| anyhow::anyhow!("load wallet: {e:?}"))?;

    let wallet = match maybe_wallet {
        Some(w) => w,
        None => {
            // Store is empty (first run) — create and persist initial state.
            match int_opt {
                Some(int) => Wallet::create(ext, int),
                None => Wallet::create_single(ext),
            }
            .network(network)
            .create_wallet(&mut store)
            .map_err(|e| anyhow::anyhow!("create wallet: {e:?}"))?
        }
    };

    Ok(HdWalletState { wallet, store })
}

/// Sync an HD wallet and return new fee data for confirmed transactions. Blocking.
pub fn sync_electrum_with_fees(
    wallet: &mut Wallet,
    cfg: &ElectrumConfig,
    existing_fees: &HashMap<String, u64>,
) -> Result<(SyncResult, HashMap<String, u64>)> {
    crate::backends::electrum::sync_wallet_with_fees(wallet, cfg, existing_fees)
}

/// Sync a single address against the Electrum server. Returns (balance, txs). Blocking.
pub fn sync_address_electrum(
    address: &str,
    cfg: &ElectrumConfig,
) -> anyhow::Result<(crate::types::Balance, Vec<crate::types::TxRecord>)> {
    crate::backends::electrum::sync_address(address, cfg)
}

/// Address info list for a single watched address.
pub fn address_info(address: &str) -> Vec<crate::types::AddressInfo> {
    crate::backends::electrum::address_info(address)
}

pub fn get_balance(wallet: &Wallet) -> Balance {
    let b = wallet.balance();
    Balance {
        confirmed_sats: b.confirmed.to_sat(),
        unconfirmed_sats: (b.trusted_pending + b.untrusted_pending).to_sat(),
        // `trusted_spendable` already excludes immature coinbase per BDK's
        // semantics — coinbase outputs <100 confs aren't selectable.
        spendable_sats: b.trusted_spendable().to_sat(),
        immature_sats: b.immature.to_sat(),
        last_synced: None,
    }
}

pub fn get_transactions(wallet: &Wallet, fees: &HashMap<String, u64>) -> Vec<TxRecord> {
    let tip = wallet.latest_checkpoint().height();

    let mut records: Vec<TxRecord> = wallet
        .transactions()
        .map(|canonical_tx| {
            let txid = canonical_tx.tx_node.txid.to_string();

            let (block_height, confirmations, timestamp) = match &canonical_tx.chain_position {
                bdk_wallet::chain::ChainPosition::Confirmed { anchor, .. } => {
                    let h = anchor.block_id.height;
                    let ts = DateTime::from_timestamp(anchor.confirmation_time as i64, 0);
                    (Some(h), tip.saturating_sub(h).saturating_add(1), ts)
                }
                bdk_wallet::chain::ChainPosition::Unconfirmed { .. } => (None, 0, None),
            };

            let (sent, received) = wallet.sent_and_received(&canonical_tx.tx_node.tx);
            let amount_sats = received.to_sat() as i64 - sent.to_sat() as i64;
            let fee_sats = fees.get(&txid).copied();
            let vsize = Some(canonical_tx.tx_node.tx.vsize() as u32);
            let is_coinbase = canonical_tx.tx_node.tx.is_coinbase();
            // Coinbase txs always have a single input with sequence 0xFFFFFFFF
            // (final) — don't mark them as RBF even though that sequence isn't
            // technically "RBF-signaling-disabled" by the bare numeric check.
            let is_rbf = !is_coinbase
                && canonical_tx
                    .tx_node
                    .tx
                    .input
                    .iter()
                    .any(|i| i.sequence.to_consensus_u32() <= 0xFFFFFFFD);

            TxRecord {
                txid,
                amount_sats,
                fee_sats,
                vsize,
                confirmations,
                block_height,
                timestamp,
                is_rbf,
                is_coinbase,
            }
        })
        .collect();

    // Unconfirmed (block_height = None) sorts first; confirmed by descending height.
    records.sort_by(|a, b| match (a.block_height, b.block_height) {
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
        (Some(ah), Some(bh)) => bh.cmp(&ah),
    });
    records
}

pub fn get_utxos(wallet: &Wallet) -> Vec<UtxoRecord> {
    let tip = wallet.latest_checkpoint().height();

    // Build a txid → is_coinbase map up front so the per-UTXO loop doesn't
    // re-walk the canonical-tx iterator (which would be O(N²)).
    let coinbase_txids: std::collections::HashSet<bdk_wallet::bitcoin::Txid> = wallet
        .transactions()
        .filter(|c| c.tx_node.tx.is_coinbase())
        .map(|c| c.tx_node.txid)
        .collect();

    wallet
        .list_unspent()
        .map(|utxo| {
            let txid_obj = utxo.outpoint.txid;
            let txid = txid_obj.to_string();
            let vout = utxo.outpoint.vout;
            let amount_sats = utxo.txout.value.to_sat();
            let address = Address::from_script(&utxo.txout.script_pubkey, wallet.network())
                .ok()
                .map(|a| a.to_string());
            let (confirmations, block_height) = match &utxo.chain_position {
                ChainPosition::Confirmed { anchor, .. } => {
                    let h = anchor.block_id.height;
                    (tip.saturating_sub(h).saturating_add(1), Some(h))
                }
                ChainPosition::Unconfirmed { .. } => (0, None),
            };
            let is_coinbase = coinbase_txids.contains(&txid_obj);
            // Bitcoin consensus: coinbase outputs require 100 confirmations.
            // Non-coinbase UTXOs are always mature regardless of confs.
            let is_mature = !is_coinbase || confirmations >= 100;
            UtxoRecord {
                txid,
                vout,
                amount_sats,
                address,
                confirmations,
                block_height,
                is_coinbase,
                is_mature,
                suspected_dust: false,
            }
        })
        .collect()
}

pub fn balance_history_from_records(txs: &[TxRecord]) -> Vec<BalancePoint> {
    let mut confirmed: Vec<&TxRecord> = txs.iter().filter(|tx| tx.block_height.is_some()).collect();
    confirmed.sort_by_key(|tx| tx.block_height.unwrap_or(0));

    let mut running: i64 = 0;
    confirmed
        .into_iter()
        .map(|tx| {
            running += tx.amount_sats;
            BalancePoint {
                // Safe: `confirmed` was filtered to `block_height.is_some()` above.
                block_height: tx.block_height.expect("confirmed tx has a block height"),
                timestamp: tx.timestamp,
                balance_sats: running,
            }
        })
        .collect()
}

pub fn get_balance_history(wallet: &Wallet, fees: &HashMap<String, u64>) -> Vec<BalancePoint> {
    let txs = get_transactions(wallet, fees);
    balance_history_from_records(&txs)
}

pub fn get_addresses(wallet: &Wallet) -> Vec<AddressInfo> {
    let mut addrs = Vec::new();

    // Collect (txid, outputs) upfront to avoid borrow conflicts.
    let tx_data: Vec<(
        bdk_wallet::bitcoin::Txid,
        Vec<bdk_wallet::bitcoin::ScriptBuf>,
    )> = wallet
        .transactions()
        .map(|tx| {
            let txid = tx.tx_node.txid;
            let spks = tx
                .tx_node
                .tx
                .output
                .iter()
                .map(|o| o.script_pubkey.clone())
                .collect();
            (txid, spks)
        })
        .collect();

    // spk → set of txids that paid to it (count = tx_count, non-empty = used)
    let mut spk_txids: std::collections::HashMap<
        bdk_wallet::bitcoin::ScriptBuf,
        std::collections::HashSet<bdk_wallet::bitcoin::Txid>,
    > = std::collections::HashMap::new();
    for (txid, spks) in &tx_data {
        for spk in spks {
            spk_txids.entry(spk.clone()).or_default().insert(*txid);
        }
    }

    for (index, spk) in wallet
        .spk_index()
        .revealed_keychain_spks(KeychainKind::External)
    {
        if let Ok(addr) = bdk_wallet::bitcoin::Address::from_script(&spk, wallet.network()) {
            let tx_count = spk_txids.get(&spk).map(|s| s.len() as u32).unwrap_or(0);
            addrs.push(AddressInfo {
                address: addr.to_string(),
                index,
                kind: AddressKind::External,
                used: tx_count > 0,
                tx_count,
            });
        }
    }

    if is_hd(&wallet.public_descriptor(KeychainKind::External).to_string()) {
        for (index, spk) in wallet
            .spk_index()
            .revealed_keychain_spks(KeychainKind::Internal)
        {
            if let Ok(addr) = bdk_wallet::bitcoin::Address::from_script(&spk, wallet.network()) {
                let tx_count = spk_txids.get(&spk).map(|s| s.len() as u32).unwrap_or(0);
                addrs.push(AddressInfo {
                    address: addr.to_string(),
                    index,
                    kind: AddressKind::Internal,
                    used: tx_count > 0,
                    tx_count,
                });
            }
        }
    }

    addrs
}
