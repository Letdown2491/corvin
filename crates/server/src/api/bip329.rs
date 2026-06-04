//! BIP-329 label export/import.
//!
//! Format: JSON Lines. One object per line, top-level shape:
//!   { "type": "tx" | "addr" | "output" | "input" | "pubkey" | "xpub",
//!     "ref":  "<txid | address | outpoint | ...>",
//!     "label": "<note>",
//!     "spendable": "true" | "false"   // only on output, optional
//!   }
//!
//! We map:
//!   - txid → "tx"
//!   - address → "addr"
//!   - outpoint ("txid:vout") → "output", with `spendable: "false"` for frozen
//!
//! We don't currently emit "input", "pubkey", or "xpub" entries — Corvin
//! doesn't label those things. Inbound import ignores unknown types.

use crate::api::ApiError;
use crate::state::AppState;
use axum::{extract::State, http::header, response::IntoResponse};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Serialize, Deserialize)]
pub struct LabelEntry {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Per BIP-329 this is a string "true"/"false", not a boolean.
    /// Only meaningful for "output" entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spendable: Option<String>,
}

/// GET /labels/export-bip329 → JSON Lines file download.
pub async fn export_bip329(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let tx_labels = state.annotations.tx_labels().await;
    let addr_labels = state.annotations.address_labels().await;
    let utxo_labels = state.annotations.utxo_labels().await;
    let frozen = state.annotations.frozen_utxos().await;

    let mut lines: Vec<String> = Vec::new();
    for (txid, note) in tx_labels.iter() {
        lines.push(serde_json::to_string(&LabelEntry {
            kind: "tx".into(),
            reference: txid.clone(),
            label: note.clone(),
            spendable: None,
        })?);
    }
    for (addr, note) in addr_labels.iter() {
        lines.push(serde_json::to_string(&LabelEntry {
            kind: "addr".into(),
            reference: addr.clone(),
            label: note.clone(),
            spendable: None,
        })?);
    }

    // Emit one "output" entry per outpoint that has a label OR a freeze flag.
    let mut outpoints: HashSet<String> = utxo_labels.keys().cloned().collect();
    outpoints.extend(frozen.iter().cloned());
    for op in outpoints {
        let label = utxo_labels.get(&op).cloned().unwrap_or_default();
        let is_frozen = frozen.contains(&op);
        lines.push(serde_json::to_string(&LabelEntry {
            kind: "output".into(),
            reference: op,
            label,
            // Only emit `spendable` for frozen outputs — the spec says it
            // defaults to true if absent.
            spendable: if is_frozen {
                Some("false".into())
            } else {
                None
            },
        })?);
    }

    let body = lines.join("\n") + "\n";
    let filename = format!(
        "corvin-labels-{}.jsonl",
        chrono::Utc::now().format("%Y-%m-%d")
    );
    Ok((
        [
            (header::CONTENT_TYPE, "application/jsonl".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        body,
    ))
}

#[derive(Deserialize)]
pub struct ImportBip329Request {
    /// Raw JSONL content. We accept it as a single string to keep the API
    /// trivially callable from the frontend.
    pub jsonl: String,
    /// If true, drop any existing labels not present in the file. Default
    /// is merge-only (add/overwrite from file, keep the rest).
    #[serde(default)]
    pub replace: bool,
}

#[derive(Serialize)]
pub struct ImportBip329Result {
    pub tx_labels: usize,
    pub address_labels: usize,
    pub utxo_labels: usize,
    pub frozen_changes: usize,
    pub skipped: usize,
}

pub async fn import_bip329(
    State(state): State<AppState>,
    axum::Json(req): axum::Json<ImportBip329Request>,
) -> Result<axum::Json<ImportBip329Result>, ApiError> {
    let mut tx_count = 0usize;
    let mut addr_count = 0usize;
    let mut output_count = 0usize;
    let mut frozen_changes = 0usize;
    let mut skipped = 0usize;

    // Build the new label/freeze state in locals, then write it through in one
    // call. Start empty on replace, otherwise from the current maps.
    let mut tx_labels = if req.replace {
        HashMap::new()
    } else {
        state.annotations.tx_labels().await
    };
    let mut addr_labels = if req.replace {
        HashMap::new()
    } else {
        state.annotations.address_labels().await
    };
    let mut utxo_labels = if req.replace {
        HashMap::new()
    } else {
        state.annotations.utxo_labels().await
    };
    let mut frozen = if req.replace {
        HashSet::new()
    } else {
        state.annotations.frozen_utxos().await
    };

    for raw in req.jsonl.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let entry: LabelEntry = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        match entry.kind.as_str() {
            "tx" => {
                if !entry.label.is_empty() {
                    tx_labels.insert(entry.reference, entry.label);
                    tx_count += 1;
                }
            }
            "addr" => {
                if !entry.label.is_empty() {
                    addr_labels.insert(entry.reference, entry.label);
                    addr_count += 1;
                }
            }
            "output" => {
                let outpoint = entry.reference;
                let mut touched = false;
                if !entry.label.is_empty() {
                    utxo_labels.insert(outpoint.clone(), entry.label);
                    output_count += 1;
                    touched = true;
                }
                // The spec uses string "false" / "true".
                match entry.spendable.as_deref() {
                    Some("false") if frozen.insert(outpoint.clone()) => {
                        frozen_changes += 1;
                        touched = true;
                    }
                    Some("true") if frozen.remove(&outpoint) => {
                        frozen_changes += 1;
                        touched = true;
                    }
                    _ => {}
                }
                if !touched {
                    skipped += 1;
                }
            }
            // input/pubkey/xpub aren't currently tracked by Corvin — ignore.
            _ => {
                skipped += 1;
            }
        }
    }

    state
        .annotations
        .replace_labels(tx_labels, addr_labels, utxo_labels, frozen)
        .await?;

    Ok(axum::Json(ImportBip329Result {
        tx_labels: tx_count,
        address_labels: addr_count,
        utxo_labels: output_count,
        frozen_changes,
        skipped,
    }))
}
