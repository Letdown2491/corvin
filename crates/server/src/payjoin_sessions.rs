//! On-disk store for payjoin (BIP-77 / v2) sessions.
//!
//! The `payjoin` crate models a session as an event-sourced log: each typestate
//! transition emits a `SessionEvent` which we persist, and on restart the crate
//! replays the log to reconstruct the session. We therefore back each session
//! with one JSON file (`<session_id>.json`, a JSON array of events) under
//! `~/.config/corvin/payjoin/`, plus a small `index.json` mapping session →
//! owning wallet so we can list a wallet's sessions and respawn poll tasks on
//! boot. The crate's `SessionHistory` derives fallback tx / expiry / status
//! from the log, so we don't duplicate that here.

use anyhow::Result;
use payjoin::persist::SessionPersister;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::PathBuf;
use uuid::Uuid;

use crate::config::payjoin_dir;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Send,
    Receive,
}

/// App-level lifecycle, distinct from the crate's negotiation status. The crate
/// considers a session "Completed" once the proposal arrives; we still have to
/// re-sign + broadcast, so we track that separately for the UI.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PjStatus {
    #[default]
    Negotiating,
    ProposalReady,
    Sent,
    FellBack,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub wallet_id: Uuid,
    pub kind: SessionKind,
    pub created_at: String,
    #[serde(default)]
    pub status: PjStatus,
    #[serde(default)]
    pub result_txid: Option<String>,
}

fn index_path() -> PathBuf {
    payjoin_dir().join("index.json")
}

fn log_path(session_id: Uuid) -> PathBuf {
    payjoin_dir().join(format!("{session_id}.json"))
}

/// Atomic + 0600 + at-rest-sealed write via the shared writer.
fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::state::write_private(path, bytes)
}

// ── session index ────────────────────────────────────────────────────────────

type Index = HashMap<Uuid, SessionMeta>;

fn load_index() -> Index {
    crate::state::load_or_quarantine(&index_path())
}

fn save_index(index: &Index) -> Result<()> {
    atomic_write(&index_path(), &serde_json::to_vec_pretty(index)?)?;
    Ok(())
}

/// Record a new session against its owning wallet.
pub fn register(session_id: Uuid, meta: SessionMeta) -> Result<()> {
    let mut index = load_index();
    index.insert(session_id, meta);
    save_index(&index)
}

pub fn get(session_id: Uuid) -> Option<SessionMeta> {
    load_index().remove(&session_id)
}

/// Update a session's app-level status (and optionally its broadcast txid).
/// No-op if the session is unknown.
pub fn set_status(session_id: Uuid, status: PjStatus, txid: Option<String>) -> Result<()> {
    let mut index = load_index();
    if let Some(meta) = index.get_mut(&session_id) {
        meta.status = status;
        if txid.is_some() {
            meta.result_txid = txid;
        }
        save_index(&index)?;
    }
    Ok(())
}

pub fn list_for_wallet(wallet_id: Uuid) -> Vec<(Uuid, SessionMeta)> {
    load_index()
        .into_iter()
        .filter(|(_, m)| m.wallet_id == wallet_id)
        .collect()
}

/// Every known session — used at startup to respawn poll tasks.
pub fn all_sessions() -> Vec<(Uuid, SessionMeta)> {
    load_index().into_iter().collect()
}

/// Drop a session: remove its index entry and delete its event log.
pub fn forget(session_id: Uuid) -> Result<()> {
    let mut index = load_index();
    index.remove(&session_id);
    save_index(&index)?;
    let _ = std::fs::remove_file(log_path(session_id));
    Ok(())
}

/// Drop every session belonging to a wallet — called when the wallet is deleted.
pub fn forget_wallet(wallet_id: Uuid) -> Result<()> {
    for (sid, _) in list_for_wallet(wallet_id) {
        forget(sid)?;
    }
    Ok(())
}

// ── seen-inputs replay protection (receiver) ──────────────────────────────────
//
// BIP-77 receivers must reject an original PSBT whose inputs we've already
// processed in a prior session (anti-replay). We persist the set of outpoints
// ("txid:vout") we've contributed alongside across all receive sessions.

fn seen_path() -> PathBuf {
    payjoin_dir().join("seen_inputs.json")
}

pub fn load_seen() -> std::collections::HashSet<String> {
    crate::state::load_or_quarantine(&seen_path())
}

/// Record outpoints (formatted "txid:vout") as seen.
pub fn mark_seen(outpoints: &[String]) -> Result<()> {
    if outpoints.is_empty() {
        return Ok(());
    }
    let mut set = load_seen();
    set.extend(outpoints.iter().cloned());
    atomic_write(&seen_path(), &serde_json::to_vec_pretty(&set)?)?;
    Ok(())
}

// ── event-log persister ───────────────────────────────────────────────────────

#[derive(Debug)]
pub struct PersistError(pub String);

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "payjoin session persistence: {}", self.0)
    }
}
impl std::error::Error for PersistError {}

/// File-backed [`SessionPersister`] over one session's `<session_id>.json` event
/// log. Generic over the event type so the same impl serves both the send
/// (`payjoin::send::v2::SessionEvent`) and receive event logs. Events are few
/// (≤ a handful per session), so read-modify-write of the whole array per save
/// is fine and keeps writes atomic.
pub struct FileSessionPersister<E> {
    path: PathBuf,
    _marker: PhantomData<fn() -> E>,
}

impl<E: Serialize + DeserializeOwned> FileSessionPersister<E> {
    pub fn new(session_id: Uuid) -> Self {
        Self {
            path: log_path(session_id),
            _marker: PhantomData,
        }
    }

    fn read_events(&self) -> Vec<E> {
        crate::state::load_or_quarantine(&self.path)
    }
}

impl<E: Serialize + DeserializeOwned + 'static> SessionPersister for FileSessionPersister<E> {
    type InternalStorageError = PersistError;
    type SessionEvent = E;

    fn save_event(&self, event: E) -> Result<(), PersistError> {
        let mut events = self.read_events();
        events.push(event);
        let bytes = serde_json::to_vec_pretty(&events).map_err(|e| PersistError(e.to_string()))?;
        atomic_write(&self.path, &bytes).map_err(|e| PersistError(e.to_string()))
    }

    fn load(&self) -> Result<Box<dyn Iterator<Item = E>>, PersistError> {
        Ok(Box::new(self.read_events().into_iter()))
    }

    fn close(&self) -> Result<(), PersistError> {
        // Leave the log in place; lifecycle is driven by the index via forget().
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // index.json + seen_inputs.json are single shared files under the one
    // process-wide test config dir, so these tests serialize and clear that
    // shared state before each run.
    fn clean_slate() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        crate::config::test_isolate_config_dir();
        let _ = std::fs::remove_file(index_path());
        let _ = std::fs::remove_file(seen_path());
        guard
    }

    fn meta(wallet_id: Uuid, kind: SessionKind) -> SessionMeta {
        SessionMeta {
            wallet_id,
            kind,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            status: PjStatus::Negotiating,
            result_txid: None,
        }
    }

    #[test]
    fn session_index_lifecycle() {
        let _g = clean_slate();
        let (w, v) = (Uuid::new_v4(), Uuid::new_v4());
        let (s1, s2, s3) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());

        register(s1, meta(w, SessionKind::Send)).unwrap();
        register(s2, meta(w, SessionKind::Receive)).unwrap();
        register(s3, meta(v, SessionKind::Send)).unwrap();

        // list_for_wallet filters by owner.
        assert_eq!(list_for_wallet(w).len(), 2);
        assert_eq!(list_for_wallet(v).len(), 1);

        // set_status updates status + txid and is readable back.
        set_status(s1, PjStatus::Sent, Some("deadbeef".to_string())).unwrap();
        let m = get(s1).expect("s1 present");
        assert_eq!(m.status, PjStatus::Sent);
        assert_eq!(m.result_txid.as_deref(), Some("deadbeef"));

        // set_status on an unknown session is a silent no-op (no panic, no insert).
        set_status(Uuid::new_v4(), PjStatus::Failed, None).unwrap();

        // forget drops one; forget_wallet drops the rest of w's, leaving v's.
        forget(s1).unwrap();
        assert!(get(s1).is_none());
        forget_wallet(w).unwrap();
        assert_eq!(list_for_wallet(w).len(), 0);
        assert_eq!(list_for_wallet(v).len(), 1, "another wallet's sessions are untouched");
    }

    #[test]
    fn seen_inputs_anti_replay_unions_without_dupes() {
        let _g = clean_slate();
        assert!(load_seen().is_empty(), "starts empty");

        let (a, b, c) = ("aa:0".to_string(), "bb:1".to_string(), "cc:2".to_string());
        mark_seen(&[a.clone(), b.clone()]).unwrap();
        let seen = load_seen();
        assert!(seen.contains(&a) && seen.contains(&b));
        assert_eq!(seen.len(), 2);

        // Overlapping mark unions in only the new outpoint (set semantics).
        mark_seen(&[b.clone(), c.clone()]).unwrap();
        let seen = load_seen();
        assert_eq!(seen.len(), 3, "the duplicate bb:1 is not double-counted");
        assert!(seen.contains(&c));

        // Empty mark is a no-op.
        mark_seen(&[]).unwrap();
        assert_eq!(load_seen().len(), 3);
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
    struct Ev {
        step: u32,
    }

    #[test]
    fn event_log_persists_and_replays_in_order() {
        let _g = clean_slate();
        let sid = Uuid::new_v4();
        let _ = std::fs::remove_file(log_path(sid));

        let persister = FileSessionPersister::<Ev>::new(sid);
        persister.save_event(Ev { step: 1 }).unwrap();
        persister.save_event(Ev { step: 2 }).unwrap();
        persister.save_event(Ev { step: 3 }).unwrap();

        // A fresh persister over the same session id replays the whole log in
        // order — i.e. the session survives a restart.
        let reloaded = FileSessionPersister::<Ev>::new(sid);
        let events: Vec<Ev> = reloaded.load().unwrap().collect();
        assert_eq!(events, vec![Ev { step: 1 }, Ev { step: 2 }, Ev { step: 3 }]);

        // forget() removes the index entry and the event log.
        forget(sid).unwrap();
        assert!(FileSessionPersister::<Ev>::new(sid).load().unwrap().next().is_none());
    }
}
