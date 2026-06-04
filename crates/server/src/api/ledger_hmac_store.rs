//! Persistent store of Ledger multisig wallet-policy registration HMACs.
//!
//! Ledger's Bitcoin app requires every non-standard wallet policy (anything
//! that isn't a singlesig default — multisig, miniscript) to be registered on
//! the device first via `register_wallet()`, which returns a 32-byte HMAC.
//! Subsequent operations (get_wallet_address, sign_psbt) need that HMAC.
//! Registration requires user interaction on the device to confirm every
//! cosigner — without persistence we'd be asking the user to verify the full
//! cosigner list every single signing session.
//!
//! Schema (JSON):
//!   { "<wallet_uuid>": { "<device_fingerprint_hex>": "<hmac_hex>" } }
//!
//! Same wallet, multiple devices (multisig with several Ledgers) → multiple
//! fingerprint entries. Same device, multiple wallets → multiple wallet entries.
//!
//! `forget_wallet` is used on wallet delete (always); `get`/`set` only by the USB
//! Ledger module, so they read as dead code in the `hw`-off (Start9) build.
#![cfg_attr(not(feature = "hw"), allow(dead_code))]

use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

type FingerprintHex = String;
type HmacHex = String;
type Store = HashMap<Uuid, HashMap<FingerprintHex, HmacHex>>;

fn store_path() -> PathBuf {
    crate::config::ledger_hmacs_path()
}

fn load() -> Store {
    crate::state::load_or_quarantine(&store_path())
}

fn save(store: &Store) -> anyhow::Result<()> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Atomic + 0600 + at-rest-sealed via the shared writer.
    crate::state::write_private(&path, &serde_json::to_vec_pretty(store)?)
}

pub fn get(wallet_id: Uuid, device_fingerprint: &str) -> Option<[u8; 32]> {
    let store = load();
    let hex = store
        .get(&wallet_id)?
        .get(&device_fingerprint.to_lowercase())?;
    let bytes = hex_decode(hex)?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(arr)
}

pub fn set(wallet_id: Uuid, device_fingerprint: &str, hmac: &[u8; 32]) -> anyhow::Result<()> {
    let mut store = load();
    store
        .entry(wallet_id)
        .or_default()
        .insert(device_fingerprint.to_lowercase(), hex_encode(hmac));
    save(&store)
}

/// Drop all entries for a wallet — called from the wallet-delete handler so
/// stale registration state doesn't accumulate on disk.
pub fn forget_wallet(wallet_id: Uuid) -> anyhow::Result<()> {
    let mut store = load();
    if store.remove(&wallet_id).is_some() {
        save(&store)?;
    }
    Ok(())
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
