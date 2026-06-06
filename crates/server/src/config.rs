use anyhow::{Context, Result};
use corvin_core::backends::{electrum::ElectrumConfig, rpc::RpcConfig};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendType {
    #[default]
    Electrum,
    Rpc,
}

fn default_electrum_host() -> String {
    "127.0.0.1".to_string()
}
fn default_electrum_port() -> u16 {
    50001
}
fn default_rpc_url() -> String {
    "http://127.0.0.1:8332".to_string()
}
fn default_mempool_url() -> String {
    "https://mempool.space".to_string()
}
// Cloudflare (not Quad9): Quad9's DoH endpoint is HTTP/2-only and returns HTTP 505
// when reqwest falls back to HTTP/1.1, which it does through a SOCKS5/Tor proxy. So
// the privacy-optimal resolver silently broke BIP-353 for exactly the Tor users. This
// one speaks HTTP/1.1 too; DNSSEC still makes the answer unforgeable.
fn default_bip353_doh_url() -> String {
    "https://cloudflare-dns.com/dns-query".to_string()
}
fn default_poll_interval_secs() -> u64 {
    300
}
/// Default Silent Payments dust-attack threshold (sats). Received SP UTXOs below
/// this are flagged as suspected dust. Higher than the on-chain dust limit because
/// SP outputs are P2TR and the privacy stakes of spending an attacker's dust are
/// higher. Matches Sparrow's default.
pub const DEFAULT_SP_DUST_THRESHOLD_SATS: u64 = 5000;
fn default_show_price_data() -> bool {
    false
}
fn default_show_current_price() -> bool {
    false
}
fn default_show_fiat_balance() -> bool {
    false
}
fn default_sp_electrum_host() -> String {
    "frigate.2140.dev".to_string()
}
fn default_sp_electrum_port() -> u16 {
    50002
}
fn default_sp_electrum_ssl() -> bool {
    true
}
fn default_sp_validate_tls() -> bool {
    true
}
fn default_payjoin_directory_url() -> String {
    "https://payjo.in".to_string()
}
fn default_payjoin_ohttp_relay_url() -> String {
    "https://pj.bobspacebkk.com".to_string()
}
fn default_payjoin_fallback_secs() -> u64 {
    120
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    #[serde(rename = "type", default)]
    pub kind: BackendType,
    #[serde(default = "default_electrum_host")]
    pub electrum_host: String,
    #[serde(default = "default_electrum_port")]
    pub electrum_port: u16,
    #[serde(default)]
    pub electrum_ssl: bool,
    #[serde(default)]
    pub validate_tls: bool,
    /// Path to a PEM or DER CA certificate for trusting a self-signed Electrum server cert
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_cert_path: Option<String>,
    /// Skip ALL TLS verification — safe only for localhost connections
    #[serde(default)]
    pub danger_accept_invalid_certs: bool,
    /// SOCKS5 proxy address, e.g. "127.0.0.1:9050" for Tor
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socks5_proxy: Option<String>,
    #[serde(default = "default_rpc_url")]
    pub rpc_url: String,
    #[serde(default)]
    pub rpc_user: String,
    #[serde(default)]
    pub rpc_pass: String,
    #[serde(default = "default_mempool_url")]
    pub mempool_url: String,
    /// Accept a self-signed / invalid TLS cert for the mempool server (fees +
    /// price). Per-service, like a saved backend's own cert option — the mempool
    /// is often a self-hosted instance separate from your chain backend.
    #[serde(default)]
    pub mempool_danger_accept_invalid_certs: bool,
    /// DNS-over-HTTPS resolver used for BIP-353 name resolution (`₿user@domain`).
    /// DNSSEC makes the answer unforgeable, but whichever resolver this points at
    /// still *sees* the name being resolved (i.e. who you're about to pay), even
    /// over Tor. Configurable so privacy-conscious users can pick a resolver they
    /// trust instead of the `cloudflare-dns.com` default.
    #[serde(default = "default_bip353_doh_url")]
    pub bip353_doh_url: String,
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    /// Whether to fetch and display historical BTC/USD prices via the mempool server.
    #[serde(default = "default_show_price_data")]
    pub show_price_data: bool,
    /// Whether to show the live BTC/USD price in the status bar.
    #[serde(default = "default_show_current_price")]
    pub show_current_price: bool,
    /// Whether to show fiat equivalent of wallet balances.
    #[serde(default = "default_show_fiat_balance")]
    pub show_fiat_balance: bool,

    // ── Silent Payments scanner ──────────────────────────────────────────────
    //
    // The default SP scanner for SP wallets with no pinned backend. Defaults to
    // the public Frigate (`frigate.2140.dev`). A regular Electrum server can't
    // scan BIP-352, so this is always a Frigate-capable server, never the main
    // connection. No longer surfaced as its own settings section — SP wallets
    // pin a Frigate backend from the registry, or fall back to this default.
    #[serde(default = "default_sp_electrum_host")]
    pub sp_electrum_host: String,
    #[serde(default = "default_sp_electrum_port")]
    pub sp_electrum_port: u16,
    #[serde(default = "default_sp_electrum_ssl")]
    pub sp_electrum_ssl: bool,
    #[serde(default = "default_sp_validate_tls")]
    pub sp_validate_tls: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sp_ca_cert_path: Option<String>,
    #[serde(default)]
    pub sp_danger_accept_invalid_certs: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sp_socks5_proxy: Option<String>,

    // ── Payjoin (BIP-77 async / v2) ──────────────────────────────────────────
    //
    // Sender-side v2 with v1 fallback. The directory + OHTTP relay are public
    // store-and-forward infra (configurable). All payjoin HTTP goes through the
    // same socks5_proxy as the main backend. fallback_secs is how long a send
    // session waits for the receiver's proposal before broadcasting the
    // original (non-payjoin) transaction.
    #[serde(default)]
    pub payjoin_enabled: bool,
    #[serde(default = "default_payjoin_directory_url")]
    pub payjoin_directory_url: String,
    #[serde(default = "default_payjoin_ohttp_relay_url")]
    pub payjoin_ohttp_relay_url: String,
    #[serde(default = "default_payjoin_fallback_secs")]
    pub payjoin_fallback_secs: u64,
}

impl BackendConfig {
    pub fn electrum_url(&self) -> String {
        let scheme = if self.electrum_ssl { "ssl" } else { "tcp" };
        format!("{scheme}://{}:{}", self.electrum_host, self.electrum_port)
    }

    pub fn sp_electrum_url(&self) -> String {
        let scheme = if self.sp_electrum_ssl { "ssl" } else { "tcp" };
        format!(
            "{scheme}://{}:{}",
            self.sp_electrum_host, self.sp_electrum_port
        )
    }
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            kind: BackendType::Electrum,
            // Fresh installs default to a public Electrum server so the app works
            // out of the box; users move sensitive wallets to their own node via
            // the saved-backends registry. (Matches a host in the frontend's
            // PUBLIC_SERVERS list so the Default dropdown preselects it cleanly.)
            electrum_host: "electrum.blockstream.info".to_string(),
            electrum_port: 50002,
            electrum_ssl: true,
            validate_tls: true,
            ca_cert_path: None,
            danger_accept_invalid_certs: false,
            socks5_proxy: None,
            rpc_url: "http://127.0.0.1:8332".to_string(),
            rpc_user: String::new(),
            rpc_pass: String::new(),
            mempool_url: "https://mempool.space".to_string(),
            mempool_danger_accept_invalid_certs: false,
            bip353_doh_url: default_bip353_doh_url(),
            poll_interval_secs: 300,
            show_price_data: false,
            show_current_price: false,
            show_fiat_balance: false,
            sp_electrum_host: "frigate.2140.dev".to_string(),
            sp_electrum_port: 50002,
            sp_electrum_ssl: true,
            sp_validate_tls: true,
            sp_ca_cert_path: None,
            sp_danger_accept_invalid_certs: false,
            sp_socks5_proxy: None,
            payjoin_enabled: false,
            payjoin_directory_url: "https://payjo.in".to_string(),
            payjoin_ohttp_relay_url: "https://pj.bobspacebkk.com".to_string(),
            payjoin_fallback_secs: 120,
        }
    }
}

/// A named chain backend in the registry. The *default* backend is still
/// `Config.backend` (unchanged); these are *additional* backends a wallet can
/// be pinned to (privacy compartmentalization). A `WalletEntry.backend` of
/// `None` means the default; otherwise it's the `id` of one of these. Honored
/// by the subscriber in a later phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendEntry {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(rename = "type", default)]
    pub kind: BackendType,
    /// Marks an Electrum-protocol server that also speaks BIP-352 (Frigate), so
    /// it can be used as a Silent Payments scanner. Such a backend connects like
    /// any Electrum server (`kind = Electrum`); this flag only drives which wallet
    /// kinds may pick it (SP wallets pick Frigate backends; HD wallets don't).
    #[serde(default)]
    pub frigate: bool,
    #[serde(default = "default_electrum_host")]
    pub electrum_host: String,
    #[serde(default = "default_electrum_port")]
    pub electrum_port: u16,
    #[serde(default)]
    pub electrum_ssl: bool,
    #[serde(default)]
    pub validate_tls: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_cert_path: Option<String>,
    #[serde(default)]
    pub danger_accept_invalid_certs: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socks5_proxy: Option<String>,
    #[serde(default = "default_rpc_url")]
    pub rpc_url: String,
    #[serde(default)]
    pub rpc_user: String,
    #[serde(default)]
    pub rpc_pass: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
    pub bind: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 5757,
            bind: "127.0.0.1".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkKind {
    #[default]
    Bitcoin,
    Testnet,
    Signet,
    Regtest,
}

impl NetworkKind {
    // Only the USB hardware-wallet modules consume this today.
    #[cfg_attr(not(feature = "hw"), allow(dead_code))]
    pub fn is_testnet_like(self) -> bool {
        !matches!(self, Self::Bitcoin)
    }

    pub fn to_bitcoin_network(self) -> bdk_wallet::bitcoin::Network {
        use bdk_wallet::bitcoin::Network;
        match self {
            Self::Bitcoin => Network::Bitcoin,
            Self::Testnet => Network::Testnet,
            Self::Signet => Network::Signet,
            Self::Regtest => Network::Regtest,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkConfig {
    #[serde(rename = "type", default)]
    pub kind: NetworkKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub backend: BackendConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    /// Additional named backends a wallet can be pinned to. The default backend
    /// is `backend` above; these are extras. Empty for the common single-backend
    /// case. Preserved across settings saves (the settings form doesn't carry it).
    #[serde(default)]
    pub backends: Vec<BackendEntry>,
    /// Which backend unpinned wallets (`WalletEntry.backend == None`) use. `None`
    /// = the built-in `backend` connection above (the selected public server);
    /// `Some(id)` = a saved backend from `backends` (e.g. your own node). Lets the
    /// default be one of your saved servers without copying its secrets around.
    #[serde(default)]
    pub default_backend: Option<String>,
    /// Silent Payments dust-attack threshold in sats. Received SP UTXOs below this
    /// are flagged as suspected dust in the UI. `None` uses
    /// `DEFAULT_SP_DUST_THRESHOLD_SATS`; set a value to override.
    #[serde(default)]
    pub sp_dust_threshold_sats: Option<u64>,
    /// Set once the first-run onboarding wizard has been completed or skipped.
    /// The frontend shows the wizard only when this is false and no wallets exist.
    /// Preserved across settings saves (the settings form doesn't carry it).
    #[serde(default)]
    pub onboarding_complete: bool,
    /// Air-gapped/offline mode: never open any backend connection (no Electrum,
    /// RPC, or SP scanner). Reads serve cached/persisted state; you can still
    /// import, sign, and export PSBTs. Sync and broadcast are unavailable.
    #[serde(default)]
    pub offline: bool,
}

impl BackendEntry {
    pub fn electrum_url(&self) -> String {
        let scheme = if self.electrum_ssl { "ssl" } else { "tcp" };
        format!("{scheme}://{}:{}", self.electrum_host, self.electrum_port)
    }
}

impl Config {
    pub fn electrum_config(&self) -> ElectrumConfig {
        ElectrumConfig {
            url: self.backend.electrum_url(),
            validate_tls: self.backend.validate_tls,
            ca_cert_path: self.backend.ca_cert_path.clone(),
            danger_accept_invalid_certs: self.backend.danger_accept_invalid_certs,
            socks5_proxy: self.backend.socks5_proxy.clone(),
        }
    }

    /// A registered backend by id, or None for an unknown/absent id.
    pub fn backend_entry(&self, id: &str) -> Option<&BackendEntry> {
        self.backends.iter().find(|b| b.id == id)
    }

    /// The registered backend id a wallet effectively uses, accounting for the
    /// global default. A pinned wallet uses its own id; an unpinned wallet falls
    /// back to `default_backend`. Returns `None` (= the built-in `backend`
    /// connection) when neither resolves to a *registered* backend, so a dangling
    /// id gracefully degrades to the default connection. SP resolution does NOT
    /// use this — SP wallets keep their own scanner config.
    pub fn effective_backend_id(&self, wallet_backend: Option<&str>) -> Option<String> {
        let id = match wallet_backend {
            Some(id) => Some(id.to_string()),
            None => self.default_backend.clone(),
        };
        id.filter(|i| self.backend_entry(i).is_some())
    }

    /// Electrum config for a wallet's effective backend. `None` (or an unknown
    /// id) resolves to the default backend — so a wallet with no pinned backend
    /// behaves exactly as before.
    pub fn electrum_config_for(&self, backend: Option<&str>) -> ElectrumConfig {
        match self
            .effective_backend_id(backend)
            .and_then(|id| self.backend_entry(&id))
        {
            Some(e) => ElectrumConfig {
                url: e.electrum_url(),
                validate_tls: e.validate_tls,
                ca_cert_path: e.ca_cert_path.clone(),
                danger_accept_invalid_certs: e.danger_accept_invalid_certs,
                socks5_proxy: e.socks5_proxy.clone(),
            },
            None => self.electrum_config(),
        }
    }

    /// Backend kind (Electrum/Rpc) for a wallet's effective backend.
    pub fn backend_kind_for(&self, backend: Option<&str>) -> BackendType {
        match self
            .effective_backend_id(backend)
            .and_then(|id| self.backend_entry(&id))
        {
            Some(e) => e.kind.clone(),
            None => self.backend.kind.clone(),
        }
    }

    /// Default SP-scanner connection (for SP wallets with no pinned backend).
    /// Always the dedicated SP server (`sp_electrum_*`, default frigate.2140.dev)
    /// — a regular Electrum server can't scan BIP-352, so this never mirrors the
    /// main connection.
    pub fn sp_electrum_config(&self) -> ElectrumConfig {
        ElectrumConfig {
            url: self.backend.sp_electrum_url(),
            validate_tls: self.backend.sp_validate_tls,
            ca_cert_path: self.backend.sp_ca_cert_path.clone(),
            danger_accept_invalid_certs: self.backend.sp_danger_accept_invalid_certs,
            socks5_proxy: self.backend.sp_socks5_proxy.clone(),
        }
    }

    /// SP scanner config for an SP wallet's pinned backend. `None` keeps the
    /// existing global SP-scanner config (unchanged behavior); a pinned backend
    /// (a saved Frigate-capable server) uses that server's connection instead —
    /// so a private SP wallet's receipts go to its own scanner.
    pub fn sp_electrum_config_for(&self, backend: Option<&str>) -> ElectrumConfig {
        match backend.and_then(|id| self.backend_entry(id)) {
            Some(e) => ElectrumConfig {
                url: e.electrum_url(),
                validate_tls: e.validate_tls,
                ca_cert_path: e.ca_cert_path.clone(),
                danger_accept_invalid_certs: e.danger_accept_invalid_certs,
                socks5_proxy: e.socks5_proxy.clone(),
            },
            None => self.sp_electrum_config(),
        }
    }

    pub fn rpc_config(&self) -> RpcConfig {
        RpcConfig {
            url: self.backend.rpc_url.clone(),
            user: self.backend.rpc_user.clone(),
            pass: self.backend.rpc_pass.clone(),
        }
    }

    /// RPC config for a wallet's effective backend. `None` (or an unknown id)
    /// resolves to the default backend.
    pub fn rpc_config_for(&self, backend: Option<&str>) -> RpcConfig {
        match self
            .effective_backend_id(backend)
            .and_then(|id| self.backend_entry(&id))
        {
            Some(e) => RpcConfig {
                url: e.rpc_url.clone(),
                user: e.rpc_user.clone(),
                pass: e.rpc_pass.clone(),
            },
            None => self.rpc_config(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    // CORVIN_CONFIG_DIR overrides the default location — useful for an isolated
    // dev/test instance, and what the test suite points at so it never touches
    // the real config.
    if let Ok(dir) = std::env::var("CORVIN_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("corvin")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Effective bind address: the `CORVIN_BIND` env override wins over the config
/// value. Lets a container set the bind without a pre-seeded config.toml (a
/// fresh container has none). Pure (env value passed in) so it's testable
/// without touching the process environment.
pub fn resolve_bind(env_bind: Option<String>, cfg_bind: &str) -> String {
    match env_bind {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => cfg_bind.to_string(),
    }
}

/// Effective port: `CORVIN_PORT` wins over config when it parses as a valid
/// `u16`; an unset/blank/garbage value falls back to the config port.
pub fn resolve_port(env_port: Option<String>, cfg_port: u16) -> u16 {
    env_port
        .and_then(|s| s.trim().parse::<u16>().ok())
        .unwrap_or(cfg_port)
}

/// Parse a `CORVIN_ELECTRUM_URL` value into `(host, port, ssl)`. Accepts an
/// optional scheme: `ssl://` / `tls://` (TLS), `tcp://` (plaintext), or none
/// (plaintext). Returns `None` for anything malformed. Pure for testability.
pub fn parse_electrum_url(raw: &str) -> Option<(String, u16, bool)> {
    let raw = raw.trim();
    let (ssl, rest) = if let Some(r) = raw.strip_prefix("ssl://").or_else(|| raw.strip_prefix("tls://")) {
        (true, r)
    } else if let Some(r) = raw.strip_prefix("tcp://") {
        (false, r)
    } else {
        (false, raw)
    };
    let (host, port_s) = rest.rsplit_once(':')?;
    let port: u16 = port_s.trim().parse().ok()?;
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port, ssl))
}

/// On a fresh install (no config.toml yet) seed the default Electrum backend
/// from `CORVIN_ELECTRUM_URL` if set — lets a container/Start9 point Corvin at
/// its bundled server (e.g. Fulcrum) without a pre-seeded config. Ignored once a
/// config exists, so it never overrides the user's later choice.
fn seed_backend_from_env(config: &mut Config) {
    let Ok(url) = std::env::var("CORVIN_ELECTRUM_URL") else {
        return;
    };
    if let Some((host, port, ssl)) = parse_electrum_url(&url) {
        config.backend.kind = BackendType::Electrum;
        config.backend.electrum_host = host;
        config.backend.electrum_port = port;
        config.backend.electrum_ssl = ssl;
    } else {
        tracing::warn!("CORVIN_ELECTRUM_URL is malformed ({url:?}); ignoring");
    }
}

/// Seed the mempool base URL from `CORVIN_MEMPOOL_URL` if set — lets Start9 point
/// Corvin at a self-hosted mempool instance (a privacy win over leaking queries to
/// mempool.space). The value is a base URL with no `/api` suffix (the proxy/price
/// handlers append the path).
///
/// Runs on every boot, not just first install, but only while the URL is still the
/// untouched default: a self-hosted mempool that's added *after* Corvin is already
/// installed (or that wasn't up yet on first boot) still gets picked up, while a URL
/// the user changed in Settings is never clobbered. Limitation: a user who explicitly
/// re-selects mempool.space can't be told apart from the default, so the env would
/// reassert the local instance on the next boot — acceptable for the Start9 case.
fn seed_mempool_from_env(config: &mut Config) {
    if config.backend.mempool_url != default_mempool_url() {
        return;
    }
    let Ok(url) = std::env::var("CORVIN_MEMPOOL_URL") else {
        return;
    };
    let url = url.trim().trim_end_matches('/');
    if !url.is_empty() {
        config.backend.mempool_url = url.to_string();
    }
}

// Existing installs persisted the old Quad9 default, which is HTTP/2-only and 505s over a
// SOCKS5/Tor proxy (breaking BIP-353). Migrate anyone still on that exact value to the new
// h1.1-compatible default. Returns true if it changed something (so the caller re-saves).
fn migrate_bip353_doh(config: &mut Config) -> bool {
    if config.backend.bip353_doh_url == "https://dns.quad9.net/dns-query" {
        config.backend.bip353_doh_url = default_bip353_doh_url();
        return true;
    }
    false
}

// Point CORVIN_CONFIG_DIR at one throwaway temp dir for the whole test binary.
// A single OnceLock means every test (across modules) shares that dir, so the
// env var is never racily reassigned; tests must touch disjoint files.
#[cfg(test)]
pub(crate) fn test_isolate_config_dir() {
    use std::sync::OnceLock;
    static DIR: OnceLock<()> = OnceLock::new();
    DIR.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("corvin-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        std::env::set_var("CORVIN_CONFIG_DIR", &d);
        corvin_core::at_rest::set_config_root(d);
    });
}

pub fn wallets_path() -> PathBuf {
    config_dir().join("wallets.json")
}

pub fn labels_path() -> PathBuf {
    config_dir().join("labels.json")
}

pub fn price_cache_path() -> PathBuf {
    config_dir().join("price_cache.json")
}

pub fn cost_basis_path() -> PathBuf {
    config_dir().join("cost_basis.json")
}

pub fn utxo_labels_path() -> PathBuf {
    config_dir().join("utxo_labels.json")
}

pub fn address_labels_path() -> PathBuf {
    config_dir().join("address_labels.json")
}

pub fn frozen_utxos_path() -> PathBuf {
    config_dir().join("frozen_utxos.json")
}

pub fn categories_path() -> PathBuf {
    config_dir().join("categories.json")
}

/// At-rest encryption sentinel. Always plaintext; its presence means "encryption
/// is on" so the app boots locked. Holds the KDF salt/params + verifier.
pub fn vault_path() -> PathBuf {
    config_dir().join("vault.json")
}

/// On-disk store of Ledger multisig wallet-policy registration HMACs.
/// Schema is `{wallet_id: {device_fingerprint: hmac_hex}}`. Cleared whenever
/// the wallet is removed.
pub fn ledger_hmacs_path() -> PathBuf {
    config_dir().join("ledger_hmacs.json")
}

/// On-disk store of per-wallet BIP-352 silent-payment keys. Holds the scan
/// secret key (required for ongoing scanning) and the spend public key. The
/// spend secret key is never stored — re-derived from the mnemonic at send
/// time. Schema: `{wallet_id: {scan_secret_hex, spend_pubkey_hex, address}}`.
pub fn silent_payments_path() -> PathBuf {
    config_dir().join("silent_payments.json")
}

/// Directory holding per-session payjoin event logs (one JSON file per session)
/// plus the session index. See `payjoin_sessions.rs`.
pub fn payjoin_dir() -> PathBuf {
    config_dir().join("payjoin")
}

pub fn wallet_db_dir() -> PathBuf {
    config_dir().join("wallets")
}

pub fn wallet_db_path(id: uuid::Uuid) -> PathBuf {
    wallet_db_dir().join(format!("{id}.db"))
}

pub fn load_config() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        let mut config = Config::default();
        seed_backend_from_env(&mut config);
        seed_mempool_from_env(&mut config);
        save_config(&config)?;
        return Ok(config);
    }
    let bytes = crate::state::read_private(&path)?
        .ok_or_else(|| anyhow::anyhow!("config.toml disappeared"))?;
    let raw = String::from_utf8(bytes).context("config.toml is not valid UTF-8")?;
    let mut config: Config = toml::from_str(&raw).context("parsing config.toml")?;
    // Re-apply the mempool env seed on every boot (no-op unless it's still the default
    // and CORVIN_MEMPOOL_URL is set), so a mempool added after install is picked up.
    let mempool_before = config.backend.mempool_url.clone();
    seed_mempool_from_env(&mut config);
    let mempool_changed = config.backend.mempool_url != mempool_before;
    let doh_migrated = migrate_bip353_doh(&mut config);
    // The SP reframe removed `sp_use_main_electrum`; serde ignores the unknown key
    // but it lingers in the file. Re-save once to strip it (best-effort).
    if raw.contains("sp_use_main_electrum") || mempool_changed || doh_migrated {
        let _ = save_config(&config);
    }
    Ok(config)
}

pub fn save_config(config: &Config) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir).with_context(|| format!("creating config dir {}", dir.display()))?;
    let raw = toml::to_string_pretty(config).context("serialising config")?;
    // Atomic + 0600 + at-rest-sealed via the shared writer (config.toml can hold
    // the RPC password, so it's encrypted when the vault is unlocked).
    crate::state::write_private(&config_path(), raw.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_env_override_wins_over_config() {
        assert_eq!(resolve_bind(Some("0.0.0.0".into()), "127.0.0.1"), "0.0.0.0");
        // Unset or blank falls back to config (trimmed).
        assert_eq!(resolve_bind(None, "127.0.0.1"), "127.0.0.1");
        assert_eq!(resolve_bind(Some("   ".into()), "127.0.0.1"), "127.0.0.1");
        assert_eq!(resolve_bind(Some(" 0.0.0.0 ".into()), "127.0.0.1"), "0.0.0.0");
    }

    #[test]
    fn port_env_override_wins_when_valid() {
        assert_eq!(resolve_port(Some("8080".into()), 5757), 8080);
        // Unset / blank / non-numeric / out-of-range fall back to config.
        assert_eq!(resolve_port(None, 5757), 5757);
        assert_eq!(resolve_port(Some("".into()), 5757), 5757);
        assert_eq!(resolve_port(Some("nope".into()), 5757), 5757);
        assert_eq!(resolve_port(Some("70000".into()), 5757), 5757);
    }

    #[test]
    fn electrum_url_parses_scheme_host_port() {
        assert_eq!(
            parse_electrum_url("ssl://fulcrum.embassy:50002"),
            Some(("fulcrum.embassy".to_string(), 50002, true))
        );
        assert_eq!(
            parse_electrum_url("tls://host:50002"),
            Some(("host".to_string(), 50002, true))
        );
        assert_eq!(
            parse_electrum_url("tcp://10.0.0.5:50001"),
            Some(("10.0.0.5".to_string(), 50001, false))
        );
        // No scheme defaults to plaintext.
        assert_eq!(
            parse_electrum_url("node.local:50001"),
            Some(("node.local".to_string(), 50001, false))
        );
    }

    #[test]
    fn electrum_url_rejects_malformed() {
        assert_eq!(parse_electrum_url(""), None);
        assert_eq!(parse_electrum_url("nohost"), None);
        assert_eq!(parse_electrum_url("host:notaport"), None);
        assert_eq!(parse_electrum_url(":50001"), None);
        assert_eq!(parse_electrum_url("ssl://host:99999"), None);
    }
}
