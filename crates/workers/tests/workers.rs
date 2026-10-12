//! The jobs end to end on the real `config/`, the saved HKO responses and the in-memory ledger.
use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::consent::Endorsement;
use microinsurance_engine::consent::OfferKind;
use microinsurance_engine::money::{Bps, Money, Rate};
use microinsurance_engine::optimiser::PeriodContext;
use microinsurance_engine::optimiser::rating;
use microinsurance_engine::triggers;
use microinsurance_engine::types::{
    Allocation, Policy, PolicyPeriod, Profile, Schedule, WorkWindow, add_policy_periods,
};
use microinsurance_ledger::{Account, LedgerStore, MemoryLedger};
use microinsurance_triggers::adapters::hko::HkoWarnings;
use microinsurance_triggers::{Feed, TriggerSource};
use microinsurance_workers::config_dir::{self, ConfigDir};
use microinsurance_workers::desk::{Book, TriggerDesk};
use microinsurance_workers::renewal;
use microinsurance_workers::state::PgState;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn cfg() -> EngineConfig {
    config_dir::load(&repo().join("config"), "insurer.example.yaml").expect("config loads")
}

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .expect("time")
        .with_timezone(&Utc)
}

fn september() -> PolicyPeriod {
    let starts = NaiveDate::from_ymd_opt(2026, 9, 1).expect("date");
    PolicyPeriod {
        index: 3,
        starts,
        ends: add_policy_periods(starts, 1),
    }
}

fn weekday_rider(cfg: &EngineConfig, id: &str) -> Policy {
    let plan = cfg.plan("essential").expect("plan").clone();
    Policy {
        policy_id: id.into(),
        customer_id: format!("cust-{id}"),
        schedule: Schedule {
            plan_id: plan.id.clone(),
            allocations: vec![Allocation {
                cover_id: "weather_income".into(),
                premium: plan.price,
                sum_insured: Money::from_major(480),
            }],
        },
        plan,
        period: september(),
        cover_starts: [("weather_income".to_string(), september().starts)].into(),
        work_windows: (1..=5)
            .map(|d| WorkWindow {
                weekday: d,
                start_minute: 9 * 60,
                end_minute: 18 * 60,
            })
            .collect(),
        commute_routes: Vec::new(),
        work_areas: Vec::new(),
        upcoming_events: Vec::new(),
        subscribed_at: utc("2026-06-01T00:00:00Z"),
    }
}

async fn poll(
    desk: &mut TriggerDesk,
    fixture: &str,
    now: &str,
    book: &Book<'_>,
) -> microinsurance_workers::desk::PollOutcome {
    let source = HkoWarnings {
        live: Feed::Fixture(repo().join("fixtures").join(fixture)),
        history: None,
    };
    let reports = source.poll().await.expect("fixture");
    desk.poll("hko_warnings", &reports, utc(now), book)
        .expect("poll")
}

#[tokio::test]
async fn poll_triggers_pays_once_and_posts_idempotently() {
    let cfg = cfg();
    let policies = vec![weekday_rider(&cfg, "pol-1"), weekday_rider(&cfg, "pol-2")];
    let index = triggers::build_index(&policies, &cfg.catalogue);
    let due: BTreeMap<String, Money> = [("pol-2".to_string(), Money::from_major(80))].into();
    let suitable = BTreeSet::new();
    let book = Book {
        cfg: &cfg,
        policies: &policies,
        index: &index,
        premium_due: &due,
        suitable: &suitable,
    };
    let ledger = MemoryLedger::new();
    let mut desk = TriggerDesk::default();

    // Signal 8 from Monday 09:40; polled at noon and again in the evening.
    let noon = poll(
        &mut desk,
        "hko_warnsum_signal8.json",
        "2026-09-14T04:00:00Z",
        &book,
    )
    .await;
    assert_eq!(noon.decided.len(), 2);
    assert!(desk.flush(&ledger).await.is_empty());
    let evening = poll(
        &mut desk,
        "hko_warnsum_signal8.json",
        "2026-09-14T12:00:00Z",
        &book,
    )
    .await;
    assert!(
        evening.decided.is_empty(),
        "one guaranteed payout per policy (R27)"
    );
    // A retried flush of entries already posted changes nothing.
    let (key, entries) = microinsurance_ledger::postings::payout_decided(&noon.decided[0]);
    desk.outbox.insert(key, entries);
    assert!(desk.flush(&ledger).await.is_empty());

    // Cancelled before Tuesday's shift: nothing more is owed.
    let closed = poll(
        &mut desk,
        "hko_warnsum_all_clear.json",
        "2026-09-14T22:30:00Z",
        &book,
    )
    .await;
    assert!(closed.decided.is_empty());
    assert_eq!(
        desk.occurrences.len(),
        1,
        "open until a day passes quietly (R51)"
    );
    let a_day_later = poll(
        &mut desk,
        "hko_warnsum_all_clear.json",
        "2026-09-15T23:00:00Z",
        &book,
    )
    .await;
    assert_eq!(a_day_later.grouped.closed.len(), 1);
    assert!(
        a_day_later.decided.is_empty(),
        "work resumed before Tuesday's shift"
    );
    assert!(desk.events.is_empty() && desk.payouts.is_empty());
    assert!(desk.flush(&ledger).await.is_empty());

    let risk = ledger.balance(Account::InsurerRisk).await.expect("balance");
    assert_eq!(risk, Money::from_major(960), "HK$480 each, once");
    // pol-2 is in the grace period: HK$80 premium set off (R53).
    assert_eq!(
        ledger.balance(Account::Cash).await.expect("balance"),
        Money::from_major(-880)
    );
    assert_eq!(
        ledger
            .balance(Account::CustomerPremiums)
            .await
            .expect("balance"),
        Money::from_major(-80)
    );
    assert_eq!(
        ledger
            .balance(Account::PayoutsPayable)
            .await
            .expect("balance"),
        Money::ZERO
    );
}

#[test]
fn renewal_re_rates_and_opens_the_next_period() {
    let cfg = cfg();
    let policy = weekday_rider(&cfg, "pol-1");
    let next = renewal::next_period(&policy.period);
    assert_eq!(next.index, 4);
    assert_eq!(next.starts, september().ends);
    let profile = Profile {
        customer_id: policy.customer_id.clone(),
        work_windows: policy.work_windows.clone(),
        typical_daily_earnings: Some(Money::from_major(700)),
        ..Profile::default()
    };
    // A flat rate for every approved cover: 4% of the sum insured a month.
    let ratings: BTreeMap<String, rating::CoverRating> = cfg
        .approved_products
        .iter()
        .filter_map(|id| cfg.catalogue.get(id))
        .map(|c| {
            let rating = rating::CoverRating {
                cover_id: c.id.clone(),
                class_of_business: c.class_of_business.clone(),
                rate: Rate(40_000),
                base_cost: Rate(20_000),
                relativity: Bps::ONE,
                shadow_relativity: Bps::ONE,
                experience: Bps::ONE,
                risk_class: rating::risk_class(Bps::ONE, &cfg),
                factors: Vec::new(),
            };
            (c.id.clone(), rating)
        })
        .collect();
    let (statuses, in_force, full) = (BTreeMap::new(), BTreeSet::new(), BTreeSet::new());
    let ctx = PeriodContext {
        cfg: &cfg,
        period: next,
        ratings: &ratings,
        statuses: &statuses,
        in_force: &in_force,
        full_zones: &full,
    };
    let proposal = renewal::propose(&policy, &profile, &ctx).expect("proposal");
    assert_eq!(proposal.notice.period_starts, next.starts);

    // Without an update, the default applies; the premium is billed for the new period.
    let (renewed, (key, entries)) = renewal::open_period(&policy, &proposal.default.schedule, None);
    assert_eq!(renewed.period, next);
    assert_eq!(renewed.schedule, proposal.default.schedule);
    assert_eq!(key, "premium/pol-1/4");
    assert_eq!(entries[0].amount, renewed.schedule.total_premium());
    assert_eq!(
        renewed.cover_starts.get("weather_income"),
        Some(&september().starts),
        "a continuing cover keeps its start"
    );

    // The customer's accepted reallocation for that period applies instead (R03, R04).
    let own = Schedule {
        plan_id: "essential".into(),
        allocations: vec![Allocation {
            cover_id: "commute_disruption".into(),
            premium: policy.plan.price,
            sum_insured: Money::from_major(300),
        }],
    };
    let update = Endorsement {
        policy_id: "pol-1".into(),
        effective_period: next.index,
        change: OfferKind::Reallocation {
            schedule: own.clone(),
        },
    };
    let (renewed, _) = renewal::open_period(&policy, &proposal.default.schedule, Some(&update));
    assert_eq!(renewed.schedule, own);
    assert_eq!(
        renewed.cover_starts.get("commute_disruption"),
        Some(&next.starts)
    );
}

/// A worker restarted between a typhoon's guaranteed payouts and its close pays the balance
/// once, from the saved desk, and never repays the guaranteed share (R27, R51).
#[tokio::test]
async fn restart_mid_event_resumes_from_saved_state() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL not set: PostgreSQL worker state test skipped");
        return;
    };
    let state = PgState::connect(&url).await.expect("connects");
    state.migrate().await.expect("migrates");
    let job = format!(
        "test/restart/{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    // One leader at a time.
    let lead = state.try_lead(&job).await.expect("lock").expect("leads");
    assert!(state.try_lead(&job).await.expect("lock").is_none());

    let cfg = cfg();
    // The day rider qualifies by noon and is guaranteed at once; the night courier's shift
    // (22:00-06:00) starts after the restart point and is paid in the balance.
    let mut night = weekday_rider(&cfg, "pol-night");
    for w in &mut night.work_windows {
        w.start_minute = 22 * 60;
        w.end_minute = 6 * 60;
    }
    let policies = vec![weekday_rider(&cfg, "pol-day"), night];
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
    let mut desk = TriggerDesk::default();
    poll(
        &mut desk,
        "hko_warnsum_signal8.json",
        "2026-09-14T04:00:00Z",
        &book,
    )
    .await;
    state.save(&job, &desk).await.expect("saved");
    assert!(desk.flush(&ledger).await.is_empty());
    drop(desk);
    drop(lead);

    // Restarted after the signal was cancelled: the next poll is the all-clear.
    let _lead = state
        .try_lead(&job)
        .await
        .expect("lock")
        .expect("leads again");
    let mut desk: TriggerDesk = state.load(&job).await.expect("loads").expect("saved state");
    let source = HkoWarnings {
        live: Feed::Fixture(repo().join("fixtures").join("hko_warnsum_all_clear.json")),
        history: None,
    };
    let reports = source.poll().await.expect("fixture");
    let resumed = desk
        .poll("hko_warnings", &reports, utc("2026-09-15T08:30:00Z"), &book)
        .expect("poll");
    let paid: Vec<&str> = resumed
        .decided
        .iter()
        .map(|d| d.policy_id.as_str())
        .collect();
    assert_eq!(
        paid,
        ["pol-night"],
        "the night shift qualified meanwhile; the day rider's guaranteed share is not repaid"
    );
    let close = desk
        .poll("hko_warnings", &reports, utc("2026-09-15T23:00:00Z"), &book)
        .expect("poll");
    assert_eq!(close.grouped.closed.len(), 1);
    assert!(close.decided.is_empty(), "both paid in full at once");
    assert!(desk.flush(&ledger).await.is_empty());
    assert_eq!(
        ledger.balance(Account::InsurerRisk).await.expect("balance"),
        Money::from_major(960),
        "each paid HK$480 once"
    );
}

/// A Tsuen Wan Line commuter and a concert-goer in November 2026 (synthetic).
fn november_book(cfg: &EngineConfig) -> Vec<Policy> {
    let starts = NaiveDate::from_ymd_opt(2026, 11, 1).expect("date");
    let november = PolicyPeriod {
        index: 5,
        starts,
        ends: add_policy_periods(starts, 1),
    };
    let one = |id: &str, cover: &str, si: i64| {
        let mut p = weekday_rider(cfg, id);
        p.period = november;
        p.schedule.allocations[0].cover_id = cover.into();
        p.schedule.allocations[0].sum_insured = Money::from_major(si);
        p.cover_starts = [(cover.to_string(), starts)].into();
        p
    };
    let mut commuter = one("commuter", "commute_disruption", 150);
    commuter.commute_routes = vec!["mtr.tsuen_wan".into()];
    let mut elsewhere = one("island-line-commuter", "commute_disruption", 150);
    elsewhere.commute_routes = vec!["mtr.island".into()];
    let mut fan = one("fan", "event_ticket", 1000);
    fan.upcoming_events = vec![microinsurance_engine::types::UpcomingEvent {
        kind: "event.concert".into(),
        starts: NaiveDate::from_ymd_opt(2026, 11, 21).expect("date"),
        ends: None,
        reference: Some("EVT-2026-1121-ARENA".into()),
        value: Some(Money::from_major(880)),
    }];
    vec![commuter, elsewhere, fan]
}

#[tokio::test]
async fn rail_and_organiser_sources_pay_through_their_own_desks() {
    use microinsurance_workers::sources;
    let cfg = cfg();
    let policies = november_book(&cfg);
    let index = triggers::build_index(&policies, &cfg.catalogue);
    let (due, suitable) = (BTreeMap::new(), BTreeSet::new());
    let book = Book {
        cfg: &cfg,
        policies: &policies,
        index: &index,
        premium_due: &due,
        suitable: &suitable,
    };
    let config = ConfigDir::base(&repo().join("config"));
    let ledger = MemoryLedger::new();
    let mut paid: Vec<(String, Money)> = Vec::new();
    for (id, script) in [
        (
            "mtr_line_status",
            vec![
                ("mtr_line_status_suspended.json", "2026-11-03T00:30:00Z"),
                ("mtr_line_status_suspended.json", "2026-11-03T01:30:00Z"),
                ("mtr_line_status_normal.json", "2026-11-03T02:10:00Z"),
                ("mtr_line_status_normal.json", "2026-11-03T04:00:00Z"),
            ],
        ),
        (
            "organiser_notices",
            vec![
                ("organiser_cancellation.json", "2026-11-18T07:05:00Z"),
                ("organiser_cancellation.json", "2026-11-21T13:00:00Z"),
            ],
        ),
    ] {
        let mut desk = TriggerDesk::default();
        for (file, at) in script {
            let source = sources::adapter(
                id,
                Feed::Fixture(repo().join("fixtures").join(file)),
                &config,
                &cfg,
            )
            .expect("adapter");
            let reports = source.poll().await.expect("fixture");
            let out = desk.poll(id, &reports, utc(at), &book).expect("poll");
            paid.extend(out.decided.iter().map(|d| (d.policy_id.clone(), d.amount)));
            assert!(desk.flush(&ledger).await.is_empty());
        }
        assert!(desk.occurrences.is_empty() || id == "organiser_notices");
    }
    // The commuter on the suspended line is paid once; the Island Line ran normally. The fan
    // gets the ticket's declared value, once, though the notice was listed twice (R27).
    assert_eq!(
        paid,
        [
            ("commuter".to_string(), Money::from_major(150)),
            ("fan".to_string(), Money::from_major(880)),
        ]
    );
    assert_eq!(
        ledger.balance(Account::InsurerRisk).await.expect("balance"),
        Money::from_major(1030)
    );
}

fn traveller() -> (Profile, microinsurance_engine::types::UpcomingEvent) {
    let profile = Profile {
        customer_id: "cust-synthetic-traveller".into(),
        consents: ["photos".to_string()].into(),
        ..Profile::default()
    };
    let trip = microinsurance_engine::types::UpcomingEvent {
        kind: "trip.air".into(),
        starts: NaiveDate::from_ymd_opt(2026, 12, 20).expect("date"),
        ends: NaiveDate::from_ymd_opt(2026, 12, 27),
        reference: Some("CX500/2026-12-20".into()),
        value: None,
    };
    (profile, trip)
}

fn dec(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 12, d).expect("date")
}

/// The prevention loop on saved replies: the AI proposes a registered measure, the engine sizes
/// and offers it (R18, R40), the verifier passes the photo, the engine credits points and the
/// reward within the cap (R10, R17), the customer spends it on a partner reward (R16), the
/// supplier delivers and is paid on its statement (R56).
#[tokio::test]
async fn prevention_task_to_supplier_statement_on_saved_replies() {
    use microinsurance_ai::verify::Evidence;
    use microinsurance_engine::guardrails::audit;
    use microinsurance_engine::prevention::{self, HeldCover, PreventionBasis, TaskState};
    use microinsurance_engine::rewards::{self, RewardOffer};
    use microinsurance_engine::suppliers::{self as engine_suppliers, OrderState};
    use microinsurance_ledger::postings;
    use microinsurance_workers::prevention::{PreventionDesk, TaskRequest, replay_ai};
    use microinsurance_workers::suppliers;

    let cfg = cfg();
    let (ai, vocab) =
        replay_ai(&repo(), &ConfigDir::base(&repo().join("config")), &cfg).expect("offline AI");
    let read = |name: &str| std::fs::read(repo().join("fixtures").join(name)).expect(name);
    let basis: PreventionBasis =
        serde_json::from_slice(&read("prevention_basis_synthetic.json")).expect("basis");
    let offers: Vec<RewardOffer> =
        serde_json::from_slice(&read("reward_offers_synthetic.json")).expect("offers");
    let (profile, trip) = traveller();
    let policy_id = "pol-traveller".to_string();
    let held = HeldCover {
        cover_id: "device_damage",
        sum_insured: Money::from_major(4_000),
        relativity: Bps(10_000),
    };
    let request = TaskRequest {
        profile: &profile,
        policy_id: &policy_id,
        event: &trip,
        held,
        offered_on: dec(1),
        history: &[],
        cohort: microinsurance_engine::pilot::Cohort::Offered,
    };
    // A seed that does not draw this task for audit, so the pass is credited at once.
    let id = "task/cust-synthetic-traveller/screen_protector_fitted/CX500/2026-12-20";
    let seed = (0..1_000)
        .find(|s| !audit::should_audit(id, cfg.guardrails.audit_rate, *s))
        .expect("seed");
    let desk = PreventionDesk {
        ai: &ai,
        vocab: &vocab,
        cfg: &cfg,
        basis: &basis,
        seed,
    };
    let task = desk.propose(&request).await.expect("a task");
    assert_eq!(task.task_id, id);
    assert_eq!(task.due, dec(20), "due when the trip starts");
    assert_eq!(
        task.draft.reward_cost,
        Money::from_major(3),
        "a quarter of HK$12 avoided"
    );

    let task = prevention::accept(task, dec(2)).expect("accepted");
    let photo = Evidence {
        media_type: "image/png".into(),
        bytes: read("evidence/screen_protector_synthetic.png"),
    };
    let (task, credit) = desk
        .check(
            task,
            std::slice::from_ref(&photo),
            &BTreeSet::new(),
            None,
            dec(3),
        )
        .await
        .expect("checked");
    assert!(matches!(task.state, TaskState::Verified { .. }));
    let credit = credit.expect("credited");
    assert_eq!(credit.activity.class_of_business(), "property.device");

    let ledger = MemoryLedger::new();
    let grant = rewards::grant(credit.reward, &[], &cfg);
    let (key, entries) = postings::reward_granted(&grant);
    ledger.post(&key, &entries).await.expect("granted");

    let plan = cfg.plan("essential").expect("plan");
    let menu = rewards::menu(grant.granted, &offers, plan, &cfg);
    let ids: Vec<&str> = menu.iter().map(|o| o.offer_id.as_str()).collect();
    assert_eq!(
        ids,
        ["cash", "lounge_day_pass", "screen_protector_fitting"],
        "cash first; nothing out of stock"
    );
    let pass = rewards::redeem(
        &profile.customer_id,
        "redeem/traveller/1",
        "lounge_day_pass",
        &menu,
    )
    .expect("redeemed");
    let (key, entries) = postings::reward_redeemed(&pass);
    ledger.post(&key, &entries).await.expect("redeemed");
    let order = engine_suppliers::order_for_reward(&pass, dec(3), &cfg).expect("order");
    let order = engine_suppliers::deliver(order, dec(20), None).expect("delivered");
    assert!(matches!(order.state, OrderState::Delivered { .. }));
    let paid = suppliers::settle(
        &[order],
        dec(1),
        dec(1) + chrono::Months::new(1),
        &cfg,
        &ledger,
    )
    .await
    .expect("settled");
    assert_eq!(paid.len(), 1);
    assert_eq!(
        paid[0].statement_id,
        "supplier/airport_lounge_network/2026-12-01"
    );
    assert_eq!(paid[0].total, Money::from_major(2));
    for (account, expected) in [
        (Account::RewardExpense, 3),
        (Account::RewardsPayable, -1), // HK$1 left in the reward balance
        (Account::InKindPayable, 0),
        (Account::Cash, -2),
    ] {
        let got = ledger.balance(account).await.expect("balance");
        assert_eq!(got, Money::from_major(expected), "{account:?}");
    }

    // The same photo again: caught by fingerprint and sent to a person, never credited.
    let (profile2, trip2) = traveller();
    let trip2 = microinsurance_engine::types::UpcomingEvent {
        reference: Some("CX502/2026-12-20".into()),
        ..trip2
    };
    let again = TaskRequest {
        profile: &profile2,
        event: &trip2,
        offered_on: dec(5),
        ..request
    };
    let task = desk.propose(&again).await.expect("a second trip");
    let task = prevention::accept(task, again.offered_on).expect("accepted");
    let seen: BTreeSet<String> = [microinsurance_ai::verify::fingerprint(&photo.bytes)].into();
    let (task, credit) = desk
        .check(task, &[photo], &seen, None, again.offered_on)
        .await
        .expect("checked");
    assert!(credit.is_none());
    assert!(matches!(task.state, TaskState::WithPerson { .. }));
}

/// The claims demo on saved AI replies (ADR-010): one claim paid straight through, the others with
/// people in the right order, each with only the actions a person may take; the committed demo
/// snapshot the ops console reads is what the demo makes.
#[tokio::test]
async fn claims_jobs_on_saved_replies() {
    use microinsurance_engine::guardrails::queue::{OpsSnapshot, PersonAction, WorkKind};
    use microinsurance_workers::claims::ClaimsDesk;
    use microinsurance_workers::ops;
    use microinsurance_workers::prevention::replay_ai;
    let cfg = cfg();
    let (ai, _) =
        replay_ai(&repo(), &ConfigDir::base(&repo().join("config")), &cfg).expect("offline AI");
    let desk = ClaimsDesk {
        ai: &ai,
        cfg: &cfg,
        seed: 2026,
    };
    let ledger = MemoryLedger::new();
    let out = ops::claims_demo(&desk, &ledger, &repo(), Vec::new())
        .await
        .expect("demo");
    // R13, R57: the cracked screen was paid at the amount claimed, nothing decided by the AI.
    assert_eq!(
        ledger.balance(Account::InsurerRisk).await.expect("balance"),
        Money::from_major(1_050)
    );
    let items = &out.snapshot.items;
    assert_eq!(items.len(), 4);
    // R46: the injury first.
    assert_eq!(items[0].item_id, "claim-demo-3");
    assert!(items[0].urgent);
    for item in items {
        let declines = item.actions.contains(&PersonAction::Decline);
        assert_eq!(
            declines,
            matches!(item.work, WorkKind::ClaimReferral { .. })
        );
    }
    // R15: the declined claim is with someone other than the person who declined it.
    assert!(items.iter().any(|i| matches!(&i.work,
        WorkKind::ClaimReferral { claim_id, referral: microinsurance_engine::claims::ClaimReferral::Appeal { declined_by }, .. }
            if claim_id == "claim-demo-2" && declined_by == "handler-amy")));
    // R19: the synthetic audit store flags one group.
    assert!(out.snapshot.fairness.iter().any(|r| !r.flagged.is_empty()));
    // Every decision is logged; the AI's are marked as such.
    assert!(
        out.records
            .iter()
            .any(|r| r.actor == microinsurance_engine::guardrails::Actor::Ai)
    );
    let committed: OpsSnapshot = serde_json::from_slice(
        &std::fs::read(repo().join("apps/ops-console/demo/snapshot.json")).expect("snapshot"),
    )
    .expect("the committed snapshot matches the contract");
    assert_eq!(committed.items, out.snapshot.items);
    assert_eq!(committed.fairness, out.snapshot.fairness);
}

#[tokio::test]
async fn the_book_comes_from_the_policy_store_and_each_payout_reaches_its_customer_once() {
    use microinsurance_store::{By, Keys, Store, collections as c};
    use microinsurance_workers::book::{self, AuditRecord, PayoutNotice, PolicyRecord};
    let cfg = cfg();
    let store = Store::memory();
    for (id, paid) in [("pol-1", true), ("pol-2", false)] {
        let policy = weekday_rider(&cfg, id);
        let record = PolicyRecord {
            premium_due: policy.period.starts,
            premium_paid_on: paid.then_some(policy.period.starts),
            partner_id: None,
            quote_id: "q".into(),
            policy: policy.clone(),
        };
        store
            .insert(c::POLICIES, id, Keys::customer(&policy.customer_id), record)
            .await
            .expect("insert");
    }
    // Two days into September: pol-2's premium is unpaid, in the grace period (R53).
    let today = NaiveDate::from_ymd_opt(2026, 9, 3).expect("date");
    let loaded = book::load(&store, today, &cfg).await.expect("load");
    assert_eq!(loaded.policies.len(), 2);
    assert_eq!(loaded.premium_due.keys().collect::<Vec<_>>(), ["pol-2"]);
    let index = triggers::build_index(&loaded.policies, &cfg.catalogue);
    let suitable = BTreeSet::new();
    let book_view = Book {
        cfg: &cfg,
        policies: &loaded.policies,
        index: &index,
        premium_due: &loaded.premium_due,
        suitable: &suitable,
    };
    let mut changes = store.subscribe();
    let mut desk = TriggerDesk::default();
    let noon = poll(
        &mut desk,
        "hko_warnsum_signal8.json",
        "2026-09-14T04:00:00Z",
        &book_view,
    )
    .await;
    assert_eq!(noon.decided.len(), 2);
    for _ in 0..2 {
        book::record(
            &store,
            &noon,
            &loaded.policies,
            utc("2026-09-14T04:00:00Z"),
            2026,
            &cfg,
        )
        .await
        .expect("record");
    }
    let notices = store
        .list::<PayoutNotice>(c::PAYOUTS, By::Customer("cust-pol-2"))
        .await
        .expect("list");
    assert_eq!(notices.len(), 1, "recorded once, however often");
    let settled = notices[0]
        .body
        .settled
        .as_ref()
        .expect("cash only: settled");
    assert!(
        settled.premium_set_off > Money::ZERO,
        "net of the premium due (R53)"
    );
    let heard = changes.try_recv().expect("the customer's stream hears it");
    assert_eq!(heard.collection, c::PAYOUTS);
    let decisions = store
        .list::<serde_json::Value>(c::DECISIONS, By::Customer("cust-pol-1"))
        .await
        .expect("list");
    assert_eq!(
        decisions.len(),
        1,
        "a decision the customer can ask a person to review"
    );
    let audits = store
        .list::<AuditRecord>(c::AUDITS, By::All)
        .await
        .expect("list");
    assert!(audits.iter().all(|a| a.body.open));
}
