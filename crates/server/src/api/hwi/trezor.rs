//! Trezor (Model One, Model T, Safe 3, Safe 5) operations.
//!
//! Uses the official `trezor-client` crate (synchronous API over USB via rusb).
//! Each operation is wrapped in `tokio::task::spawn_blocking` so the blocking
//! USB I/O doesn't stall the async runtime.
//!
//! User-interaction model differs from BitBox / Ledger:
//!   - PIN / passphrase entry happens **on-device** (Model T / Safe family) or
//!     via PIN-matrix prompts (Model One). We currently only support on-device
//!     entry — if the device returns a `PinMatrixRequest` or
//!     `PassphraseRequest` we surface an error asking the user to switch the
//!     device to on-device entry. The Model One workaround would be a separate
//!     follow-up.
//!   - Every operation that requires confirmation surfaces a `ButtonRequest`
//!     which we acknowledge programmatically (emitting `waiting_confirm` for
//!     the UI) while the user verifies on the device screen.
//!
//! Untested against real hardware as of this commit. Written against the
//! published `trezor-client` 0.1.5 docs and example code.

use axum::response::sse::Event;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use bdk_wallet::bitcoin::psbt::Psbt as BdkPsbt;
use bitcoin::{bip32::DerivationPath, Network, Psbt as TrezorPsbt};
use std::convert::Infallible;
use std::str::FromStr;
use tokio::sync::mpsc;
use trezor_client::{protos::InputScriptType, Trezor, TrezorMessage, TrezorResponse};

use super::common::{parse_descriptor_origin, MultisigInfo, SimpleScriptKind};
use crate::config::NetworkKind;

pub type EventTx = mpsc::Sender<Result<Event, Infallible>>;

/// Synchronous probe used by `detect_hw`. Hits the same USB enumeration the
/// real connection uses, but bails as soon as we know whether any Trezor is
/// on the bus.
pub fn detect_sync() -> bool {
    !trezor_client::find_devices(false).is_empty()
}

fn send_blocking(tx: &EventTx, event: &str, data: serde_json::Value) {
    let _ = tx.blocking_send(Ok(Event::default().event(event).data(data.to_string())));
}

fn network_to_bitcoin(net: NetworkKind) -> Network {
    net.to_bitcoin_network()
}

fn map_kind(account_type: &str) -> InputScriptType {
    match account_type {
        "legacy" => InputScriptType::SPENDADDRESS,
        "p2sh_segwit" | "wrapped_segwit" => InputScriptType::SPENDP2SHWITNESS,
        "taproot" => InputScriptType::SPENDTAPROOT,
        "multisig_p2wsh" => InputScriptType::SPENDMULTISIG,
        _ => InputScriptType::SPENDWITNESS,
    }
}

fn script_kind_to_trezor(kind: &SimpleScriptKind) -> InputScriptType {
    match kind {
        SimpleScriptKind::P2Wpkh => InputScriptType::SPENDWITNESS,
        SimpleScriptKind::P2WpkhP2sh => InputScriptType::SPENDP2SHWITNESS,
        SimpleScriptKind::P2Tr => InputScriptType::SPENDTAPROOT,
    }
}

/// Resolve a `TrezorResponse` to its `Ok(T)` payload, auto-acknowledging
/// `ButtonRequest`s (the user confirms on-device while we wait) and surfacing
/// PIN / passphrase requests as user-facing errors.
fn resolve<'a, T, R>(tx: &EventTx, resp: TrezorResponse<'a, T, R>) -> Result<T, String>
where
    R: TrezorMessage,
{
    let mut current = resp;
    loop {
        match current {
            TrezorResponse::Ok(v) => return Ok(v),
            TrezorResponse::Failure(f) => {
                return Err(format!("Device returned failure: {:?}", f.message()));
            }
            TrezorResponse::ButtonRequest(br) => {
                send_blocking(tx, "waiting_confirm", serde_json::json!({}));
                let next = br.ack().map_err(|e| format!("button ack failed: {e}"))?;
                current = next;
            }
            TrezorResponse::PinMatrixRequest(_) => {
                return Err("Trezor is asking for PIN matrix entry on the host. \
                     Switch the device to on-device PIN entry in the Trezor app \
                     (Settings → Security) before signing with Corvin."
                    .into());
            }
            TrezorResponse::PassphraseRequest(_) => {
                return Err("Trezor is asking for passphrase entry on the host. \
                     Switch the device to on-device passphrase entry in the \
                     Trezor app (Settings → Passphrase) before signing with Corvin."
                    .into());
            }
        }
    }
}

// ── Connection helper ─────────────────────────────────────────────────────────

/// Acquire the unique attached Trezor and run the supplied closure with it.
/// Emits `connecting` and `paired` events. The closure runs on the same
/// blocking thread, so it can call the sync trezor-client API freely.
fn with_device<F, T>(tx: &EventTx, op: F) -> Result<T, String>
where
    F: FnOnce(&EventTx, &mut Trezor) -> Result<T, String>,
{
    send_blocking(tx, "connecting", serde_json::json!({}));
    let mut device =
        trezor_client::unique(false).map_err(|e| format!("Trezor not found or not unique: {e}"))?;
    // `init_device` calls Initialize which clears any prior session state and
    // tells us whether the device is unlocked and ready. Without this, follow-up
    // calls can return stale state from a previous app's session.
    device
        .init_device(None)
        .map_err(|e| format!("Trezor init failed: {e}"))?;
    send_blocking(tx, "paired", serde_json::json!({}));
    op(tx, &mut device)
}

// ── xpub export ────────────────────────────────────────────────────────────────

pub async fn run_xpub_export(
    tx: EventTx,
    network_kind: NetworkKind,
    _config_dir: std::path::PathBuf,
    account_type: String,
    account_index: u32,
) {
    let _ = tokio::task::spawn_blocking(move || {
        let is_testnet = network_kind.is_testnet_like();
        let coin_num: u32 = if is_testnet { 1 } else { 0 };
        let path_str = match account_type.as_str() {
            "p2sh_segwit" | "wrapped_segwit" => format!("m/49'/{coin_num}'/{account_index}'"),
            "legacy" => format!("m/44'/{coin_num}'/{account_index}'"),
            "multisig_p2wsh" => format!("m/48'/{coin_num}'/{account_index}'/2'"),
            "taproot" => format!("m/86'/{coin_num}'/{account_index}'"),
            _ => format!("m/84'/{coin_num}'/{account_index}'"),
        };
        let path = match DerivationPath::from_str(&path_str) {
            Ok(p) => p,
            Err(e) => {
                send_blocking(
                    &tx,
                    "hw_error",
                    serde_json::json!({ "message": format!("Invalid path: {e}") }),
                );
                return;
            }
        };
        let net = network_to_bitcoin(network_kind);
        let kind = map_kind(&account_type);

        let result = with_device(&tx, |tx, dev| {
            let resp = dev
                .get_public_key(&path, kind, net, false)
                .map_err(|e| format!("get_public_key failed: {e}"))?;
            let xpub = resolve(tx, resp)?;
            // Trezor doesn't expose a separate get_fingerprint call — the
            // master fingerprint comes from the parent of the requested xpub.
            // Easiest path: ask for the master (empty path) xpub separately.
            let master_resp = dev
                .get_public_key(
                    &DerivationPath::default(),
                    InputScriptType::SPENDADDRESS,
                    net,
                    false,
                )
                .map_err(|e| format!("master pubkey failed: {e}"))?;
            let master = resolve(tx, master_resp)?;
            Ok((xpub, master.fingerprint()))
        });

        match result {
            Ok((xpub, fingerprint)) => {
                send_blocking(
                    &tx,
                    "xpub",
                    serde_json::json!({
                        "xpub": xpub.to_string(),
                        "fingerprint": fingerprint.to_string(),
                        "path": path_str,
                    }),
                );
            }
            Err(e) => {
                send_blocking(&tx, "hw_error", serde_json::json!({ "message": e }));
            }
        }
    })
    .await;
}

// ── Address verification ───────────────────────────────────────────────────────

pub async fn run_show_address(
    tx: EventTx,
    network_kind: NetworkKind,
    _config_dir: std::path::PathBuf,
    descriptor: String,
    keychain: u32,
    address_index: u32,
) {
    let _ = tokio::task::spawn_blocking(move || {
        let Some(origin) = parse_descriptor_origin(&descriptor) else {
            send_blocking(&tx, "hw_error", serde_json::json!({
                "message": "Can't verify this address on device: the wallet descriptor doesn't carry hardware-wallet origin info, or it's a script type the device can't display directly (e.g. multisig). Add the wallet via the Hardware wallet tab to enable verification."
            }));
            return;
        };
        let net = network_to_bitcoin(network_kind);
        let kind = script_kind_to_trezor(&origin.kind);

        let full_path = format!("{}/{keychain}/{address_index}", origin.base_path);
        let path = match DerivationPath::from_str(&full_path) {
            Ok(p) => p,
            Err(e) => {
                send_blocking(&tx, "hw_error", serde_json::json!({ "message": format!("Invalid keypath: {e}") }));
                return;
            }
        };

        let result = with_device(&tx, |tx, dev| {
            send_blocking(tx, "verifying", serde_json::json!({}));
            let resp = dev
                .get_address(&path, kind, net, true)
                .map_err(|e| format!("get_address failed: {e}"))?;
            resolve(tx, resp)
        });

        match result {
            Ok(addr) => {
                send_blocking(&tx, "address", serde_json::json!({ "address": addr.to_string() }));
            }
            Err(e) => {
                send_blocking(&tx, "hw_error", serde_json::json!({ "message": format!("Address verification failed: {e}") }));
            }
        }
    })
    .await;
}

// ── PSBT signing ──────────────────────────────────────────────────────────────

pub async fn run_sign_psbt(
    tx: EventTx,
    network_kind: NetworkKind,
    _config_dir: std::path::PathBuf,
    psbt_b64: String,
    multisig_info: Option<MultisigInfo>,
    // Trezor has no miniscript support — policy wallets fall back to software.
    _policy_info: Option<super::common::PolicyInfo>,
    _wallet_id: Option<uuid::Uuid>,
) {
    if multisig_info.is_some() {
        let _ = tx
            .send(Ok(Event::default().event("hw_error").data(
                serde_json::json!({
                    "message": "Multisig signing on Trezor isn't wired up yet — only singlesig works. Use BitBox for multisig, or wait for the next update."
                }).to_string())))
            .await;
        return;
    }

    let _ = tokio::task::spawn_blocking(move || {
        let psbt_bytes = match STANDARD.decode(&psbt_b64) {
            Ok(b) => b,
            Err(e) => {
                send_blocking(
                    &tx,
                    "hw_error",
                    serde_json::json!({ "message": format!("Invalid PSBT: {e}") }),
                );
                return;
            }
        };
        let mut bdk_psbt = match BdkPsbt::deserialize(&psbt_bytes) {
            Ok(p) => p,
            Err(e) => {
                send_blocking(
                    &tx,
                    "hw_error",
                    serde_json::json!({ "message": format!("Invalid PSBT: {e}") }),
                );
                return;
            }
        };
        let trezor_psbt = match TrezorPsbt::deserialize(&psbt_bytes) {
            Ok(p) => p,
            Err(e) => {
                send_blocking(
                    &tx,
                    "hw_error",
                    serde_json::json!({ "message": format!("Invalid PSBT (trezor): {e}") }),
                );
                return;
            }
        };
        let net = network_to_bitcoin(network_kind);

        let result = with_device(&tx, |tx, dev| {
            send_blocking(tx, "signing", serde_json::json!({}));

            // Kick off sign_tx → returns a progress object. Each ack_psbt
            // round-trips one chunk of metadata; the device emits signatures
            // back via has_signature() / get_signature() along the way.
            let resp = dev
                .sign_tx(&trezor_psbt, net)
                .map_err(|e| format!("sign_tx failed: {e}"))?;
            let mut progress = resolve(tx, resp)?;

            let mut sigs: Vec<(usize, Vec<u8>)> = Vec::new();
            loop {
                if progress.has_signature() {
                    if let Some((idx, sig)) = progress.get_signature() {
                        sigs.push((idx, sig.to_vec()));
                    }
                }
                if progress.finished() {
                    break;
                }
                let next_resp = progress
                    .ack_psbt(&trezor_psbt, net)
                    .map_err(|e| format!("ack_psbt failed: {e}"))?;
                progress = resolve(tx, next_resp)?;
            }
            Ok(sigs)
        });

        let sigs = match result {
            Ok(s) => s,
            Err(e) => {
                send_blocking(
                    &tx,
                    "hw_error",
                    serde_json::json!({ "message": format!("Signing failed: {e}") }),
                );
                return;
            }
        };

        // Apply each signature to the corresponding input's partial_sigs map.
        // The device returns raw DER-encoded ECDSA signatures and tells us the
        // input index — we have to pair it with the right pubkey from the
        // PSBT's bip32_derivation (the entry whose fingerprint matches the
        // signing device).
        if let Err(e) = apply_trezor_sigs(&mut bdk_psbt, &sigs) {
            send_blocking(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Could not apply Trezor signatures: {e}") }),
            );
            return;
        }
        super::common::finalize_psbt_inputs(&mut bdk_psbt);

        let signed_b64 = STANDARD.encode(bdk_psbt.serialize());
        send_blocking(&tx, "signed", serde_json::json!({ "psbt": signed_b64 }));
    })
    .await;
}

/// Apply the (input_index, raw_signature) pairs Trezor returned onto the
/// matching input's `partial_sigs` map. For singlesig wallets each input has
/// exactly one bip32_derivation entry, so we take that pubkey directly. The
/// signature is DER-encoded with the sighash byte stripped — we re-attach
/// `SIGHASH_ALL` since that's what Trezor produces for default signing flows.
fn apply_trezor_sigs(psbt: &mut BdkPsbt, sigs: &[(usize, Vec<u8>)]) -> Result<(), String> {
    for (idx, sig_bytes) in sigs {
        let input = psbt
            .inputs
            .get_mut(*idx)
            .ok_or_else(|| format!("input index {idx} out of range"))?;
        // Singlesig: exactly one bip32_derivation. (For multisig we'd need to
        // match by fingerprint — deferred until Trezor multisig signing lands.)
        let (pk, _) = input
            .bip32_derivation
            .iter()
            .next()
            .ok_or_else(|| format!("input #{idx} has no bip32_derivation"))?;
        let pk = bdk_wallet::bitcoin::PublicKey::new(*pk);
        let signature = bdk_wallet::bitcoin::secp256k1::ecdsa::Signature::from_der(sig_bytes)
            .map_err(|e| format!("sig parse: {e}"))?;
        let sig = bdk_wallet::bitcoin::ecdsa::Signature {
            signature,
            sighash_type: bdk_wallet::bitcoin::sighash::EcdsaSighashType::All,
        };
        input.partial_sigs.insert(pk, sig);
    }
    Ok(())
}
