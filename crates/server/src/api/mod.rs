pub mod address_labels;
pub mod backends;
pub mod backup;
pub mod backup_test;
pub mod bip329;
pub mod bip353;
pub mod broadcast;
pub mod categories;
pub mod cost_basis;
pub mod events;
// Descriptor/PSBT/policy parsing shared by every signing path (HW, SP-send, payjoin),
// and the Ledger HMAC store used on wallet delete — always compiled. Only the USB
// device module (`hwi`) is gated behind `hw`.
pub(crate) mod descriptor_util;
pub(crate) mod ledger_hmac_store;
#[cfg(feature = "hw")]
pub mod hwi;
pub mod labels;
pub mod messages;
pub mod prices;
pub mod proxy;
pub mod security;
pub mod settings;
pub mod silent_payments;
pub mod sweep;
pub mod utxo_freeze;
pub mod utxo_labels;
pub mod wallets;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Unified API error type. Wraps an `anyhow::Error` (so `?` on any error still
/// works) and optionally carries a stable machine `code` + a specific HTTP
/// status. Plain `?`-converted errors default to 500 with no code (unchanged
/// behavior); use the constructors below to classify the cases worth
/// distinguishing client-side. The JSON body is `{ error, code? }`.
pub struct ApiError {
    err: anyhow::Error,
    code: Option<&'static str>,
    status: StatusCode,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, msg: impl Into<String>) -> Self {
        Self { err: anyhow::anyhow!(msg.into()), code: Some(code), status }
    }
    /// 404 — the requested wallet/resource doesn't exist.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", msg)
    }
    /// 400 — the request was malformed or failed validation.
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", msg)
    }
    /// 401 — a supplied secret (seed/passphrase) didn't match / couldn't sign.
    pub fn wrong_secret(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "wrong_secret", msg)
    }
    /// 409 — the wallet can't cover the requested amount + fee.
    pub fn insufficient_funds(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "insufficient_funds", msg)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": format!("{:#}", self.err) });
        if let Some(code) = self.code {
            body["code"] = json!(code);
        }
        (self.status, Json(body)).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self { err: e.into(), code: None, status: StatusCode::INTERNAL_SERVER_ERROR }
    }
}
