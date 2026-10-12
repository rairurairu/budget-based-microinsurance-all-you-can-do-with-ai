//! Demo mode's synthetic world, as data: a book of synthetic customers and the saved feed
//! responses each source replays, with the time of each poll (`fixtures/demo_script.yaml`, or a
//! market's own, `fixtures/markets/<market>/demo_script.yaml`, ADR-012). Every response goes
//! through the source's adapter, its trigger desk, the engine and the ledger, as live; nothing
//! calls the network and no record is real.
use crate::config_dir::ConfigDir;
use crate::desk::{Book, TriggerDesk};
use crate::sources;
use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::money::Money;
use microinsurance_engine::payouts::PayoutDecision;
use microinsurance_engine::types::{
    Allocation, Policy, PolicyPeriod, Schedule, UpcomingEvent, WorkWindow, add_policy_periods,
};
use microinsurance_ledger::LedgerStore;
use microinsurance_triggers::{Feed, TriggerSource};
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// One synthetic customer with one cover for one policy period.
#[derive(Debug, Clone, Deserialize)]
pub struct DemoPolicy {
    pub id: String,
    #[serde(default = "essential")]
    pub plan: String,
    pub starts: NaiveDate,
    #[serde(default = "third")]
    pub period_index: u32,
    pub cover: String,
    /// In major units of the market's currency.
    pub sum_insured: i64,
    /// ISO weekdays worked (1 is Monday).
    #[serde(default)]
    pub days: Vec<u8>,
    /// Working hours, local, `HH:MM-HH:MM`; past midnight when the end is earlier.
    #[serde(default)]
    pub hours: Option<String>,
    #[serde(default)]
    pub routes: Vec<String>,
    #[serde(default)]
    pub areas: Vec<String>,
    #[serde(default)]
    pub tickets: Vec<UpcomingEvent>,
}

fn essential() -> String {
    "essential".into()
}

fn third() -> u32 {
    3
}

/// One saved response, polled at a moment.
#[derive(Debug, Clone, Deserialize)]
pub struct DemoStep {
    /// Relative to the script's directory.
    pub file: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DemoPolls {
    pub source: String,
    pub steps: Vec<DemoStep>,
}

/// What demo mode runs after the polls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DemoPart {
    /// A prevention task from the AI's saved proposal to the supplier's statement.
    Prevention,
    /// Claims on saved AI replies and the people's queue.
    Claims,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DemoScript {
    pub book: Vec<DemoPolicy>,
    pub polls: Vec<DemoPolls>,
    #[serde(default)]
    pub then: Vec<DemoPart>,
    /// The script's directory, where its responses are.
    #[serde(skip)]
    pub dir: PathBuf,
}

fn minutes(hhmm: &str) -> Result<u16, String> {
    let (h, m) = hhmm
        .split_once(':')
        .ok_or_else(|| format!("{hhmm:?}: HH:MM"))?;
    let h: u16 = h.trim().parse().map_err(|_| format!("{hhmm:?}: hour"))?;
    let m: u16 = m.trim().parse().map_err(|_| format!("{hhmm:?}: minute"))?;
    Ok(h * 60 + m)
}

impl DemoScript {
    /// The market's script under `root/fixtures`.
    pub fn load(root: &Path, config: &ConfigDir) -> Result<DemoScript, String> {
        let dir = match config.market() {
            Some(m) => root.join("fixtures").join("markets").join(m),
            None => root.join("fixtures"),
        };
        let path = dir.join("demo_script.yaml");
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut script: DemoScript =
            serde_saphyr::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        script.dir = dir;
        Ok(script)
    }

    /// The synthetic book.
    pub fn book(&self, cfg: &EngineConfig) -> Result<Vec<Policy>, String> {
        self.book
            .iter()
            .map(|c| {
                let plan = cfg
                    .plan(&c.plan)
                    .ok_or_else(|| format!("{}: plan {}", c.id, c.plan))?
                    .clone();
                let work_windows = match &c.hours {
                    None => Vec::new(),
                    Some(h) => {
                        let (from, to) = h
                            .split_once('-')
                            .ok_or_else(|| format!("{}: hours {h:?}", c.id))?;
                        let (start_minute, end_minute) = (minutes(from)?, minutes(to)?);
                        c.days
                            .iter()
                            .map(|d| WorkWindow {
                                weekday: *d,
                                start_minute,
                                end_minute,
                            })
                            .collect()
                    }
                };
                Ok(Policy {
                    policy_id: c.id.clone(),
                    customer_id: format!("cust-{}", c.id),
                    schedule: Schedule {
                        plan_id: plan.id.clone(),
                        allocations: vec![Allocation {
                            cover_id: c.cover.clone(),
                            premium: plan.price,
                            sum_insured: Money::from_major(c.sum_insured),
                        }],
                    },
                    plan,
                    period: PolicyPeriod {
                        index: c.period_index,
                        starts: c.starts,
                        ends: add_policy_periods(c.starts, 1),
                    },
                    cover_starts: [(c.cover.clone(), c.starts)].into(),
                    work_windows,
                    commute_routes: c.routes.clone(),
                    work_areas: c.areas.clone(),
                    upcoming_events: c.tickets.clone(),
                    subscribed_at: c
                        .starts
                        .and_hms_opt(0, 0, 0)
                        .map(|t| t.and_utc())
                        .unwrap_or_default(),
                })
            })
            .collect()
    }
}

/// Money as the console shows it: the market's currency code and the amount.
pub fn money(m: Money, currency: &str) -> String {
    let sign = if m.0 < 0 { "-" } else { "" };
    let a = m.0.unsigned_abs();
    format!("{currency} {sign}{}.{:02}", a / 100, a % 100)
}

/// What replaying the polls decided.
#[derive(Debug, Clone, Default)]
pub struct Replayed {
    pub decided: Vec<PayoutDecision>,
    /// What each poll did, as the console shows it.
    pub lines: Vec<String>,
}

/// Replays every source's saved responses through its adapter and its own trigger desk, posting
/// each payout to the ledger.
pub async fn replay(
    script: &DemoScript,
    config: &ConfigDir,
    book: &Book<'_>,
    ledger: &impl LedgerStore,
) -> Result<Replayed, String> {
    let mut out = Replayed::default();
    let currency = &book.cfg.currency;
    for polls in &script.polls {
        out.lines.push(format!("-- {}", polls.source));
        let mut desk = TriggerDesk::default();
        for step in &polls.steps {
            let feed = Feed::Fixture(script.dir.join(&step.file));
            let source = sources::adapter(&polls.source, feed, config, book.cfg)?;
            let reports = source.poll().await.map_err(|e| e.to_string())?;
            let outcome = desk.poll(source.id(), &reports, step.at, book)?;
            for id in &outcome.tracked.opened {
                out.lines.push(format!("opened  {id}"));
            }
            for id in &outcome.tracked.closed {
                out.lines.push(format!("closed  {id}"));
            }
            for d in &outcome.decided {
                out.lines.push(format!(
                    "payout  {:<26} {:<10} {}{}",
                    d.policy_id,
                    format!("{:?}", d.stage),
                    money(d.amount, currency),
                    if d.options.len() > 1 {
                        "  (cash or an alternative: the customer chooses)"
                    } else {
                        ""
                    }
                ));
            }
            for e in desk.flush(ledger).await {
                out.lines.push(format!("ledger  {e}"));
            }
            out.decided.extend(outcome.decided);
        }
    }
    Ok(out)
}
