//! Per-wallet on-disk store for Silent Payments output records.
//!
//! Single file `sp_outputs.json` in the config dir, keyed by wallet UUID →
//! `Vec<SpOutputRecord>`. Atomic writes (temp file + rename). Loaded once at
//! startup to populate each SP wallet's in-memory cache; updated each time
//! the scanner discovers a new match.

use corvin_core::types::SpOutputRecord;
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

type Store = HashMap<Uuid, Vec<SpOutputRecord>>;

fn store_path() -> PathBuf {
    crate::config::config_dir().join("sp_outputs.json")
}

pub fn load() -> Store {
    // Holds tweak_t_n needed to SPEND discovered SP outputs (non-derivable without a
    // rescan) — quarantine on corruption rather than defaulting + overwriting.
    crate::state::load_or_quarantine(&store_path())
}

pub fn load_for_wallet(wallet_id: Uuid) -> Vec<SpOutputRecord> {
    load().remove(&wallet_id).unwrap_or_default()
}

fn save(store: &Store) -> anyhow::Result<()> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::state::write_private(&path, &serde_json::to_vec_pretty(store)?)
}

/// Append a newly-discovered output for `wallet_id`. Idempotent on
/// (txid, vout) — a duplicate entry replaces the existing one rather than
/// piling on.
pub fn append(wallet_id: Uuid, record: SpOutputRecord) -> anyhow::Result<()> {
    let mut store = load();
    let bucket = store.entry(wallet_id).or_default();
    bucket.retain(|r| !(r.txid == record.txid && r.vout == record.vout));
    bucket.push(record);
    save(&store)
}

/// Mark the given `(txid, vout)` outputs spent — called after an SP-spend tx is
/// broadcast, since the scanner only ever *adds* outputs and never records
/// spends itself.
pub fn mark_spent(wallet_id: Uuid, outpoints: &[(String, u32)]) -> anyhow::Result<()> {
    let mut store = load();
    if let Some(bucket) = store.get_mut(&wallet_id) {
        for r in bucket.iter_mut() {
            if outpoints.iter().any(|(t, v)| *t == r.txid && *v == r.vout) {
                r.spent = true;
            }
        }
        save(&store)?;
    }
    Ok(())
}

/// Drop every SP output for the wallet — called when an SP wallet is deleted.
pub fn forget_wallet(wallet_id: Uuid) -> anyhow::Result<()> {
    let mut store = load();
    if store.remove(&wallet_id).is_some() {
        save(&store)?;
    }
    Ok(())
}
