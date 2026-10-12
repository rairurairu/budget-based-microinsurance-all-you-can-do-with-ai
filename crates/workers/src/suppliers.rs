//! `SettleSuppliers` (monthly) and `ExpireInKindOrders` (daily): suppliers are paid for what they
//! delivered, and what was not used in time goes back to the customer (R56). The engine decides
//! every amount; this posts them to the ledger unchanged.
use chrono::NaiveDate;
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::suppliers::{self, InKindOrder, SupplierStatement};
use microinsurance_ledger::{LedgerStore, postings};

/// Each supplier's statement for deliveries in `from..until`, posted once under its own key.
/// Suppliers with nothing delivered get no statement.
pub async fn settle(
    orders: &[InKindOrder],
    from: NaiveDate,
    until: NaiveDate,
    cfg: &EngineConfig,
    ledger: &impl LedgerStore,
) -> Result<Vec<SupplierStatement>, String> {
    let mut out = Vec::new();
    for id in cfg.suppliers.suppliers.keys() {
        let s = suppliers::statement(id, from, until, orders);
        if s.lines.is_empty() {
            continue;
        }
        let (key, entries) = postings::supplier_paid(&s);
        ledger
            .post(&key, &entries)
            .await
            .map_err(|e| format!("{key}: {e}"))?;
        out.push(s);
    }
    Ok(out)
}

/// Expires orders not delivered by their expiry and posts the amount back to each customer.
pub async fn expire(
    orders: Vec<InKindOrder>,
    today: NaiveDate,
    ledger: &impl LedgerStore,
) -> Result<Vec<InKindOrder>, String> {
    let mut out = Vec::with_capacity(orders.len());
    for o in orders {
        let o = suppliers::expire(o, today);
        if let Some((key, entries)) = postings::in_kind_returned(&o) {
            ledger
                .post(&key, &entries)
                .await
                .map_err(|e| format!("{key}: {e}"))?;
        }
        out.push(o);
    }
    Ok(out)
}
