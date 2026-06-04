//! BitBox02-specific HWI operations.
//!
//! Each function takes ownership of the SSE event channel and runs the full
//! brand-specific flow (USB connect → Noise pairing → operation → result event).
//! The caller is responsible for acquiring the global HWI mutex before invoking
//! these, since BitBox doesn't tolerate concurrent USB access.

use axum::response::sse::Event;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use bdk_wallet::bitcoin::psbt::Psbt;
use bitbox_api::{pb, runtime::TokioRuntime, BitBox, Keypath, PersistedNoiseConfig};
use std::convert::Infallible;
use std::path::Path;
use tokio::sync::mpsc;

use super::common::{
    finalize_psbt_inputs, parse_descriptor_origin, MultisigInfo, SimpleScriptKind,
};
use crate::config::NetworkKind;

pub type EventTx = mpsc::Sender<Result<Event, Infallible>>;

/// Synchronous probe used by `detect_hw`. Returns true if any BitBox02 is
/// currently enumerable over HID.
pub fn detect_sync() -> bool {
    bitbox_api::usb::get_any_bitbox02().is_ok()
}

/// Send an SSE event. Returns `Err(())` if the client disconnected so callers
/// can early-exit and release the device lock instead of finishing the flow.
async fn send(tx: &EventTx, event: &str, data: serde_json::Value) -> Result<(), ()> {
    tx.send(Ok(Event::default().event(event).data(data.to_string())))
        .await
        .map_err(|_| ())
}

/// Connect to the BitBox over USB and complete the Noise pairing dance.
/// Emits `connecting`, optional `pairing_code`, `waiting_confirm`, and `paired`
/// events along the way. Returns the paired device handle on success.
async fn connect_and_pair(
    tx: &EventTx,
    config_dir: &Path,
) -> Result<bitbox_api::PairedBitBox<TokioRuntime>, ()> {
    send(tx, "connecting", serde_json::json!({})).await?;

    let hid_device = match tokio::task::spawn_blocking(bitbox_api::usb::get_any_bitbox02).await {
        Ok(Ok(d)) => d,
        Ok(Err(e)) => {
            send(
                tx,
                "hw_error",
                serde_json::json!({ "message": format!("Device not found: {e}") }),
            )
            .await?;
            return Err(());
        }
        Err(e) => {
            send(
                tx,
                "hw_error",
                serde_json::json!({ "message": format!("Task error: {e}") }),
            )
            .await?;
            return Err(());
        }
    };

    let config_dir_str = config_dir.to_string_lossy().into_owned();
    let noise_config = Box::new(PersistedNoiseConfig::new(&config_dir_str));
    let bitbox = match BitBox::<TokioRuntime>::from_hid_device(hid_device, noise_config).await {
        Ok(b) => b,
        Err(e) => {
            send(
                tx,
                "hw_error",
                serde_json::json!({ "message": format!("Connection failed: {e}") }),
            )
            .await?;
            return Err(());
        }
    };

    let pairing = match bitbox.unlock_and_pair().await {
        Ok(p) => p,
        Err(e) => {
            send(
                tx,
                "hw_error",
                serde_json::json!({ "message": format!("Unlock failed: {e}") }),
            )
            .await?;
            return Err(());
        }
    };

    if let Some(code) = pairing.get_pairing_code() {
        send(tx, "pairing_code", serde_json::json!({ "code": code })).await?;
    }
    send(tx, "waiting_confirm", serde_json::json!({})).await?;

    let paired = match pairing.wait_confirm().await {
        Ok(p) => p,
        Err(e) => {
            send(
                tx,
                "hw_error",
                serde_json::json!({ "message": format!("Pairing rejected: {e}") }),
            )
            .await?;
            return Err(());
        }
    };
    send(tx, "paired", serde_json::json!({})).await?;
    Ok(paired)
}

// ── xpub export ────────────────────────────────────────────────────────────────

pub async fn run_xpub_export(
    tx: EventTx,
    network_kind: NetworkKind,
    config_dir: std::path::PathBuf,
    account_type: String,
    account_index: u32,
) {
    let paired = match connect_and_pair(&tx, &config_dir).await {
        Ok(p) => p,
        Err(()) => return,
    };

    let is_testnet = network_kind.is_testnet_like();
    let coin = if is_testnet {
        pb::BtcCoin::Tbtc
    } else {
        pb::BtcCoin::Btc
    };
    let coin_num: u32 = if is_testnet { 1 } else { 0 };

    let (xpub_type, path) = match account_type.as_str() {
        "p2sh_segwit" => (
            if is_testnet {
                pb::btc_pub_request::XPubType::Upub
            } else {
                pb::btc_pub_request::XPubType::Ypub
            },
            format!("m/49'/{coin_num}'/{account_index}'"),
        ),
        "legacy" => (
            if is_testnet {
                pb::btc_pub_request::XPubType::Tpub
            } else {
                pb::btc_pub_request::XPubType::Xpub
            },
            format!("m/44'/{coin_num}'/{account_index}'"),
        ),
        // BIP48 native-segwit multisig (P2WSH, script type = 2')
        "multisig_p2wsh" => (
            if is_testnet {
                pb::btc_pub_request::XPubType::Tpub
            } else {
                pb::btc_pub_request::XPubType::Xpub
            },
            format!("m/48'/{coin_num}'/{account_index}'/2'"),
        ),
        // BIP86 Taproot single-key (P2TR)
        "taproot" => (
            if is_testnet {
                pb::btc_pub_request::XPubType::Tpub
            } else {
                pb::btc_pub_request::XPubType::Xpub
            },
            format!("m/86'/{coin_num}'/{account_index}'"),
        ),
        _ => (
            if is_testnet {
                pb::btc_pub_request::XPubType::Vpub
            } else {
                pb::btc_pub_request::XPubType::Zpub
            },
            format!("m/84'/{coin_num}'/{account_index}'"),
        ),
    };

    let keypath = match Keypath::try_from(path.as_str()) {
        Ok(k) => k,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Invalid keypath: {e}") }),
            )
            .await;
            return;
        }
    };

    let fingerprint = match paired.root_fingerprint().await {
        Ok(fp) => fp,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Fingerprint request failed: {e}") }),
            )
            .await;
            return;
        }
    };

    match paired.btc_xpub(coin, &keypath, xpub_type, false).await {
        Ok(xpub) => {
            let _ = send(
                &tx,
                "xpub",
                serde_json::json!({
                    "xpub": xpub,
                    "fingerprint": fingerprint,
                    "path": path,
                }),
            )
            .await;
        }
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("XPub request failed: {e}") }),
            )
            .await;
        }
    }
}

// ── Address verification ───────────────────────────────────────────────────────

pub async fn run_show_address(
    tx: EventTx,
    network_kind: NetworkKind,
    config_dir: std::path::PathBuf,
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

    let paired = match connect_and_pair(&tx, &config_dir).await {
        Ok(p) => p,
        Err(()) => return,
    };

    let is_testnet = network_kind.is_testnet_like();
    let coin = if is_testnet {
        pb::BtcCoin::Tbtc
    } else {
        pb::BtcCoin::Btc
    };

    let full_path = format!("{}/{keychain}/{address_index}", origin.base_path);
    let keypath = match Keypath::try_from(full_path.as_str()) {
        Ok(k) => k,
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Invalid keypath: {e}") }),
            )
            .await;
            return;
        }
    };

    let script_config = match origin.kind {
        SimpleScriptKind::P2Wpkh => {
            bitbox_api::btc::make_script_config_simple(pb::btc_script_config::SimpleType::P2wpkh)
        }
        SimpleScriptKind::P2WpkhP2sh => bitbox_api::btc::make_script_config_simple(
            pb::btc_script_config::SimpleType::P2wpkhP2sh,
        ),
        SimpleScriptKind::P2Tr => {
            bitbox_api::btc::make_script_config_simple(pb::btc_script_config::SimpleType::P2tr)
        }
    };

    let _ = send(&tx, "verifying", serde_json::json!({})).await;
    match paired
        .btc_address(coin, &keypath, &script_config, true)
        .await
    {
        Ok(addr) => {
            let _ = send(&tx, "address", serde_json::json!({ "address": addr })).await;
        }
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Address verification failed: {e}") }),
            )
            .await;
        }
    }
}

// ── PSBT signing ──────────────────────────────────────────────────────────────

pub async fn run_sign_psbt(
    tx: EventTx,
    network_kind: NetworkKind,
    config_dir: std::path::PathBuf,
    psbt_b64: String,
    multisig_info: Option<MultisigInfo>,
    policy_info: Option<super::common::PolicyInfo>,
    // Unused by BitBox — script-config state lives on the device, not in our
    // HMAC store. Kept in the signature so the dispatcher calls all brands
    // uniformly.
    _wallet_id: Option<uuid::Uuid>,
) {
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

    let mut psbt = match Psbt::deserialize(&psbt_bytes) {
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

    let paired = match connect_and_pair(&tx, &config_dir).await {
        Ok(p) => p,
        Err(()) => return,
    };

    let is_testnet = network_kind.is_testnet_like();
    let coin = if is_testnet {
        pb::BtcCoin::Tbtc
    } else {
        pb::BtcCoin::Btc
    };

    // For multisig wallets, register the script config on the device
    // (or check that it's already registered) before signing. The BitBox
    // refuses to sign multisig PSBTs whose script config isn't registered,
    // and the error message from the device is opaque.
    let force_script_config = if let Some(ms) = &multisig_info {
        let device_fp = match paired.root_fingerprint().await {
            Ok(fp) => fp,
            Err(e) => {
                let _ = send(
                    &tx,
                    "hw_error",
                    serde_json::json!({ "message": format!("Fingerprint request failed: {e}") }),
                )
                .await;
                return;
            }
        };
        let device_fp_str = device_fp.to_lowercase();
        let our_idx_opt = ms
            .signers
            .iter()
            .position(|s| s.fingerprint.to_string().to_lowercase() == device_fp_str);
        let Some(our_idx) = our_idx_opt else {
            let _ = send(&tx, "hw_error", serde_json::json!({
                "message": format!("This device's fingerprint ({device_fp_str}) doesn't match any cosigner in this multisig wallet. Are you using the right device?")
            })).await;
            return;
        };
        let our_signer = &ms.signers[our_idx];
        let xpubs: Vec<bdk_wallet::bitcoin::bip32::Xpub> =
            ms.signers.iter().map(|s| s.xpub).collect();
        let script_config = bitbox_api::btc::make_script_config_multisig(
            ms.threshold,
            &xpubs,
            our_idx as u32,
            pb::btc_script_config::multisig::ScriptType::P2wsh,
        );
        let our_keypath = bitbox_api::Keypath::from(&our_signer.account_path);

        let _ = send(&tx, "verifying", serde_json::json!({})).await;
        let registered = match paired
            .btc_is_script_config_registered(coin, &script_config, Some(&our_keypath))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = send(
                    &tx,
                    "hw_error",
                    serde_json::json!({ "message": format!("Could not check script config: {e}") }),
                )
                .await;
                return;
            }
        };
        if !registered {
            let _ = send(&tx, "registering", serde_json::json!({})).await;
            if let Err(e) = paired
                .btc_register_script_config(
                    coin,
                    &script_config,
                    Some(&our_keypath),
                    pb::btc_register_script_config_request::XPubType::AutoElectrum,
                    Some(&ms.label),
                )
                .await
            {
                let _ = send(&tx, "hw_error", serde_json::json!({
                    "message": format!("Multisig registration failed: {e}. Confirm on the device when prompted.")
                })).await;
                return;
            }
        }

        Some(pb::BtcScriptConfigWithKeypath {
            script_config: Some(script_config),
            keypath: our_keypath.to_vec(),
        })
    } else if let Some(pol) = &policy_info {
        use std::str::FromStr;
        let device_fp = match paired.root_fingerprint().await {
            Ok(fp) => fp.to_lowercase(),
            Err(e) => {
                let _ = send(
                    &tx,
                    "hw_error",
                    serde_json::json!({ "message": format!("Fingerprint request failed: {e}") }),
                )
                .await;
                return;
            }
        };

        // Build the BIP-388 key list in @i order; find the device's key path.
        let mut key_origins: Vec<bitbox_api::btc::KeyOriginInfo> =
            Vec::with_capacity(pol.keys.len());
        let mut our_keypath: Option<bitbox_api::Keypath> = None;
        for k in &pol.keys {
            let fp = match bdk_wallet::bitcoin::bip32::Fingerprint::from_str(&k.fingerprint) {
                Ok(f) => f,
                Err(e) => {
                    let _ = send(&tx, "hw_error", serde_json::json!({ "message": format!("Bad key fingerprint in policy: {e}") })).await;
                    return;
                }
            };
            let kp = match bitbox_api::Keypath::try_from(k.path.as_str()) {
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
            if our_keypath.is_none() && k.fingerprint.to_lowercase() == device_fp {
                our_keypath = Some(kp.clone());
            }
            key_origins.push(bitbox_api::btc::KeyOriginInfo {
                root_fingerprint: Some(fp),
                keypath: Some(kp),
                xpub,
            });
        }
        let Some(our_keypath) = our_keypath else {
            let _ = send(&tx, "hw_error", serde_json::json!({
                "message": format!("This device's fingerprint ({device_fp}) doesn't match any key in this wallet's policy. Are you using the right device?")
            })).await;
            return;
        };

        let script_config = bitbox_api::btc::make_script_config_policy(&pol.template, &key_origins);

        let _ = send(&tx, "verifying", serde_json::json!({})).await;
        let registered = match paired
            .btc_is_script_config_registered(coin, &script_config, Some(&our_keypath))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = send(&tx, "hw_error", serde_json::json!({ "message": format!("Could not check policy registration: {e}") })).await;
                return;
            }
        };
        if !registered {
            let _ = send(&tx, "registering", serde_json::json!({})).await;
            if let Err(e) = paired
                .btc_register_script_config(
                    coin,
                    &script_config,
                    Some(&our_keypath),
                    pb::btc_register_script_config_request::XPubType::AutoElectrum,
                    Some(&pol.label),
                )
                .await
            {
                let _ = send(&tx, "hw_error", serde_json::json!({
                    "message": format!("Policy registration failed: {e}. Confirm on the device when prompted.")
                })).await;
                return;
            }
        }

        Some(pb::BtcScriptConfigWithKeypath {
            script_config: Some(script_config),
            keypath: our_keypath.to_vec(),
        })
    } else {
        None
    };

    let _ = send(&tx, "signing", serde_json::json!({})).await;
    match paired
        .btc_sign_psbt(
            coin,
            &mut psbt,
            force_script_config,
            pb::btc_sign_init_request::FormatUnit::Default,
        )
        .await
    {
        Ok(_) => {
            // For singlesig, finalize partial sigs into full witness/scriptSig.
            // For multisig, keep partial sigs so combine_psbt can aggregate
            // signatures from all cosigners.
            if multisig_info.is_none() {
                finalize_psbt_inputs(&mut psbt);
            }
            let signed_b64 = STANDARD.encode(psbt.serialize());
            let _ = send(&tx, "signed", serde_json::json!({ "psbt": signed_b64 })).await;
        }
        Err(e) => {
            let _ = send(
                &tx,
                "hw_error",
                serde_json::json!({ "message": format!("Signing failed: {e}") }),
            )
            .await;
        }
    }
}
