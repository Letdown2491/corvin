//! Ledger Nano S/S+/X/Stax/Flex operations.
//!
//! Uses `ledger_bitcoin_client` for the Bitcoin app's APDU protocol over
//! `ledger-transport-hid`. The Bitcoin app must be open on the device for any
//! of these calls to work — the Ledger main menu won't respond to Bitcoin
//! APDUs, the user has to navigate to and open the app before signing.
//!
//! Untested against real hardware as of this commit — written against the
//! crate's published examples and docs. Expect at least one round of fixes
//! once a Nano is plugged in.

use axum::response::sse::Event;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use bdk_wallet::bitcoin::{bip32::DerivationPath, psbt::Psbt as BdkPsbt};
use bitcoin::Psbt as LedgerPsbt;
use ledger_bitcoin_client::{
    apdu::{APDUCommand, StatusWord},
    async_client::{BitcoinClient, Transport},
    wallet::{Version as PolicyVersion, WalletPolicy, WalletPubKey},
};
use ledger_transport_hid::{hidapi::HidApi, TransportNativeHID};
use std::convert::Infallible;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::mpsc;

use super::common::{parse_descriptor_origin, MultisigInfo, SimpleScriptKind};
use super::ledger_hmac_store;
use crate::config::NetworkKind;

pub type EventTx = mpsc::Sender<Result<Event, Infallible>>;

/// Ledger's USB vendor ID. All Ledger hardware (Nano S, Nano X, Nano S+, Stax,
/// Flex) shares this VID across products.
const LEDGER_USB_VENDOR_ID: u16 = 0x2c97;

/// Synchronous probe used by `detect_hw`. Enumerates HID devices and returns
/// true if any Ledger is on the bus. The actual transport check (whether the
/// Bitcoin app is open) happens inside the per-operation flows.
pub fn detect_sync() -> bool {
    let Ok(api) = HidApi::new() else {
        return false;
    };
    let found = api
        .device_list()
        .any(|d| d.vendor_id() == LEDGER_USB_VENDOR_ID);
    found
}

async fn send(tx: &EventTx, event: &str, data: serde_json::Value) -> Result<(), ()> {
    tx.send(Ok(Event::default().event(event).data(data.to_string())))
        .await
        .map_err(|_| ())
}

// ── Transport adapter ─────────────────────────────────────────────────────────
//
// `ledger_bitcoin_client::async_client::Transport` and `ledger_transport::Exchange`
// are two distinct traits with slightly different shapes. The HID crate
// implements `Exchange`; the Bitcoin app client wants `Transport`. Bridge them.

struct HidTransport {
    inner: Arc<TransportNativeHID>,
}

impl HidTransport {
    fn open() -> Result<Self, String> {
        let api = HidApi::new().map_err(|e| format!("hidapi init failed: {e}"))?;
        let transport =
            TransportNativeHID::new(&api).map_err(|e| format!("Ledger not found: {e}"))?;
        Ok(Self {
            inner: Arc::new(transport),
        })
    }
}

#[async_trait::async_trait]
impl Transport for HidTransport {
    type Error = String;

    async fn exchange(&self, cmd: &APDUCommand) -> Result<(StatusWord, Vec<u8>), Self::Error> {
        // HID I/O is blocking — bounce to a blocking thread.
        let transport = Arc::clone(&self.inner);
        let transport_cmd = ledger_transport::APDUCommand {
            cla: cmd.cla,
            ins: cmd.ins,
            p1: cmd.p1,
            p2: cmd.p2,
            data: cmd.data.clone(),
        };
        let answer = tokio::task::spawn_blocking(move || transport.exchange(&transport_cmd))
            .await
            .map_err(|e| format!("hid join error: {e}"))?
            .map_err(|e| format!("hid exchange: {e}"))?;
        let sw_u16 = answer.retcode();
        let sw = StatusWord::try_from(sw_u16)
            .map_err(|_| format!("unknown status word 0x{sw_u16:04X}"))?;
        Ok((sw, answer.data().to_vec()))
    }
}

/// Open a Ledger over HID, verify the Bitcoin app responds, and return a
/// `BitcoinClient` ready for use. Emits `connecting` and `paired` events to
/// match the BitBox state machine the frontend already handles.
async fn open_client(tx: &EventTx) -> Result<BitcoinClient<HidTransport>, ()> {
    let _ = send(tx, "connecting", serde_json::json!({})).await;

    let transport = match HidTransport::open() {
        Ok(t) => t,
        Err(e) => {
            let _ = send(
                tx,
                "hw_error",
                serde_json::json!({ "message": format!("Ledger not found: {e}") }),
            )
            .await;
            return Err(());
        }
    };
    let client = BitcoinClient::new(transport);

    // A trivial APDU round-trip verifies the Bitcoin app is the current screen.
    // If the device is on the dashboard, this errors with a clear-ish message
    // we surface to the user.
    let _ = send(tx, "waiting_confirm", serde_json::json!({})).await;
    match client.get_version().await {
        Ok(_) => {}
        Err(e) => {
            let _ = send(tx, "hw_error", serde_json::json!({
                "message": format!("Open the Bitcoin app on the Ledger before continuing. ({e:?})")
            })).await;
            return Err(());
        }
    }
    let _ = send(tx, "paired", serde_json::json!({})).await;
    Ok(client)
}

// ── xpub export ────────────────────────────────────────────────────────────────

pub async fn run_xpub_export(
    tx: EventTx,
    network_kind: NetworkKind,
    _config_dir: std::path::PathBuf,
    account_type: String,
    account_index: u32,
) {
    let client = match open_client(&tx).await {
        Ok(c) => c,
        Err(()) => return,
    };

    let is_testnet = network_kind.is_testnet_like();
    let coin_num: u32 = if is_testnet { 1 } else { 0 };
    let path_str = match account_type.as_str() {
        "p2sh_segwit" => format!("m/49'/{coin_num}'/{account_index}'"),
        "legacy" => format!("m/44'/{coin_num}'/{account_index}'"),
        "multisig_p2wsh" => format!("m/48'/{coin_num}'/{account_index}'/2'"),
        "taproot" => format!("m/86'/{coin_num}'/{account_index}'"),
        _ => format!("m/84'/{coin_num}'/{account_index}'"),
    };
    let path = match DerivationPath::from_str(&path_str) {
        Ok(p) => p,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Invalid path: {e}") }),
            )
            .await;
            return;
        }
    };

    let fingerprint = match client.get_master_fingerprint().await {
        Ok(fp) => fp,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Fingerprint request failed: {e:?}") }),
            )
            .await;
            return;
        }
    };

    // `display=true` would prompt the user to confirm the xpub on the device.
    // For initial xpub export during wallet setup the user is going through
    // the verification flow on the next screen anyway, so `false` is fine here.
    match client.get_extended_pubkey(&path, false).await {
        Ok(xpub) => {
            let _ = send(
                &tx,
                "xpub",
                serde_json::json!({
                    "xpub": xpub.to_string(),
                    "fingerprint": fingerprint.to_string(),
                    "path": path_str,
                }),
            )
            .await;
        }
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("XPub request failed: {e:?}") }),
            )
            .await;
        }
    }
}

// ── Address verification ───────────────────────────────────────────────────────

/// Build a Ledger `WalletPolicy` matching one of the standard single-sig
/// "default" templates the Bitcoin app recognises without prior registration:
/// `wpkh(@0/**)`, `sh(wpkh(@0/**))`, or `tr(@0/**)`. The xpub comes from the
/// device itself via a fresh `get_extended_pubkey` call so the policy is
/// internally consistent.
async fn build_default_singlesig_policy(
    client: &BitcoinClient<HidTransport>,
    base_path: &str,
    kind: &SimpleScriptKind,
) -> Result<WalletPolicy, String> {
    let path = DerivationPath::from_str(base_path)
        .map_err(|e| format!("invalid base path '{base_path}': {e}"))?;
    let xpub = client
        .get_extended_pubkey(&path, false)
        .await
        .map_err(|e| format!("get_extended_pubkey failed: {e:?}"))?;
    let fp = client
        .get_master_fingerprint()
        .await
        .map_err(|e| format!("get_master_fingerprint failed: {e:?}"))?;

    let descriptor_template = match kind {
        SimpleScriptKind::P2Wpkh => "wpkh(@0/**)".to_string(),
        SimpleScriptKind::P2WpkhP2sh => "sh(wpkh(@0/**))".to_string(),
        SimpleScriptKind::P2Tr => "tr(@0/**)".to_string(),
    };

    // The key string format Ledger expects: `[fp/path]xpub`. The path inside
    // the bracket omits the leading `m/`.
    let path_inside = base_path.trim_start_matches("m/");
    let key_str = format!("[{}/{}]{}", fp, path_inside, xpub);
    let pubkey =
        WalletPubKey::from_str(&key_str).map_err(|e| format!("wallet pubkey parse: {e:?}"))?;

    Ok(WalletPolicy::new(
        String::new(), // default policies use the empty name
        PolicyVersion::V2,
        descriptor_template,
        vec![pubkey],
    ))
}

pub async fn run_show_address(
    tx: EventTx,
    _network_kind: NetworkKind,
    _config_dir: std::path::PathBuf,
    descriptor: String,
    keychain: u32,
    address_index: u32,
) {
    let Some(origin) = parse_descriptor_origin(&descriptor) else {
        let _ = send(&tx, "hw_error", serde_json::json!({
            "message": "Can't verify this address on device: the wallet descriptor doesn't carry hardware-wallet origin info, or it's a script type the device can't display directly (e.g. multisig). Add the wallet via the Hardware wallet tab to enable verification."
        })).await;
        return;
    };

    let client = match open_client(&tx).await {
        Ok(c) => c,
        Err(()) => return,
    };

    let policy =
        match build_default_singlesig_policy(&client, &origin.base_path, &origin.kind).await {
            Ok(p) => p,
            Err(e) => {
                let _ = send(&tx, "hw_error", serde_json::json!({ "message": e })).await;
                return;
            }
        };

    let _ = send(&tx, "verifying", serde_json::json!({})).await;
    // No hmac for default policies. `display=true` prompts the user to verify
    // on the device screen.
    match client
        .get_wallet_address(&policy, None, keychain != 0, address_index, true)
        .await
    {
        Ok(addr) => {
            let _ = send(
                &tx,
                "address",
                serde_json::json!({ "address": addr.assume_checked().to_string() }),
            )
            .await;
        }
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Address verification failed: {e:?}") }),
            )
            .await;
        }
    }
}

// ── PSBT signing ──────────────────────────────────────────────────────────────

pub async fn run_sign_psbt(
    tx: EventTx,
    _network_kind: NetworkKind,
    _config_dir: std::path::PathBuf,
    psbt_b64: String,
    multisig_info: Option<MultisigInfo>,
    policy_info: Option<super::common::PolicyInfo>,
    wallet_id: Option<uuid::Uuid>,
) {
    if let Some(ms) = multisig_info {
        let Some(wid) = wallet_id else {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({
                    "message": "Multisig signing requires a wallet id."
                }),
            )
            .await;
            return;
        };
        run_sign_psbt_multisig(tx, psbt_b64, ms, wid).await;
        return;
    }
    if let Some(pol) = policy_info {
        let Some(wid) = wallet_id else {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({
                    "message": "Policy signing requires a wallet id."
                }),
            )
            .await;
            return;
        };
        run_sign_psbt_policy(tx, psbt_b64, pol, wid).await;
        return;
    }

    let psbt_bytes = match STANDARD.decode(&psbt_b64) {
        Ok(b) => b,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Invalid PSBT: {e}") }),
            )
            .await;
            return;
        }
    };

    // The bdk_wallet PSBT and the bare bitcoin PSBT are conceptually the
    // same type but reach us through different re-exports. Parse once via
    // BDK so the rest of the codebase stays uniform, then deserialize again
    // through the bare bitcoin crate that the Ledger client expects.
    let mut bdk_psbt = match BdkPsbt::deserialize(&psbt_bytes) {
        Ok(p) => p,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Invalid PSBT: {e}") }),
            )
            .await;
            return;
        }
    };
    let ledger_psbt = match LedgerPsbt::deserialize(&psbt_bytes) {
        Ok(p) => p,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Invalid PSBT (ledger): {e}") }),
            )
            .await;
            return;
        }
    };

    let client = match open_client(&tx).await {
        Ok(c) => c,
        Err(()) => return,
    };

    // Build the singlesig policy from the first input's bip32 derivation.
    // We assume all inputs share the same account-level path (standard
    // for wallets built from a single descriptor — change keychain plus
    // address index varies, the rest is constant).
    let Some(origin_info) = bdk_psbt.inputs.iter().find_map(|inp| {
        inp.bip32_derivation
            .iter()
            .next()
            .map(|(_, (fp, path))| (*fp, path.clone()))
    }) else {
        let _ = send(&tx, "hw_error", serde_json::json!({
            "message": "PSBT has no bip32 derivation info — can't determine which wallet policy to sign with."
        })).await;
        return;
    };

    // The bip32_derivation path is full ("m/84'/0'/0'/0/5"). Trim the trailing
    // `/keychain/index` to get the account-level path the policy expects.
    let full_path = origin_info.1;
    let comps: Vec<bdk_wallet::bitcoin::bip32::ChildNumber> =
        full_path.into_iter().copied().collect();
    if comps.len() < 4 {
        let _ = send(&tx, "hw_error", serde_json::json!({
            "message": "PSBT derivation path is shorter than expected — can't extract account path."
        })).await;
        return;
    }
    let account_components = &comps[..comps.len() - 2];
    let account_path: bdk_wallet::bitcoin::bip32::DerivationPath =
        account_components.to_vec().into();
    let account_path_str = format!("m/{account_path}");

    // Guess script kind from the input's witness_utxo. P2WPKH covers the
    // overwhelming majority of new wallets; the others mirror what BitBox
    // accepts via `parse_descriptor_origin`.
    let kind = bdk_psbt.inputs.iter().find_map(|inp| {
        inp.witness_utxo.as_ref().map(|u| {
            if u.script_pubkey.is_p2wpkh() {
                SimpleScriptKind::P2Wpkh
            } else if u.script_pubkey.is_p2sh() {
                SimpleScriptKind::P2WpkhP2sh
            } else if u.script_pubkey.is_p2tr() {
                SimpleScriptKind::P2Tr
            } else {
                SimpleScriptKind::P2Wpkh
            }
        })
    });
    let Some(kind) = kind else {
        let _ = send(
            &tx,
            "hw_error",
            serde_json::json!({
                "message": "PSBT input has no witness_utxo — can't determine script type."
            }),
        )
        .await;
        return;
    };

    let policy = match build_default_singlesig_policy(&client, &account_path_str, &kind).await {
        Ok(p) => p,
        Err(e) => {
            let _ = send(&tx, "hw_error", serde_json::json!({ "message": e })).await;
            return;
        }
    };

    // Verify the device fingerprint matches what's expected in the PSBT — same
    // sanity check as the BitBox multisig flow but applied to singlesig too,
    // so the wrong physical device doesn't sign with the wrong key.
    let device_fp = match client.get_master_fingerprint().await {
        Ok(fp) => fp,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Fingerprint request failed: {e:?}") }),
            )
            .await;
            return;
        }
    };
    if device_fp.to_string().to_lowercase() != origin_info.0.to_string().to_lowercase() {
        let _ = send(&tx, "hw_error", serde_json::json!({
            "message": format!(
                "Connected Ledger fingerprint {device_fp} doesn't match this wallet's expected fingerprint {expected}. Are you using the right device?",
                expected = origin_info.0
            )
        })).await;
        return;
    }

    let _ = send(&tx, "signing", serde_json::json!({})).await;
    let yielded = match client.sign_psbt(&ledger_psbt, &policy, None).await {
        Ok(y) => y,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Signing failed: {e:?}") }),
            )
            .await;
            return;
        }
    };

    // Apply the yielded partial signatures back onto the BDK psbt and finalize.
    if let Err(e) = apply_yielded_sigs(&mut bdk_psbt, &yielded) {
        let _ = send(
            &tx,
            "hw_error",
            serde_json::json!({ "message": format!("Could not apply Ledger signatures: {e}") }),
        )
        .await;
        return;
    }
    super::common::finalize_psbt_inputs(&mut bdk_psbt);

    let signed_b64 = STANDARD.encode(bdk_psbt.serialize());
    let _ = send(&tx, "signed", serde_json::json!({ "psbt": signed_b64 })).await;
}

// ── Multisig signing ──────────────────────────────────────────────────────────

async fn run_sign_psbt_multisig(
    tx: EventTx,
    psbt_b64: String,
    ms: MultisigInfo,
    wallet_id: uuid::Uuid,
) {
    use ledger_bitcoin_client::wallet::{AddressType, Version as PolicyVersion};

    let psbt_bytes = match STANDARD.decode(&psbt_b64) {
        Ok(b) => b,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Invalid PSBT: {e}") }),
            )
            .await;
            return;
        }
    };
    let mut bdk_psbt = match BdkPsbt::deserialize(&psbt_bytes) {
        Ok(p) => p,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Invalid PSBT: {e}") }),
            )
            .await;
            return;
        }
    };
    let ledger_psbt = match LedgerPsbt::deserialize(&psbt_bytes) {
        Ok(p) => p,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Invalid PSBT (ledger): {e}") }),
            )
            .await;
            return;
        }
    };

    let client = match open_client(&tx).await {
        Ok(c) => c,
        Err(()) => return,
    };

    // The connected device must be one of the cosigners — verify by matching
    // its master fingerprint against the multisig's signer list.
    let device_fp = match client.get_master_fingerprint().await {
        Ok(fp) => fp,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Fingerprint request failed: {e:?}") }),
            )
            .await;
            return;
        }
    };
    let device_fp_str = device_fp.to_string().to_lowercase();
    if !ms
        .signers
        .iter()
        .any(|s| s.fingerprint.to_string().to_lowercase() == device_fp_str)
    {
        let _ = send(&tx, "hw_error", serde_json::json!({
            "message": format!(
                "This Ledger's fingerprint ({device_fp_str}) doesn't match any cosigner in this multisig wallet. Are you using the right device?"
            )
        })).await;
        return;
    }

    // Construct the wallet policy from the multisig info. We use sortedmulti
    // (BIP67) since that's what Corvin always emits for multisig descriptors.
    let policy_keys: Vec<ledger_bitcoin_client::wallet::WalletPubKey> = ms
        .signers
        .iter()
        .map(|s| {
            let key_source: bdk_wallet::bitcoin::bip32::KeySource =
                (s.fingerprint, s.account_path.clone());
            ledger_bitcoin_client::wallet::WalletPubKey::from((key_source, s.xpub))
        })
        .collect();
    let policy = match ledger_bitcoin_client::wallet::WalletPolicy::new_multisig(
        ms.label.clone(),
        PolicyVersion::V2,
        AddressType::NativeSegwit,
        ms.threshold as usize,
        policy_keys,
        true, // sortedmulti
    ) {
        Ok(p) => p,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({
                    "message": format!("Could not build wallet policy: {e:?}")
                }),
            )
            .await;
            return;
        }
    };

    // Look up a previously-saved HMAC; if missing, register the policy on the
    // device (one-time per device per wallet) and persist the returned HMAC.
    let hmac: [u8; 32] = match ledger_hmac_store::get(wallet_id, &device_fp_str) {
        Some(h) => h,
        None => {
            let _ = send(&tx, "registering", serde_json::json!({})).await;
            let (_id, hmac) = match client.register_wallet(&policy).await {
                Ok(r) => r,
                Err(e) => {
                    let _ = send(&tx, "hw_error", serde_json::json!({
                        "message": format!("Multisig registration failed: {e:?}. Confirm each cosigner on the device when prompted.")
                    })).await;
                    return;
                }
            };
            if let Err(e) = ledger_hmac_store::set(wallet_id, &device_fp_str, &hmac) {
                tracing::warn!("Couldn't persist Ledger HMAC: {e}");
                // Continue anyway — signing still works for this session;
                // the user will just have to re-register next time.
            }
            hmac
        }
    };

    let _ = send(&tx, "signing", serde_json::json!({})).await;
    let yielded = match client.sign_psbt(&ledger_psbt, &policy, Some(&hmac)).await {
        Ok(y) => y,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Signing failed: {e:?}") }),
            )
            .await;
            return;
        }
    };

    if let Err(e) = apply_yielded_sigs(&mut bdk_psbt, &yielded) {
        let _ = send(
            &tx,
            "hw_error",
            serde_json::json!({ "message": format!("Could not apply Ledger signatures: {e}") }),
        )
        .await;
        return;
    }
    // For multisig we deliberately don't finalize — partial sigs need to remain
    // so combine_psbt can aggregate them with other cosigners' signatures.
    let signed_b64 = STANDARD.encode(bdk_psbt.serialize());
    let _ = send(&tx, "signed", serde_json::json!({ "psbt": signed_b64 })).await;
}

/// Sign a miniscript/taproot policy (vault) PSBT on a Ledger: build the
/// BIP-388 wallet policy from the descriptor template + keys, register it on
/// the device (cached HMAC, like multisig), sign, and apply the signatures.
async fn run_sign_psbt_policy(
    tx: EventTx,
    psbt_b64: String,
    pol: super::common::PolicyInfo,
    wallet_id: uuid::Uuid,
) {
    use std::str::FromStr;

    let psbt_bytes = match STANDARD.decode(&psbt_b64) {
        Ok(b) => b,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Invalid PSBT: {e}") }),
            )
            .await;
            return;
        }
    };
    let mut bdk_psbt = match BdkPsbt::deserialize(&psbt_bytes) {
        Ok(p) => p,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Invalid PSBT: {e}") }),
            )
            .await;
            return;
        }
    };
    let ledger_psbt = match LedgerPsbt::deserialize(&psbt_bytes) {
        Ok(p) => p,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Invalid PSBT (ledger): {e}") }),
            )
            .await;
            return;
        }
    };

    let client = match open_client(&tx).await {
        Ok(c) => c,
        Err(()) => return,
    };

    let device_fp = match client.get_master_fingerprint().await {
        Ok(fp) => fp,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Fingerprint request failed: {e:?}") }),
            )
            .await;
            return;
        }
    };
    let device_fp_str = device_fp.to_string().to_lowercase();
    if !pol
        .keys
        .iter()
        .any(|k| k.fingerprint.to_lowercase() == device_fp_str)
    {
        let _ = send(&tx, "hw_error", serde_json::json!({
            "message": format!("This Ledger's fingerprint ({device_fp_str}) doesn't match any key in this wallet's policy. Are you using the right device?")
        })).await;
        return;
    }

    // WalletPubKey list — order matches the @i placeholders in the template.
    let mut policy_keys: Vec<WalletPubKey> = Vec::with_capacity(pol.keys.len());
    for k in &pol.keys {
        let fp = match bdk_wallet::bitcoin::bip32::Fingerprint::from_str(&k.fingerprint) {
            Ok(f) => f,
            Err(e) => {
                let _ = send(
                    &tx,
                    "hw_error",
                    serde_json::json!({ "message": format!("Bad key fingerprint in policy: {e}") }),
                )
                .await;
                return;
            }
        };
        let path = match bdk_wallet::bitcoin::bip32::DerivationPath::from_str(
            k.path.strip_prefix("m/").unwrap_or(&k.path),
        ) {
            Ok(p) => p,
            Err(e) => {
                let _ = send(
                    &tx,
                    "hw_error",
                    serde_json::json!({ "message": format!("Bad key path in policy: {e}") }),
                )
                .await;
                return;
            }
        };
        let xpub = match bdk_wallet::bitcoin::bip32::Xpub::from_str(&k.xpub) {
            Ok(x) => x,
            Err(e) => {
                let _ = send(
                    &tx,
                    "hw_error",
                    serde_json::json!({ "message": format!("Bad xpub in policy: {e}") }),
                )
                .await;
                return;
            }
        };
        let key_source: bdk_wallet::bitcoin::bip32::KeySource = (fp, path);
        policy_keys.push(WalletPubKey::from((key_source, xpub)));
    }

    let policy = WalletPolicy::new(
        pol.label.clone(),
        PolicyVersion::V2,
        pol.template.clone(),
        policy_keys,
    );

    let hmac: [u8; 32] = match ledger_hmac_store::get(wallet_id, &device_fp_str) {
        Some(h) => h,
        None => {
            let _ = send(&tx, "registering", serde_json::json!({})).await;
            let (_id, hmac) = match client.register_wallet(&policy).await {
                Ok(r) => r,
                Err(e) => {
                    let _ = send(&tx, "hw_error", serde_json::json!({
                        "message": format!("Policy registration failed: {e:?}. Confirm the wallet policy on the device when prompted.")
                    })).await;
                    return;
                }
            };
            if let Err(e) = ledger_hmac_store::set(wallet_id, &device_fp_str, &hmac) {
                tracing::warn!("Couldn't persist Ledger HMAC: {e}");
            }
            hmac
        }
    };

    let _ = send(&tx, "signing", serde_json::json!({})).await;
    let yielded = match client.sign_psbt(&ledger_psbt, &policy, Some(&hmac)).await {
        Ok(y) => y,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Signing failed: {e:?}") }),
            )
            .await;
            return;
        }
    };

    if let Err(e) = apply_yielded_sigs(&mut bdk_psbt, &yielded) {
        let _ = send(
            &tx,
            "hw_error",
            serde_json::json!({ "message": format!("Could not apply Ledger signatures: {e}") }),
        )
        .await;
        return;
    }
    // Finalize the key-path (taproot vault primary) into a witness; script-path
    // and wsh inputs stay partially signed for the downstream BDK finalize.
    super::common::finalize_psbt_inputs(&mut bdk_psbt);
    let signed_b64 = STANDARD.encode(bdk_psbt.serialize());
    let _ = send(&tx, "signed", serde_json::json!({ "psbt": signed_b64 })).await;
}

/// Copy Ledger's `(input_index, SignPsbtYieldedObject)` results into the BDK
/// psbt. Handles ECDSA `partial_sigs` and taproot key-path/script-path sigs.
fn apply_yielded_sigs(
    psbt: &mut BdkPsbt,
    yielded: &[(usize, ledger_bitcoin_client::SignPsbtYieldedObject)],
) -> Result<(), String> {
    use ledger_bitcoin_client::psbt::PartialSignature;
    use ledger_bitcoin_client::SignPsbtYieldedObject;
    for (idx, obj) in yielded {
        let input = psbt
            .inputs
            .get_mut(*idx)
            .ok_or_else(|| format!("input index {idx} out of range"))?;
        match obj {
            SignPsbtYieldedObject::Partial(part) => match part {
                PartialSignature::Sig(pk, sig) => {
                    // The Ledger crate uses `bitcoin = 0.32` types directly;
                    // BDK 3 also uses `bitcoin = 0.32`. They're the same
                    // version, but the compiler still sees them through two
                    // import paths — round-trip via bytes to bridge cleanly.
                    let pk_bytes = pk.to_bytes();
                    let bdk_pk = bdk_wallet::bitcoin::PublicKey::from_slice(&pk_bytes)
                        .map_err(|e| format!("pubkey parse: {e}"))?;
                    let sig_bytes = sig.signature.serialize_der();
                    let bdk_sig = bdk_wallet::bitcoin::ecdsa::Signature {
                        signature: bdk_wallet::bitcoin::secp256k1::ecdsa::Signature::from_der(
                            &sig_bytes,
                        )
                        .map_err(|e| format!("sig parse: {e}"))?,
                        sighash_type: bdk_wallet::bitcoin::sighash::EcdsaSighashType::All,
                    };
                    input.partial_sigs.insert(bdk_pk, bdk_sig);
                }
                PartialSignature::TapScriptSig(xonly, leaf, sig) => {
                    use bdk_wallet::bitcoin::hashes::Hash;
                    // Bridge ledger-crate bitcoin 0.32 types → bdk's via bytes.
                    let sig_bytes = sig.to_vec();
                    let bdk_sig = bdk_wallet::bitcoin::taproot::Signature::from_slice(&sig_bytes)
                        .map_err(|e| format!("taproot sig parse: {e}"))?;
                    match leaf {
                        // No leaf hash → key-path spend.
                        None => input.tap_key_sig = Some(bdk_sig),
                        // Leaf hash present → script-path (tapleaf) spend.
                        Some(lh) => {
                            let bdk_xonly =
                                bdk_wallet::bitcoin::XOnlyPublicKey::from_slice(&xonly.serialize())
                                    .map_err(|e| format!("xonly parse: {e}"))?;
                            let bdk_leaf =
                                bdk_wallet::bitcoin::taproot::TapLeafHash::from_byte_array(
                                    lh.to_byte_array(),
                                );
                            input.tap_script_sigs.insert((bdk_xonly, bdk_leaf), bdk_sig);
                        }
                    }
                }
            },
            _ => {
                return Err("unexpected MuSig2 / unknown yield for singlesig PSBT".into());
            }
        }
    }
    Ok(())
}
