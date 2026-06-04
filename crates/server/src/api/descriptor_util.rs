//! Shared helpers used across all hardware-wallet brand modules:
//! descriptor parsing, multisig info extraction, and PSBT finalization.
//!
//! Always compiled (non-hardware signing uses `parse_descriptor_origin` etc.), but
//! several helpers are only used by the USB device modules, so they read as dead
//! code in the `hw`-off (Start9) build.
#![cfg_attr(not(feature = "hw"), allow(dead_code))]

use bdk_wallet::bitcoin::{
    bip32::{DerivationPath, Fingerprint, Xpub},
    psbt::Psbt,
    OutPoint, ScriptBuf, Witness,
};

/// One participant in a multisig wallet — extracted from a stored descriptor.
pub struct MultisigSignerSpec {
    pub fingerprint: Fingerprint,
    /// Account-level path, e.g. "m/48'/0'/0'/2'" (no /0/* or /1/* suffix).
    pub account_path: DerivationPath,
    pub xpub: Xpub,
}

pub struct MultisigInfo {
    pub threshold: u32,
    pub signers: Vec<MultisigSignerSpec>,
    pub label: String,
}

/// Strip the optional `#xxxxxxxx` checksum suffix from a BDK/miniscript
/// descriptor string so the rest of the parser can ignore it.
pub fn strip_checksum(desc: &str) -> &str {
    if let Some(hash_pos) = desc.rfind('#') {
        // Checksum is exactly 8 lowercase alphanumeric chars; anything else
        // means '#' is part of the descriptor body (shouldn't happen with
        // BDK output, but stay conservative).
        let after = &desc[hash_pos + 1..];
        if after.len() == 8 && after.chars().all(|c| c.is_ascii_alphanumeric()) {
            return &desc[..hash_pos];
        }
    }
    desc
}

/// Parse a `wsh(sortedmulti(M, [fp/path]xpub/0/*, ...))` descriptor and pull
/// out the threshold and each signer's origin info. Returns None for any
/// shape we don't recognise (multi/non-wsh/etc.).
pub fn parse_wsh_sortedmulti(desc: &str) -> Option<(u32, Vec<MultisigSignerSpec>)> {
    use std::str::FromStr;
    let desc = strip_checksum(desc);
    let (_, after_open) = desc
        .strip_prefix("wsh(sortedmulti(")
        .map(|s| ("sortedmulti", s))
        .or_else(|| desc.strip_prefix("wsh(multi(").map(|s| ("multi", s)))?;
    let inner = after_open.rsplit_once("))")?.0;
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() < 3 {
        return None;
    }
    let threshold: u32 = parts[0].trim().parse().ok()?;
    let mut signers = Vec::with_capacity(parts.len() - 1);
    for p in &parts[1..] {
        let s = p.trim();
        let open = s.find('[')?;
        let close_rel = s[open..].find(']')?;
        let close = open + close_rel;
        let origin = &s[open + 1..close];
        let slash = origin.find('/')?;
        let fp_hex = &origin[..slash];
        let path_str = &origin[slash + 1..];
        let fingerprint = Fingerprint::from_str(fp_hex).ok()?;
        let account_path = DerivationPath::from_str(&format!("m/{path_str}")).ok()?;
        let after = &s[close + 1..];
        // Strip the keychain suffix ("/0/*" or "/1/*") and parse the rest as an xpub.
        let xpub_str = after.trim_end_matches("/0/*").trim_end_matches("/1/*");
        let xpub = Xpub::from_str(xpub_str).ok()?;
        signers.push(MultisigSignerSpec {
            fingerprint,
            account_path,
            xpub,
        });
    }
    Some((threshold, signers))
}

/// Script types we know how to ask a device to display directly. Multisig and
/// exotic scripts aren't covered here — they'd need the wallet's full
/// descriptor to be registered with the device first via the brand-specific
/// equivalent of `btc_register_script_config`.
pub enum SimpleScriptKind {
    P2Wpkh,     // BIP84 — native segwit
    P2WpkhP2sh, // BIP49 — wrapped segwit
    P2Tr,       // BIP86 — taproot
}

pub struct DescriptorOrigin {
    /// BIP32 path of the account ("m/84'/0'/0'" etc.), without the receive/change suffix.
    pub base_path: String,
    pub kind: SimpleScriptKind,
}

/// Pull the `[fingerprint/path]` origin + the outer script type out of a BDK
/// descriptor string like `wpkh([abcd1234/84'/0'/0']xpub...AB/0/*)`.
/// Returns None for descriptors that don't have an origin or aren't a flavour
/// the device can display directly (e.g. multisig).
pub fn parse_descriptor_origin(descriptor: &str) -> Option<DescriptorOrigin> {
    let descriptor = strip_checksum(descriptor);
    let kind = if descriptor.starts_with("wpkh(") {
        SimpleScriptKind::P2Wpkh
    } else if descriptor.starts_with("sh(wpkh(") {
        SimpleScriptKind::P2WpkhP2sh
    } else if descriptor.starts_with("tr(") {
        SimpleScriptKind::P2Tr
    } else {
        return None;
    };
    // Find the bracketed origin "[fingerprint/path]".
    let open = descriptor.find('[')?;
    let close = descriptor[open..].find(']')?;
    let inside = &descriptor[open + 1..open + close];
    let slash = inside.find('/')?;
    let path = &inside[slash + 1..];
    Some(DescriptorOrigin {
        base_path: format!("m/{path}"),
        kind,
    })
}

/// Finalize a PSBT that has been signed by a hardware wallet
/// (partial_sigs populated). Supports P2WPKH, P2SH-P2WPKH, and P2PKH inputs.
pub fn finalize_psbt_inputs(psbt: &mut Psbt) {
    enum Kind {
        P2Wpkh,
        P2ShP2Wpkh(ScriptBuf),
        P2Pkh,
        Unknown,
    }

    let prev_outpoints: Vec<OutPoint> = psbt
        .unsigned_tx
        .input
        .iter()
        .map(|i| i.previous_output)
        .collect();

    for (idx, input) in psbt.inputs.iter_mut().enumerate() {
        if input.final_script_witness.is_some() || input.final_script_sig.is_some() {
            continue;
        }

        // ── Taproot key-spend ─────────────────────────────────────────────
        // BIP-86 / P2TR signatures come back from the device in `tap_key_sig`
        // rather than `partial_sigs`. The witness is a single item: the
        // Schnorr signature, optionally with a trailing sighash byte if the
        // sighash type is anything other than Default. Finalize this before
        // the ECDSA branch so Taproot inputs don't fall through into the
        // "no partial_sigs → skip" path.
        if let Some(tap_sig) = input.tap_key_sig {
            let is_p2tr = input
                .witness_utxo
                .as_ref()
                .map(|u| u.script_pubkey.is_p2tr())
                .unwrap_or(false);
            if is_p2tr {
                let mut wit = Witness::new();
                wit.push(tap_sig.to_vec());
                input.final_script_witness = Some(wit);
                input.tap_key_sig = None;
                input.tap_internal_key = None;
                input.tap_merkle_root = None;
                input.tap_key_origins.clear();
                input.bip32_derivation.clear();
                input.sighash_type = None;
                continue;
            }
        }

        let Some((&pk, &sig)) = input.partial_sigs.iter().next() else {
            continue;
        };

        let kind = if input
            .witness_utxo
            .as_ref()
            .map(|u| u.script_pubkey.is_p2wpkh())
            .unwrap_or(false)
        {
            Kind::P2Wpkh
        } else if input
            .witness_utxo
            .as_ref()
            .map(|u| u.script_pubkey.is_p2sh())
            .unwrap_or(false)
        {
            match input.redeem_script.as_ref() {
                Some(r) if r.is_p2wpkh() => Kind::P2ShP2Wpkh(r.clone()),
                _ => Kind::Unknown,
            }
        } else {
            let vout = prev_outpoints.get(idx).map(|o| o.vout).unwrap_or(0) as usize;
            let is_p2pkh = input
                .non_witness_utxo
                .as_ref()
                .and_then(|tx| tx.output.get(vout))
                .map(|o| o.script_pubkey.is_p2pkh())
                .unwrap_or(false);
            if is_p2pkh {
                Kind::P2Pkh
            } else {
                Kind::Unknown
            }
        };

        match kind {
            Kind::P2Wpkh => {
                let mut wit = Witness::new();
                wit.push_ecdsa_signature(&sig);
                wit.push(pk.to_bytes());
                input.final_script_witness = Some(wit);
            }
            Kind::P2ShP2Wpkh(redeem) => {
                let mut wit = Witness::new();
                wit.push_ecdsa_signature(&sig);
                wit.push(pk.to_bytes());
                input.final_script_witness = Some(wit);
                input.final_script_sig = Some(push_data_script(redeem.as_bytes()));
            }
            Kind::P2Pkh => {
                input.final_script_sig = Some(push_two_data_script(&sig.to_vec(), &pk.to_bytes()));
            }
            Kind::Unknown => continue,
        }

        input.partial_sigs.clear();
        input.bip32_derivation.clear();
        input.sighash_type = None;
        input.redeem_script = None;
        input.witness_script = None;
    }
}

fn push_data_script(data: &[u8]) -> ScriptBuf {
    let n = data.len();
    let mut v = Vec::with_capacity(n + 3);
    if n < 76 {
        v.push(n as u8);
    } else if n < 256 {
        v.push(0x4c); // OP_PUSHDATA1
        v.push(n as u8);
    } else {
        v.push(0x4d); // OP_PUSHDATA2
        v.extend_from_slice(&(n as u16).to_le_bytes());
    }
    v.extend_from_slice(data);
    ScriptBuf::from_bytes(v)
}

fn push_two_data_script(a: &[u8], b: &[u8]) -> ScriptBuf {
    fn append(v: &mut Vec<u8>, d: &[u8]) {
        let n = d.len();
        if n < 76 {
            v.push(n as u8);
        } else {
            v.push(0x4c);
            v.push(n as u8);
        }
        v.extend_from_slice(d);
    }
    let mut v = Vec::new();
    append(&mut v, a);
    append(&mut v, b);
    ScriptBuf::from_bytes(v)
}

/// One key in a BIP-388 wallet policy: an account xpub plus its origin. The
/// per-address `/<0;1>/*` suffix lives in the template's `@i/**`, not here.
#[derive(Clone)]
pub struct PolicyKey {
    pub fingerprint: String,
    /// Origin path, e.g. "m/48'/0'/0'/2'".
    pub path: String,
    pub xpub: String,
}

/// A wallet's miniscript/taproot spending policy, ready for HW registration.
/// `keys` are ordered to match the `@0,@1,…` placeholders in `template`.
pub struct PolicyInfo {
    pub template: String,
    pub keys: Vec<PolicyKey>,
    pub label: String,
}

/// Convert a wallet's (external) descriptor into a BIP-388 wallet policy:
/// a template with `@0/**`, `@1/**`, … placeholders plus the ordered key list.
/// Both the BitBox and Ledger flows consume this; each maps `PolicyKey` to its
/// own key type.
///
/// Errors if any key lacks a `[fingerprint/path]` origin (devices need it) or
/// isn't an extended key — notably the taproot-savings NUMS internal key, which
/// can't be expressed as a BIP-388 key, so such wallets fall back to software.
pub fn descriptor_to_wallet_policy(external: &str) -> anyhow::Result<(String, Vec<PolicyKey>)> {
    use bdk_wallet::miniscript::{Descriptor, DescriptorPublicKey, ForEachKey};
    use std::str::FromStr;

    let stripped = strip_checksum(external.trim());
    let desc = Descriptor::<DescriptorPublicKey>::from_str(stripped)
        .map_err(|e| anyhow::anyhow!("can't parse descriptor: {e}"))?;

    // Collect keys in first-seen order so @i indices are stable.
    let mut keys: Vec<DescriptorPublicKey> = Vec::new();
    desc.for_each_key(|k| {
        if !keys.iter().any(|x| x == k) {
            keys.push(k.clone());
        }
        true
    });

    let mut policy_keys = Vec::with_capacity(keys.len());
    for k in &keys {
        match k {
            DescriptorPublicKey::XPub(xk) => {
                let (fp, path) = xk.origin.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "a descriptor key has no [fingerprint/path] origin — hardware signing needs it"
                    )
                })?;
                policy_keys.push(PolicyKey {
                    fingerprint: fp.to_string(),
                    path: format!("m/{path}"),
                    xpub: xk.xkey.to_string(),
                });
            }
            _ => anyhow::bail!(
                "descriptor contains a non-extended key (e.g. a taproot NUMS internal key); \
                 hardware wallet-policy signing isn't supported for it — use software signing"
            ),
        }
    }

    // Build the template from the canonical re-serialization so each key's
    // Display string is guaranteed to be a substring we can replace with @i.
    let canonical = desc.to_string();
    let mut template = strip_checksum(&canonical).to_string();
    for (i, k) in keys.iter().enumerate() {
        template = template.replace(&k.to_string(), &format!("@{i}/**"));
    }

    Ok((template, policy_keys))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdk_wallet::bitcoin::{
        absolute::LockTime,
        hashes::Hash,
        secp256k1::{Keypair, Message, Secp256k1, SecretKey},
        sighash::TapSighashType,
        taproot,
        transaction::Version,
        Amount, Sequence, Transaction, TxIn, TxOut, Txid,
    };

    /// A 1-input PSBT spending a key-path-only P2TR output, with `tap_key_sig`
    /// already populated (as a device would leave it pre-finalize).
    fn p2tr_psbt_with_tap_sig(sighash_type: TapSighashType) -> Psbt {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x42; 32]).unwrap();
        let (xonly, _) = sk.x_only_public_key(&secp);
        let spk = ScriptBuf::new_p2tr(&secp, xonly, None);

        let txin = TxIn {
            previous_output: OutPoint {
                txid: Txid::all_zeros(),
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        };
        let txout = TxOut {
            value: Amount::from_sat(9_000),
            script_pubkey: spk.clone(),
        };
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![txin],
            output: vec![txout],
        };
        let mut psbt = Psbt::from_unsigned_tx(tx).unwrap();

        // A real Schnorr sig over an arbitrary digest — finalize doesn't verify
        // it, but this dodges any from_slice API churn.
        let keypair = Keypair::from_secret_key(&secp, &sk);
        let sig = secp.sign_schnorr_no_aux_rand(&Message::from_digest([0x11; 32]), &keypair);

        psbt.inputs[0].witness_utxo = Some(TxOut {
            value: Amount::from_sat(10_000),
            script_pubkey: spk,
        });
        psbt.inputs[0].tap_internal_key = Some(xonly);
        psbt.inputs[0].tap_key_sig = Some(taproot::Signature {
            signature: sig,
            sighash_type,
        });
        psbt
    }

    fn signer(seed: u8, path: &str) -> corvin_core::descriptor::MultisigSigner {
        use bdk_wallet::bitcoin::{bip32::Xpriv, bip32::Xpub, secp256k1::Secp256k1, Network};
        let secp = Secp256k1::new();
        let xpriv = Xpriv::new_master(Network::Bitcoin, &[seed; 32]).unwrap();
        let xpub = Xpub::from_priv(&secp, &xpriv).to_string();
        corvin_core::descriptor::MultisigSigner {
            fingerprint: format!("{:08x}", 0x1000_0000u32 + seed as u32),
            path: path.to_string(),
            xpub,
        }
    }

    #[test]
    fn wsh_vault_to_policy_template() {
        use bdk_wallet::bitcoin::Network;
        use corvin_core::descriptor::{descriptor_from_inheritance_vault, VaultTimelock};
        let p = [signer(1, "m/48'/0'/0'/2'")];
        let r = [signer(2, "m/48'/0'/0'/2'")];
        let v = descriptor_from_inheritance_vault(
            1,
            &p,
            1,
            &r,
            VaultTimelock::RelativeBlocks(26280),
            Network::Bitcoin,
        )
        .unwrap();
        let (template, keys) = descriptor_to_wallet_policy(&v.external).unwrap();
        assert_eq!(keys.len(), 2);
        assert!(
            template.contains("@0/**") && template.contains("@1/**"),
            "template: {template}"
        );
        assert!(template.contains("older(26280)"), "template: {template}");
        assert!(
            !template.contains("xpub"),
            "keys must be replaced by placeholders: {template}"
        );
        assert!(keys[0].fingerprint.len() == 8 && keys[0].path.starts_with("m/48'"));
    }

    #[test]
    fn taproot_vault_to_policy_template() {
        use bdk_wallet::bitcoin::Network;
        use corvin_core::descriptor::{descriptor_from_taproot_vault, VaultTimelock};
        let p = signer(1, "m/86'/0'/0'");
        let r = [signer(2, "m/86'/0'/0'")];
        let v = descriptor_from_taproot_vault(
            &p,
            1,
            &r,
            VaultTimelock::RelativeBlocks(26280),
            Network::Bitcoin,
        )
        .unwrap();
        let (template, keys) = descriptor_to_wallet_policy(&v.external).unwrap();
        // Key-path placeholder + a tapleaf placeholder; for_each_key order isn't
        // guaranteed, so just assert structure + consistent @i/keys pairing.
        assert!(template.starts_with("tr(@"), "taproot template: {template}");
        assert!(
            template.contains("@0/**") && template.contains("@1/**"),
            "template: {template}"
        );
        assert!(
            template.contains("older(26280)") && !template.contains("xpub"),
            "template: {template}"
        );
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn taproot_savings_nums_rejected_for_hw_policy() {
        use bdk_wallet::bitcoin::Network;
        use corvin_core::descriptor::{descriptor_from_taproot_savings, VaultTimelock};
        let s = signer(1, "m/86'/0'/0'");
        let v = descriptor_from_taproot_savings(
            &s,
            VaultTimelock::RelativeBlocks(1000),
            Network::Bitcoin,
        )
        .unwrap();
        // NUMS internal key can't be a BIP-388 key → must error (software fallback).
        assert!(descriptor_to_wallet_policy(&v.external).is_err());
    }

    #[test]
    fn finalizes_taproot_keyspend_default_sighash() {
        let mut psbt = p2tr_psbt_with_tap_sig(TapSighashType::Default);
        finalize_psbt_inputs(&mut psbt);
        let input = &psbt.inputs[0];
        let wit = input
            .final_script_witness
            .as_ref()
            .expect("witness finalized");
        assert_eq!(wit.len(), 1, "key-spend witness is a single item");
        assert_eq!(
            wit.iter().next().unwrap().len(),
            64,
            "Default sighash → bare 64-byte schnorr sig"
        );
        // Signing material cleared so the finalized PSBT is clean.
        assert!(input.tap_key_sig.is_none());
        assert!(input.tap_internal_key.is_none());
        assert!(input.sighash_type.is_none());
    }

    #[test]
    fn finalizes_taproot_keyspend_nondefault_sighash_appends_byte() {
        let mut psbt = p2tr_psbt_with_tap_sig(TapSighashType::All);
        finalize_psbt_inputs(&mut psbt);
        let wit = psbt.inputs[0].final_script_witness.as_ref().unwrap();
        assert_eq!(
            wit.iter().next().unwrap().len(),
            65,
            "non-Default sighash → 64-byte sig + trailing sighash byte"
        );
    }

    #[test]
    fn leaves_taproot_input_without_sig_untouched() {
        let mut psbt = p2tr_psbt_with_tap_sig(TapSighashType::Default);
        psbt.inputs[0].tap_key_sig = None;
        finalize_psbt_inputs(&mut psbt);
        assert!(
            psbt.inputs[0].final_script_witness.is_none(),
            "no tap_key_sig → input must not be finalized"
        );
    }
}
