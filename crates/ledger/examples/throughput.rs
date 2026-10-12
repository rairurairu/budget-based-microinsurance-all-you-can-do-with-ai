//! Ledger postings after a trigger (docs/performance.md: ≥ 5,000 idempotent postings/s).
//! Needs `DATABASE_URL`: `cargo run --release -p microinsurance-ledger --example throughput`.
use microinsurance_engine::money::Money;
use microinsurance_ledger::{Account, LedgerStore, PgLedger, Posting};
use std::time::Instant;

const BATCH: usize = 1_000;
const BATCHES: usize = 20;

#[tokio::main]
async fn main() -> Result<(), String> {
    let url = std::env::var("DATABASE_URL").map_err(|_| "set DATABASE_URL")?;
    let ledger = PgLedger::connect(&url).await.map_err(|e| e.to_string())?;
    ledger.migrate().await.map_err(|e| e.to_string())?;
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let entry = |i: usize| {
        (
            format!("throughput/{run}/{i}"),
            vec![
                Posting {
                    account: Account::InsurerRisk,
                    amount: Money(48_000),
                },
                Posting {
                    account: Account::PayoutsPayable,
                    amount: Money(-48_000),
                },
            ],
        )
    };
    for label in ["first post", "retry (already posted)"] {
        let started = Instant::now();
        for b in 0..BATCHES {
            let batch: Vec<_> = (b * BATCH..(b + 1) * BATCH).map(entry).collect();
            let results = ledger.post_many(&batch).await;
            if let Some(Err(e)) = results.into_iter().find(Result::is_err) {
                return Err(e.to_string());
            }
        }
        let secs = started.elapsed().as_secs_f64();
        let n = BATCH * BATCHES;
        println!(
            "{label:<24} {n} postings in {secs:.2} s: {:.0}/s",
            n as f64 / secs
        );
    }
    Ok(())
}
