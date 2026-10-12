//! The batch jobs over the book (ADR-012), run from the job queue on an in-memory store: the
//! renewal notice is issued and kept as issued (R35), the period opens on it or on the customer's
//! accepted change and bills the premium once (R03, R04, R27), a cancelled customer's policy is
//! not renewed and their optional data is erased with a record (R20, R41, R54), and the month's
//! experience study counts the book. Running the day twice changes nothing.
use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::consent::{self, Endorsement, OfferKind};
use microinsurance_engine::money::Money;
use microinsurance_engine::optimiser::rating::RatingBasis;
use microinsurance_engine::types::{
    Allocation, Policy, PolicyPeriod, Profile, Schedule, WorkWindow, add_policy_periods,
};
use microinsurance_ledger::{Account, LedgerStore, MemoryLedger};
use microinsurance_store::{By, Keys, Store, collections as c};
use microinsurance_workers::book::{
    CustomerRecord, ErasureRecord, PolicyRecord, RatingAnswers, RenewalRecord,
};
use microinsurance_workers::config_dir::ConfigDir;
use microinsurance_workers::jobs::{self, Batch, PolicyPeriodRecord, Work};
use microinsurance_workers::queue::{JobStatus, Queue};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn cfg() -> EngineConfig {
    ConfigDir::base(&repo().join("config"))
        .load()
        .expect("config loads")
}

fn basis() -> RatingBasis {
    let text = std::fs::read(repo().join("fixtures/rating_basis_synthetic.json")).expect("basis");
    serde_json::from_slice(&text).expect("rating basis")
}

fn date(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).expect("date")
}

/// 09:00 local on a day of 2026.
fn morning(m: u32, d: u32) -> DateTime<Utc> {
    date(m, d).and_hms_opt(1, 0, 0).expect("time").and_utc()
}

fn rider(id: &str) -> Profile {
    Profile {
        customer_id: format!("cust-{id}"),
        work_windows: (1..=5)
            .map(|d| WorkWindow {
                weekday: d,
                start_minute: 9 * 60,
                end_minute: 18 * 60,
            })
            .collect(),
        consents: ["trip_data".to_string()].into(),
        ..Profile::default()
    }
}

/// A customer with weather income cover for October 2026, its premium paid.
async fn bound(store: &Store, cfg: &EngineConfig, id: &str) {
    let profile = rider(id);
    let plan = cfg.plan("essential").expect("plan").clone();
    let starts = date(10, 1);
    let policy = Policy {
        policy_id: format!("pol-{id}"),
        customer_id: profile.customer_id.clone(),
        schedule: Schedule {
            plan_id: plan.id.clone(),
            allocations: vec![Allocation {
                cover_id: "weather_income".into(),
                premium: plan.price,
                sum_insured: Money::from_major(480),
            }],
        },
        plan,
        period: PolicyPeriod {
            index: 1,
            starts,
            ends: add_policy_periods(starts, 1),
        },
        cover_starts: [("weather_income".to_string(), starts)].into(),
        work_windows: profile.work_windows.clone(),
        commute_routes: Vec::new(),
        work_areas: Vec::new(),
        upcoming_events: Vec::new(),
        subscribed_at: morning(9, 30),
    };
    let keys = Keys::customer(&profile.customer_id);
    store
        .insert(
            c::CUSTOMERS,
            &profile.customer_id.clone(),
            keys.clone(),
            CustomerRecord {
                profile: profile.clone(),
                partner_id: None,
                created_at: morning(9, 30),
                cancelled: None,
            },
        )
        .await
        .expect("customer");
    store
        .insert(
            c::RATING_ANSWERS,
            &profile.customer_id,
            keys.clone(),
            RatingAnswers {
                answers: [
                    ("road_exposure".to_string(), "high".to_string()),
                    ("occupation_class".to_string(), "outdoor".to_string()),
                ]
                .into(),
            },
        )
        .await
        .expect("answers");
    store
        .insert(
            c::POLICIES,
            &policy.policy_id.clone(),
            keys,
            PolicyRecord {
                policy,
                partner_id: None,
                premium_due: starts,
                premium_paid_on: Some(starts),
                quote_id: "test".into(),
            },
        )
        .await
        .expect("policy");
}

async fn run_day(
    queue: &Queue,
    batch: &Batch<'_, MemoryLedger>,
    now: DateTime<Utc>,
) -> Vec<(String, Result<String, String>)> {
    queue
        .enqueue(jobs::day(now, batch.cfg).job(now).expect("the day's job"))
        .await
        .expect("enqueue");
    let mut out = Vec::new();
    // The day's job enqueues the rest, which run in the next claims.
    loop {
        let ran = jobs::run_due(queue, "worker-1", batch, now, 50)
            .await
            .expect("runs");
        if ran.is_empty() {
            break;
        }
        out.extend(ran.into_iter().map(|r| (r.key, r.outcome)));
    }
    out
}

fn keys(ran: &[(String, Result<String, String>)]) -> BTreeSet<&str> {
    ran.iter()
        .filter(|(_, o)| o.is_ok())
        .map(|(k, _)| k.as_str())
        .collect()
}

#[tokio::test]
async fn r35_r54_batch_jobs_renew_the_book_once() {
    let cfg = cfg();
    let basis = basis();
    let store = Store::memory();
    let ledger = MemoryLedger::new();
    let queue = Queue::memory();
    let in_force = BTreeSet::new();
    let batch = Batch {
        store: &store,
        ledger: &ledger,
        cfg: &cfg,
        basis: &basis,
        in_force: &in_force,
        full_credibility_claims: 1_082,
    };
    for id in ["stays", "changes", "leaves"] {
        bound(&store, &cfg, id).await;
    }
    // "changes" accepts a reallocation for November; "leaves" cancels in October.
    let changed = Schedule {
        plan_id: "essential".into(),
        allocations: vec![Allocation {
            cover_id: "weather_income".into(),
            premium: cfg.plan("essential").expect("plan").price,
            sum_insured: Money::from_major(300),
        }],
    };
    store
        .insert(
            c::ENDORSEMENTS,
            "endorsement-changes-2",
            Keys::customer("cust-changes"),
            Endorsement {
                policy_id: "pol-changes".into(),
                effective_period: 2,
                change: OfferKind::Reallocation {
                    schedule: changed.clone(),
                },
            },
        )
        .await
        .expect("endorsement");
    let mut leaving = store
        .get::<CustomerRecord>(c::CUSTOMERS, "cust-leaves")
        .await
        .expect("read")
        .expect("customer");
    leaving.body.cancelled = Some(consent::on_cancellation(
        &leaving.body.profile,
        date(10, 20),
        &cfg,
    ));
    store.update(c::CUSTOMERS, leaving).await.expect("cancel");

    // 25 October, `renewal_notice_days` before November: notices for those who stay, and the
    // cancelled customer's optional data erased.
    let ran = run_day(&queue, &batch, morning(10, 25)).await;
    assert_eq!(
        keys(&ran),
        BTreeSet::from([
            "schedule_day/2026-10-25",
            "propose_renewal/pol-stays/2",
            "propose_renewal/pol-changes/2",
            "erase_cancelled/cust-leaves",
        ]),
        "{ran:?}"
    );
    let notice = store
        .get::<RenewalRecord>(c::RENEWALS, "pol-stays/2")
        .await
        .expect("read")
        .expect("issued");
    assert_eq!(notice.body.issued_on, date(10, 25));
    assert_eq!(notice.body.period.starts, date(11, 1));
    let erased = store
        .get::<ErasureRecord>(c::ERASURES, "cust-leaves")
        .await
        .expect("read")
        .expect("erased");
    assert_eq!(
        erased.body.rating_factors,
        BTreeSet::from(["road_exposure".to_string()]),
        "the answer from trip data goes; the declared answer stays with the records (R54)"
    );
    let answers = store
        .get::<RatingAnswers>(c::RATING_ANSWERS, "cust-leaves")
        .await
        .expect("read")
        .expect("answers");
    assert_eq!(answers.body.answers.len(), 1);

    // 1 November: November opens on the notice, or on the accepted change; the cancelled
    // customer's cover ends with October.
    let ran = run_day(&queue, &batch, morning(11, 1)).await;
    assert_eq!(
        keys(&ran),
        BTreeSet::from([
            "schedule_day/2026-11-01",
            "open_policy_period/pol-stays/2",
            "open_policy_period/pol-changes/2",
            "experience_study/2026-10",
        ]),
        "{ran:?}"
    );
    let policy = |id: &str| {
        let store = store.clone();
        let id = id.to_string();
        async move {
            store
                .get::<PolicyRecord>(c::POLICIES, &id)
                .await
                .expect("read")
                .expect("policy")
                .body
        }
    };
    let stays = policy("pol-stays").await;
    assert_eq!(stays.policy.period.index, 2);
    assert_eq!(
        stays.policy.schedule, notice.body.default,
        "as notified (R35)"
    );
    assert_eq!(stays.premium_due, date(11, 1));
    assert_eq!(stays.premium_paid_on, None, "billed, not yet paid (R53)");
    assert_eq!(
        policy("pol-changes").await.policy.schedule,
        changed,
        "the customer's accepted change (R03, R04)"
    );
    assert_eq!(policy("pol-leaves").await.policy.period.index, 1, "R41");
    let billed = ledger
        .balance(Account::CustomerPremiums)
        .await
        .expect("balance");
    let price = cfg.plan("essential").expect("plan").price;
    assert_eq!(billed, Money(price.0 * 2), "two premiums billed");
    assert!(
        store
            .get::<PolicyPeriodRecord>(c::POLICY_PERIODS, "pol-stays/2")
            .await
            .expect("read")
            .is_some()
    );
    let study = store
        .get::<microinsurance_learning::experience::ExperienceStudy>(
            c::EXPERIENCE_STUDIES,
            "2026-10",
        )
        .await
        .expect("read")
        .expect("study");
    let october = &study.body.classes[0];
    assert_eq!(
        (october.class_of_business.as_str(), october.period.as_str()),
        ("pecuniary.income_weather", "2026-10")
    );
    assert_eq!(october.policy_periods, 3);
    assert!(
        october.expected_minor > 0,
        "expected cost from the rating basis"
    );

    // The same day again, and the next: nothing more is billed, nothing opens twice.
    let again = run_day(&queue, &batch, morning(11, 1)).await;
    assert!(again.is_empty(), "{again:?}");
    let next = run_day(&queue, &batch, morning(11, 2)).await;
    assert_eq!(keys(&next), BTreeSet::from(["schedule_day/2026-11-02"]));
    assert_eq!(
        ledger
            .balance(Account::CustomerPremiums)
            .await
            .expect("balance"),
        billed
    );
    // A job asked to open a period out of turn fails and is retried, never skips one.
    queue
        .enqueue(
            Work::OpenPolicyPeriod {
                policy_id: "pol-stays".into(),
                period_index: 4,
            }
            .job(morning(11, 2))
            .expect("job"),
        )
        .await
        .expect("enqueue");
    let ran = jobs::run_due(&queue, "worker-1", &batch, morning(11, 2), 10)
        .await
        .expect("runs");
    assert!(ran[0].outcome.is_err());
    assert_eq!(
        queue
            .counts()
            .await
            .expect("counts")
            .get(&JobStatus::Queued),
        Some(&1)
    );
    // Every policy period opened is on record for experience studies.
    let periods = store
        .list::<PolicyPeriodRecord>(c::POLICY_PERIODS, By::All)
        .await
        .expect("periods");
    assert_eq!(periods.len(), 5);
}

#[tokio::test]
async fn r51_new_allocation_is_checked_against_the_latest_measure_of_each_zone() {
    use microinsurance_workers::rating::{self, AccumulationSnapshot, CURRENT};
    let cfg = cfg();
    let basis = basis();
    let store = Store::memory();
    let ledger = MemoryLedger::new();
    let queue = Queue::memory();
    let in_force = BTreeSet::new();
    let batch = Batch {
        store: &store,
        ledger: &ledger,
        cfg: &cfg,
        basis: &basis,
        in_force: &in_force,
        full_credibility_claims: 1_082,
    };
    bound(&store, &cfg, "rider").await;
    let now = morning(10, 25);
    // One snapshot per `refresh_minutes`, however many workers enqueue it.
    let work = jobs::accumulation(now, &cfg);
    assert!(
        queue
            .enqueue(work.job(now).expect("job"))
            .await
            .expect("enqueue")
    );
    let later = now + chrono::Duration::seconds(30);
    assert!(
        !queue
            .enqueue(jobs::accumulation(later, &cfg).job(later).expect("job"))
            .await
            .expect("enqueue"),
        "the same interval is measured once"
    );
    let ran = jobs::run_due(&queue, "worker-1", &batch, now, 10)
        .await
        .expect("runs");
    assert!(ran[0].outcome.is_ok(), "{ran:?}");
    let kept = store
        .get::<AccumulationSnapshot>(c::ACCUMULATION, CURRENT)
        .await
        .expect("read")
        .expect("snapshot");
    assert_eq!(kept.body.as_of, now);
    // A fresh snapshot is what a quote reads: a zone marked full there is full.
    let full: BTreeSet<_> = [("windstorm_and_rain".to_string(), None)].into();
    let mut marked = kept;
    marked.body.full_zones = full.clone();
    store.update(c::ACCUMULATION, marked).await.expect("mark");
    assert_eq!(
        rating::full_zones::<microinsurance_store::StoreError>(&store, &cfg, later)
            .await
            .expect("zones"),
        full
    );
    // A stale one is not trusted: the book is measured again.
    let stale = now + chrono::Duration::minutes(i64::from(cfg.accumulation.refresh_minutes) * 3);
    assert!(
        rating::full_zones::<microinsurance_store::StoreError>(&store, &cfg, stale)
            .await
            .expect("zones")
            .is_empty()
    );
}
