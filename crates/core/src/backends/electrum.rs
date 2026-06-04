use super::BackendError;
use crate::types::{
    AddressInfo, AddressKind, BackendKind, Balance, NodeStatus, SyncResult, TxRecord,
};
use anyhow::Context;
use bdk_electrum::electrum_client::ElectrumApi;
use bdk_electrum::{electrum_client, BdkElectrumClient};
use bdk_wallet::chain::ChainPosition;
use bdk_wallet::{KeychainKind, Wallet};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::net::TcpStream;
use uuid::Uuid;

const STOP_GAP: usize = 20;
const BATCH_SIZE: usize = 5;

pub struct ElectrumConfig {
    pub url: String,
    /// Validate TLS hostname and certificate chain.
    pub validate_tls: bool,
    /// Absolute path to a PEM or DER CA certificate to trust (for self-signed certs).
    pub ca_cert_path: Option<String>,
    /// Skip all TLS verification — safe only for localhost.
    pub danger_accept_invalid_certs: bool,
    /// SOCKS5 proxy address (e.g. "127.0.0.1:9050" for Tor).
    pub socks5_proxy: Option<String>,
}

impl Default for ElectrumConfig {
    fn default() -> Self {
        Self {
            url: "tcp://127.0.0.1:50001".to_string(),
            validate_tls: false,
            ca_cert_path: None,
            danger_accept_invalid_certs: false,
            socks5_proxy: None,
        }
    }
}

/// True when we need native-tls to handle cert validation ourselves:
/// only for ssl:// URLs with a custom CA cert or danger mode enabled.
pub fn needs_custom_tls(cfg: &ElectrumConfig) -> bool {
    cfg.url.starts_with("ssl://") && (cfg.danger_accept_invalid_certs || cfg.ca_cert_path.is_some())
}

/// Extract (host, port) from an ssl:// URL.
fn parse_ssl_url(url: &str) -> Result<(String, u16), BackendError> {
    let rest = url
        .strip_prefix("ssl://")
        .ok_or_else(|| BackendError::Electrum("expected ssl:// URL".into()))?;
    if let Some((host, port_str)) = rest.rsplit_once(':') {
        let port = port_str
            .parse::<u16>()
            .map_err(|_| BackendError::Electrum(format!("invalid port in URL: {url}")))?;
        Ok((host.to_string(), port))
    } else {
        Ok((rest.to_string(), 50002))
    }
}

/// Build a native-tls client for ssl:// URLs that need a custom CA cert or danger mode.
pub fn build_native_tls_client(
    cfg: &ElectrumConfig,
) -> Result<
    BdkElectrumClient<electrum_client::raw_client::RawClient<native_tls::TlsStream<TcpStream>>>,
    BackendError,
> {
    use native_tls::{Certificate, TlsConnector};

    let (host, port) = parse_ssl_url(&cfg.url)?;

    let mut builder = TlsConnector::builder();

    if cfg.danger_accept_invalid_certs {
        builder.danger_accept_invalid_certs(true);
        builder.danger_accept_invalid_hostnames(true);
    }

    if let Some(cert_path) = &cfg.ca_cert_path {
        let cert_bytes = std::fs::read(cert_path)
            .map_err(|e| BackendError::Electrum(format!("reading CA cert '{cert_path}': {e}")))?;
        // Try PEM first, then DER.
        let cert = Certificate::from_pem(&cert_bytes)
            .or_else(|_| Certificate::from_der(&cert_bytes))
            .map_err(|e| BackendError::Electrum(format!("parsing CA cert '{cert_path}': {e}")))?;
        builder.add_root_certificate(cert);
    }

    let connector = builder
        .build()
        .map_err(|e| BackendError::Electrum(format!("building TLS connector: {e}")))?;

    let tcp = if let Some(proxy) = &cfg.socks5_proxy {
        socks::Socks5Stream::connect(proxy.as_str(), (host.as_str(), port))
            .map_err(|e| {
                BackendError::Electrum(connection_error(
                    cfg,
                    &format!("SOCKS5 connect via {proxy} to {host}:{port}: {e}"),
                ))
            })?
            .into_inner()
    } else {
        TcpStream::connect(format!("{host}:{port}")).map_err(|e| {
            BackendError::Electrum(connection_error(
                cfg,
                &format!("TCP connect to {host}:{port}: {e}"),
            ))
        })?
    };

    let tls = connector.connect(&host, tcp).map_err(|e| {
        BackendError::Electrum(connection_error(
            cfg,
            &format!("TLS handshake with {host}:{port}: {e}"),
        ))
    })?;

    Ok(BdkElectrumClient::new(
        electrum_client::raw_client::RawClient::from(tls),
    ))
}

/// Build a standard electrum-client for tcp:// or ssl:// without custom cert config.
pub fn build_client(
    cfg: &ElectrumConfig,
) -> Result<BdkElectrumClient<electrum_client::Client>, BackendError> {
    let validate = !cfg.danger_accept_invalid_certs && cfg.validate_tls;
    let mut builder = electrum_client::ConfigBuilder::new().validate_domain(validate);
    if let Some(proxy) = &cfg.socks5_proxy {
        builder = builder.socks5(Some(electrum_client::Socks5Config::new(proxy)));
    }
    let ecfg = builder.build();
    electrum_client::Client::from_config(&cfg.url, ecfg)
        .map(BdkElectrumClient::new)
        .map_err(|e| BackendError::Electrum(connection_error(cfg, &e.to_string())))
}

fn is_onion_url(url: &str) -> bool {
    let host = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url)
        .split(':')
        .next()
        .unwrap_or("");
    host.ends_with(".onion")
}

fn connection_error(cfg: &ElectrumConfig, raw: &str) -> String {
    let hint = if is_onion_url(&cfg.url) && cfg.socks5_proxy.is_none() {
        ".onion addresses require Tor. Enable the SOCKS5 proxy in Backend settings and set it to 127.0.0.1:9050"
    } else if !cfg.url.starts_with("ssl://") {
        "Check the host and port, or enable 'Use SSL / TLS' if your Electrum server requires it (typically port 50002)"
    } else if cfg.danger_accept_invalid_certs || cfg.ca_cert_path.is_some() {
        "TLS handshake failed — verify the host, port, and certificate settings"
    } else {
        "For a self-signed cert enable 'Accept invalid certificates', or provide the CA certificate path"
    };
    format!("{raw}\n\nHint: {hint}")
}

fn wallet_is_hd(wallet: &Wallet) -> bool {
    wallet
        .public_descriptor(KeychainKind::External)
        .to_string()
        .contains("/*")
}

fn do_sync<E: ElectrumApi>(
    client: &BdkElectrumClient<E>,
    wallet: &mut Wallet,
) -> anyhow::Result<SyncResult> {
    let prev = wallet.transactions().count();

    if wallet_is_hd(wallet) {
        let update = client
            .full_scan(wallet.start_full_scan(), STOP_GAP, BATCH_SIZE, false)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        wallet.apply_update(update)?;
        // Anchor to last *used*, not last *revealed* — otherwise the
        // address list grows by STOP_GAP every sync.
        let last_used_ext = wallet
            .spk_index()
            .last_used_index(KeychainKind::External)
            .unwrap_or(0);
        wallet
            .reveal_addresses_to(KeychainKind::External, last_used_ext + STOP_GAP as u32)
            .count();
    } else {
        let update = client
            .sync(wallet.start_sync_with_revealed_spks(), BATCH_SIZE, false)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        wallet.apply_update(update)?;
    }

    Ok(SyncResult {
        wallet_id: Uuid::nil(),
        new_txs: wallet.transactions().count().saturating_sub(prev),
        synced_at: Utc::now(),
    })
}

fn do_probe<E: ElectrumApi>(client: &BdkElectrumClient<E>) -> NodeStatus {
    match client.inner.block_headers_subscribe() {
        Ok(h) => NodeStatus {
            backend: BackendKind::Electrum,
            connected: true,
            network: if h.height > 800_000 {
                "bitcoin".into()
            } else {
                "unknown".into()
            },
            tip_height: Some(h.height as u32),
            error: None,
            offline: false,
        },
        Err(e) => NodeStatus {
            backend: BackendKind::Electrum,
            connected: false,
            network: "unknown".into(),
            tip_height: None,
            error: Some(e.to_string()),
            offline: false,
        },
    }
}

fn collect_fees(wallet: &Wallet, existing: &HashMap<String, u64>) -> HashMap<String, u64> {
    let mut new_fees = HashMap::new();
    for canonical_tx in wallet.transactions() {
        if !matches!(canonical_tx.chain_position, ChainPosition::Confirmed { .. }) {
            continue;
        }
        let key = canonical_tx.tx_node.txid.to_string();
        if existing.contains_key(&key) {
            continue;
        }
        if let Ok(fee) = wallet.calculate_fee(&canonical_tx.tx_node.tx) {
            new_fees.insert(key, fee.to_sat());
        }
    }
    new_fees
}

/// Sync a wallet and return updated fee data for any newly confirmed transactions.
pub fn sync_wallet_with_fees(
    wallet: &mut Wallet,
    cfg: &ElectrumConfig,
    existing_fees: &HashMap<String, u64>,
) -> anyhow::Result<(SyncResult, HashMap<String, u64>)> {
    if needs_custom_tls(cfg) {
        let client = build_native_tls_client(cfg)?;
        let result = do_sync(&client, wallet)?;
        let new_fees = collect_fees(wallet, existing_fees);
        Ok((result, new_fees))
    } else {
        let client = build_client(cfg)?;
        let result = do_sync(&client, wallet)?;
        let new_fees = collect_fees(wallet, existing_fees);
        Ok((result, new_fees))
    }
}

/// Reconcile spent Silent Payment outputs: the SP scanner only ever *adds*
/// discovered outputs, so we check on-chain whether any are now spent. Each SP
/// output has a unique P2TR script, so its script history has exactly one entry
/// (the funding tx) while unspent, and ≥2 once a spend lands. We require that
/// positive evidence — an empty history (e.g. a server that hasn't indexed the
/// script) or a network error never marks an output spent. `outputs` is
/// `(txid, vout, script_pubkey_hex)`; returns the spent `(txid, vout)` pairs.
pub fn find_spent_sp_outputs(
    outputs: &[(String, u32, String)],
    cfg: &ElectrumConfig,
) -> anyhow::Result<Vec<(String, u32)>> {
    if needs_custom_tls(cfg) {
        let client = build_native_tls_client(cfg)?;
        do_find_spent(&client.inner, outputs)
    } else {
        let client = build_client(cfg)?;
        do_find_spent(&client.inner, outputs)
    }
}

fn do_find_spent<E: ElectrumApi>(
    client: &E,
    outputs: &[(String, u32, String)],
) -> anyhow::Result<Vec<(String, u32)>> {
    use bdk_electrum::electrum_client::bitcoin::ScriptBuf;
    let mut spent = Vec::new();
    for (txid, vout, spk_hex) in outputs {
        let script = ScriptBuf::from_hex(spk_hex)
            .with_context(|| format!("bad SP scriptPubKey for {txid}:{vout}"))?;
        let history = client.script_get_history(script.as_script())?;
        if history.len() >= 2 {
            spent.push((txid.clone(), *vout));
        }
    }
    Ok(spent)
}

/// Sync a single address against the Electrum server using scripthash queries. Blocking.
pub fn sync_address(
    address_str: &str,
    cfg: &ElectrumConfig,
) -> anyhow::Result<(Balance, Vec<TxRecord>)> {
    if needs_custom_tls(cfg) {
        let client = build_native_tls_client(cfg)?;
        do_sync_address(address_str, &client.inner)
    } else {
        let client = build_client(cfg)?;
        do_sync_address(address_str, &client.inner)
    }
}

fn do_sync_address<E: ElectrumApi>(
    address_str: &str,
    client: &E,
) -> anyhow::Result<(Balance, Vec<TxRecord>)> {
    let address = address_str
        .parse::<bitcoin::Address<bitcoin::address::NetworkUnchecked>>()
        .context("invalid address")?
        .assume_checked();

    let script = address.script_pubkey();

    let bal = client
        .script_get_balance(&script)
        .map_err(|e| anyhow::anyhow!("script_get_balance: {e}"))?;

    let balance = Balance {
        confirmed_sats: bal.confirmed,
        unconfirmed_sats: bal.unconfirmed.max(0) as u64,
        spendable_sats: bal.confirmed,
        // Electrum's script_get_balance doesn't distinguish coinbase maturity.
        // For watch-only address sync this is a known limitation — assume zero
        // immature; users mining into a watched address won't see the breakdown.
        immature_sats: 0,
        last_synced: None,
    };

    let tip = client
        .block_headers_subscribe()
        .map(|h| h.height as u32)
        .unwrap_or(0);

    let history = client
        .script_get_history(&script)
        .map_err(|e| anyhow::anyhow!("script_get_history: {e}"))?;

    // Fetch block timestamps for each unique confirmed height.
    let mut height_times: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for h in &history {
        if h.height > 0 {
            let height = h.height as u32;
            if let std::collections::hash_map::Entry::Vacant(e) = height_times.entry(height) {
                if let Ok(header) = client.block_header(h.height as usize) {
                    e.insert(header.time);
                }
            }
        }
    }

    let mut txs = Vec::new();
    for h in &history {
        let tx = client
            .transaction_get(&h.tx_hash)
            .map_err(|e| anyhow::anyhow!("transaction_get {}: {e}", h.tx_hash))?;

        let received: u64 = tx
            .output
            .iter()
            .filter(|o| o.script_pubkey == script)
            .map(|o| o.value.to_sat())
            .sum();

        let (confirmations, block_height) = if h.height > 0 {
            let height = h.height as u32;
            (tip.saturating_sub(height).saturating_add(1), Some(height))
        } else {
            (0u32, None)
        };

        let timestamp = block_height
            .and_then(|bh| height_times.get(&bh))
            .and_then(|&t| DateTime::from_timestamp(t as i64, 0));

        // Coinbase: single input with prev txid all zeros and prev vout = u32::MAX.
        let is_coinbase = tx.input.len() == 1 && tx.input[0].previous_output.is_null();

        txs.push(TxRecord {
            txid: h.tx_hash.to_string(),
            amount_sats: received as i64,
            fee_sats: h.fee,
            vsize: None,
            confirmations,
            block_height,
            timestamp,
            is_rbf: false,
            is_coinbase,
        });
    }

    // Unconfirmed (block_height = None) sorts first; confirmed by descending height.
    txs.sort_by(|a, b| match (a.block_height, b.block_height) {
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
        (Some(ah), Some(bh)) => bh.cmp(&ah),
    });

    Ok((balance, txs))
}

/// Return address info for a single watched address (used by the addresses endpoint).
pub fn address_info(address_str: &str) -> Vec<AddressInfo> {
    vec![AddressInfo {
        address: address_str.to_string(),
        index: 0,
        kind: AddressKind::External,
        used: true,
        tx_count: 0,
    }]
}

pub fn broadcast_tx(
    tx: &bdk_wallet::bitcoin::Transaction,
    cfg: &ElectrumConfig,
) -> Result<(), BackendError> {
    if needs_custom_tls(cfg) {
        build_native_tls_client(cfg)?
            .inner
            .transaction_broadcast(tx)
            .map_err(|e| BackendError::Electrum(e.to_string()))?;
    } else {
        build_client(cfg)?
            .inner
            .transaction_broadcast(tx)
            .map_err(|e| BackendError::Electrum(e.to_string()))?;
    }
    Ok(())
}

pub fn probe_status(cfg: &ElectrumConfig) -> NodeStatus {
    let err_status = |msg: String| NodeStatus {
        backend: BackendKind::Electrum,
        connected: false,
        network: "unknown".into(),
        tip_height: None,
        error: Some(msg),
        offline: false,
    };

    if needs_custom_tls(cfg) {
        match build_native_tls_client(cfg) {
            Ok(client) => do_probe(&client),
            Err(e) => err_status(e.to_string()),
        }
    } else {
        match build_client(cfg) {
            Ok(client) => do_probe(&client),
            Err(e) => err_status(e.to_string()),
        }
    }
}
