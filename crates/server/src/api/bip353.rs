//! BIP-353 human-readable name resolution (`₿user@domain` → bitcoin: URI).
//!
//! The name maps to a DNS TXT record at `{user}.user._bitcoin-payment.{domain}`
//! containing a BIP-21 `bitcoin:` URI. Resolution MUST be DNSSEC-validated (a
//! plain TXT lookup is spoofable — this decides where money goes), so we build
//! a full DNSSEC proof to the root with `dnssec-prover` and verify it. The DoH
//! server is only transport: the proof is validated locally, so it can't forge
//! the answer. We fetch over the shared socks5-aware HTTP client, so the lookup
//! honors the configured Tor proxy.

use axum::{extract::State, Json};
use dnssec_prover::query::{ProofBuilder, QueryBuf};
use dnssec_prover::rr::{Name, RR, TXT_TYPE};
use dnssec_prover::ser::parse_rr_stream;
use dnssec_prover::validation::verify_rr_stream;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api::ApiError;
use crate::state::AppState;

/// Default RFC 8484 DoH endpoint when none is configured. Transport only (DNSSEC
/// validation makes it untrusted), and requests go through the socks5 proxy when one
/// is set. Cloudflare because it also speaks HTTP/1.1; Quad9 is HTTP/2-only and 505s
/// when reqwest falls back to HTTP/1.1 through a proxy. Overridable via
/// `backend.bip353_doh_url`.
const DEFAULT_DOH_URL: &str = "https://cloudflare-dns.com/dns-query";

#[derive(Deserialize)]
pub struct ResolveNameRequest {
    /// `user@domain` or `₿user@domain`.
    pub name: String,
}

#[derive(Serialize)]
pub struct ResolveNameResponse {
    /// The DNSSEC-validated BIP-21 `bitcoin:` URI from the record.
    pub uri: String,
    /// Normalized `user@domain` that was resolved.
    pub hrn: String,
}

pub async fn resolve_name(
    State(state): State<AppState>,
    Json(req): Json<ResolveNameRequest>,
) -> Result<Json<ResolveNameResponse>, ApiError> {
    let raw = req.name.trim().trim_start_matches('₿').trim();
    let (user, domain) = raw
        .split_once('@')
        .ok_or_else(|| anyhow::anyhow!("not a name@domain address"))?;
    if user.is_empty() || domain.is_empty() || user.contains('@') {
        return Err(anyhow::anyhow!("invalid human-readable name").into());
    }

    let qname = format!("{user}.user._bitcoin-payment.{domain}.");
    let name = Name::try_from(qname.as_str())
        .map_err(|_| anyhow::anyhow!("name is not a valid DNS name"))?;

    let client = state.http.read().await.clone();
    let doh_url = {
        let u = state.config.read().await.backend.bip353_doh_url.trim().to_string();
        if u.is_empty() { DEFAULT_DOH_URL.to_string() } else { u }
    };

    // Build the DNSSEC proof: the builder hands us DNS queries to send, we POST
    // each over DoH and feed back the response, until the chain is complete.
    let (mut builder, first) = ProofBuilder::new(&name, TXT_TYPE);
    let mut pending = vec![first];
    let mut steps = 0u32;
    while let Some(q) = pending.pop() {
        steps += 1;
        if steps > 64 {
            return Err(anyhow::anyhow!("DNSSEC proof exceeded step limit").into());
        }
        let resp = client
            .post(&doh_url)
            .header("content-type", "application/dns-message")
            .header("accept", "application/dns-message")
            .body(q.to_vec())
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("DNS-over-HTTPS request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("DNS-over-HTTPS error: HTTP {}", resp.status()).into());
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| anyhow::anyhow!("reading DoH response: {e}"))?;
        let mut buf = QueryBuf::new_zeroed(0);
        buf.extend_from_slice(&bytes);
        let more = builder.process_response(&buf).map_err(|e| {
            use dnssec_prover::query::ProofBuildingError as E;
            match e {
                E::NoSuchName | E::MissingRecord => anyhow::anyhow!(
                    "No BIP-353 record for {raw}. Note: this is also the Lightning Address format — \
                     if it's a Lightning address (e.g. Strike, a Proton email), Corvin can't pay it \
                     (on-chain only). BIP-353 names publish an on-chain or Silent Payment address via DNS."
                ),
                E::Unauthenticated => anyhow::anyhow!(
                    "{domain} isn't DNSSEC-signed — BIP-353 requires DNSSEC, so this name can't be verified."
                ),
                E::ServerFailure => anyhow::anyhow!(
                    "DNS lookup for {raw} failed at the server — try again in a moment."
                ),
                E::InvalidResponse => anyhow::anyhow!("The DNS server returned a response we couldn't parse."),
                E::NoResponseExpected => anyhow::anyhow!("Internal DNS proof-building error."),
            }
        })?;
        pending.extend(more);
    }

    let (proof, _ttl) = builder.finish_proof().map_err(|_| {
        anyhow::anyhow!(
            "couldn't complete a DNSSEC proof for {raw} — the name may not exist, or DNSSEC isn't enabled for {domain}"
        )
    })?;

    let rrs = parse_rr_stream(&proof).map_err(|_| anyhow::anyhow!("malformed DNSSEC proof"))?;
    let verified =
        verify_rr_stream(&rrs).map_err(|e| anyhow::anyhow!("DNSSEC validation failed: {e:?}"))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now < verified.valid_from || now > verified.expires {
        return Err(anyhow::anyhow!(
            "the DNSSEC proof isn't valid right now — check your system clock"
        )
        .into());
    }

    // Find the validated TXT record (resolve_name follows CNAME/DNAME) and pull
    // the bitcoin: URI out of it.
    let uri = verified
        .resolve_name(&name)
        .into_iter()
        .find_map(|rr| {
            if let RR::Txt(txt) = rr {
                String::from_utf8(txt.data.as_vec())
                    .ok()
                    .filter(|s| s.trim().to_lowercase().starts_with("bitcoin:"))
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("no Bitcoin payment instruction found for {raw}"))?;

    Ok(Json(ResolveNameResponse {
        uri: uri.trim().to_string(),
        hrn: format!("{user}@{domain}"),
    }))
}
