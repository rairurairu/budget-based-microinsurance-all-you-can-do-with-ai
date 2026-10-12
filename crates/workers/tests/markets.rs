//! A second market by config (ADR-012): Taiwan's demo script replays the CAP feeds of a typhoon's
//! work suspensions through the same desks, engine and ledger as Hong Kong's, and pays only where
//! the customer declared they work (R08, R51, R55).
use microinsurance_engine::money::Money;
use microinsurance_engine::payouts::PayoutStage;
use microinsurance_engine::triggers;
use microinsurance_ledger::{Account, LedgerStore, MemoryLedger};
use microinsurance_workers::config_dir::ConfigDir;
use microinsurance_workers::demo::{self, DemoScript};
use microinsurance_workers::desk::Book;
use microinsurance_workers::sources;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[tokio::test]
async fn r55_taiwan_pays_where_the_local_government_suspends_work() {
    let config = ConfigDir::new(&repo().join("config"), Some("tw"), None).expect("tw");
    let cfg = config.load().expect("tw loads");
    assert_eq!(
        sources::built(&config).expect("sources"),
        [
            "cwa_rain_advisories",
            "cwa_typhoon_warnings",
            "dgpa_work_suspension",
            "organiser_notices"
        ],
        "every source is built from its config file's adapter"
    );
    let script = DemoScript::load(&repo(), &config).expect("script");
    let policies = script.book(&cfg).expect("book");
    let index = triggers::build_index(&policies, &cfg.catalogue);
    let (due, suitable) = (BTreeMap::new(), BTreeSet::new());
    let book = Book {
        cfg: &cfg,
        policies: &policies,
        index: &index,
        premium_due: &due,
        suitable: &suitable,
    };
    let ledger = MemoryLedger::new();
    let replayed = demo::replay(&script, &config, &book, &ledger)
        .await
        .expect("replays");
    let mut paid: BTreeMap<&str, Vec<(PayoutStage, Money)>> = BTreeMap::new();
    for d in &replayed.decided {
        paid.entry(d.policy_id.as_str())
            .or_default()
            .push((d.stage, d.amount));
    }
    let ntd = Money::from_major;
    assert_eq!(
        paid,
        BTreeMap::from([
            (
                "demo-taipei-rider",
                vec![(PayoutStage::Guaranteed, ntd(1_200))]
            ),
            (
                "demo-new-taipei-courier",
                vec![(PayoutStage::Guaranteed, ntd(1_000))]
            ),
            (
                "demo-hualien-stall",
                vec![
                    (PayoutStage::Guaranteed, ntd(800)),
                    (PayoutStage::Balance, ntd(800))
                ]
            ),
        ]),
        "Taoyuan was not suspended, Taichung suspended classes only, Kaohsiung's was an exercise"
    );
    assert_eq!(
        ledger.balance(Account::InsurerRisk).await.expect("balance"),
        ntd(3_800),
        "posted once each (R27)"
    );
    // One typhoon across three counties is one loss occurrence (R51).
    let occurrences: BTreeSet<&str> = replayed
        .decided
        .iter()
        .map(|d| d.occurrence_id.as_str())
        .collect();
    assert_eq!(occurrences.len(), 1, "{occurrences:?}");
    assert!(replayed.lines.iter().any(|l| l.contains("TWD 1200.00")));
}

#[tokio::test]
async fn the_base_market_replays_its_own_script() {
    let config = ConfigDir::base(&repo().join("config"));
    let cfg = config.load().expect("config");
    let script = DemoScript::load(&repo(), &config).expect("script");
    let policies = script.book(&cfg).expect("book");
    let index = triggers::build_index(&policies, &cfg.catalogue);
    let (due, suitable) = (BTreeMap::new(), BTreeSet::new());
    let book = Book {
        cfg: &cfg,
        policies: &policies,
        index: &index,
        premium_due: &due,
        suitable: &suitable,
    };
    let replayed = demo::replay(&script, &config, &book, &MemoryLedger::new())
        .await
        .expect("replays");
    let total: i64 = replayed.decided.iter().map(|d| d.amount.0).sum();
    assert_eq!(Money(total), Money::from_major(1_910));
}
