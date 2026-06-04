use bdk_electrum::electrum_client::ElectrumApi;
use bdk_wallet::bitcoin::ScriptBuf;
use corvin_core::backends::electrum::{build_client, build_native_tls_client, needs_custom_tls};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::api::wallets::background_sync;
use crate::config::BackendType;
use crate::state::{AppState, SubCommand};

/// Supervisor: route each wallet's commands to a worker dedicated to that
/// wallet's backend, so each distinct backend gets its own connection. Wallets
/// with no pinned backend (the common case) all share the default group.
pub fn spawn_subscriber(state: AppState, cmd_rx: mpsc::UnboundedReceiver<SubCommand>) {
    tokio::spawn(async move {
        let mut cmd_rx = cmd_rx;
        let mut groups: HashMap<Option<String>, mpsc::UnboundedSender<SubCommand>> = HashMap::new();
        let mut wallet_group: HashMap<Uuid, Option<String>> = HashMap::new();

        while let Some(cmd) = cmd_rx.recv().await {
            match &cmd {
                SubCommand::AddWallet { id, .. } | SubCommand::UpdateScripts { id, .. } => {
                    let id = *id;
                    let key = resolve_group_key(&state, id).await;
                    // If the wallet changed backends, drop it from its old group.
                    if let Some(prev) = wallet_group.get(&id) {
                        if *prev != key {
                            if let Some(tx) = groups.get(prev) {
                                let _ = tx.send(SubCommand::RemoveWallet(id));
                            }
                        }
                    }
                    wallet_group.insert(id, key.clone());
                    let tx = ensure_group(&mut groups, &state, key);
                    let _ = tx.send(cmd);
                }
                SubCommand::RemoveWallet(id) => {
                    let id = *id;
                    match wallet_group.remove(&id) {
                        Some(key) => {
                            if let Some(tx) = groups.get(&key) {
                                let _ = tx.send(cmd);
                            }
                        }
                        // Group unknown — tell every worker; only the owner acts.
                        None => {
                            for tx in groups.values() {
                                let _ = tx.send(SubCommand::RemoveWallet(id));
                            }
                        }
                    }
                }
            }
        }
    });
}

/// The backend group a wallet belongs to: its pinned backend id if that id is a
/// registered backend, else the default group (`None`).
async fn resolve_group_key(state: &AppState, id: Uuid) -> Option<String> {
    let backend = {
        let mgr = state.manager.read().await;
        mgr.get(&id).and_then(|m| m.backend())
    };
    state
        .config
        .read()
        .await
        .effective_backend_id(backend.as_deref())
}

fn ensure_group(
    groups: &mut HashMap<Option<String>, mpsc::UnboundedSender<SubCommand>>,
    state: &AppState,
    key: Option<String>,
) -> mpsc::UnboundedSender<SubCommand> {
    if let Some(tx) = groups.get(&key) {
        return tx.clone();
    }
    let (tx, rx) = mpsc::unbounded_channel();
    spawn_backend_worker(state.clone(), key.clone(), rx);
    groups.insert(key, tx.clone());
    tx
}

/// One connection lifetime manager for a single backend. The original
/// single-connection loop, now scoped to the wallets routed to `backend` and
/// resolving its server config by that key.
fn spawn_backend_worker(
    state: AppState,
    backend: Option<String>,
    cmd_rx: mpsc::UnboundedReceiver<SubCommand>,
) {
    tokio::task::spawn_blocking(move || {
        let rt = Handle::current();
        let mut script_to_wallet: HashMap<ScriptBuf, Uuid> = HashMap::new();
        let mut wallet_scripts: HashMap<Uuid, HashSet<ScriptBuf>> = HashMap::new();
        let mut cmd_rx = cmd_rx;
        // Exponential backoff: 10s → 20s → 40s → … → 300s max.
        let mut backoff_secs: u64 = 10;
        let key = backend.as_deref();

        // Drain commands queued before this worker started.
        while let Ok(cmd) = cmd_rx.try_recv() {
            apply_command(cmd, &mut script_to_wallet, &mut wallet_scripts);
        }

        loop {
            // Offline mode: never connect. Keep our script map current so a later
            // toggle-off resumes cleanly, mark the backend offline, and park until
            // `wake_signal` fires (settings toggle or frontend reconnect kick).
            if rt.block_on(async { state.config.read().await.offline }) {
                while let Ok(cmd) = cmd_rx.try_recv() {
                    apply_command(cmd, &mut script_to_wallet, &mut wallet_scripts);
                }
                state.backend_disconnected(key, Some("offline".into()));
                rt.block_on(async {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(3600),
                        state.wake_signal.notified(),
                    )
                    .await;
                });
                continue;
            }

            let (backend_kind, ecfg) = rt.block_on(async {
                let cfg = state.config.read().await;
                (cfg.backend_kind_for(key), cfg.electrum_config_for(key))
            });

            if matches!(backend_kind, BackendType::Rpc) {
                // RPC has no persistent connection — probe reachability each cycle.
                let rcfg = rt.block_on(async { state.config.read().await.rpc_config_for(key) });
                let status = corvin_core::backends::rpc::probe_status(&rcfg);
                if status.connected {
                    state.backend_connected(key, status.tip_height);
                } else {
                    state.backend_disconnected(key, status.error);
                }
                // RPC backend has no push mechanism — periodic poll.
                for &id in wallet_scripts.keys() {
                    let s = state.clone();
                    rt.spawn(async move { background_sync(s, id).await });
                }
                while let Ok(cmd) = cmd_rx.try_recv() {
                    apply_command(cmd, &mut script_to_wallet, &mut wallet_scripts);
                }
                let interval =
                    rt.block_on(async { state.config.read().await.backend.poll_interval_secs });
                // Interruptible by `wake_signal` (frontend reconnect kick).
                rt.block_on(async {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(interval),
                        state.wake_signal.notified(),
                    )
                    .await;
                });
                continue;
            }

            // ── Electrum backend ──────────────────────────────────────────────

            let poll_interval = Duration::from_secs(
                rt.block_on(async { state.config.read().await.backend.poll_interval_secs }),
            );

            let result = if needs_custom_tls(&ecfg) {
                match build_native_tls_client(&ecfg) {
                    Ok(c) => Some(run_session(
                        &c.inner,
                        &state,
                        &mut cmd_rx,
                        &mut script_to_wallet,
                        &mut wallet_scripts,
                        &rt,
                        poll_interval,
                        key,
                    )),
                    Err(e) => {
                        tracing::warn!("Subscriber[{backend:?}]: connect failed: {e:#}");
                        state.backend_disconnected(key, Some(format!("{e:#}")));
                        None
                    }
                }
            } else {
                match build_client(&ecfg) {
                    Ok(c) => Some(run_session(
                        &c.inner,
                        &state,
                        &mut cmd_rx,
                        &mut script_to_wallet,
                        &mut wallet_scripts,
                        &rt,
                        poll_interval,
                        key,
                    )),
                    Err(e) => {
                        tracing::warn!("Subscriber[{backend:?}]: connect failed: {e:#}");
                        state.backend_disconnected(key, Some(format!("{e:#}")));
                        None
                    }
                }
            };

            match result {
                Some(true) => {
                    // Connection was established but then lost — reset backoff.
                    tracing::warn!("Subscriber[{backend:?}]: connection lost — reconnecting in {backoff_secs}s");
                    state.backend_disconnected(key, Some("connection lost".into()));
                    backoff_secs = 10;
                }
                Some(false) | None => {
                    // Session never got going — back off to reduce log spam.
                    tracing::warn!(
                        "Subscriber[{backend:?}]: session failed — retrying in {backoff_secs}s"
                    );
                    state.backend_disconnected(key, Some("connection failed".into()));
                    backoff_secs = (backoff_secs * 2).min(300);
                }
            }
            // Interruptible — wake-kick endpoint can cut the backoff short.
            rt.block_on(async {
                let _ = tokio::time::timeout(
                    Duration::from_secs(backoff_secs),
                    state.wake_signal.notified(),
                )
                .await;
            });
        }
    });
}

/// Run the subscription notification loop for one connection lifetime.
/// Returns `true` if the loop ended due to a network error (caller should reconnect),
/// `false` if connect/subscribe itself failed before the loop started.
// The args are all distinct live state for one connection; bundling them into a
// struct would just move the noise without improving clarity.
#[allow(clippy::too_many_arguments)]
fn run_session<E: ElectrumApi>(
    client: &E,
    state: &AppState,
    cmd_rx: &mut mpsc::UnboundedReceiver<SubCommand>,
    script_to_wallet: &mut HashMap<ScriptBuf, Uuid>,
    wallet_scripts: &mut HashMap<Uuid, HashSet<ScriptBuf>>,
    rt: &Handle,
    poll_interval: Duration,
    key: Option<&str>,
) -> bool {
    // Subscribe to new block headers for confirmation updates.
    // Non-fatal: if this fails we still subscribe to scripts and get mempool notifications;
    // we just won't get pushed confirmations (block_headers_pop will always return None).
    let has_block_sub = match client.block_headers_subscribe() {
        Ok(h) => {
            state.backend_connected(key, Some(h.height as u32));
            true
        }
        Err(e) => {
            tracing::warn!("Subscriber: block_headers_subscribe failed (continuing without block notifications): {e}");
            false
        }
    };
    // Reached the server even if the block sub failed — the client is connected.
    state.backend_connected(key, None);

    // Subscribe to every known script.
    let mut subscribed = 0usize;
    for script in script_to_wallet.keys() {
        if client.script_subscribe(script).is_ok() {
            subscribed += 1;
        }
    }

    // Initial sync for every wallet (repopulates data after server restart / reconnect).
    for &id in wallet_scripts.keys() {
        let s = state.clone();
        rt.spawn(async move { background_sync(s, id).await });
    }

    tracing::info!(
        "Subscriber: connected — {subscribed}/{} scripts across {} wallet(s), block_sub={has_block_sub}",
        script_to_wallet.len(),
        wallet_scripts.len()
    );

    // Periodic fallback: sync all wallets on this interval regardless of push notifications.
    // Catches confirmation updates when block_headers_subscribe is unavailable, and acts as
    // a safety net if push notifications are silently dropped.
    let mut last_poll = Instant::now();

    loop {
        // ── Script notifications ───────────────────────────────────────────
        // Check each subscribed script's per-script notification queue.
        // Deduplicate: if multiple scripts for the same wallet fired, sync once.
        let mut wallets_to_sync: HashSet<Uuid> = HashSet::new();
        for (script, &id) in script_to_wallet.iter() {
            match client.script_pop(script) {
                Ok(Some(_)) => {
                    wallets_to_sync.insert(id);
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("Subscriber: script_pop error — reconnecting: {e}");
                    return true;
                }
            }
        }
        if !wallets_to_sync.is_empty() {
            tracing::info!(
                "Subscriber: script notification — syncing {} wallet(s)",
                wallets_to_sync.len()
            );
        }
        for id in wallets_to_sync {
            let s = state.clone();
            rt.spawn(async move { background_sync(s, id).await });
        }

        // ── Block notifications → refresh confirmations for all wallets ────
        if has_block_sub {
            match client.block_headers_pop() {
                Ok(Some(h)) => {
                    tracing::info!("Subscriber: new block {} — syncing all wallets", h.height);
                    state.backend_connected(key, Some(h.height as u32));
                    for &id in wallet_scripts.keys() {
                        let s = state.clone();
                        rt.spawn(async move { background_sync(s, id).await });
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("Subscriber: block_headers_pop error — reconnecting: {e}");
                    return true;
                }
            }
        }

        // ── Commands (wallet added/removed, scripts updated after sync) ────
        while let Ok(cmd) = cmd_rx.try_recv() {
            let new_scripts: Vec<ScriptBuf> = match &cmd {
                SubCommand::AddWallet { scripts, .. }
                | SubCommand::UpdateScripts { scripts, .. } => scripts
                    .iter()
                    .filter(|s| !script_to_wallet.contains_key(*s))
                    .cloned()
                    .collect(),
                SubCommand::RemoveWallet(_) => vec![],
            };
            apply_command(cmd, script_to_wallet, wallet_scripts);
            for script in &new_scripts {
                let _ = client.script_subscribe(script);
            }
            if !new_scripts.is_empty() {
                tracing::info!(
                    "Subscriber: subscribed to {} new script(s)",
                    new_scripts.len()
                );
            }
        }

        // ── Periodic fallback sync ─────────────────────────────────────────
        if last_poll.elapsed() >= poll_interval {
            tracing::info!(
                "Subscriber: periodic sync ({} wallet(s))",
                wallet_scripts.len()
            );
            for &id in wallet_scripts.keys() {
                let s = state.clone();
                rt.spawn(async move { background_sync(s, id).await });
            }
            last_poll = Instant::now();
        }

        // 500ms is plenty responsive for SSE-pushed sync triggers — the user-
        // initiated sync path goes through the API directly, so this loop only
        // needs to catch server-pushed events (new blocks, external sends). The
        // old 100ms interval was 5x as much CPU for no perceptible UX gain.
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn apply_command(
    cmd: SubCommand,
    script_to_wallet: &mut HashMap<ScriptBuf, Uuid>,
    wallet_scripts: &mut HashMap<Uuid, HashSet<ScriptBuf>>,
) {
    match cmd {
        SubCommand::AddWallet { id, scripts } | SubCommand::UpdateScripts { id, scripts } => {
            let entry = wallet_scripts.entry(id).or_default();
            for script in scripts {
                script_to_wallet.entry(script.clone()).or_insert(id);
                entry.insert(script);
            }
        }
        SubCommand::RemoveWallet(id) => {
            if let Some(scripts) = wallet_scripts.remove(&id) {
                for s in &scripts {
                    // Only drop the routing entry if no other wallet still claims
                    // this script. Otherwise transfer ownership to one of them so
                    // they keep receiving notifications.
                    let mut other_owner: Option<Uuid> = None;
                    for (&other_id, other_scripts) in wallet_scripts.iter() {
                        if other_scripts.contains(s) {
                            other_owner = Some(other_id);
                            break;
                        }
                    }
                    match other_owner {
                        Some(new_id) => {
                            script_to_wallet.insert(s.clone(), new_id);
                        }
                        None => {
                            script_to_wallet.remove(s);
                        }
                    }
                }
            }
        }
    }
}
