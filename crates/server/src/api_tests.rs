//! Integration tests that drive the real axum router (no live backend). Each
//! test builds an in-memory AppState and dispatches requests with
//! `tower::ServiceExt::oneshot`, so routing, extractors, the ApiError taxonomy,
//! and JSON (de)serialization are all exercised end to end.

use crate::build_router;
use crate::config::Config;
use crate::state::AppState;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use bdk_wallet::bitcoin::Network;
use serde_json::{json, Value};
use tower::ServiceExt;

// A config whose backend points at a dead local port, so any background sync a
// handler spawns fails instantly offline instead of dialing a public server.
fn test_config() -> Config {
    crate::config::test_isolate_config_dir();
    let mut cfg = Config::default();
    cfg.backend.electrum_host = "127.0.0.1".to_string();
    cfg.backend.electrum_port = 1;
    cfg.backend.electrum_ssl = false;
    cfg
}

// A fresh, empty-manager state. Handlers ignore the `sub_tx.send(...)` result,
// so dropping the receiver is harmless; any spawned background sync just fails
// against the dead local port.
fn fresh_state() -> AppState {
    let (state, _rx) = AppState::new(test_config(), Network::Bitcoin);
    state
}

fn app() -> axum::Router {
    build_router(fresh_state())
}

async fn body_json(res: axum::response::Response) -> Value {
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn post_json(uri: &str, payload: &Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(payload).unwrap()))
        .unwrap()
}

#[tokio::test]
async fn version_endpoint_reports_build_info() {
    let res = app().oneshot(get("/api/version")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert!(v["version"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(v["os"].is_string());
    assert!(v["arch"].is_string());
    // The frontend gates the USB-HW UI on this flag, so it must always be present.
    assert!(v["hw_enabled"].is_boolean());
}

#[tokio::test]
async fn settings_endpoint_returns_config() {
    let res = app().oneshot(get("/api/settings")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn add_wallet_rejects_garbage_input_as_bad_request() {
    let req = post_json(
        "/api/wallets",
        &json!({ "label": "junk", "input": "not-an-address-or-xpub" }),
    );
    let res = app().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let v = body_json(res).await;
    assert_eq!(v["code"], "bad_request");
}

#[tokio::test]
async fn unknown_wallet_is_not_found_with_code() {
    let uri = format!("/api/wallets/{}/balance", uuid::Uuid::new_v4());
    let res = app().oneshot(get(&uri)).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let v = body_json(res).await;
    assert_eq!(v["code"], "not_found");
}

#[tokio::test]
async fn watch_only_wallet_add_then_list() {
    // One state shared across the three requests (its manager is Arc-backed);
    // build_router doesn't load wallets from disk, so a fresh state per request
    // wouldn't see the add.
    let state = fresh_state();

    // Empty to start.
    let res = build_router(state.clone())
        .oneshot(get("/api/wallets"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await.as_array().unwrap().len(), 0);

    // Adding a watch-only address needs no live backend.
    let req = post_json(
        "/api/wallets",
        &json!({
            "label": "donations",
            "input": "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
        }),
    );
    let res = build_router(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let entry = body_json(res).await;
    assert_eq!(entry["label"], "donations");
    assert_eq!(entry["kind"], "address");

    // It now shows up in the list served from the shared manager.
    let res = build_router(state.clone())
        .oneshot(get("/api/wallets"))
        .await
        .unwrap();
    assert_eq!(body_json(res).await.as_array().unwrap().len(), 1);
}

fn offline_state() -> AppState {
    let mut cfg = test_config();
    cfg.offline = true;
    let (state, _rx) = AppState::new(cfg, Network::Bitcoin);
    state
}

fn post_empty(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn offline_mode_status_reports_offline() {
    let res = build_router(offline_state())
        .oneshot(get("/api/status"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["offline"], true);
    assert_eq!(v["connected"], false);
}

#[tokio::test]
async fn offline_mode_refuses_sync() {
    let state = offline_state();

    // Add a watch-only wallet (works offline — no sync spawned).
    let res = build_router(state.clone())
        .oneshot(post_json(
            "/api/wallets",
            &json!({ "label": "w", "input": "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4" }),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let id = body_json(res).await["id"].as_str().unwrap().to_string();

    // Explicit sync is refused in offline mode.
    let res = build_router(state.clone())
        .oneshot(post_empty(&format!("/api/wallets/{id}/sync")))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(res).await["code"], "bad_request");
}

// ── Send-path regtest tests ────────────────────────────────────────────────
//
// Hermetic "regtest" coverage for `build_send_psbt`: fund a real BDK wallet
// in memory (no bitcoind / electrs — the build path never touches the network),
// drop it into the manager, then drive the live handler through the router and
// inspect the resulting PSBT's inputs. These lock the coin-selection fix: an
// auto send must NOT consolidate the whole wallet, send-max MUST drain it, and
// coin control must spend exactly the picked coins.

struct Funded {
    state: AppState,
    id: uuid::Uuid,
    recipient: String,
    /// A wallet address that already received funds — so it appears in the
    /// wallet's tx history (used to trigger the repeat-recipient warning).
    funded_address: String,
    /// Every funded outpoint as "txid:vout", in funding order.
    outpoints: Vec<String>,
}

/// A Regtest wallet funded with `n` equal-value confirmed UTXOs, inserted into
/// a fresh state's manager.
async fn funded_regtest_wallet(n: usize, utxo_sats: u64) -> Funded {
    use bdk_wallet::bitcoin::{hashes::Hash, Amount, BlockHash};
    use bdk_wallet::chain::BlockId;
    use bdk_wallet::test_utils::{insert_checkpoint, receive_output_in_latest_block};
    use bdk_wallet::KeychainKind;
    use corvin_core::types::{InputKind, WalletEntry};

    crate::config::test_isolate_config_dir();
    let mut cfg = Config::default();
    cfg.network.kind = crate::config::NetworkKind::Regtest;
    cfg.backend.electrum_host = "127.0.0.1".to_string();
    cfg.backend.electrum_port = 1;
    cfg.backend.electrum_ssl = false;
    let (state, _rx) = AppState::new(cfg, Network::Regtest);

    // A fixed BIP-39 test vector → xpub-based wpkh descriptors (watch-only is
    // fine; the send path returns an unsigned PSBT).
    let mnemonic =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let d = corvin_core::seed::derive_descriptors(
        mnemonic,
        "",
        "m/84'/1'/0'",
        "native_segwit",
        Network::Regtest,
    )
    .expect("derive descriptors");

    let id = uuid::Uuid::new_v4();
    let entry = WalletEntry {
        id,
        label: "regtest".to_string(),
        input: String::new(),
        kind: InputKind::Zpub,
        external_descriptor: d.external.clone(),
        internal_descriptor: Some(d.internal.clone()),
        threshold: None,
        backend: None,
        created_at: chrono::Utc::now(),
    };

    let db_path = std::env::temp_dir()
        .join(format!("corvin-send-test-{id}"))
        .join("w.db");
    let mut hd =
        corvin_core::wallet::open_or_create_wallet(&entry, Network::Regtest, &db_path).unwrap();

    insert_checkpoint(
        &mut hd.wallet,
        BlockId {
            height: 1_000,
            hash: BlockHash::all_zeros(),
        },
    );
    for _ in 0..n {
        let _ = receive_output_in_latest_block(&mut hd.wallet, Amount::from_sat(utxo_sats));
    }

    // A clean recipient address (high index, not one of the funded ones) and the
    // funded outpoint list — captured before the wallet moves into the manager.
    let recipient = hd
        .wallet
        .peek_address(KeychainKind::External, 500)
        .address
        .to_string();
    // External index 0 received the first funding output, so it's in history.
    let funded_address = hd
        .wallet
        .peek_address(KeychainKind::External, 0)
        .address
        .to_string();
    let mut outpoints: Vec<String> = hd
        .wallet
        .list_unspent()
        .map(|u| format!("{}:{}", u.outpoint.txid, u.outpoint.vout))
        .collect();
    outpoints.sort();
    assert_eq!(outpoints.len(), n, "wallet should see all funded UTXOs");

    state
        .manager
        .write()
        .await
        .add(entry, crate::state::WalletInner::Hd(tokio::sync::Mutex::new(hd)));

    Funded {
        state,
        id,
        recipient,
        funded_address,
        outpoints,
    }
}

fn psbt_input_count(v: &Value) -> usize {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let b64 = v["psbt"].as_str().expect("psbt in response");
    let bytes = STANDARD.decode(b64).expect("base64 psbt");
    let psbt = bdk_wallet::bitcoin::Psbt::deserialize(&bytes).expect("valid psbt");
    psbt.unsigned_tx.input.len()
}

#[tokio::test]
async fn auto_send_does_not_consolidate_whole_wallet() {
    // 6 × 100k sats = 600k available; pay 50k. The fix means BDK coin-selects a
    // minimal input set — NOT every UTXO (the old bug spent all 6 on every send).
    let f = funded_regtest_wallet(6, 100_000).await;
    let req = post_json(
        &format!("/api/wallets/{}/send-psbt", f.id),
        &json!({
            "outputs": [{ "recipient": f.recipient, "amount_sats": 50_000 }],
            "fee_rate_sat_vb": 2.0
        }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let inputs = psbt_input_count(&body_json(res).await);
    assert!(
        inputs < 6,
        "auto send must not consolidate the whole wallet; spent {inputs} of 6 inputs"
    );
}

#[tokio::test]
async fn send_psbt_signs_and_finalizes_with_the_seed() {
    // The full software-signing money path: the API returns an UNSIGNED PSBT from a
    // watch-only (xpub) wallet; the same seed re-derives the xpriv, signs, and
    // finalizes it to a valid tx. Proves build → seed-sign → finalize end to end.
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let f = funded_regtest_wallet(2, 100_000).await;
    let req = post_json(
        &format!("/api/wallets/{}/send-psbt", f.id),
        &json!({ "outputs": [{ "recipient": f.recipient, "amount_sats": 50_000 }], "fee_rate_sat_vb": 2.0 }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let b64 = body_json(res).await["psbt"].as_str().expect("psbt").to_string();
    let mut psbt =
        bdk_wallet::bitcoin::Psbt::deserialize(&STANDARD.decode(&b64).unwrap()).unwrap();

    // Same mnemonic + path the harness derived the watch-only wallet from.
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let finalized = crate::api::wallets::seed_signer::sign_with_seed(
        "m/84'/1'/0'",
        "native_segwit",
        Network::Regtest,
        mnemonic,
        "",
        &mut psbt,
    )
    .expect("sign_with_seed");
    assert!(finalized, "the seed must fully sign + finalize its own wallet's PSBT");
    let n_inputs = psbt.inputs.len();
    let tx = psbt.extract_tx().expect("a finalized PSBT extracts to a valid tx");
    assert_eq!(tx.input.len(), n_inputs, "every input is present in the final tx");
}

#[tokio::test]
async fn send_max_drains_the_whole_wallet() {
    // Send-max (amount_sats omitted) must spend every spendable coin.
    let f = funded_regtest_wallet(4, 100_000).await;
    let req = post_json(
        &format!("/api/wallets/{}/send-psbt", f.id),
        &json!({
            "outputs": [{ "recipient": f.recipient, "amount_sats": null }],
            "fee_rate_sat_vb": 2.0
        }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let inputs = psbt_input_count(&body_json(res).await);
    assert_eq!(inputs, 4, "send-max must drain all 4 inputs");
}

#[tokio::test]
async fn coin_control_spends_exactly_the_picked_inputs() {
    // Explicitly picking 2 of 5 UTXOs spends exactly those two.
    let f = funded_regtest_wallet(5, 100_000).await;
    let picked: Vec<String> = f.outpoints.iter().take(2).cloned().collect();
    let req = post_json(
        &format!("/api/wallets/{}/send-psbt", f.id),
        &json!({
            "outputs": [{ "recipient": f.recipient, "amount_sats": 50_000 }],
            "fee_rate_sat_vb": 2.0,
            "utxos": picked
        }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let inputs = psbt_input_count(&body_json(res).await);
    assert_eq!(inputs, 2, "coin control must spend exactly the 2 picked inputs");
}

// ── Privacy-warning coverage (compute_send_warnings) ────────────────────────
//
// Same funded-wallet harness, asserting the advisory warning `code`s the send
// preview returns. Each builds the exact input/output condition that should
// trip one warning.

fn warning_codes(v: &Value) -> Vec<String> {
    v["warnings"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|w| w["code"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn warns_on_mixed_labels() {
    // Two inputs carrying different UTXO labels, spent together → mixed_labels.
    let f = funded_regtest_wallet(3, 100_000).await;
    f.state
        .annotations
        .set_utxo_label(f.outpoints[0].clone(), "exchange")
        .await
        .unwrap();
    f.state
        .annotations
        .set_utxo_label(f.outpoints[1].clone(), "donation")
        .await
        .unwrap();
    let picked: Vec<String> = f.outpoints.iter().take(2).cloned().collect();
    let req = post_json(
        &format!("/api/wallets/{}/send-psbt", f.id),
        &json!({
            "outputs": [{ "recipient": f.recipient, "amount_sats": 50_000 }],
            "fee_rate_sat_vb": 2.0,
            "utxos": picked
        }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(warning_codes(&body_json(res).await).contains(&"mixed_labels".to_string()));
}

#[tokio::test]
async fn warns_on_mixed_categories() {
    // Two inputs in different categories, spent together → mixed_categories.
    let f = funded_regtest_wallet(3, 100_000).await;
    let savings = f.state.annotations.add_category("Savings", "#0a0").await.unwrap();
    let spending = f.state.annotations.add_category("Spending", "#a00").await.unwrap();
    f.state
        .annotations
        .set_utxo_category(f.outpoints[0].clone(), Some(&savings.id))
        .await
        .unwrap();
    f.state
        .annotations
        .set_utxo_category(f.outpoints[1].clone(), Some(&spending.id))
        .await
        .unwrap();
    let picked: Vec<String> = f.outpoints.iter().take(2).cloned().collect();
    let req = post_json(
        &format!("/api/wallets/{}/send-psbt", f.id),
        &json!({
            "outputs": [{ "recipient": f.recipient, "amount_sats": 50_000 }],
            "fee_rate_sat_vb": 2.0,
            "utxos": picked
        }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(warning_codes(&body_json(res).await).contains(&"mixed_categories".to_string()));
}

#[tokio::test]
async fn warns_on_repeat_recipient() {
    // Paying an address already in the wallet's history → repeat_recipient.
    let f = funded_regtest_wallet(3, 100_000).await;
    let req = post_json(
        &format!("/api/wallets/{}/send-psbt", f.id),
        &json!({
            "outputs": [{ "recipient": f.funded_address, "amount_sats": 50_000 }],
            "fee_rate_sat_vb": 2.0
        }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(warning_codes(&body_json(res).await).contains(&"repeat_recipient".to_string()));
}

#[tokio::test]
async fn warns_on_round_amount_with_change() {
    // A round 100k-sat payment that leaves change → round_amount_reveal. Two
    // 100k inputs guarantee a change output remains after the 100k send + fee.
    let f = funded_regtest_wallet(3, 100_000).await;
    let picked: Vec<String> = f.outpoints.iter().take(2).cloned().collect();
    let req = post_json(
        &format!("/api/wallets/{}/send-psbt", f.id),
        &json!({
            "outputs": [{ "recipient": f.recipient, "amount_sats": 100_000 }],
            "fee_rate_sat_vb": 2.0,
            "utxos": picked
        }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert!(body["change_sats"].as_u64().unwrap_or(0) > 0, "test needs change present");
    assert!(warning_codes(&body).contains(&"round_amount_reveal".to_string()));
}

#[tokio::test]
async fn warns_on_change_script_mismatch() {
    use bdk_wallet::bitcoin::secp256k1::{Secp256k1, SecretKey};
    use bdk_wallet::bitcoin::{Address, PublicKey};
    // The funded wallet is native-segwit (p2wpkh change). Paying a legacy p2pkh
    // recipient leaves a change output of a different type → change_script_mismatch.
    let f = funded_regtest_wallet(2, 100_000).await;
    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(&[7u8; 32]).unwrap();
    let pk = PublicKey::new(sk.public_key(&secp));
    let legacy = Address::p2pkh(pk, Network::Regtest).to_string();
    let picked: Vec<String> = f.outpoints.iter().take(2).cloned().collect();
    let req = post_json(
        &format!("/api/wallets/{}/send-psbt", f.id),
        &json!({
            "outputs": [{ "recipient": legacy, "amount_sats": 50_000 }],
            "fee_rate_sat_vb": 2.0,
            "utxos": picked
        }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert!(body["change_sats"].as_u64().unwrap_or(0) > 0, "test needs change present");
    assert!(warning_codes(&body).contains(&"change_script_mismatch".to_string()));
}

#[tokio::test]
async fn sp_send_auto_does_not_consolidate_whole_wallet() {
    // The SP-send path got the same coin-selection fix as the regular send. Drive
    // it in estimate-only mode (no seed/signing needed): the returned input total
    // must be a minimal selection, not the whole 600k balance.
    let f = funded_regtest_wallet(6, 100_000).await;
    // Any valid regtest SP address as the recipient — the recipient's identity
    // doesn't affect coin selection.
    let sp_addr = corvin_core::silent_payments::derive_from_mnemonic(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        "",
        Network::Regtest,
        0,
    )
    .unwrap()
    .address;
    let req = post_json(
        &format!("/api/wallets/{}/sp-send", f.id),
        &json!({
            "outputs": [{ "recipient": sp_addr, "amount_sats": 50_000 }],
            "fee_rate_sat_vb": 2.0,
            "estimate_only": true
        }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let input_sats = body_json(res).await["input_sats"].as_u64().unwrap();
    assert!(
        input_sats < 600_000,
        "SP auto send must not consolidate the whole wallet; selected {input_sats} of 600000 sats"
    );
}

#[tokio::test]
async fn delete_wallet_aborts_sp_scanner_task() {
    // Deleting a wallet must abort its SP scanner task so the scan secret doesn't
    // linger in RAM (and the scanner can't resurrect sp_outputs.json).
    let f = funded_regtest_wallet(1, 100_000).await;
    // Stand in for a running scanner: a task that never finishes on its own.
    let handle = tokio::spawn(async {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
    f.state.sp_scanners.lock().await.insert(f.id, handle);
    assert!(f.state.sp_scanners.lock().await.contains_key(&f.id));

    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/wallets/{}", f.id))
        .body(Body::empty())
        .unwrap();
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(
        !f.state.sp_scanners.lock().await.contains_key(&f.id),
        "the scanner handle must be removed (and aborted) on delete"
    );
}

#[tokio::test]
async fn security_status_reports_off_when_unencrypted() {
    // With no vault sentinel (default), encryption is off and protected routes
    // are reachable (the lock gate is a no-op).
    let res = build_router(fresh_state())
        .oneshot(get("/api/security/status"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["state"], "off");
}

// The off-state guards below bail before any migration or global-vault change, so
// they never touch the shared config dir or flip process state — safe in parallel.
// (The full enable→migrate→disable mechanics are covered by the unit tests
// `migration_roundtrip_and_recovery` + `rekey_reencrypts_under_new_key`.)

#[tokio::test]
async fn enable_rejects_short_password() {
    let res = build_router(fresh_state())
        .oneshot(post_json("/api/security/enable", &json!({ "password": "short" })))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(res).await["code"], "bad_request");
}

#[tokio::test]
async fn disable_when_off_is_bad_request() {
    let res = build_router(fresh_state())
        .oneshot(post_json(
            "/api/security/disable",
            &json!({ "password": "irrelevant12" }),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(res).await["code"], "bad_request");
}

#[tokio::test]
async fn change_password_when_off_is_bad_request() {
    let res = build_router(fresh_state())
        .oneshot(post_json(
            "/api/security/change-password",
            &json!({ "current_password": "old12345", "new_password": "new12345" }),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(res).await["code"], "bad_request");
}

#[tokio::test]
async fn unlock_when_not_locked_is_bad_request() {
    let res = build_router(fresh_state())
        .oneshot(post_json(
            "/api/security/unlock",
            &json!({ "password": "irrelevant12" }),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(res).await["code"], "bad_request");
}

// ── BIP-322 message signing (sign → verify roundtrip) ───────────────────────

#[tokio::test]
async fn bip322_sign_then_verify_roundtrips() {
    // Sign a message with the wallet's own funded (revealed) address via the
    // mnemonic, then verify the signature keylessly. Exercises the derivation +
    // path reconstruction + the bip322 crate end to end on a real wallet.
    let f = funded_regtest_wallet(1, 100_000).await;
    let message = "corvin proves control of this address";
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    let req = post_json(
        &format!("/api/wallets/{}/sign-message", f.id),
        &json!({ "address": f.funded_address, "message": message, "mnemonic": mnemonic }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let signature = body_json(res).await["signature"].as_str().unwrap().to_string();
    assert!(!signature.is_empty(), "a BIP-322 signature is produced");

    // Keyless verify of (address, message, signature) must accept it.
    let req = post_json(
        "/api/messages/verify",
        &json!({ "address": f.funded_address, "message": message, "signature": signature }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["valid"], true, "the signature verifies");
}

#[tokio::test]
async fn bip322_verify_rejects_a_tampered_message() {
    // A valid signature must not verify against a different message.
    let f = funded_regtest_wallet(1, 100_000).await;
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let req = post_json(
        &format!("/api/wallets/{}/sign-message", f.id),
        &json!({ "address": f.funded_address, "message": "original", "mnemonic": mnemonic }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    let signature = body_json(res).await["signature"].as_str().unwrap().to_string();

    let req = post_json(
        "/api/messages/verify",
        &json!({ "address": f.funded_address, "message": "tampered", "signature": signature }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["valid"], false, "a tampered message must not verify");
}

#[tokio::test]
async fn sign_message_rejects_wrong_mnemonic() {
    // A mnemonic that doesn't derive the wallet's address must be refused before
    // any signature is emitted (the paranoia check in sign_message).
    let f = funded_regtest_wallet(1, 100_000).await;
    let wrong = "legal winner thank year wave sausage worth useful legal winner thank yellow";
    let req = post_json(
        &format!("/api/wallets/{}/sign-message", f.id),
        &json!({ "address": f.funded_address, "message": "hi", "mnemonic": wrong }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

// ── Broadcast decode (/decode) ──────────────────────────────────────────────

#[tokio::test]
async fn decode_tx_agrees_between_psbt_and_raw_hex() {
    // The same transaction decoded as an (unsigned) PSBT and as finalized raw hex
    // must report the same txid (segwit txid is witness-independent).
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use bdk_wallet::bitcoin::consensus::encode::serialize_hex;
    let f = funded_regtest_wallet(2, 100_000).await;
    let req = post_json(
        &format!("/api/wallets/{}/send-psbt", f.id),
        &json!({ "outputs": [{ "recipient": f.recipient, "amount_sats": 50_000 }], "fee_rate_sat_vb": 2.0 }),
    );
    let res = build_router(f.state.clone()).oneshot(req).await.unwrap();
    let b64 = body_json(res).await["psbt"].as_str().unwrap().to_string();

    // Decode as PSBT.
    let res = build_router(f.state.clone())
        .oneshot(post_json("/api/decode", &json!({ "psbt": b64 })))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let psbt_decoded = body_json(res).await;
    let txid_from_psbt = psbt_decoded["txid"].as_str().unwrap().to_string();

    // Sign + finalize → raw hex, then decode that.
    let mut psbt =
        bdk_wallet::bitcoin::Psbt::deserialize(&STANDARD.decode(&b64).unwrap()).unwrap();
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    crate::api::wallets::seed_signer::sign_with_seed(
        "m/84'/1'/0'", "native_segwit", Network::Regtest, mnemonic, "", &mut psbt,
    )
    .unwrap();
    let raw_hex = serialize_hex(&psbt.extract_tx().unwrap());

    let res = build_router(f.state.clone())
        .oneshot(post_json("/api/decode", &json!({ "raw_hex": raw_hex })))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let raw_decoded = body_json(res).await;
    assert_eq!(raw_decoded["txid"].as_str().unwrap(), txid_from_psbt, "same tx, same txid");
    assert_eq!(raw_decoded["vsize_approximate"], false, "a finalized raw tx has an exact vsize");
}

#[tokio::test]
async fn decode_tx_rejects_garbage_and_empty_body() {
    let res = build_router(fresh_state())
        .oneshot(post_json("/api/decode", &json!({ "raw_hex": "not-hex-zz" })))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let res = build_router(fresh_state())
        .oneshot(post_json("/api/decode", &json!({})))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

// ── Multisig PSBT combine (/combine-psbt) ───────────────────────────────────

/// Build a real 2-of-2 P2WSH wallet, fund it, build a drain PSBT, and sign it
/// separately with each cosigner. Returns the state (with a Multisig entry under
/// `id`) plus the two single-signature PSBTs, base64-encoded.
async fn multisig_2of2_two_partials(fund_sats: u64) -> (AppState, uuid::Uuid, String, String) {
    use bdk_wallet::bitcoin::bip32::{Xpriv, Xpub};
    use bdk_wallet::bitcoin::hashes::Hash;
    use bdk_wallet::bitcoin::secp256k1::Secp256k1;
    use bdk_wallet::bitcoin::{Amount, BlockHash, FeeRate};
    use bdk_wallet::chain::BlockId;
    use bdk_wallet::test_utils::{insert_checkpoint, receive_output_in_latest_block};
    use bdk_wallet::{KeychainKind, SignOptions, Wallet};
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use corvin_core::types::{InputKind, WalletEntry};

    let secp = Secp256k1::new();
    let a = Xpriv::new_master(Network::Regtest, &[11u8; 32]).unwrap();
    let b = Xpriv::new_master(Network::Regtest, &[22u8; 32]).unwrap();
    let a_pub = Xpub::from_priv(&secp, &a);
    let b_pub = Xpub::from_priv(&secp, &b);

    let watch_ext = format!("wsh(sortedmulti(2,{a_pub}/0/*,{b_pub}/0/*))");
    let watch_int = format!("wsh(sortedmulti(2,{a_pub}/1/*,{b_pub}/1/*))");
    let a_ext = format!("wsh(sortedmulti(2,{a}/0/*,{b_pub}/0/*))");
    let a_int = format!("wsh(sortedmulti(2,{a}/1/*,{b_pub}/1/*))");
    let b_ext = format!("wsh(sortedmulti(2,{a_pub}/0/*,{b}/0/*))");
    let b_int = format!("wsh(sortedmulti(2,{a_pub}/1/*,{b}/1/*))");

    let mut watch = Wallet::create(watch_ext.clone(), watch_int.clone())
        .network(Network::Regtest)
        .create_wallet_no_persist()
        .unwrap();
    insert_checkpoint(
        &mut watch,
        BlockId { height: 1_000, hash: BlockHash::all_zeros() },
    );
    receive_output_in_latest_block(&mut watch, Amount::from_sat(fund_sats));

    let dest = watch.peek_address(KeychainKind::External, 50).address;
    let psbt = {
        let mut builder = watch.build_tx();
        builder
            .drain_wallet()
            .drain_to(dest.script_pubkey())
            .fee_rate(FeeRate::from_sat_per_vb(2).unwrap());
        builder.finish().unwrap()
    };

    let sign_opts = SignOptions { trust_witness_utxo: true, ..Default::default() };
    let mut wa = Wallet::create(a_ext, a_int)
        .network(Network::Regtest)
        .create_wallet_no_persist()
        .unwrap();
    let _ = wa.reveal_next_address(KeychainKind::External);
    let mut psbt_a = psbt.clone();
    wa.sign(&mut psbt_a, sign_opts.clone()).unwrap();

    let mut wb = Wallet::create(b_ext, b_int)
        .network(Network::Regtest)
        .create_wallet_no_persist()
        .unwrap();
    let _ = wb.reveal_next_address(KeychainKind::External);
    let mut psbt_b = psbt.clone();
    wb.sign(&mut psbt_b, sign_opts).unwrap();

    crate::config::test_isolate_config_dir();
    let mut cfg = Config::default();
    cfg.network.kind = crate::config::NetworkKind::Regtest;
    let (state, _rx) = AppState::new(cfg, Network::Regtest);
    let id = uuid::Uuid::new_v4();
    let entry = WalletEntry {
        id,
        label: "ms".to_string(),
        input: String::new(),
        kind: InputKind::Multisig,
        external_descriptor: watch_ext,
        internal_descriptor: Some(watch_int),
        threshold: Some(2),
        backend: None,
        created_at: chrono::Utc::now(),
    };
    state.manager.write().await.add(
        entry,
        crate::state::WalletInner::Address(tokio::sync::Mutex::new(None)),
    );

    (
        state,
        id,
        STANDARD.encode(psbt_a.serialize()),
        STANDARD.encode(psbt_b.serialize()),
    )
}

#[tokio::test]
async fn combine_psbt_reaches_threshold_and_finalizes() {
    // Two single-cosigner PSBTs for the same 2-of-2 tx combine into a finalized,
    // broadcast-ready PSBT carrying both signers' fingerprints.
    let (state, id, psbt_a, psbt_b) = multisig_2of2_two_partials(200_000).await;
    let req = post_json(
        &format!("/api/wallets/{id}/combine-psbt"),
        &json!({ "psbt_a": psbt_a, "psbt_b": psbt_b }),
    );
    let res = build_router(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["sigs_required"], 2);
    assert_eq!(v["sigs_present"], 2, "both cosigners' sigs are present after combine");
    assert_eq!(v["ready"], true, "a 2-of-2 with both sigs finalizes");
    assert_eq!(
        v["signed_fingerprints"].as_array().unwrap().len(),
        2,
        "both signer fingerprints are reported"
    );
}

#[tokio::test]
async fn combine_psbt_one_signer_is_not_ready() {
    // Combining a signed PSBT with the unsigned original leaves it short of the
    // 2-of-2 threshold: one sig present, not finalized.
    let (state, id, psbt_a, _psbt_b) = multisig_2of2_two_partials(200_000).await;
    let req = post_json(
        &format!("/api/wallets/{id}/combine-psbt"),
        &json!({ "psbt_a": psbt_a.clone(), "psbt_b": psbt_a }),
    );
    let res = build_router(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["sigs_present"], 1, "combining a PSBT with itself is still one sig");
    assert_eq!(v["ready"], false, "one of two sigs cannot finalize");
}

#[tokio::test]
async fn combine_psbt_rejects_mismatched_transactions() {
    // Two PSBTs for different txs must be refused, not silently merged.
    let (state, id, psbt_a, _) = multisig_2of2_two_partials(200_000).await;
    let (_state2, _id2, other, _) = multisig_2of2_two_partials(199_000).await;
    let req = post_json(
        &format!("/api/wallets/{id}/combine-psbt"),
        &json!({ "psbt_a": psbt_a, "psbt_b": other }),
    );
    let res = build_router(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn backup_export_roundtrips_through_restore() {
    // Regression: `export_backup` writes the current version; `import_backup` must
    // accept it. (A stale version guard once rejected freshly-exported v3 backups.)
    let state = fresh_state();
    let res = build_router(state.clone())
        .oneshot(get("/api/backup"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/api/restore")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let res = build_router(state.clone()).oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::NO_CONTENT,
        "a freshly-exported backup must restore (export/import version lockstep)"
    );
}
