//! Sweep a single private key (WIF) into a destination address.
//!
//! The WIF is parsed in memory, used to build transient single-key BDK
//! wallets (one per compatible script type), synced against the configured
//! Electrum server, and — when funds are found — used to build and sign a
//! drain transaction. The signed tx hex is returned for the frontend to feed
//! into the existing /broadcast endpoint.
//!
//! The WIF never touches disk: descriptors live in `Zeroizing<String>`, the
//! transient wallets use `create_wallet_no_persist()`, and the request
//! payload is dropped before the response is sent.

use axum::{extract::State, Json};
use bdk_electrum::electrum_client::ElectrumApi;
use bdk_electrum::BdkElectrumClient;
use bdk_wallet::{
    bitcoin::{
        consensus::encode::serialize, secp256k1::Secp256k1, Address, CompressedPublicKey, FeeRate,
        Network, PrivateKey, ScriptBuf,
    },
    KeychainKind, SignOptions, Wallet,
};
use corvin_core::backends::electrum::{
    build_client, build_native_tls_client, needs_custom_tls, ElectrumConfig,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use zeroize::Zeroizing;

use crate::api::ApiError;
use crate::state::AppState;

const STOP_GAP: usize = 20;
const BATCH_SIZE: usize = 5;

#[derive(Deserialize)]
pub struct SweepRequest {
    pub wif: String,
    pub destination: String,
    pub fee_rate_sat_vb: f64,
}

#[derive(Serialize)]
pub struct SweepCandidate {
    /// One of "p2pkh", "p2wpkh", "p2sh-p2wpkh".
    pub script_type: String,
    pub address: String,
    pub utxos_found: usize,
    pub input_sats: u64,
    pub output_sats: u64,
    pub fee_sats: u64,
    pub signed_tx_hex: String,
    pub txid: String,
}

#[derive(Serialize)]
pub struct SweepPreview {
    /// Per-script-type results where UTXOs were found and a signed sweep tx
    /// was successfully built. The frontend renders one row per entry; each
    /// row can be broadcast independently via the existing /broadcast endpoint.
    pub found: Vec<SweepCandidate>,
    /// Script types we scanned but found empty (so the user knows we checked).
    pub empty: Vec<String>,
}

pub async fn preview_sweep(
    State(state): State<AppState>,
    Json(req): Json<SweepRequest>,
) -> Result<Json<SweepPreview>, ApiError> {
    let cfg = state.config.read().await;
    let network: Network = cfg.network.kind.to_bitcoin_network();
    let ecfg = cfg.electrum_config();
    drop(cfg);

    let wif_input = Zeroizing::new(req.wif.trim().to_string());
    let priv_key =
        PrivateKey::from_wif(&wif_input).map_err(|e| anyhow::anyhow!("invalid WIF: {e}"))?;
    if priv_key.network != network.into() {
        return Err(anyhow::anyhow!(
            "WIF is for {:?}, wallet is on {:?}",
            priv_key.network,
            network,
        )
        .into());
    }

    let dest_addr = Address::from_str(&req.destination)
        .map_err(|e| anyhow::anyhow!("invalid destination: {e}"))?
        .require_network(network)
        .map_err(|_| anyhow::anyhow!("destination is for the wrong network"))?;

    // Same NaN/upper-bound guard as the send path (defense-in-depth).
    let fee_rate_sats = crate::api::wallets::send::validated_fee_rate_sats(req.fee_rate_sat_vb)?;
    let fee_rate = FeeRate::from_sat_per_vb(fee_rate_sats)
        .ok_or_else(|| anyhow::anyhow!("invalid fee rate"))?;

    let candidates = sweep_candidates(&priv_key, &wif_input, network)?;

    let dest_script = dest_addr.script_pubkey();
    let mut found = Vec::new();
    let mut empty = Vec::new();

    for (kind, desc, addr_str) in candidates {
        let ecfg = clone_ecfg(&ecfg);
        let dest_script = dest_script.clone();
        let kind_owned = kind.to_string();
        let result =
            tokio::task::spawn_blocking(move || -> anyhow::Result<Option<SweepCandidate>> {
                sweep_one(
                    &desc,
                    &ecfg,
                    dest_script,
                    fee_rate,
                    network,
                    &kind_owned,
                    &addr_str,
                )
            })
            .await
            .map_err(|e| anyhow::anyhow!("sweep task panicked: {e}"))?;

        match result {
            Ok(Some(c)) => found.push(c),
            Ok(None) => empty.push(kind.to_string()),
            Err(e) => {
                tracing::warn!("sweep {kind} failed: {e:#}");
                empty.push(format!("{kind} (error: {e})"));
            }
        }
    }

    Ok(Json(SweepPreview { found, empty }))
}

/// The `(script_type, single-key descriptor, address)` set to scan for a swept
/// WIF. A compressed key can have paid into all three single-key script types;
/// an uncompressed key only into legacy p2pkh. The descriptor carries the WIF,
/// so it stays in `Zeroizing`.
fn sweep_candidates(
    priv_key: &PrivateKey,
    wif: &str,
    network: Network,
) -> anyhow::Result<Vec<(&'static str, Zeroizing<String>, String)>> {
    let secp = Secp256k1::new();
    let pubkey = priv_key.public_key(&secp);
    let mut candidates: Vec<(&'static str, Zeroizing<String>, String)> = Vec::new();
    if priv_key.compressed {
        let comp = CompressedPublicKey::try_from(pubkey)
            .map_err(|e| anyhow::anyhow!("compressed pubkey: {e}"))?;
        candidates.push((
            "p2wpkh",
            Zeroizing::new(format!("wpkh({wif})")),
            Address::p2wpkh(&comp, network).to_string(),
        ));
        candidates.push((
            "p2sh-p2wpkh",
            Zeroizing::new(format!("sh(wpkh({wif}))")),
            Address::p2shwpkh(&comp, network).to_string(),
        ));
        candidates.push((
            "p2pkh",
            Zeroizing::new(format!("pkh({wif})")),
            Address::p2pkh(pubkey, network).to_string(),
        ));
    } else {
        candidates.push((
            "p2pkh",
            Zeroizing::new(format!("pkh({wif})")),
            Address::p2pkh(pubkey, network).to_string(),
        ));
    }
    Ok(candidates)
}

fn sweep_one(
    descriptor: &str,
    ecfg: &ElectrumConfig,
    dest_script: ScriptBuf,
    fee_rate: FeeRate,
    network: Network,
    kind: &str,
    address: &str,
) -> anyhow::Result<Option<SweepCandidate>> {
    let mut wallet = Wallet::create_single(descriptor.to_string())
        .network(network)
        .create_wallet_no_persist()
        .map_err(|e| anyhow::anyhow!("descriptor build: {e}"))?;

    if needs_custom_tls(ecfg) {
        let client = build_native_tls_client(ecfg).map_err(|e| anyhow::anyhow!("{e}"))?;
        sync_wallet(&client, &mut wallet)?;
    } else {
        let client = build_client(ecfg).map_err(|e| anyhow::anyhow!("{e}"))?;
        sync_wallet(&client, &mut wallet)?;
    }

    build_sweep_tx(&mut wallet, dest_script, fee_rate, kind, address)
}

/// Build + sign the drain tx for an already-synced single-key wallet. `None`
/// when the wallet holds nothing. Split out of `sweep_one` so the money logic
/// (full drain + signing + fee math) is testable against an injected funded
/// wallet, with no Electrum backend — only the sync ahead of it needs the network.
fn build_sweep_tx(
    wallet: &mut Wallet,
    dest_script: ScriptBuf,
    fee_rate: FeeRate,
    kind: &str,
    address: &str,
) -> anyhow::Result<Option<SweepCandidate>> {
    if wallet.balance().total().to_sat() == 0 {
        return Ok(None);
    }

    let utxos_found = wallet.list_unspent().count();
    let input_sats: u64 = wallet.list_unspent().map(|u| u.txout.value.to_sat()).sum();

    let mut psbt = {
        let mut builder = wallet.build_tx();
        builder.fee_rate(fee_rate);
        builder.drain_wallet();
        builder.drain_to(dest_script);
        builder
            .finish()
            .map_err(|e| anyhow::anyhow!("build_tx: {e}"))?
    };

    let finalized = wallet
        .sign(&mut psbt, SignOptions::default())
        .map_err(|e| anyhow::anyhow!("sign: {e}"))?;
    if !finalized {
        return Err(anyhow::anyhow!("PSBT did not fully finalize after signing"));
    }

    let tx = psbt
        .extract_tx()
        .map_err(|e| anyhow::anyhow!("extract_tx: {e}"))?;

    let output_sats: u64 = tx.output.iter().map(|o| o.value.to_sat()).sum();
    let fee_sats = input_sats.saturating_sub(output_sats);

    Ok(Some(SweepCandidate {
        script_type: kind.to_string(),
        address: address.to_string(),
        utxos_found,
        input_sats,
        output_sats,
        fee_sats,
        signed_tx_hex: hex::encode(serialize(&tx)),
        txid: tx.compute_txid().to_string(),
    }))
}

fn sync_wallet<E: ElectrumApi>(
    client: &BdkElectrumClient<E>,
    wallet: &mut Wallet,
) -> anyhow::Result<()> {
    // Single-key descriptors expose a fixed script (no derivation). One
    // reveal call gives us the address index 0 SPK.
    let _ = wallet.reveal_next_address(KeychainKind::External);
    let update = client
        .full_scan(wallet.start_full_scan(), STOP_GAP, BATCH_SIZE, false)
        .map_err(|e| anyhow::anyhow!("electrum full_scan: {e}"))?;
    wallet
        .apply_update(update)
        .map_err(|e| anyhow::anyhow!("apply_update: {e}"))?;
    Ok(())
}

fn clone_ecfg(c: &ElectrumConfig) -> ElectrumConfig {
    ElectrumConfig {
        url: c.url.clone(),
        validate_tls: c.validate_tls,
        ca_cert_path: c.ca_cert_path.clone(),
        danger_accept_invalid_certs: c.danger_accept_invalid_certs,
        socks5_proxy: c.socks5_proxy.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bdk_wallet::bitcoin::secp256k1::SecretKey;

    fn key(compressed: bool) -> PrivateKey {
        let sk = SecretKey::from_slice(&[0x11u8; 32]).unwrap();
        if compressed {
            PrivateKey::new(sk, Network::Bitcoin)
        } else {
            PrivateKey::new_uncompressed(sk, Network::Bitcoin)
        }
    }

    fn addr_type(s: &str) -> bdk_wallet::bitcoin::AddressType {
        Address::from_str(s)
            .unwrap()
            .assume_checked()
            .address_type()
            .expect("a standard address type")
    }

    #[test]
    fn compressed_wif_yields_all_three_script_types() {
        use bdk_wallet::bitcoin::AddressType;
        let pk = key(true);
        let wif = pk.to_wif();
        let cands = sweep_candidates(&pk, &wif, Network::Bitcoin).unwrap();

        let kinds: Vec<&str> = cands.iter().map(|(k, _, _)| *k).collect();
        assert_eq!(kinds, ["p2wpkh", "p2sh-p2wpkh", "p2pkh"]);

        // Each derived address must actually be of the script type it claims —
        // catches a mislabel between descriptor and address.
        for (kind, _desc, addr) in &cands {
            let expected = match *kind {
                "p2wpkh" => AddressType::P2wpkh,
                "p2sh-p2wpkh" => AddressType::P2sh,
                "p2pkh" => AddressType::P2pkh,
                other => panic!("unexpected kind {other}"),
            };
            assert_eq!(addr_type(addr), expected, "{kind} address is the wrong type");
        }

        // The three addresses for one key are all distinct.
        let mut uniq: Vec<&String> = cands.iter().map(|(_, _, a)| a).collect();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), 3, "the three script types give three distinct addresses");
    }

    #[test]
    fn uncompressed_wif_is_legacy_only() {
        use bdk_wallet::bitcoin::AddressType;
        let pk = key(false);
        let wif = pk.to_wif();
        let cands = sweep_candidates(&pk, &wif, Network::Bitcoin).unwrap();
        assert_eq!(cands.len(), 1, "an uncompressed key only sweeps legacy p2pkh");
        assert_eq!(cands[0].0, "p2pkh");
        assert_eq!(addr_type(&cands[0].2), AddressType::P2pkh);
    }

    /// Fund a transient single-key wallet via the BDK test harness (no node) and
    /// drive the extracted drain+sign logic — the part of `sweep_one` that runs
    /// after the (network-only) sync.
    fn funded_single_key(sk: [u8; 32], sats: u64) -> (Wallet, Network) {
        use bdk_wallet::bitcoin::{hashes::Hash, Amount, BlockHash};
        use bdk_wallet::chain::BlockId;
        use bdk_wallet::test_utils::{insert_checkpoint, receive_output_in_latest_block};

        let net = Network::Regtest;
        let wif = PrivateKey::new(SecretKey::from_slice(&sk).unwrap(), net).to_wif();
        let mut wallet = Wallet::create_single(format!("wpkh({wif})"))
            .network(net)
            .create_wallet_no_persist()
            .unwrap();
        insert_checkpoint(
            &mut wallet,
            BlockId { height: 1_000, hash: BlockHash::all_zeros() },
        );
        let _ = wallet.reveal_next_address(KeychainKind::External);
        if sats > 0 {
            receive_output_in_latest_block(&mut wallet, Amount::from_sat(sats));
        }
        (wallet, net)
    }

    fn p2wpkh_dest(sk: [u8; 32], net: Network) -> Address {
        let pk = PrivateKey::new(SecretKey::from_slice(&sk).unwrap(), net);
        let comp = CompressedPublicKey::try_from(pk.public_key(&Secp256k1::new())).unwrap();
        Address::p2wpkh(&comp, net)
    }

    #[test]
    fn build_sweep_tx_drains_a_funded_wallet_to_one_signed_output() {
        let (mut wallet, net) = funded_single_key([0x11u8; 32], 120_000);
        let dest = p2wpkh_dest([0x22u8; 32], net);

        let out = build_sweep_tx(
            &mut wallet,
            dest.script_pubkey(),
            FeeRate::from_sat_per_vb(2).unwrap(),
            "p2wpkh",
            &dest.to_string(),
        )
        .unwrap()
        .expect("a funded wallet yields a sweep candidate");

        assert_eq!(out.input_sats, 120_000, "the whole balance is drained");
        assert_eq!(out.utxos_found, 1);
        assert!(out.fee_sats > 0, "a fee is charged");
        assert_eq!(
            out.output_sats,
            out.input_sats - out.fee_sats,
            "the single output is balance minus fee"
        );

        // The signed tx is real: it deserializes, has one input and one output to
        // the destination, and the p2wpkh input actually carries a witness.
        let raw = hex::decode(&out.signed_tx_hex).unwrap();
        let tx: bdk_wallet::bitcoin::Transaction =
            bdk_wallet::bitcoin::consensus::deserialize(&raw).unwrap();
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.output.len(), 1, "a sweep drains to exactly one output");
        assert_eq!(tx.output[0].script_pubkey, dest.script_pubkey());
        assert!(!tx.input[0].witness.is_empty(), "the input is signed");
        assert_eq!(tx.compute_txid().to_string(), out.txid);
    }

    #[test]
    fn build_sweep_tx_returns_none_for_an_empty_wallet() {
        let (mut wallet, _net) = funded_single_key([0x44u8; 32], 0);
        let out = build_sweep_tx(
            &mut wallet,
            ScriptBuf::new(),
            FeeRate::from_sat_per_vb(2).unwrap(),
            "p2wpkh",
            "addr",
        )
        .unwrap();
        assert!(out.is_none(), "nothing to sweep from an empty wallet");
    }
}
