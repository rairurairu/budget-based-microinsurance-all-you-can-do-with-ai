//! The worker process, for the market `MARKET` names (ADR-012; none: the base market).
//!
//! `DEMO_MODE=1` replays the market's demo script (`demo`): saved feed responses against a
//! synthetic book, printing each payout and the payouts drawn for audit, then, where the script
//! says so, one prevention task from the AI's saved proposal to the supplier's statement and four
//! claims with the people's queue (with `OPS_SNAPSHOT=<path>`, written for the ops console). It
//! never calls the network.
//!
//! Otherwise it polls every source with a built adapter and a live URL, each every
//! `poll_seconds`, and runs the batch jobs from the job queue (`jobs`, `queue`). With
//! `DATABASE_URL` the book is the policy store the API keeps (read afresh for each poll), each
//! payout is recorded there for the customer, who sees it live, with its decision and any audit
//! draw (`book`, ADR-011, `AUDIT_SEED` the insurer's secret); the ledger, each source's desk state
//! and the job queue are in PostgreSQL, one worker leads each source's poll and any number share
//! the jobs; renewals rate on `RATING_BASIS`. `WORKER_POLLS=0` runs a worker for the job queue only. Without it, everything is in memory and the book is
//! empty.
use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_ai::verify::Evidence;
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::guardrails::{audit, queue};
use microinsurance_engine::money::Bps;
use microinsurance_engine::optimiser::rating::RatingBasis;
use microinsurance_engine::prevention::{self, HeldCover, PreventionBasis, TaskState};
use microinsurance_engine::rewards::{self, RewardOffer};
use microinsurance_engine::triggers::{self, Occurrence};
use microinsurance_engine::types::{UpcomingEvent, add_policy_periods};
use microinsurance_ledger::{Account, LedgerStore, MemoryLedger, PgLedger, postings};
use microinsurance_store::{PgStore, Store};
use microinsurance_triggers::TriggerSource;
use microinsurance_triggers::adapters::Adapter;
use microinsurance_workers::book;
use microinsurance_workers::claims::ClaimsDesk;
use microinsurance_workers::config_dir::ConfigDir;
use microinsurance_workers::demo::{self, DemoPart, DemoScript, money};
use microinsurance_workers::desk::{Book, PollOutcome, TriggerDesk};
use microinsurance_workers::jobs::{self, Batch};
use microinsurance_workers::ops;
use microinsurance_workers::prevention::{PreventionDesk, TaskRequest, replay_ai};
use microinsurance_workers::queue::{PgQueue, Queue};
use microinsurance_workers::sources;
use microinsurance_workers::state::{Lead, PgState};
use microinsurance_workers::suppliers;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn root() -> PathBuf {
    std::env::var("REPO_ROOT").map_or_else(|_| PathBuf::from("."), PathBuf::from)
}

/// The audit seed of demo mode. Live, the seed is the insurer's secret (R14, ADR-009).
const DEMO_AUDIT_SEED: u64 = 2026;

/// How often a worker schedules the day's jobs and claims due ones.
const JOB_TICK: Duration = Duration::from_secs(5);

/// Jobs one worker claims at a time.
const JOB_BATCH: u32 = 50;

fn utc(s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| e.to_string())
}

/// What demo mode and a live process share.
struct Market {
    config: ConfigDir,
    cfg: EngineConfig,
}

async fn demo(market: &Market, ledger: &impl LedgerStore) -> Result<(), String> {
    let cfg = &market.cfg;
    let script = DemoScript::load(&root(), &market.config)?;
    let policies = script.book(cfg)?;
    let index = triggers::build_index(&policies, &cfg.catalogue);
    let (due, suitable) = (BTreeMap::new(), BTreeSet::new());
    let book = Book {
        cfg,
        policies: &policies,
        index: &index,
        premium_due: &due,
        suitable: &suitable,
    };
    let replayed = demo::replay(&script, &market.config, &book, ledger).await?;
    for line in &replayed.lines {
        println!("{line}");
    }
    let paid = ledger
        .balance(Account::InsurerRisk)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "settled in cash on these events: {}",
        money(paid, &cfg.currency)
    );
    // R14: payouts are audited after payment; a person checks the sample.
    let decided: Vec<String> = replayed
        .decided
        .iter()
        .map(|d| d.idempotency_key.clone())
        .collect();
    let sample = audit::sample(
        decided.iter().map(|k| (k.as_str(), None)),
        DEMO_AUDIT_SEED,
        &cfg.guardrails,
    );
    println!(
        "audit   {} of {} payouts drawn for a person (R14)",
        sample.len(),
        decided.len()
    );
    if script.then.contains(&DemoPart::Prevention) {
        prevention_demo(market, ledger).await?;
    }
    if script.then.contains(&DemoPart::Claims) {
        claims_part(market, &book, ledger, &sample).await?;
    }
    Ok(())
}

/// The claims part (`ops::claims_demo`): four synthetic claims on saved AI replies, a review
/// request, the payouts drawn for audit and the fairness audit, as the ops console's queue. With
/// `OPS_SNAPSHOT=<path>` the snapshot is written there as JSON.
async fn claims_part(
    market: &Market,
    book: &Book<'_>,
    ledger: &impl LedgerStore,
    sample: &[(String, audit::AuditReason)],
) -> Result<(), String> {
    println!("-- claims and guardrails");
    let cfg = book.cfg;
    let (ai, _) = replay_ai(&root(), &market.config, cfg)?;
    let desk = ClaimsDesk {
        ai: &ai,
        cfg,
        seed: DEMO_AUDIT_SEED,
    };
    let drawn_at = utc("2026-09-16T01:00:00Z")?;
    let audits = sample
        .iter()
        .map(|(key, reason)| {
            let customer = book
                .policies
                .iter()
                .find(|p| key.contains(&format!("/{}/", p.policy_id)))
                .map_or_else(String::new, |p| p.customer_id.clone());
            queue::audit_item(key, "payout", &customer, *reason, drawn_at, cfg)
        })
        .collect();
    let out = ops::claims_demo(&desk, ledger, &root(), audits).await?;
    for line in &out.lines {
        println!("{line}");
    }
    if let Ok(path) = std::env::var("OPS_SNAPSHOT") {
        let mut text = serde_json::to_string_pretty(&out.snapshot).map_err(|e| e.to_string())?;
        text.push('\n');
        std::fs::write(&path, text).map_err(|e| format!("{path}: {e}"))?;
        println!("wrote   the ops console's snapshot to {path}");
    }
    Ok(())
}

fn fixture_json<T: serde::de::DeserializeOwned>(name: &str) -> Result<T, String> {
    let path = root().join("fixtures").join(name);
    let text = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn dec(d: u32) -> Result<NaiveDate, String> {
    NaiveDate::from_ymd_opt(2026, 12, d).ok_or_else(|| "date".to_string())
}

/// A synthetic traveller with a new phone and a December trip: the AI's saved proposal, the
/// engine's sizing, the verifier's saved verdict on a synthetic photo, the reward spent on a
/// partner reward and the supplier paid on its statement.
async fn prevention_demo(market: &Market, ledger: &impl LedgerStore) -> Result<(), String> {
    println!("-- prevention and rewards");
    let cfg = &market.cfg;
    let cash = |m| money(m, &cfg.currency);
    let (ai, vocab) = replay_ai(&root(), &market.config, cfg)?;
    let basis: PreventionBasis = fixture_json("prevention_basis_synthetic.json")?;
    let offers: Vec<RewardOffer> = fixture_json("reward_offers_synthetic.json")?;
    let profile = microinsurance_engine::types::Profile {
        customer_id: "cust-synthetic-traveller".into(),
        consents: ["photos".to_string()].into(),
        ..Default::default()
    };
    let trip = UpcomingEvent {
        kind: "trip.air".into(),
        starts: dec(20)?,
        ends: Some(dec(27)?),
        reference: Some("CX500/2026-12-20".into()),
        value: None,
    };
    let policy_id = "demo-traveller".to_string();
    let desk = PreventionDesk {
        ai: &ai,
        vocab: &vocab,
        cfg,
        basis: &basis,
        seed: DEMO_AUDIT_SEED,
    };
    let request = TaskRequest {
        profile: &profile,
        policy_id: &policy_id,
        event: &trip,
        held: HeldCover {
            cover_id: "device_damage",
            sum_insured: microinsurance_engine::money::Money::from_major(4_000),
            relativity: Bps(10_000),
        },
        offered_on: dec(1)?,
        history: &[],
        cohort: microinsurance_engine::pilot::Cohort::Offered,
    };
    let task = desk
        .propose(&request)
        .await
        .map_err(|e| format!("no prevention task: {e:?}"))?;
    println!(
        "task    {}  reward {} (claims avoided {})",
        task.task_id,
        cash(task.draft.reward_cost),
        cash(prevention::claims_avoided(&task.draft)),
    );
    println!("        {}", task.draft.action);
    let task = prevention::accept(task, dec(2)?).map_err(|e| e.to_string())?;
    let path = root().join("fixtures/evidence/screen_protector_synthetic.png");
    let photo = Evidence {
        media_type: "image/png".into(),
        bytes: std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
    };
    let (task, credit) = desk
        .check(task, &[photo], &BTreeSet::new(), None, dec(3)?)
        .await?;
    let credit = match (credit, &task.state) {
        (Some(c), _) => c,
        (None, TaskState::WithPerson { referral, .. }) => {
            println!("held    with a person ({referral:?}); the person confirms it");
            let (_, c) = prevention::person_decides(task, true, "Confirmed", dec(4)?, cfg)
                .map_err(|e| e.to_string())?;
            c.ok_or("not credited")?
        }
        (None, state) => return Err(format!("not verified: {state:?}")),
    };
    let grant = rewards::grant(credit.reward, &[], cfg);
    let (key, entries) = postings::reward_granted(&grant);
    ledger
        .post(&key, &entries)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "credit  {} points in {}; {} into the reward balance",
        credit.activity.points(),
        grant.reward.pool,
        cash(grant.granted)
    );
    let plan = cfg.plan("essential").ok_or("plan essential")?;
    let menu = rewards::menu(grant.granted, &offers, plan, cfg);
    let shown: Vec<String> = menu
        .iter()
        .map(|o| format!("{} {}", o.offer_id, cash(o.price)))
        .collect();
    println!("menu    {}", shown.join(" | "));
    let chosen = rewards::redeem(
        &profile.customer_id,
        "redeem/demo/1",
        "lounge_day_pass",
        &menu,
    )
    .map_err(|e| e.to_string())?;
    let (key, entries) = postings::reward_redeemed(&chosen);
    ledger
        .post(&key, &entries)
        .await
        .map_err(|e| e.to_string())?;
    let delivered_on = dec(20)?;
    let order = microinsurance_engine::suppliers::order_for_reward(&chosen, dec(3)?, cfg)
        .and_then(|o| microinsurance_engine::suppliers::deliver(o, delivered_on, None))
        .map_err(|e| e.to_string())?;
    let next = add_policy_periods(dec(1)?, 1);
    for s in suppliers::settle(&[order], dec(1)?, next, cfg, ledger).await? {
        println!(
            "paid    {} {} for {} delivery",
            s.statement_id,
            cash(s.total),
            s.lines.len()
        );
    }
    Ok(())
}

/// One live poll of one source: when it is next due, and, with PostgreSQL, its lead and saved
/// state.
struct Poller {
    source: Adapter,
    every: Duration,
    next: Instant,
    lead: Option<Lead>,
    desk: Option<TriggerDesk>,
}

/// What a live process needs: the market, the policy store holding the book, the audit seed
/// (R14), the job queue and the rating basis the renewal jobs rate on.
struct Live<'a> {
    market: &'a Market,
    store: Option<&'a Store>,
    seed: u64,
    queue: &'a Queue,
    basis: Option<&'a RatingBasis>,
    worker: String,
}

/// One live poll: the book read afresh from the policy store (none without one), the poll, and
/// what it decided recorded where the customer sees it (`book::record`).
async fn live_poll(
    live: &Live<'_>,
    desk: &mut TriggerDesk,
    source: &Adapter,
    ledger: &impl LedgerStore,
    state: Option<&PgState>,
) -> Result<(), String> {
    let cfg = &live.market.cfg;
    let now = Utc::now();
    let today = now.with_timezone(&cfg.local_offset()).date_naive();
    let loaded = match live.store {
        Some(store) => book::load(store, today, cfg).await?,
        None => book::Loaded {
            policies: Vec::new(),
            premium_due: BTreeMap::new(),
        },
    };
    let index = triggers::build_index(&loaded.policies, &cfg.catalogue);
    let suitable: BTreeSet<String> = cfg.suppliers.in_kind.keys().cloned().collect();
    let book = Book {
        cfg,
        policies: &loaded.policies,
        index: &index,
        premium_due: &loaded.premium_due,
        suitable: &suitable,
    };
    let reports = source.poll().await.map_err(|e| e.to_string())?;
    let outcome: PollOutcome = desk.poll(source.id(), &reports, now, &book)?;
    for d in &outcome.decided {
        println!(
            "payout  {} {:?} {}",
            d.policy_id,
            d.stage,
            money(d.amount, &cfg.currency)
        );
    }
    let key = sources::desk_key(source.id());
    // Saved before posting: a crash leaves every decision in the saved outbox.
    if let Some(s) = state {
        s.save(&key, desk).await?;
    }
    for e in desk.flush(ledger).await {
        eprintln!("ledger  {e}");
    }
    if let Some(s) = state {
        s.save(&key, desk).await?;
    }
    if let Some(store) = live.store {
        book::record(store, &outcome, &loaded.policies, now, live.seed, cfg).await?;
    }
    Ok(())
}

/// Event types and stages in force now, from every source's saved desk (R06).
async fn in_force(pollers: &[Poller], state: Option<&PgState>) -> BTreeSet<Occurrence> {
    let mut out = BTreeSet::new();
    for p in pollers {
        let saved = match (&p.desk, state) {
            (Some(_), _) | (None, None) => None,
            (None, Some(s)) => s
                .load::<TriggerDesk>(&sources::desk_key(p.source.id()))
                .await
                .ok()
                .flatten(),
        };
        if let Some(desk) = p.desk.as_ref().or(saved.as_ref()) {
            out.extend(
                desk.events
                    .values()
                    .filter(|e| !e.is_closed())
                    .map(triggers::TriggerEvent::occurrence),
            );
        }
    }
    out
}

/// Schedules today's jobs and runs those due (`jobs`), when the process has a book to run them on.
async fn run_jobs(
    live: &Live<'_>,
    pollers: &[Poller],
    ledger: &impl LedgerStore,
    state: Option<&PgState>,
) -> Result<(), String> {
    let (Some(store), Some(basis)) = (live.store, live.basis) else {
        return Ok(());
    };
    let cfg = &live.market.cfg;
    let now = Utc::now();
    for work in [jobs::day(now, cfg), jobs::accumulation(now, cfg)] {
        live.queue.enqueue(work.job(now).map_err(|e| e.0)?).await?;
    }
    let in_force = in_force(pollers, state).await;
    let batch = Batch {
        store,
        ledger,
        cfg,
        basis,
        in_force: &in_force,
        full_credibility_claims: live
            .market
            .config
            .experience_settings()?
            .full_credibility_claims,
    };
    for ran in jobs::run_due(live.queue, &live.worker, &batch, now, JOB_BATCH).await? {
        match ran.outcome {
            Ok(done) => println!("job     {done}"),
            Err(e) => eprintln!("job     {}: {e} (retried)", ran.key),
        }
    }
    Ok(())
}

async fn live(
    live: &Live<'_>,
    ledger: &impl LedgerStore,
    state: Option<&PgState>,
) -> Result<(), String> {
    let config = &live.market.config;
    let cfg = &live.market.cfg;
    let mut pollers = Vec::new();
    // `WORKER_POLLS=0`: this worker runs the job queue only; others poll the sources.
    let polls = std::env::var("WORKER_POLLS").map_or(true, |v| v != "0");
    for id in sources::built(config)?.into_iter().filter(|_| polls) {
        let Some((feed, seconds)) = sources::live_feed(config, &id)? else {
            eprintln!("skip    {id}: no live feed configured");
            continue;
        };
        pollers.push(Poller {
            source: sources::adapter(&id, feed, config, cfg)?,
            every: Duration::from_secs(seconds),
            next: Instant::now(),
            lead: None,
            desk: None,
        });
    }
    if live.basis.is_none() && live.store.is_some() {
        eprintln!("jobs    RATING_BASIS is not set: renewals cannot be rated, so no jobs run");
    }
    let mut jobs_next = Instant::now();
    loop {
        for p in &mut pollers {
            if Instant::now() < p.next {
                continue;
            }
            p.next = Instant::now() + p.every;
            let key = sources::desk_key(p.source.id());
            // One poller per source: another worker waits until this one stops.
            if let Some(s) = state
                && p.lead.is_none()
            {
                p.lead = s.try_lead(&key).await?;
                if p.lead.is_none() {
                    continue;
                }
            }
            if p.desk.is_none() {
                p.desk = Some(match state {
                    Some(s) => s.load(&key).await?.unwrap_or_default(),
                    None => TriggerDesk::default(),
                });
            }
            if let Some(desk) = p.desk.as_mut()
                && let Err(e) = live_poll(live, desk, &p.source, ledger, state).await
            {
                eprintln!("poll    {}: {e}", p.source.id());
            }
        }
        if Instant::now() >= jobs_next {
            jobs_next = Instant::now() + JOB_TICK;
            if let Err(e) = run_jobs(live, &pollers, ledger, state).await {
                eprintln!("jobs    {e}");
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// The rating basis the renewal jobs rate on (`RATING_BASIS`), when set.
fn rating_basis() -> Result<Option<RatingBasis>, String> {
    let Ok(path) = std::env::var("RATING_BASIS") else {
        return Ok(None);
    };
    let text = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    serde_json::from_slice(&text)
        .map(Some)
        .map_err(|e| format!("{path}: {e}"))
}

async fn run(
    market: &Market,
    ledger: &impl LedgerStore,
    state: Option<&PgState>,
    store: Option<&Store>,
    queue: &Queue,
    demo_mode: bool,
) -> Result<(), String> {
    if demo_mode {
        return demo(market, ledger).await;
    }
    let seed = match (std::env::var("AUDIT_SEED"), store) {
        (Ok(s), _) => s
            .parse()
            .map_err(|_| "AUDIT_SEED is a number".to_string())?,
        (Err(_), None) => DEMO_AUDIT_SEED,
        (Err(_), Some(_)) => return Err("AUDIT_SEED, the insurer's secret, is needed".into()),
    };
    let basis = rating_basis()?;
    let worker =
        std::env::var("WORKER_ID").unwrap_or_else(|_| format!("worker-{}", std::process::id()));
    live(
        &Live {
            market,
            store,
            seed,
            queue,
            basis: basis.as_ref(),
            worker,
        },
        ledger,
        state,
    )
    .await
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let demo_mode = std::env::var("DEMO_MODE").is_ok_and(|v| v == "1");
    let config = ConfigDir::from_env(&root())?;
    let market = Market {
        cfg: config.load()?,
        config,
    };
    match std::env::var("DATABASE_URL") {
        Ok(url) if !demo_mode => {
            let ledger = PgLedger::connect(&url).await.map_err(|e| e.to_string())?;
            ledger.migrate().await.map_err(|e| e.to_string())?;
            let state = PgState::connect(&url).await?;
            state.migrate().await?;
            let store = PgStore::connect(&url).await.map_err(|e| e.to_string())?;
            store.migrate().await.map_err(|e| e.to_string())?;
            let queue = PgQueue::connect(&url).await?;
            queue.migrate().await?;
            let store = Store::Pg(store);
            run(
                &market,
                &ledger,
                Some(&state),
                Some(&store),
                &Queue::Pg(queue),
                demo_mode,
            )
            .await
        }
        _ => {
            run(
                &market,
                &MemoryLedger::new(),
                None,
                None,
                &Queue::memory(),
                demo_mode,
            )
            .await
        }
    }
}
