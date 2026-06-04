use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::get_managed;
use crate::api::ApiError;
use crate::state::{AppState, WalletInner};

#[derive(Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum CostBasisMethod {
    Fifo,
    #[default]
    Hifo,
    Lifo,
}

#[derive(Deserialize)]
pub struct TaxReportQuery {
    pub year: i32,
    #[serde(default)]
    pub method: CostBasisMethod,
}

#[derive(Serialize)]
pub struct TaxEntry {
    pub txid: String,
    pub date: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub btc: f64,
    pub usd_price: Option<f64>,
    pub usd_value: Option<f64>,
    pub cost_basis: Option<f64>,
    pub gain_loss: Option<f64>,
}

struct Lot {
    acquired_at: i64,
    sats: u64,
    price_per_btc: f64,
}

/// Consume `sats_needed` from the lot pool using the given method.
/// Mutates the pool in place (removes fully consumed lots, trims partial ones).
/// Returns the total cost basis in USD for the consumed amount.
///
/// Amounts are tracked as integer sats so multi-year lot bookkeeping doesn't
/// accumulate float drift. Only the final price-times-sats multiplication is
/// done in f64 (USD prices are <6 sig figs anyway).
fn consume_lots(lots: &mut Vec<Lot>, sats_needed: u64, method: CostBasisMethod) -> f64 {
    match method {
        CostBasisMethod::Fifo => lots.sort_by_key(|l| l.acquired_at),
        CostBasisMethod::Lifo => lots.sort_by_key(|l| std::cmp::Reverse(l.acquired_at)),
        CostBasisMethod::Hifo => lots.sort_by(|a, b| {
            b.price_per_btc
                .partial_cmp(&a.price_per_btc)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
    }
    let mut cost = 0.0f64;
    let mut remaining = sats_needed;
    for lot in lots.iter_mut() {
        if remaining == 0 {
            break;
        }
        let take = remaining.min(lot.sats);
        cost += (take as f64 / 1e8) * lot.price_per_btc;
        lot.sats -= take;
        remaining -= take;
    }
    lots.retain(|l| l.sats > 0);
    cost
}

pub async fn get_tax_report(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<TaxReportQuery>,
) -> Result<Json<Vec<TaxEntry>>, ApiError> {
    let managed = get_managed(&state, &id).await?;

    // All txs needed — lot state depends on full history. Owned Vec
    // because we sort it.
    let mut all_txs: Vec<corvin_core::types::TxRecord> = match &managed.inner {
        WalletInner::Hd(_) => {
            let arc = managed.txs_snapshot.lock().await.clone();
            (*arc).clone()
        }
        WalletInner::Address(cache) => cache
            .lock()
            .await
            .as_ref()
            .map(|c| c.txs.clone())
            .unwrap_or_default(),
        WalletInner::SilentPayments(cache) => cache.lock().await.txs(),
    };

    all_txs.sort_by_key(|tx| tx.timestamp);

    // Pre-fetch prices in parallel, deduplicated by UTC day. Capped
    // concurrency keeps us from rate-limiting the mempool price API.
    let unique_days: std::collections::HashSet<i64> = all_txs
        .iter()
        .filter_map(|tx| {
            tx.timestamp.map(|dt| {
                let ts = dt.timestamp();
                ts - (ts % 86_400)
            })
        })
        .collect();

    use futures_util::stream::{self, StreamExt};
    const PRICE_FETCH_CONCURRENCY: usize = 8;
    let day_prices: std::collections::HashMap<i64, f64> = stream::iter(unique_days)
        .map(|day| {
            let s = state.clone();
            async move {
                let price = crate::api::prices::fetch_price_cached(&s, day, true).await;
                (day, price)
            }
        })
        .buffer_unordered(PRICE_FETCH_CONCURRENCY)
        .filter_map(|(day, price)| async move { price.map(|p| (day, p)) })
        .collect()
        .await;

    let price_map: std::collections::HashMap<String, f64> = all_txs
        .iter()
        .filter_map(|tx| {
            let dt = tx.timestamp?;
            let ts = dt.timestamp();
            let day = ts - (ts % 86_400);
            Some((tx.txid.clone(), *day_prices.get(&day)?))
        })
        .collect();

    let overrides = state.annotations.cost_basis().await;
    let mut lots: Vec<Lot> = Vec::new();
    let mut entries: Vec<TaxEntry> = Vec::new();

    for tx in &all_txs {
        let Some(dt) = tx.timestamp else { continue };
        let sats = tx.amount_sats.unsigned_abs();
        let btc = sats as f64 / 1e8;
        let usd_price = price_map.get(&tx.txid).copied();
        let usd_value = usd_price.map(|p| btc * p);
        let in_year = dt.year() == q.year;

        if tx.amount_sats >= 0 {
            // Received: add a lot to the pool. A cost-basis override on a received
            // tx sets that lot's basis (total USD → per-BTC) so it flows into
            // gain/loss when these coins are later sold; otherwise use the day's
            // price. Without either, no lot is added (unknown basis).
            let lot_price = match overrides.get(&tx.txid) {
                Some(&ov) if btc > 0.0 => Some(ov / btc),
                _ => usd_price,
            };
            if let Some(price) = lot_price {
                lots.push(Lot {
                    acquired_at: dt.timestamp(),
                    sats,
                    price_per_btc: price,
                });
            }
            if in_year {
                let cb = overrides.get(&tx.txid).copied().or(usd_value);
                entries.push(TaxEntry {
                    txid: tx.txid.clone(),
                    date: dt.format("%Y-%m-%d").to_string(),
                    kind: "received".to_string(),
                    btc,
                    usd_price,
                    usd_value,
                    cost_basis: cb,
                    gain_loss: None,
                });
            }
        } else {
            // Sent: always remove the disposed coins from inventory (even when an
            // override sets the *reported* basis), so later sales don't
            // double-count them. Report the override if present, else the
            // consumed lots' cost.
            let consumed = if !lots.is_empty() {
                Some(consume_lots(&mut lots, sats, q.method))
            } else {
                None
            };
            let cb = overrides.get(&tx.txid).copied().or(consumed);
            let gain_loss = usd_value.zip(cb).map(|(v, b)| v - b);
            if in_year {
                entries.push(TaxEntry {
                    txid: tx.txid.clone(),
                    date: dt.format("%Y-%m-%d").to_string(),
                    kind: "sent".to_string(),
                    btc,
                    usd_price,
                    usd_value,
                    cost_basis: cb,
                    gain_loss,
                });
            }
        }
    }

    entries.sort_by(|a, b| a.date.cmp(&b.date));
    Ok(Json(entries))
}

#[cfg(test)]
mod tests {
    use super::{consume_lots, CostBasisMethod, Lot};

    const BTC: u64 = 100_000_000;

    fn pool() -> Vec<Lot> {
        vec![
            Lot { acquired_at: 1, sats: BTC, price_per_btc: 10_000.0 },
            Lot { acquired_at: 2, sats: BTC, price_per_btc: 20_000.0 },
        ]
    }

    #[test]
    fn fifo_takes_oldest() {
        let mut lots = pool();
        let cost = consume_lots(&mut lots, BTC, CostBasisMethod::Fifo);
        assert!((cost - 10_000.0).abs() < 1e-6);
        assert_eq!(lots.len(), 1);
        assert!((lots[0].price_per_btc - 20_000.0).abs() < 1e-6, "oldest lot consumed");
    }

    #[test]
    fn lifo_takes_newest() {
        let mut lots = pool();
        let cost = consume_lots(&mut lots, BTC, CostBasisMethod::Lifo);
        assert!((cost - 20_000.0).abs() < 1e-6);
        assert!((lots[0].price_per_btc - 10_000.0).abs() < 1e-6, "newest lot consumed");
    }

    #[test]
    fn hifo_takes_highest_price() {
        let mut lots = pool();
        let cost = consume_lots(&mut lots, BTC, CostBasisMethod::Hifo);
        assert!((cost - 20_000.0).abs() < 1e-6);
        assert!((lots[0].price_per_btc - 10_000.0).abs() < 1e-6, "highest-priced lot consumed");
    }

    #[test]
    fn partial_consume_trims_the_lot() {
        let mut lots = pool();
        let cost = consume_lots(&mut lots, BTC / 2, CostBasisMethod::Fifo);
        assert!((cost - 5_000.0).abs() < 1e-6);
        assert_eq!(lots.len(), 2);
        assert_eq!(lots[0].sats, BTC / 2, "oldest lot trimmed, not removed");
    }

    #[test]
    fn over_consume_drains_pool() {
        // Disposing more than held consumes everything; cost = what was available.
        let mut lots = pool();
        let cost = consume_lots(&mut lots, BTC * 3, CostBasisMethod::Fifo);
        assert!((cost - 30_000.0).abs() < 1e-6);
        assert!(lots.is_empty());
    }
}
