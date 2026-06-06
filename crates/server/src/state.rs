use crate::config::{
    address_labels_path, categories_path, cost_basis_path, frozen_utxos_path, labels_path,
    price_cache_path, utxo_labels_path, wallet_db_path, wallets_path, Config,
};
use anyhow::{Context, Result};
use bdk_wallet::bitcoin::Network;
use bdk_wallet::bitcoin::ScriptBuf;
use bdk_wallet::KeychainKind;
use chrono::{DateTime, Utc};
use corvin_core::types::{
    AddressInfo, Balance, Category, InputKind, TxRecord, UtxoRecord, WalletEntry,
};
use corvin_core::wallet::HdWalletState;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use uuid::Uuid;

/// SSE event payload
#[derive(Debug, Clone)]
pub struct SseEvent {
    pub kind: String,
    pub payload: serde_json::Value,
}

/// Live connection state for one backend, maintained by its subscriber worker.
/// Keyed in `AppState::backend_status` by `Option<backend id>` (`None` = the
/// default backend). Read by `GET /backends/status` for per-wallet indicators.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BackendStatus {
    pub connected: bool,
    pub tip_height: Option<u32>,
    pub error: Option<String>,
}

/// Commands sent to the subscriber task when wallets change.
pub enum SubCommand {
    AddWallet { id: Uuid, scripts: Vec<ScriptBuf> },
    UpdateScripts { id: Uuid, scripts: Vec<ScriptBuf> },
    RemoveWallet(Uuid),
}

/// Cached state for a single-address wallet (populated on first sync).
pub struct AddressCache {
    pub balance: Balance,
    pub txs: Vec<TxRecord>,
}

/// Cached state for a Silent Payments wallet. SP wallets don't have a BDK
/// descriptor — the scanner subscription discovers outputs and writes them
/// here. Balance/utxos/txs are *derived* from `outputs` on read so we only
/// have one source of truth to keep consistent.
#[derive(Default)]
pub struct SilentPaymentsCache {
    /// Discovered SP outputs in chronological order (oldest first). Populated
    /// from `sp_outputs.json` on startup, appended-to live as the scanner
    /// finds new matches.
    pub outputs: Vec<corvin_core::types::SpOutputRecord>,
    /// Latest chain tip the scanner has seen — used to compute confirmations
    /// for displayed txs/utxos. Zero until the first scan update.
    pub tip_height: u32,
}

impl SilentPaymentsCache {
    pub fn balance(&self) -> Balance {
        let mut confirmed: u64 = 0;
        let mut unconfirmed: u64 = 0;
        for o in &self.outputs {
            if o.spent {
                continue;
            }
            if o.height == 0 {
                unconfirmed += o.value_sats;
            } else {
                confirmed += o.value_sats;
            }
        }
        Balance {
            confirmed_sats: confirmed,
            unconfirmed_sats: unconfirmed,
            spendable_sats: confirmed,
            immature_sats: 0,
            last_synced: None,
        }
    }

    pub fn txs(&self) -> Vec<TxRecord> {
        self.outputs
            .iter()
            .map(|o| {
                let confirmations = if o.height == 0 || self.tip_height == 0 {
                    0
                } else {
                    self.tip_height.saturating_sub(o.height).saturating_add(1)
                };
                TxRecord {
                    txid: o.txid.clone(),
                    amount_sats: o.value_sats as i64,
                    fee_sats: None,
                    vsize: None,
                    confirmations,
                    block_height: if o.height == 0 { None } else { Some(o.height) },
                    timestamp: Some(o.found_at),
                    is_rbf: false,
                    is_coinbase: false,
                }
            })
            .collect()
    }

    pub fn utxos(&self) -> Vec<UtxoRecord> {
        self.outputs
            .iter()
            .filter(|o| !o.spent)
            .map(|o| {
                let confirmations = if o.height == 0 || self.tip_height == 0 {
                    0
                } else {
                    self.tip_height.saturating_sub(o.height).saturating_add(1)
                };
                UtxoRecord {
                    txid: o.txid.clone(),
                    vout: o.vout,
                    amount_sats: o.value_sats,
                    address: None,
                    confirmations,
                    block_height: if o.height == 0 { None } else { Some(o.height) },
                    is_coinbase: false,
                    is_mature: true,
                }
            })
            .collect()
    }
}

// The Hd variant carries ~2KB of BDK state + SQLite connection; the Address
// variant is tiny. Boxing the Hd variant to balance sizes adds a heap indirect
// on the hot signing path for negligible savings (a handful of wallets at
// most), so we accept the size disparity.
#[allow(clippy::large_enum_variant)]
pub enum WalletInner {
    /// HD wallet (xpub/ypub/zpub) backed by a BDK Wallet + SQLite persistence store.
    Hd(Mutex<HdWalletState>),
    /// Single watched address — no BDK Wallet needed; data fetched via Electrum scripthash API.
    Address(Mutex<Option<AddressCache>>),
    /// BIP-352 Silent Payments wallet — discovered outputs populated by the
    /// SP scanner, not by BDK script sync.
    SilentPayments(Mutex<SilentPaymentsCache>),
}

pub struct ManagedWallet {
    pub entry: WalletEntry,
    /// Mutable label — updated without rebuilding the wallet.
    pub label: std::sync::RwLock<String>,
    /// Mutable pinned backend (the id of a registry entry, or None for the
    /// default). Like `label`, kept here so it can change without rebuilding the
    /// wallet; seeded from `entry.backend` on load. Read it via `backend()`.
    pub backend: std::sync::RwLock<Option<String>>,
    pub inner: WalletInner,
    /// Cached fee_sats per txid, populated after each HD sync.
    pub fee_cache: Mutex<HashMap<String, u64>>,
    pub last_synced: Mutex<Option<DateTime<Utc>>>,
    /// Cached derived data so reads don't block on the BDK lock during sync.
    /// `Arc<Vec<T>>` keeps reads O(1) — sync swaps the Arc atomically.
    pub txs_snapshot: Mutex<Arc<Vec<TxRecord>>>,
    pub utxos_snapshot: Mutex<Arc<Vec<UtxoRecord>>>,
    pub balance_snapshot: Mutex<Option<Balance>>,
    pub addresses_snapshot: Mutex<Arc<Vec<AddressInfo>>>,
}

pub struct WalletSnapshot {
    pub txs: Vec<TxRecord>,
    pub utxos: Vec<UtxoRecord>,
    pub balance: Balance,
    pub addresses: Vec<AddressInfo>,
}

/// Build all snapshot fields in one pass. Caller holds the BDK lock.
pub fn compute_snapshot(
    wallet: &bdk_wallet::Wallet,
    fees: &HashMap<String, u64>,
) -> WalletSnapshot {
    WalletSnapshot {
        txs: corvin_core::wallet::get_transactions(wallet, fees),
        utxos: corvin_core::wallet::get_utxos(wallet),
        balance: corvin_core::wallet::get_balance(wallet),
        addresses: corvin_core::wallet::get_addresses(wallet),
    }
}

pub struct WalletManager {
    pub wallets: HashMap<Uuid, Arc<ManagedWallet>>,
    pub network: Network,
}

impl ManagedWallet {
    /// The wallet's currently-pinned backend id (None = default backend).
    pub fn backend(&self) -> Option<String> {
        // A poisoned lock only means some holder panicked; recover the value
        // rather than cascade the panic through every reader.
        self.backend.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn label(&self) -> String {
        self.label.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_label(&self, label: String) {
        *self.label.write().unwrap_or_else(|e| e.into_inner()) = label;
    }

    pub fn set_backend_id(&self, backend: Option<String>) {
        *self.backend.write().unwrap_or_else(|e| e.into_inner()) = backend;
    }
}

impl WalletManager {
    pub fn new(network: Network) -> Self {
        Self {
            wallets: HashMap::new(),
            network,
        }
    }

    pub fn get(&self, id: &Uuid) -> Option<Arc<ManagedWallet>> {
        self.wallets.get(id).cloned()
    }

    pub fn list_entries(&self) -> Vec<WalletEntry> {
        let mut entries: Vec<_> = self
            .wallets
            .values()
            .map(|m| {
                let mut e = m.entry.clone();
                e.label = m.label();
                e.backend = m.backend();
                e
            })
            .collect();
        entries.sort_by_key(|e| e.created_at);
        entries
    }

    pub fn set_backend(&self, id: &Uuid, backend: Option<String>) -> bool {
        if let Some(m) = self.wallets.get(id) {
            m.set_backend_id(backend);
            true
        } else {
            false
        }
    }

    pub fn rename(&self, id: &Uuid, new_label: String) -> bool {
        if let Some(m) = self.wallets.get(id) {
            m.set_label(new_label);
            true
        } else {
            false
        }
    }

    pub fn add(&mut self, entry: WalletEntry, inner: WalletInner) -> Arc<ManagedWallet> {
        let managed = Arc::new(ManagedWallet {
            label: std::sync::RwLock::new(entry.label.clone()),
            backend: std::sync::RwLock::new(entry.backend.clone()),
            entry: entry.clone(),
            inner,
            fee_cache: Mutex::new(HashMap::new()),
            last_synced: Mutex::new(None),
            txs_snapshot: Mutex::new(Arc::new(Vec::new())),
            utxos_snapshot: Mutex::new(Arc::new(Vec::new())),
            balance_snapshot: Mutex::new(None),
            addresses_snapshot: Mutex::new(Arc::new(Vec::new())),
        });
        self.wallets.insert(entry.id, Arc::clone(&managed));
        managed
    }

    pub fn remove(&mut self, id: &Uuid) -> bool {
        self.wallets.remove(id).is_some()
    }
}

/// The canonical key for a UTXO in the freeze set and label maps. Composing
/// `txid:vout` inline elsewhere risks a silent lookup miss if the format drifts.
pub fn utxo_key(txid: &str, vout: u32) -> String {
    format!("{txid}:{vout}")
}

/// Plain carrier for the full annotation set — used to push a complete state
/// in during a backup restore.
#[derive(Default)]
pub struct AnnotationData {
    pub tx_labels: HashMap<String, String>,
    pub cost_basis: HashMap<String, f64>,
    pub utxo_labels: HashMap<String, String>,
    pub address_labels: HashMap<String, String>,
    pub frozen_utxos: HashSet<String>,
}

/// Coin-category state: the user's category definitions plus the assignment maps
/// (address → category id, outpoint → category id). Persisted as one
/// `categories.json` since deleting a definition must also drop its assignments.
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct CategoryData {
    pub definitions: Vec<Category>,
    /// address → category id (inherited by coins received there)
    pub addresses: HashMap<String, String>,
    /// outpoint "txid:vout" → category id (overrides the address's category)
    pub utxos: HashMap<String, String>,
}

/// The user's private annotations: transaction/address/UTXO labels, cost-basis
/// overrides, and frozen UTXOs. Owns both the in-memory maps and their on-disk
/// persistence, so a caller can't mutate a map and forget to save it — every
/// mutator writes through to disk.
#[derive(Clone)]
pub struct Annotations {
    tx_labels: Arc<RwLock<HashMap<String, String>>>,
    cost_basis: Arc<RwLock<HashMap<String, f64>>>,
    utxo_labels: Arc<RwLock<HashMap<String, String>>>,
    address_labels: Arc<RwLock<HashMap<String, String>>>,
    frozen_utxos: Arc<RwLock<HashSet<String>>>,
    categories: Arc<RwLock<CategoryData>>,
}

impl Annotations {
    /// Load every annotation file. A missing or corrupt file starts empty —
    /// same tolerant startup as before; a bad labels file shouldn't block boot.
    pub fn load() -> Self {
        Self {
            tx_labels: Arc::new(RwLock::new(load_labels_sync().unwrap_or_default())),
            cost_basis: Arc::new(RwLock::new(load_cost_basis_sync().unwrap_or_default())),
            utxo_labels: Arc::new(RwLock::new(load_utxo_labels_sync().unwrap_or_default())),
            address_labels: Arc::new(RwLock::new(load_address_labels_sync().unwrap_or_default())),
            frozen_utxos: Arc::new(RwLock::new(load_frozen_utxos_sync().unwrap_or_default())),
            categories: Arc::new(RwLock::new(load_categories_sync().unwrap_or_default())),
        }
    }

    // Reads return a clone — callers serve them as JSON or iterate without
    // holding the lock.
    pub async fn tx_labels(&self) -> HashMap<String, String> {
        self.tx_labels.read().await.clone()
    }
    pub async fn cost_basis(&self) -> HashMap<String, f64> {
        self.cost_basis.read().await.clone()
    }
    pub async fn utxo_labels(&self) -> HashMap<String, String> {
        self.utxo_labels.read().await.clone()
    }
    pub async fn address_labels(&self) -> HashMap<String, String> {
        self.address_labels.read().await.clone()
    }
    pub async fn frozen_utxos(&self) -> HashSet<String> {
        self.frozen_utxos.read().await.clone()
    }

    /// Set, or (on an empty/whitespace note) clear, a transaction label.
    pub async fn set_tx_label(&self, txid: String, note: &str) -> Result<()> {
        let mut m = self.tx_labels.write().await;
        if note.trim().is_empty() {
            m.remove(&txid);
        } else {
            m.insert(txid, note.trim().to_string());
        }
        save_labels(&m).await
    }

    pub async fn set_cost_basis(&self, txid: String, usd: f64) -> Result<()> {
        let mut m = self.cost_basis.write().await;
        m.insert(txid, usd);
        save_cost_basis(&m).await
    }

    pub async fn remove_cost_basis(&self, txid: &str) -> Result<()> {
        let mut m = self.cost_basis.write().await;
        m.remove(txid);
        save_cost_basis(&m).await
    }

    pub async fn set_utxo_label(&self, key: String, note: &str) -> Result<()> {
        let mut m = self.utxo_labels.write().await;
        if note.trim().is_empty() {
            m.remove(&key);
        } else {
            m.insert(key, note.trim().to_string());
        }
        save_utxo_labels(&m).await
    }

    pub async fn set_address_label(&self, addr: String, note: &str) -> Result<()> {
        let mut m = self.address_labels.write().await;
        if note.trim().is_empty() {
            m.remove(&addr);
        } else {
            m.insert(addr, note.trim().to_string());
        }
        save_address_labels(&m).await
    }

    pub async fn remove_address_label(&self, addr: &str) -> Result<()> {
        let mut m = self.address_labels.write().await;
        m.remove(addr);
        save_address_labels(&m).await
    }

    /// Freeze (`frozen = true`) or unfreeze a UTXO outpoint key.
    pub async fn set_frozen(&self, key: String, frozen: bool) -> Result<()> {
        let mut m = self.frozen_utxos.write().await;
        if frozen {
            m.insert(key);
        } else {
            m.remove(&key);
        }
        save_frozen_utxos(&m).await
    }

    // ── Categories ──────────────────────────────────────────────────────────
    pub async fn categories(&self) -> CategoryData {
        self.categories.read().await.clone()
    }

    /// Create a category with a fresh id; returns the created definition.
    pub async fn add_category(&self, name: &str, color: &str) -> Result<Category> {
        let cat = Category {
            id: Uuid::new_v4().to_string(),
            name: name.trim().to_string(),
            color: color.trim().to_string(),
        };
        let mut c = self.categories.write().await;
        c.definitions.push(cat.clone());
        save_categories(&c).await?;
        Ok(cat)
    }

    /// Rename / recolor an existing category. No-op if the id is unknown.
    pub async fn update_category(&self, id: &str, name: &str, color: &str) -> Result<()> {
        let mut c = self.categories.write().await;
        if let Some(def) = c.definitions.iter_mut().find(|d| d.id == id) {
            def.name = name.trim().to_string();
            def.color = color.trim().to_string();
        }
        save_categories(&c).await
    }

    /// Delete a category definition and drop every assignment that referenced it.
    pub async fn delete_category(&self, id: &str) -> Result<()> {
        let mut c = self.categories.write().await;
        c.definitions.retain(|d| d.id != id);
        c.addresses.retain(|_, v| v != id);
        c.utxos.retain(|_, v| v != id);
        save_categories(&c).await
    }

    /// Assign (or, with `None`, clear) an address's category.
    pub async fn set_address_category(&self, addr: String, id: Option<&str>) -> Result<()> {
        let mut c = self.categories.write().await;
        match id {
            Some(id) => { c.addresses.insert(addr, id.to_string()); }
            None => { c.addresses.remove(&addr); }
        }
        save_categories(&c).await
    }

    /// Assign (or clear) a single UTXO's category override.
    pub async fn set_utxo_category(&self, outpoint: String, id: Option<&str>) -> Result<()> {
        let mut c = self.categories.write().await;
        match id {
            Some(id) => { c.utxos.insert(outpoint, id.to_string()); }
            None => { c.utxos.remove(&outpoint); }
        }
        save_categories(&c).await
    }

    pub async fn replace_categories(&self, data: CategoryData) -> Result<()> {
        let mut c = self.categories.write().await;
        *c = data;
        save_categories(&c).await
    }

    /// Replace every map wholesale — a full restore from a backup.
    pub async fn replace_all(&self, data: AnnotationData) -> Result<()> {
        {
            let mut m = self.tx_labels.write().await;
            *m = data.tx_labels;
            save_labels(&m).await?;
        }
        {
            let mut m = self.cost_basis.write().await;
            *m = data.cost_basis;
            save_cost_basis(&m).await?;
        }
        {
            let mut m = self.utxo_labels.write().await;
            *m = data.utxo_labels;
            save_utxo_labels(&m).await?;
        }
        {
            let mut m = self.address_labels.write().await;
            *m = data.address_labels;
            save_address_labels(&m).await?;
        }
        {
            let mut m = self.frozen_utxos.write().await;
            *m = data.frozen_utxos;
            save_frozen_utxos(&m).await?;
        }
        Ok(())
    }

    /// Replace just the label + freeze maps. BIP-329 import carries no cost
    /// basis, so it must leave that untouched.
    pub async fn replace_labels(
        &self,
        tx_labels: HashMap<String, String>,
        address_labels: HashMap<String, String>,
        utxo_labels: HashMap<String, String>,
        frozen: HashSet<String>,
    ) -> Result<()> {
        {
            let mut m = self.tx_labels.write().await;
            *m = tx_labels;
            save_labels(&m).await?;
        }
        {
            let mut m = self.address_labels.write().await;
            *m = address_labels;
            save_address_labels(&m).await?;
        }
        {
            let mut m = self.utxo_labels.write().await;
            *m = utxo_labels;
            save_utxo_labels(&m).await?;
        }
        {
            let mut m = self.frozen_utxos.write().await;
            *m = frozen;
            save_frozen_utxos(&m).await?;
        }
        Ok(())
    }
}

/// BTC/USD price caching: the persisted historical (day → USD) map plus an
/// in-memory 60-second cache of the live spot price. Owns persistence, so a
/// caller can't record a historical price and forget to save it.
#[derive(Clone)]
pub struct PriceCache {
    historical: Arc<Mutex<HashMap<i64, f64>>>,
    current: Arc<Mutex<Option<(f64, std::time::Instant)>>>,
}

impl PriceCache {
    pub fn load() -> Self {
        Self {
            historical: Arc::new(Mutex::new(load_price_cache_sync().unwrap_or_default())),
            current: Arc::new(Mutex::new(None)),
        }
    }

    /// Cached historical price for a UTC day-start, if present.
    pub async fn historical(&self, day: i64) -> Option<f64> {
        self.historical.lock().await.get(&day).copied()
    }

    /// Record a historical price and persist the cache.
    pub async fn store_historical(&self, day: i64, price: f64) -> Result<()> {
        let snapshot = {
            let mut c = self.historical.lock().await;
            c.insert(day, price);
            c.clone()
        };
        save_price_cache(&snapshot).await
    }

    /// The live spot price, but only if it was fetched within the last 60s.
    pub async fn current_if_fresh(&self) -> Option<f64> {
        self.current.lock().await.and_then(|(price, at)| {
            (at.elapsed() < std::time::Duration::from_secs(60)).then_some(price)
        })
    }

    /// Record the live spot price, timestamped now.
    pub async fn store_current(&self, price: f64) {
        *self.current.lock().await = Some((price, std::time::Instant::now()));
    }
}

#[derive(Clone)]
pub struct AppState {
    pub manager: Arc<RwLock<WalletManager>>,
    pub config: Arc<RwLock<Config>>,
    pub event_tx: broadcast::Sender<SseEvent>,
    pub sub_tx: mpsc::UnboundedSender<SubCommand>,
    /// User annotations (labels, cost basis, freezes) and their persistence.
    pub annotations: Annotations,
    /// Historical + live BTC/USD price caching, with persistence.
    pub prices: PriceCache,
    pub http: Arc<RwLock<reqwest::Client>>,
    /// HTTP client for the mempool server (fees + price). Separate from `http`
    /// because the mempool carries its own TLS-accept setting
    /// (`mempool_danger_accept_invalid_certs`) — it's commonly a self-hosted,
    /// self-signed instance distinct from the payjoin/BIP-353 endpoints `http` hits.
    pub mempool_http: Arc<RwLock<reqwest::Client>>,
    /// Serializes hardware-wallet device access. The BitBox noise-config file and
    /// USB device cannot tolerate concurrent operations.
    #[cfg(feature = "hw")]
    pub hwi_lock: Arc<Mutex<()>>,
    /// Token → SignJob. Consumed by `GET /hwi/sign/{token}`. PSBTs go here
    /// (rather than the SSE URL) because multisig PSBTs can exceed browser
    /// URL-length limits.
    #[cfg(feature = "hw")]
    pub hwi_sign_jobs: Arc<Mutex<HashMap<String, SignJob>>>,
    /// Wakes the subscriber from backoff sleep (called on laptop wake).
    pub wake_signal: Arc<tokio::sync::Notify>,
    /// Per-SP-wallet scanner task handles, so a wallet's scanner can be
    /// (re)started at runtime — on creation, and on label add (to re-subscribe
    /// with the new label) — without an app restart.
    pub sp_scanners: Arc<Mutex<HashMap<Uuid, tokio::task::JoinHandle<()>>>>,
    /// Per-payjoin-session poll task handles (BIP-77 sender side). Keyed by
    /// session id so a session's long-poll can be respawned at runtime (on
    /// send) and on startup, and aborted when the session resolves.
    pub payjoin_tasks: Arc<Mutex<HashMap<Uuid, tokio::task::JoinHandle<()>>>>,
    /// Live per-backend connection state, written by the subscriber workers and
    /// read by `GET /backends/status`. Keyed by `Option<backend id>` (`None` =
    /// default backend). A plain std lock — workers update it from blocking code.
    pub backend_status: Arc<std::sync::RwLock<HashMap<Option<String>, BackendStatus>>>,
    /// Live SP-scanner connection state, written by the per-wallet sp_subscriber
    /// tasks and read by `GET /sp/status`. Keyed by wallet id (one scanner per SP
    /// wallet). Separate from `backend_status` because an SP wallet's `None` backend
    /// means "public Frigate", not the default Electrum, so the two keyspaces can't
    /// share a map.
    pub sp_status: Arc<std::sync::RwLock<HashMap<Uuid, BackendStatus>>>,
    /// When booting locked (at-rest encryption on), services aren't started until
    /// unlock. The subscriber's command receiver waits here and is taken once by
    /// the post-unlock startup. `None` after a normal plaintext boot (the
    /// subscriber was spawned immediately) or after unlock consumed it.
    pub startup_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<SubCommand>>>>,
}

#[cfg(feature = "hw")]
#[derive(Clone)]
pub struct SignJob {
    pub psbt_b64: String,
    pub wallet_id: Option<Uuid>,
    pub registered_at: std::time::Instant,
}

pub fn build_http_client(
    socks5_proxy: Option<&str>,
    accept_invalid_certs: bool,
) -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder();
    if let Some(proxy) = socks5_proxy {
        let p = reqwest::Proxy::all(format!("socks5h://{proxy}"))
            .with_context(|| format!("parsing SOCKS5 proxy '{proxy}'"))?;
        builder = builder.proxy(p);
    }
    if accept_invalid_certs {
        builder = builder.danger_accept_invalid_certs(true);
    }
    builder.build().context("building HTTP client")
}

impl AppState {
    pub fn new(config: Config, network: Network) -> (Self, mpsc::UnboundedReceiver<SubCommand>) {
        let (event_tx, _) = broadcast::channel(128);
        let (sub_tx, sub_rx) = mpsc::unbounded_channel();
        let http_proxy = config.backend.socks5_proxy.clone();
        let accept_invalid_certs = config.backend.danger_accept_invalid_certs;
        let http = build_http_client(http_proxy.as_deref(), accept_invalid_certs)
            .unwrap_or_else(|e| {
                tracing::error!(
                    "HTTP client build failed at startup ({e:#}) — falling back to a default client. \
                     If you configured a SOCKS5 proxy, traffic WILL NOT be routed through it until you fix the config."
                );
                reqwest::Client::new()
            });
        let mempool_http = build_http_client(
            http_proxy.as_deref(),
            config.backend.mempool_danger_accept_invalid_certs,
        )
        .unwrap_or_else(|_| reqwest::Client::new());
        let state = Self {
            manager: Arc::new(RwLock::new(WalletManager::new(network))),
            config: Arc::new(RwLock::new(config)),
            event_tx,
            sub_tx,
            annotations: Annotations::load(),
            prices: PriceCache::load(),
            http: Arc::new(RwLock::new(http)),
            mempool_http: Arc::new(RwLock::new(mempool_http)),
            #[cfg(feature = "hw")]
            hwi_lock: Arc::new(Mutex::new(())),
            #[cfg(feature = "hw")]
            hwi_sign_jobs: Arc::new(Mutex::new(HashMap::new())),
            wake_signal: Arc::new(tokio::sync::Notify::new()),
            sp_scanners: Arc::new(Mutex::new(HashMap::new())),
            payjoin_tasks: Arc::new(Mutex::new(HashMap::new())),
            backend_status: Arc::new(std::sync::RwLock::new(HashMap::new())),
            sp_status: Arc::new(std::sync::RwLock::new(HashMap::new())),
            startup_rx: Arc::new(Mutex::new(None)),
        };
        (state, sub_rx)
    }

    pub fn emit(&self, kind: &str, payload: serde_json::Value) {
        let _ = self.event_tx.send(SseEvent {
            kind: kind.to_string(),
            payload,
        });
    }

    /// Mark a backend connected and (optionally) record its chain tip. Called by
    /// the subscriber worker when its session is live.
    pub fn backend_connected(&self, key: Option<&str>, tip_height: Option<u32>) {
        let mut m = self.backend_status.write().unwrap_or_else(|e| e.into_inner());
        let e = m.entry(key.map(str::to_string)).or_default();
        e.connected = true;
        e.error = None;
        if tip_height.is_some() {
            e.tip_height = tip_height;
        }
    }

    /// Mark a backend disconnected with an optional error message. Keeps the last
    /// known tip so the UI can still show it while reconnecting.
    pub fn backend_disconnected(&self, key: Option<&str>, error: Option<String>) {
        let mut m = self.backend_status.write().unwrap_or_else(|e| e.into_inner());
        let e = m.entry(key.map(str::to_string)).or_default();
        e.connected = false;
        e.error = error;
    }

    /// Mark an SP wallet's Frigate scanner connected. Called by its sp_subscriber task.
    pub fn sp_connected(&self, wallet_id: Uuid) {
        let mut m = self.sp_status.write().unwrap_or_else(|e| e.into_inner());
        let e = m.entry(wallet_id).or_default();
        e.connected = true;
        e.error = None;
    }

    /// Mark an SP wallet's scanner disconnected with an optional error message.
    pub fn sp_disconnected(&self, wallet_id: Uuid, error: Option<String>) {
        let mut m = self.sp_status.write().unwrap_or_else(|e| e.into_inner());
        let e = m.entry(wallet_id).or_default();
        e.connected = false;
        e.error = error;
    }

    /// Drop an SP wallet's scanner status (on wallet delete).
    pub fn sp_status_remove(&self, wallet_id: Uuid) {
        self.sp_status
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&wallet_id);
    }
}

/// Extract all scripts to subscribe to for a wallet.
/// For HD wallets this includes revealed SPKs plus a lookahead of 20 external addresses.
pub async fn get_scripts(managed: &ManagedWallet) -> Vec<ScriptBuf> {
    match &managed.inner {
        // SP wallets don't subscribe via Electrum scripthash — the SP scanner
        // session handles discovery. No scripts to register here.
        WalletInner::SilentPayments(_) => Vec::new(),
        WalletInner::Address(_) => {
            use bdk_wallet::bitcoin::address::NetworkUnchecked;
            managed
                .entry
                .input
                .parse::<bdk_wallet::bitcoin::Address<NetworkUnchecked>>()
                .map(|a| vec![a.assume_checked().script_pubkey()])
                .unwrap_or_default()
        }
        WalletInner::Hd(m) => {
            let hd = m.lock().await;
            let wallet = &hd.wallet;
            let mut scripts = Vec::new();
            let mut next_ext: u32 = 0;

            for (i, spk) in wallet
                .spk_index()
                .revealed_keychain_spks(KeychainKind::External)
            {
                scripts.push(spk.to_owned());
                next_ext = i + 1;
            }
            for (_, spk) in wallet
                .spk_index()
                .revealed_keychain_spks(KeychainKind::Internal)
            {
                scripts.push(spk.to_owned());
            }
            // Lookahead: subscribe to the next 20 external addresses so we catch
            // incoming transactions before they're revealed by a full scan.
            for i in next_ext..next_ext + 20 {
                scripts.push(
                    wallet
                        .peek_address(KeychainKind::External, i)
                        .address
                        .script_pubkey(),
                );
            }

            scripts
        }
    }
}

/// Restrict an existing file's permissions to owner-only (0600). Best-effort on errors.
#[cfg_attr(not(unix), allow(unused_variables))]
pub fn restrict_to_owner(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
            tracing::warn!("Failed to restrict permissions on {}: {e}", path.display());
        }
    }
}

/// Write `content` to `path` atomically, owner-only (0600), at-rest-sealed.
/// Thin wrapper over the shared core writer (passes plaintext through when
/// encryption is off, AEAD-seals it bound to the file name when unlocked).
pub(crate) fn write_private(path: &std::path::Path, content: &[u8]) -> Result<()> {
    corvin_core::at_rest::write_sealed(path, content)
}

/// Read a file's plaintext bytes, un-sealing it. `Ok(None)` if it doesn't exist.
pub(crate) fn read_private(path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    corvin_core::at_rest::read_sealed(path)
}

/// Load a JSON file into `T`, returning `T::default()` if it doesn't exist or
/// fails to parse. A decrypt failure (when unlocked) is a hard error rather than
/// a silent empty, so we never overwrite unreadable encrypted data. Sync.
fn load_json<T: DeserializeOwned + Default>(path: std::path::PathBuf) -> Result<T> {
    let Some(raw) = read_private(&path)? else {
        return Ok(T::default());
    };
    match serde_json::from_slice(&raw) {
        Ok(v) => Ok(v),
        Err(e) => {
            // Tolerant startup: a corrupt file shouldn't block boot. But warn
            // loudly — the next write to this store will overwrite it, so silent
            // recovery could otherwise mean unnoticed data loss.
            tracing::warn!(
                "{} is present but failed to parse ({e}); starting empty — it will be \
                 overwritten on the next change",
                path.display()
            );
            Ok(T::default())
        }
    }
}

/// Load a JSON store that holds **non-derivable** data (the SP scan secret, SP output
/// tweaks, Ledger HMACs, payjoin anti-replay). On a *parse* failure the bytes decrypted
/// fine but the JSON is garbage, so we **quarantine** the file (rename to `.corrupt`)
/// and start empty — otherwise the next write would overwrite the readable-but-corrupt
/// file and turn a recoverable corruption into permanent loss. Absent or unreadable
/// (decrypt/IO error) → default with the file left intact. Sync.
pub(crate) fn load_or_quarantine<T: DeserializeOwned + Default>(path: &std::path::Path) -> T {
    match read_private(path) {
        Ok(Some(bytes)) => match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => {
                let corrupt = path.with_extension("corrupt");
                match std::fs::rename(path, &corrupt) {
                    Ok(()) => tracing::error!(
                        "{} is corrupt ({e}); quarantined to {} — restore from a backup. Starting empty.",
                        path.display(),
                        corrupt.display()
                    ),
                    Err(re) => tracing::error!(
                        "{} is corrupt ({e}) and could not be quarantined ({re}); starting empty",
                        path.display()
                    ),
                }
                T::default()
            }
        },
        Ok(None) => T::default(),
        Err(e) => {
            tracing::error!("{} is unreadable ({e}); starting empty (file left intact)", path.display());
            T::default()
        }
    }
}

/// Atomically persist `value` as pretty JSON, owner-only. The file write runs on
/// a blocking thread so it never stalls the async runtime.
async fn save_json<T: Serialize>(path: std::path::PathBuf, value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    tokio::task::spawn_blocking(move || -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_private(&path, json.as_bytes())
    })
    .await
    .context("save_json: blocking write task panicked")?
}

/// Persist wallet entries to disk
pub async fn save_wallets(manager: &WalletManager) -> Result<()> {
    save_json(wallets_path(), &manager.list_entries()).await
}

pub fn load_labels_sync() -> Result<HashMap<String, String>> {
    load_json(labels_path())
}

pub async fn save_labels(labels: &HashMap<String, String>) -> Result<()> {
    save_json(labels_path(), labels).await
}

pub fn load_price_cache_sync() -> Result<HashMap<i64, f64>> {
    load_json(price_cache_path())
}

pub async fn save_price_cache(cache: &HashMap<i64, f64>) -> Result<()> {
    save_json(price_cache_path(), cache).await
}

pub fn load_cost_basis_sync() -> Result<HashMap<String, f64>> {
    load_json(cost_basis_path())
}

pub async fn save_cost_basis(cb: &HashMap<String, f64>) -> Result<()> {
    save_json(cost_basis_path(), cb).await
}

pub fn load_utxo_labels_sync() -> Result<HashMap<String, String>> {
    load_json(utxo_labels_path())
}

pub async fn save_utxo_labels(labels: &HashMap<String, String>) -> Result<()> {
    save_json(utxo_labels_path(), labels).await
}

pub fn load_address_labels_sync() -> Result<HashMap<String, String>> {
    load_json(address_labels_path())
}

pub async fn save_address_labels(labels: &HashMap<String, String>) -> Result<()> {
    save_json(address_labels_path(), labels).await
}

pub fn load_categories_sync() -> Result<CategoryData> {
    load_json(categories_path())
}
pub async fn save_categories(data: &CategoryData) -> Result<()> {
    save_json(categories_path(), data).await
}

pub fn load_frozen_utxos_sync() -> Result<HashSet<String>> {
    // Stored on disk as a sorted list; held in memory as a set.
    Ok(load_json::<Vec<String>>(frozen_utxos_path())?
        .into_iter()
        .collect())
}

pub async fn save_frozen_utxos(frozen: &HashSet<String>) -> Result<()> {
    let mut list: Vec<&String> = frozen.iter().collect();
    list.sort();
    save_json(frozen_utxos_path(), &list).await
}

/// Load wallet entries and recreate in-memory state
pub async fn load_wallets(manager: &mut WalletManager) -> Result<()> {
    let path = wallets_path();
    // Must go through read_private so an encrypted wallets.json is decrypted —
    // save_wallets writes it sealed, so a raw read would fail to parse when on.
    let Some(raw) = read_private(&path)? else {
        return Ok(());
    };
    // Parse entries individually so one malformed/future-version entry doesn't make
    // *every* wallet vanish. A wholesale parse failure is still surfaced to the caller.
    let raw_entries: Vec<serde_json::Value> = serde_json::from_slice(&raw)?;
    let entries: Vec<WalletEntry> = raw_entries
        .into_iter()
        .filter_map(|v| match serde_json::from_value::<WalletEntry>(v) {
            Ok(e) => Some(e),
            Err(e) => {
                tracing::error!("skipping an unparseable wallets.json entry ({e}); other wallets load normally");
                None
            }
        })
        .collect();

    for entry in entries {
        // Snapshot at load so the first request after restart serves immediately.
        let (inner, snapshot) = match entry.kind {
            InputKind::Address => (WalletInner::Address(Mutex::new(None)), None),
            InputKind::SilentPayments => {
                // Hydrate the cache from the persisted sp_outputs.json so the
                // wallet's balance/utxo/tx endpoints serve correct data on
                // first request after restart (before the scanner reconnects).
                let outputs = crate::sp_outputs::load_for_wallet(entry.id);
                let cache = SilentPaymentsCache {
                    outputs,
                    tip_height: 0,
                };
                (WalletInner::SilentPayments(Mutex::new(cache)), None)
            }
            // Explicit so adding an InputKind variant fails to compile here
            // instead of silently routing through the HD path.
            InputKind::Xpub
            | InputKind::Ypub
            | InputKind::Zpub
            | InputKind::Taproot
            | InputKind::Multisig
            | InputKind::Descriptor => {
                let db_path = wallet_db_path(entry.id);
                match corvin_core::wallet::open_or_create_wallet(&entry, manager.network, &db_path)
                {
                    Ok(hd) => {
                        restrict_to_owner(&db_path);
                        // Empty fees: tx.fee_sats stays None until first sync.
                        let snap = compute_snapshot(&hd.wallet, &HashMap::new());
                        (WalletInner::Hd(Mutex::new(hd)), Some(snap))
                    }
                    Err(e) => {
                        tracing::warn!("Failed to restore wallet {}: {e:#}", entry.id);
                        continue;
                    }
                }
            }
        };
        let managed = manager.add(entry, inner);
        if let Some(s) = snapshot {
            *managed.txs_snapshot.lock().await = Arc::new(s.txs);
            *managed.utxos_snapshot.lock().await = Arc::new(s.utxos);
            *managed.balance_snapshot.lock().await = Some(s.balance);
            *managed.addresses_snapshot.lock().await = Arc::new(s.addresses);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shared throwaway config dir for the whole test binary (see
    // config::test_isolate_config_dir). Tests touch disjoint files, so sharing
    // it is safe under parallelism.
    fn isolate_config() {
        crate::config::test_isolate_config_dir();
    }

    fn txid(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    #[test]
    fn utxo_key_format() {
        assert_eq!(utxo_key("abcd", 3), "abcd:3");
    }

    // One test owns the annotation files (labels/cost_basis/frozen/…) so it
    // can't race the price-cache test, which touches a disjoint file.
    #[tokio::test]
    async fn annotations_subsystem() {
        isolate_config();
        let ann = Annotations::load();

        // A non-empty note sets; an empty/whitespace note clears.
        ann.set_tx_label(txid('a'), "groceries").await.unwrap();
        assert_eq!(
            ann.tx_labels().await.get(&txid('a')).map(String::as_str),
            Some("groceries")
        );
        ann.set_tx_label(txid('a'), "   ").await.unwrap();
        assert!(!ann.tx_labels().await.contains_key(&txid('a')));

        // Freeze / unfreeze round-trips.
        ann.set_frozen("tx:0".into(), true).await.unwrap();
        assert!(ann.frozen_utxos().await.contains("tx:0"));
        ann.set_frozen("tx:0".into(), false).await.unwrap();
        assert!(!ann.frozen_utxos().await.contains("tx:0"));

        // Cost basis is set, and a fresh load sees it (persistence round-trip).
        ann.set_cost_basis(txid('b'), 50.0).await.unwrap();
        assert_eq!(
            Annotations::load()
                .cost_basis()
                .await
                .get(&txid('b'))
                .copied(),
            Some(50.0)
        );

        // The BIP-329 import path replaces the label/freeze maps but must NOT
        // wipe cost basis (it carries none).
        ann.replace_labels(
            HashMap::from([("ref".to_string(), "imported".to_string())]),
            HashMap::new(),
            HashMap::new(),
            HashSet::new(),
        )
        .await
        .unwrap();
        assert_eq!(ann.cost_basis().await.get(&txid('b')).copied(), Some(50.0));
        assert_eq!(
            ann.tx_labels().await.get("ref").map(String::as_str),
            Some("imported")
        );
    }

    #[tokio::test]
    async fn price_cache_spot_ttl_and_historical_persist() {
        isolate_config();
        let pc = PriceCache::load();
        assert!(pc.current_if_fresh().await.is_none());
        pc.store_current(42_000.0).await;
        assert_eq!(pc.current_if_fresh().await, Some(42_000.0));

        pc.store_historical(86_400, 30_000.0).await.unwrap();
        assert_eq!(pc.historical(86_400).await, Some(30_000.0));

        // Historical persists across a reload; the in-memory spot price does not.
        let reloaded = PriceCache::load();
        assert_eq!(reloaded.historical(86_400).await, Some(30_000.0));
        assert!(reloaded.current_if_fresh().await.is_none());
    }
}
