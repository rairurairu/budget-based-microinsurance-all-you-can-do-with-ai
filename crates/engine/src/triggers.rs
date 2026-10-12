//! M5 Trigger matching (pure). Adapters in crates/triggers report each feed's own codes
//! (`SourceReport`); `classify` resolves them to registered event types and stages (R55), and
//! `track` folds successive polls into trigger events, so covers never depend on one source.
//! `group` gathers a peril's trigger events into loss occurrences (R51), and `match_occurrence`
//! works out what each policy is owed on one: on the trigger during declared working hours, not
//! on proven loss (R08). Local time is the market's fixed UTC offset (ADR-006).
//!
//! A loss occurrence attaches to the policy period in force when it starts, and pays a cover only
//! if that cover was in force, past its waiting period, on the local day it started.
use crate::config::{EngineConfig, ZoneUnit};
use crate::money::Money;
use crate::types::{
    CoverId, CoverSpec, EventCondition, EventTypeId, MatchKey, Measure, PayoutBasis, Policy,
    PolicyId, UpcomingEvent, WorkWindow,
};
use chrono::{DateTime, Datelike, Days, Duration, FixedOffset, NaiveDate, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One report from a data source, in the source's own terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceReport {
    /// The source's own id for the report, kept for audit.
    pub report_id: String,
    /// A registered data source id (`config/triggers/*.yaml`).
    pub source: String,
    /// The source's code, for example TC8NE, WRAINB, RAIL_SUSPENDED, CANCELLED.
    pub code: String,
    pub starts: DateTime<Utc>,
    /// Set when the source reports the end, for example a cancelled warning.
    #[serde(default)]
    pub ends: Option<DateTime<Utc>>,
    /// Where it applies, in the event type's zone unit (line, flight, event); none for
    /// territory-wide events.
    #[serde(default)]
    pub zone: Option<String>,
    /// Minutes late, for events measured as a delay.
    #[serde(default)]
    pub delay_minutes: Option<u32>,
    /// Ticket or booking reference, when the source reports per customer.
    #[serde(default)]
    pub reference: Option<String>,
}

/// A source report resolved to a registered event type and stage.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Occurrence {
    pub event_type: EventTypeId,
    /// Stage rank in the event type's order, 0 = least severe.
    pub stage: u8,
    pub zone: Option<String>,
}

/// R55: the registered event type and stage a source report stands for; `None` for a source or
/// code that is not registered (ignored, never guessed).
pub fn classify(report: &SourceReport, cfg: &EngineConfig) -> Option<Occurrence> {
    let code = cfg
        .triggers
        .sources
        .get(&report.source)?
        .codes
        .get(&report.code)?;
    Some(Occurrence {
        event_type: code.event_type.clone(),
        stage: code.stage,
        zone: report.zone.clone(),
    })
}

/// True when an occurrence meets a condition's event type and stage. Minutes in force or late
/// are checked by `match_occurrence`.
pub fn meets(occurrence: &Occurrence, condition: &EventCondition) -> bool {
    occurrence.event_type == condition.event_type && occurrence.stage >= condition.from_stage
}

/// A stage reached at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageChange {
    pub at: DateTime<Utc>,
    pub stage: u8,
}

/// One trigger event: an event type in force in one zone (and, for a per-customer report, one
/// reference) from its first report until its source stops reporting it. A typhoon raised from
/// Signal 8 to Signal 9 and lowered again is one trigger event. Payouts and the event limit are
/// per loss occurrence, which groups a peril's trigger events (R27, R51).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerEvent {
    /// Event type, zone, reference and start: the same event always gets the same id.
    pub event_id: String,
    pub event_type: EventTypeId,
    pub zone: Option<String>,
    pub reference: Option<String>,
    /// The source that opened it; only that source closes it.
    pub source: String,
    /// Stage changes in time order; the first is the start.
    pub stages: Vec<StageChange>,
    /// `None` while the event is open.
    pub ends: Option<DateTime<Utc>>,
    /// Most minutes late reported, for events measured as a delay.
    pub delay_minutes: Option<u32>,
}

impl TriggerEvent {
    pub fn starts(&self) -> DateTime<Utc> {
        self.stages
            .first()
            .map_or(DateTime::<Utc>::MIN_UTC, |s| s.at)
    }

    pub fn is_closed(&self) -> bool {
        self.ends.is_some()
    }

    /// The highest stage reached.
    pub fn peak(&self) -> u8 {
        self.stages.iter().map(|s| s.stage).max().unwrap_or(0)
    }

    /// Where the event is now, for new-cover checks (R06).
    pub fn occurrence(&self) -> Occurrence {
        Occurrence {
            event_type: self.event_type.clone(),
            stage: self.stages.last().map_or(0, |s| s.stage),
            zone: self.zone.clone(),
        }
    }

    /// The stretches spent at `from_stage` or above, up to the end or, while open, `as_of`.
    fn spells(&self, from_stage: u8, as_of: DateTime<Utc>) -> Vec<Spell> {
        let end = self.ends.unwrap_or(as_of);
        self.stages
            .iter()
            .enumerate()
            .filter(|(_, s)| s.stage >= from_stage)
            .filter_map(|(i, s)| {
                let to = self.stages.get(i + 1).map_or(end, |n| n.at).min(end);
                (to > s.at).then_some(Spell {
                    from: s.at,
                    to,
                    stage: s.stage,
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
struct Spell {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    stage: u8,
}

type EventKey = (EventTypeId, Option<String>, Option<String>);

fn key_of(e: &TriggerEvent) -> EventKey {
    (e.event_type.clone(), e.zone.clone(), e.reference.clone())
}

fn event_id(key: &EventKey, starts: DateTime<Utc>) -> String {
    format!(
        "{}:{}:{}:{}",
        key.0,
        key.1.as_deref().unwrap_or("all"),
        key.2.as_deref().unwrap_or("-"),
        starts.format("%Y%m%dT%H%M%SZ")
    )
}

/// What one poll of a source changed, by event id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tracked {
    pub opened: Vec<String>,
    pub changed: Vec<String>,
    pub closed: Vec<String>,
}

/// One key's reports in a poll: the highest stage, when it was reached, and whether it ended.
struct Seen {
    stage: u8,
    stage_from: DateTime<Utc>,
    starts: DateTime<Utc>,
    ends: Option<DateTime<Utc>>,
    delay: Option<u32>,
}

/// Folds one poll of `source` into `events`. `reports` is everything the source reports now:
/// an open event of this source that is no longer reported closes at `now`, and a report with an
/// end at or before `now` closes its event then. Codes not registered are ignored (R55). Closed
/// events stay in `events` until the caller removes them after paying the balance (R51).
pub fn track(
    events: &mut BTreeMap<String, TriggerEvent>,
    source: &str,
    reports: &[SourceReport],
    now: DateTime<Utc>,
    cfg: &EngineConfig,
) -> Tracked {
    let mut seen: BTreeMap<EventKey, Seen> = BTreeMap::new();
    for r in reports.iter().filter(|r| r.source == source) {
        let Some(o) = classify(r, cfg) else {
            continue;
        };
        // An event in force starts when it starts: one announced for later (a work suspension
        // from tomorrow) is not in force until then. A delay estimated ahead counts now.
        let measure = cfg
            .triggers
            .event_types
            .get(&o.event_type)
            .map(|t| t.measure);
        if measure == Some(Measure::InForce) && r.starts > now {
            continue;
        }
        let starts = r.starts.min(now);
        let s = seen
            .entry((o.event_type, o.zone, r.reference.clone()))
            .or_insert(Seen {
                stage: o.stage,
                stage_from: starts,
                starts,
                ends: r.ends,
                delay: r.delay_minutes,
            });
        if o.stage > s.stage {
            s.stage = o.stage;
            s.stage_from = starts;
        }
        s.starts = s.starts.min(starts);
        s.ends = match (s.ends, r.ends) {
            (Some(a), Some(b)) => Some(a.max(b)),
            _ => None,
        };
        s.delay = s.delay.max(r.delay_minutes);
    }

    let open: BTreeMap<EventKey, String> = events
        .values()
        .filter(|e| e.source == source && !e.is_closed())
        .map(|e| (key_of(e), e.event_id.clone()))
        .collect();
    let mut out = Tracked::default();
    for (key, s) in &seen {
        let ends = s.ends.filter(|e| *e <= now);
        if let Some(e) = open.get(key).and_then(|id| events.get_mut(id)) {
            let mut changed = false;
            if e.stages.last().is_some_and(|l| l.stage != s.stage) {
                let last = e.stages.last().map_or(e.starts(), |l| l.at);
                let at = if s.stage_from > last {
                    s.stage_from
                } else {
                    now
                };
                e.stages.push(StageChange { at, stage: s.stage });
                changed = true;
            }
            if s.delay > e.delay_minutes {
                e.delay_minutes = s.delay;
                changed = true;
            }
            if let Some(end) = ends {
                e.ends = Some(end.max(e.starts()));
                out.closed.push(e.event_id.clone());
            } else if changed {
                out.changed.push(e.event_id.clone());
            }
            continue;
        }
        // A report of an event already recorded and closed (a cancelled warning still listed).
        let recorded = events.values().any(|e| {
            e.source == source
                && key_of(e) == *key
                && e.ends
                    .is_some_and(|end| e.starts() <= s.starts && s.starts <= end)
        });
        if recorded {
            continue;
        }
        let id = event_id(key, s.starts);
        events.insert(
            id.clone(),
            TriggerEvent {
                event_id: id.clone(),
                event_type: key.0.clone(),
                zone: key.1.clone(),
                reference: key.2.clone(),
                source: source.to_string(),
                stages: vec![StageChange {
                    at: s.starts,
                    stage: s.stage,
                }],
                ends: ends.map(|e| e.max(s.starts)),
                delay_minutes: s.delay,
            },
        );
        out.opened.push(id.clone());
        if ends.is_some() {
            out.closed.push(id);
        }
    }
    for (key, id) in &open {
        if !seen.contains_key(key)
            && let Some(e) = events.get_mut(id)
        {
            e.ends = Some(now.max(e.starts()));
            out.closed.push(id.clone());
        }
    }
    out
}

/// Trigger events from a source's historical record, for trigger pricing and backtests. Each
/// report must carry its end; reports of one event type and zone that touch or overlap form one
/// event, its stage changing at each report's start. Reports without an end are skipped.
pub fn from_history(reports: &[SourceReport], cfg: &EngineConfig) -> Vec<TriggerEvent> {
    let mut by_key: BTreeMap<EventKey, Vec<Spell>> = BTreeMap::new();
    for r in reports {
        let (Some(o), Some(ends)) = (classify(r, cfg), r.ends) else {
            continue;
        };
        if ends > r.starts {
            by_key
                .entry((o.event_type, o.zone, r.reference.clone()))
                .or_default()
                .push(Spell {
                    from: r.starts,
                    to: ends,
                    stage: o.stage,
                });
        }
    }
    let mut out = Vec::new();
    for (key, mut spans) in by_key {
        spans.sort_by_key(|s| (s.from, s.to, s.stage));
        let mut current: Option<TriggerEvent> = None;
        for Spell {
            from: starts,
            to: ends,
            stage,
        } in spans
        {
            match current.as_mut() {
                Some(e) if e.ends.is_some_and(|end| starts <= end) => {
                    if e.stages.last().is_some_and(|l| l.stage != stage) {
                        e.stages.push(StageChange { at: starts, stage });
                    }
                    e.ends = e.ends.max(Some(ends));
                }
                _ => {
                    if let Some(done) = current.take() {
                        out.push(done);
                    }
                    current = Some(TriggerEvent {
                        event_id: event_id(&key, starts),
                        event_type: key.0.clone(),
                        zone: key.1.clone(),
                        reference: key.2.clone(),
                        source: String::new(),
                        stages: vec![StageChange { at: starts, stage }],
                        ends: Some(ends),
                        delay_minutes: None,
                    });
                }
            }
        }
        out.extend(current);
    }
    out.sort_by(|a, b| (a.starts(), &a.event_id).cmp(&(b.starts(), &b.event_id)));
    out
}

/// A loss occurrence (R51): the trigger events of one peril in one zone (and reference), or in
/// every zone for a peril counted `across_zones` (its `zone` is then none), each starting within the peril's `aggregation_hours` of the last one ending. A typhoon's signals and
/// the black rain it brings are one occurrence: one event limit, one daily benefit per day. It
/// attaches to the policy period in force when it starts and closes once `aggregation_hours`
/// pass with nothing in force; the balance is then due.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LossOccurrence {
    /// Peril, zone, reference and start: the same occurrence always gets the same id.
    pub occurrence_id: String,
    pub peril: String,
    /// None for a territory-wide event, or a peril counted across zones: its events keep theirs.
    pub zone: Option<String>,
    pub reference: Option<String>,
    /// Member trigger event ids, in start order.
    pub events: Vec<String>,
    pub starts: DateTime<Utc>,
    /// When the last member ended; `None` while one is in force.
    pub quiet_since: Option<DateTime<Utc>>,
    pub closed: bool,
}

/// What one grouping pass changed, by occurrence id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grouped {
    pub opened: Vec<String>,
    pub closed: Vec<String>,
}

fn window(peril: &str, cfg: &EngineConfig) -> Duration {
    Duration::hours(
        cfg.accumulation
            .perils
            .get(peril)
            .map_or(0, |p| i64::from(p.aggregation_hours)),
    )
}

/// Adds every trigger event not yet in an occurrence to the open occurrence of its peril, zone
/// and reference, or opens one; closes occurrences quiet for their peril's `aggregation_hours`
/// at `now`. Events of a type in no peril pay nothing and are left out.
pub fn group(
    occurrences: &mut BTreeMap<String, LossOccurrence>,
    events: &BTreeMap<String, TriggerEvent>,
    now: DateTime<Utc>,
    cfg: &EngineConfig,
) -> Grouped {
    let members: BTreeSet<String> = occurrences
        .values()
        .flat_map(|o| o.events.iter().cloned())
        .collect();
    let mut fresh: Vec<&TriggerEvent> = events
        .values()
        .filter(|e| !members.contains(&e.event_id))
        .collect();
    fresh.sort_by(|a, b| (a.starts(), &a.event_id).cmp(&(b.starts(), &b.event_id)));
    let mut out = Grouped::default();
    for e in fresh {
        let Some(peril) = cfg.accumulation.peril_of(&e.event_type) else {
            continue;
        };
        let gap = window(&peril.id, cfg);
        // A peril counted across zones is one occurrence for the whole territory (ADR-012).
        let zone = if peril.across_zones {
            None
        } else {
            e.zone.clone()
        };
        let joined = occurrences.values_mut().find(|o| {
            !o.closed
                && o.peril == peril.id
                && o.zone == zone
                && o.reference == e.reference
                && o.quiet_since.is_none_or(|q| e.starts() <= q + gap)
        });
        match joined {
            Some(o) => {
                o.events.push(e.event_id.clone());
                o.quiet_since = match (o.quiet_since, e.ends) {
                    (Some(q), Some(end)) => Some(q.max(end)),
                    _ => None,
                };
            }
            None => {
                let id = format!(
                    "{}:{}:{}:{}",
                    peril.id,
                    zone.as_deref().unwrap_or("all"),
                    e.reference.as_deref().unwrap_or("-"),
                    e.starts().format("%Y%m%dT%H%M%SZ")
                );
                occurrences.insert(
                    id.clone(),
                    LossOccurrence {
                        occurrence_id: id.clone(),
                        peril: peril.id.clone(),
                        zone,
                        reference: e.reference.clone(),
                        events: vec![e.event_id.clone()],
                        starts: e.starts(),
                        quiet_since: e.ends,
                        closed: false,
                    },
                );
                out.opened.push(id);
            }
        }
    }
    for o in occurrences.values_mut().filter(|o| !o.closed) {
        let ends: Vec<Option<DateTime<Utc>>> = o
            .events
            .iter()
            .filter_map(|id| events.get(id))
            .map(|e| e.ends)
            .collect();
        o.quiet_since = ends
            .iter()
            .copied()
            .collect::<Option<Vec<_>>>()
            .and_then(|e| e.into_iter().max());
        if o.quiet_since
            .is_some_and(|q| now >= q + window(&o.peril, cfg))
        {
            o.closed = true;
            out.closed.push(o.occurrence_id.clone());
        }
    }
    out
}

/// The loss occurrences of a historical record (`from_history`), all closed.
pub fn group_history(events: &[TriggerEvent], cfg: &EngineConfig) -> Vec<LossOccurrence> {
    let by_id: BTreeMap<String, TriggerEvent> = events
        .iter()
        .map(|e| (e.event_id.clone(), e.clone()))
        .collect();
    let last = events
        .iter()
        .filter_map(|e| e.ends)
        .max()
        .unwrap_or(DateTime::<Utc>::MIN_UTC);
    let longest = cfg
        .accumulation
        .perils
        .values()
        .map(|p| p.aggregation_hours)
        .max()
        .unwrap_or(0);
    let mut occurrences = BTreeMap::new();
    group(
        &mut occurrences,
        &by_id,
        last + Duration::hours(i64::from(longest) + 1),
        cfg,
    );
    let mut out: Vec<LossOccurrence> = occurrences.into_values().collect();
    out.sort_by(|a, b| (a.starts, &a.occurrence_id).cmp(&(b.starts, &b.occurrence_id)));
    out
}

/// What one cover of one policy is owed on a loss occurrence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Match {
    pub policy_id: PolicyId,
    pub cover_id: CoverId,
    pub occurrence_id: String,
    /// Owed on the occurrence so far, before the event limit (R51); zero until it qualifies.
    pub entitlement: Money,
    /// The most the occurrence can owe at once: one day's, one event's or `max_hours` hours'
    /// benefit. Summed over policies, the benefit in force for the occurrence (R51).
    pub benefit: Money,
}

/// Active policies indexed by event type, so a Signal 8 touches only the covers that pay on
/// tropical cyclones.
#[derive(Debug, Clone, Default)]
pub struct PolicyIndex {
    pub by_event_type: BTreeMap<EventTypeId, Vec<usize>>,
}

pub fn build_index(policies: &[Policy], catalogue: &BTreeMap<CoverId, CoverSpec>) -> PolicyIndex {
    let mut index = PolicyIndex::default();
    for (i, p) in policies.iter().enumerate() {
        let types: BTreeSet<&EventTypeId> = p
            .schedule
            .allocations
            .iter()
            .filter(|a| a.sum_insured > Money::ZERO)
            .filter_map(|a| catalogue.get(&a.cover_id)?.trigger.as_ref())
            .flat_map(|t| t.events.iter().map(|c| &c.event_type))
            .collect();
        for t in types {
            index.by_event_type.entry(t.clone()).or_default().push(i);
        }
    }
    index
}

/// The time a cover's condition holds on one trigger event: the stretches at its stage or above
/// once the event qualifies (minutes late, or its longest unbroken stretch in force reaches
/// `min_minutes`), each unbroken stretch followed by `resumption` minutes once it has ended.
fn condition_time(
    e: &TriggerEvent,
    c: &EventCondition,
    measure: Option<Measure>,
    resumption: u32,
    as_of: DateTime<Utc>,
) -> Vec<Spell> {
    let spells = e.spells(c.from_stage, as_of);
    let runs = runs(&spells);
    let qualified = match (measure, c.min_minutes) {
        (Some(Measure::Delay), min) => e.delay_minutes.unwrap_or(0) >= min.unwrap_or(0),
        (_, Some(min)) => runs
            .iter()
            .any(|r| (r.1 - r.0).num_minutes() >= i64::from(min)),
        (_, None) => true,
    };
    if !qualified {
        return Vec::new();
    }
    let mut out = spells;
    if resumption > 0 {
        let cap = if e.is_closed() {
            DateTime::<Utc>::MAX_UTC
        } else {
            as_of
        };
        for (_, to, stage) in runs {
            let ended = e.is_closed() || to < as_of;
            let until = (to + Duration::minutes(i64::from(resumption))).min(cap);
            if ended && until > to {
                out.push(Spell {
                    from: to,
                    to: until,
                    stage,
                });
            }
        }
    }
    out
}

/// Unbroken stretches of consecutive spells: (from, to, stage at the end).
fn runs(spells: &[Spell]) -> Vec<(DateTime<Utc>, DateTime<Utc>, u8)> {
    let mut out: Vec<(DateTime<Utc>, DateTime<Utc>, u8)> = Vec::new();
    for s in spells {
        match out.last_mut() {
            Some(last) if last.1 == s.from => {
                last.1 = s.to;
                last.2 = s.stage;
            }
            _ => out.push((s.from, s.to, s.stage)),
        }
    }
    out
}

/// Overlapping spells as one timeline: each moment once, at the highest stage covering it.
fn flatten(spells: &[Spell]) -> Vec<Spell> {
    let mut points: Vec<DateTime<Utc>> = spells.iter().flat_map(|s| [s.from, s.to]).collect();
    points.sort();
    points.dedup();
    let mut out: Vec<Spell> = Vec::new();
    for w in points.windows(2) {
        let [from, to] = [w[0], w[1]];
        let Some(stage) = spells
            .iter()
            .filter(|s| s.from <= from && to <= s.to)
            .map(|s| s.stage)
            .max()
        else {
            continue;
        };
        match out.last_mut() {
            Some(last) if last.to == from && last.stage == stage => last.to = to,
            _ => out.push(Spell { from, to, stage }),
        }
    }
    out
}

/// R08, R55: what every policy holding a cover that pays on the occurrence's peril is owed on it
/// as of `as_of` (the end, once closed). Returns each eligible cover, with zero entitlement
/// while it has not yet qualified, so the sum of `benefit` is the benefit in force (R51).
/// Eligible: the occurrence started in the policy period, the cover was in force and past its
/// waiting period on the local day it started (R07, R47), and the customer's declaration names
/// the occurrence's zone. Owed: for the time its conditions held across the occurrence's events,
/// each moment and each day counted once; with declared hours, only days with at least
/// `min_disrupted_minutes` of declared work disrupted, resumption time included.
pub fn match_occurrence(
    occurrence: &LossOccurrence,
    events: &BTreeMap<String, TriggerEvent>,
    policies: &[Policy],
    index: &PolicyIndex,
    as_of: DateTime<Utc>,
    cfg: &EngineConfig,
) -> Vec<Match> {
    let members: Vec<&TriggerEvent> = occurrence
        .events
        .iter()
        .filter_map(|id| events.get(id))
        .collect();
    let types: BTreeSet<&EventTypeId> = members.iter().map(|e| &e.event_type).collect();
    let ids: BTreeSet<usize> = types
        .iter()
        .filter_map(|t| index.by_event_type.get(*t))
        .flatten()
        .copied()
        .collect();
    let offset = cfg.local_offset();
    let start_day = occurrence.starts.with_timezone(&offset).date_naive();
    // Each condition's time is the same for every policy (in a zone, for an occurrence counted
    // across zones): worked out once per occurrence.
    let mut times: BTreeMap<TimeKey, Vec<Spell>> = BTreeMap::new();
    let mut out = Vec::new();
    for policy in ids.iter().filter_map(|i| policies.get(*i)) {
        if start_day < policy.period.starts || start_day >= policy.period.ends {
            continue;
        }
        for alloc in &policy.schedule.allocations {
            if alloc.sum_insured <= Money::ZERO {
                continue;
            }
            let Some(cover) = cfg.catalogue.get(&alloc.cover_id) else {
                continue;
            };
            let (Some(trigger), Some(payout)) = (&cover.trigger, &cover.payout) else {
                continue;
            };
            if !trigger.events.iter().any(|c| types.contains(&c.event_type)) {
                continue;
            }
            let in_force_from = policy
                .cover_starts
                .get(&cover.id)
                .and_then(|d| d.checked_add_days(Days::new(u64::from(cover.waiting_period_days))));
            if in_force_from.is_none_or(|d| d > start_day) {
                continue;
            }
            let Some(declared) = declaration(policy, trigger.match_on, occurrence) else {
                continue;
            };
            let si = alloc.sum_insured;
            let value = declared.and_then(|u| u.value);
            let benefit = match payout.basis {
                PayoutBasis::PerHour => times_n(si, i64::from(payout.max_hours.unwrap_or(0))),
                PayoutBasis::DeclaredValue => value.map_or(si, |v| v.min(si)),
                _ => si,
            };
            let resumption = if trigger.requires_declared_hours {
                trigger.resumption_minutes
            } else {
                0
            };
            // Counted across zones, a customer is owed only for the events in their zones.
            let zones = match (trigger.match_on, &occurrence.zone) {
                (Some(k @ (MatchKey::DeclaredRoute | MatchKey::DeclaredArea)), None) => {
                    Some(declared_zones(policy, k))
                }
                _ => None,
            };
            let at = Timing {
                members: &members,
                resumption,
                as_of,
                cfg,
            };
            let mut spells = Vec::new();
            for c in trigger
                .events
                .iter()
                .filter(|c| types.contains(&c.event_type))
            {
                match zones {
                    None => spells.extend_from_slice(at.time(&mut times, c, None)),
                    Some(zones) => {
                        for zone in zones {
                            spells.extend_from_slice(at.time(&mut times, c, Some(zone)));
                        }
                    }
                }
            }
            let spells = flatten(&spells);
            let mut cover_time = if trigger.requires_declared_hours {
                work_overlap(&spells, &policy.work_windows, offset)
            } else {
                days_touched(&spells, offset)
            };
            if trigger.requires_declared_hours {
                let least = i64::from(trigger.min_disrupted_minutes.max(1));
                cover_time.retain(|_, (minutes, _)| *minutes >= least);
            }
            out.push(Match {
                policy_id: policy.policy_id.clone(),
                cover_id: cover.id.clone(),
                occurrence_id: occurrence.occurrence_id.clone(),
                entitlement: owed(payout, si, benefit, &cover_time),
                benefit,
            });
        }
    }
    out
}

/// The benefit in force for an occurrence: the sum of what it can owe at once (R51).
pub fn benefit_in_force(matches: &[Match]) -> Money {
    Money(
        matches
            .iter()
            .fold(0_i64, |sum, m| sum.saturating_add(m.benefit.0)),
    )
}

/// A condition's time over an occurrence's events, cached by condition and zone.
type TimeKey = (EventTypeId, u8, Option<u32>, u32, Option<String>);

struct Timing<'a> {
    members: &'a [&'a TriggerEvent],
    resumption: u32,
    as_of: DateTime<Utc>,
    cfg: &'a EngineConfig,
}

impl Timing<'_> {
    /// The time `c` holds over the members, or over those in `zone` only.
    fn time<'t>(
        &self,
        times: &'t mut BTreeMap<TimeKey, Vec<Spell>>,
        c: &EventCondition,
        zone: Option<&String>,
    ) -> &'t [Spell] {
        let key = (
            c.event_type.clone(),
            c.from_stage,
            c.min_minutes,
            self.resumption,
            zone.cloned(),
        );
        times.entry(key).or_insert_with(|| {
            let measure = self
                .cfg
                .triggers
                .event_types
                .get(&c.event_type)
                .map(|t| t.measure);
            self.members
                .iter()
                .filter(|e| e.event_type == c.event_type)
                .filter(|e| zone.is_none_or(|z| e.zone.as_ref() == Some(z)))
                .flat_map(|e| condition_time(e, c, measure, self.resumption, self.as_of))
                .collect()
        })
    }
}

/// The zones a customer declared for a zone-matched cover: routes, or work areas.
fn declared_zones(policy: &Policy, key: MatchKey) -> &[String] {
    match key {
        MatchKey::DeclaredArea => &policy.work_areas,
        _ => &policy.commute_routes,
    }
}

/// The customer's declaration that names the occurrence's zone: `Some(None)` for a
/// territory-wide, route- or area-matched one, `Some(Some(ticket or booking))` for a
/// reference-matched one, `None` when nothing declared matches. For an occurrence counted across
/// zones, everyone who declared a zone is exposed to it (its benefit in force, R51); what each is
/// owed counts only the events in their own zones.
fn declaration<'a>(
    policy: &'a Policy,
    key: Option<MatchKey>,
    occurrence: &LossOccurrence,
) -> Option<Option<&'a UpcomingEvent>> {
    match key {
        None => Some(None),
        Some(k @ (MatchKey::DeclaredRoute | MatchKey::DeclaredArea)) => {
            let declared = declared_zones(policy, k);
            match &occurrence.zone {
                Some(zone) => declared.contains(zone).then_some(None),
                None => (!declared.is_empty()).then_some(None),
            }
        }
        Some(MatchKey::TicketReference | MatchKey::DeclaredBooking) => policy
            .upcoming_events
            .iter()
            .find(|u| {
                u.reference.is_some()
                    && (u.reference == occurrence.zone || u.reference == occurrence.reference)
            })
            .map(Some),
    }
}

/// Owed for the time the cover's conditions held, by payout basis.
fn owed(
    payout: &crate::types::PayoutSpec,
    si: Money,
    benefit: Money,
    cover_time: &BTreeMap<NaiveDate, (i64, u8)>,
) -> Money {
    if cover_time.is_empty() {
        return Money::ZERO;
    }
    match payout.basis {
        PayoutBasis::FixedDaily => times_n(si, i64::try_from(cover_time.len()).unwrap_or(i64::MAX)),
        PayoutBasis::FixedPerEvent | PayoutBasis::DeclaredValue => benefit,
        PayoutBasis::TieredByStage => {
            let peak = cover_time.values().map(|(_, s)| *s).max().unwrap_or(0);
            payout
                .tiers
                .range(..=peak)
                .next_back()
                .map_or(Money::ZERO, |(_, share)| si.mul_bps(*share))
        }
        PayoutBasis::PerHour => {
            // Pro rata to the minute, down to the cent, up to `max_hours`.
            let minutes: i64 = cover_time.values().map(|(m, _)| m).sum();
            let cap = i64::from(payout.max_hours.unwrap_or(0)) * 60;
            let v = i128::from(si.0) * i128::from(minutes.min(cap)) / 60;
            Money(i64::try_from(v).unwrap_or(i64::MAX))
        }
    }
}

fn times_n(amount: Money, n: i64) -> Money {
    Money(amount.0.saturating_mul(n))
}

fn local(t: DateTime<Utc>, offset: FixedOffset) -> NaiveDateTime {
    t.with_timezone(&offset).naive_local()
}

fn add(day: NaiveDate, minutes: u16) -> NaiveDateTime {
    day.and_time(chrono::NaiveTime::MIN) + Duration::minutes(i64::from(minutes))
}

/// R08: minutes of declared work inside the spells and the highest stage among them, by the
/// local day each shift starts.
fn work_overlap(
    spells: &[Spell],
    windows: &[WorkWindow],
    offset: FixedOffset,
) -> BTreeMap<NaiveDate, (i64, u8)> {
    let mut out: BTreeMap<NaiveDate, (i64, u8)> = BTreeMap::new();
    for s in spells {
        let (from, to) = (local(s.from, offset), local(s.to, offset));
        let mut day = from.date().pred_opt().unwrap_or(from.date());
        while day <= to.date() {
            let weekday = day.weekday().number_from_monday();
            for w in windows.iter().filter(|w| u32::from(w.weekday) == weekday) {
                let ws = add(day, w.start_minute);
                let we = if w.end_minute > w.start_minute {
                    add(day, w.end_minute)
                } else {
                    add(day, w.end_minute) + Duration::days(1)
                };
                let overlap = (to.min(we) - from.max(ws)).num_minutes();
                if overlap > 0 {
                    let e = out.entry(day).or_insert((0, s.stage));
                    e.0 += overlap;
                    e.1 = e.1.max(s.stage);
                }
            }
            let Some(next) = day.succ_opt() else { break };
            day = next;
        }
    }
    out
}

/// Minutes of each local day inside the spells and the highest stage among them.
fn days_touched(spells: &[Spell], offset: FixedOffset) -> BTreeMap<NaiveDate, (i64, u8)> {
    let mut out: BTreeMap<NaiveDate, (i64, u8)> = BTreeMap::new();
    for s in spells {
        let (from, to) = (local(s.from, offset), local(s.to, offset));
        let mut day = from.date();
        while day <= to.date() {
            let ds = add(day, 0).max(from);
            let de = (add(day, 0) + Duration::days(1)).min(to);
            if de > ds {
                let e = out.entry(day).or_insert((0, s.stage));
                e.0 += (de - ds).num_minutes();
                e.1 = e.1.max(s.stage);
            }
            let Some(next) = day.succ_opt() else { break };
            day = next;
        }
    }
    out
}

/// A trigger zone: a peril and, for one reported per zone, the zone (`None` is territory-wide).
/// New premium into a zone stops while its benefit in force exceeds `zone_accumulation_limit`
/// (R51).
pub type TriggerZone = (String, Option<String>);

/// The trigger zones a cover would put this declaration in: its peril territory-wide, or for
/// each declared route, work area, ticket or booking its trigger matches on.
pub fn zones_of(
    cover: &CoverSpec,
    routes: &[String],
    areas: &[String],
    upcoming: &[UpcomingEvent],
    cfg: &EngineConfig,
) -> Vec<TriggerZone> {
    let Some(trigger) = &cover.trigger else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for c in &trigger.events {
        let Some(peril) = cfg.accumulation.peril_of(&c.event_type) else {
            continue;
        };
        let unit = cfg.triggers.event_types.get(&c.event_type).map(|t| t.zone);
        let p = || peril.id.clone();
        match (unit, trigger.match_on) {
            (Some(ZoneUnit::Territory) | None, _) => out.push((p(), None)),
            (_, Some(MatchKey::DeclaredRoute)) => {
                out.extend(routes.iter().map(|r| (p(), Some(r.clone()))));
            }
            (_, Some(MatchKey::DeclaredArea)) => {
                out.extend(areas.iter().map(|a| (p(), Some(a.clone()))));
            }
            (_, Some(MatchKey::TicketReference | MatchKey::DeclaredBooking)) => out.extend(
                upcoming
                    .iter()
                    .filter_map(|u| u.reference.clone())
                    .map(|r| (p(), Some(r))),
            ),
            (_, None) => {}
        }
    }
    out.sort();
    out.dedup();
    out
}

/// R51: benefit in force per trigger zone across a book of policies: for each cover, what one
/// occurrence can owe at once (one day's or one event's benefit, `max_hours` hours' for an
/// hourly benefit).
pub fn benefit_in_force_by_zone(
    policies: &[Policy],
    cfg: &EngineConfig,
) -> BTreeMap<TriggerZone, Money> {
    let mut out: BTreeMap<TriggerZone, Money> = BTreeMap::new();
    for p in policies {
        for a in p
            .schedule
            .allocations
            .iter()
            .filter(|a| a.sum_insured > Money::ZERO)
        {
            let Some(cover) = cfg.catalogue.get(&a.cover_id) else {
                continue;
            };
            let benefit = match cover.payout.as_ref() {
                Some(spec) if spec.basis == PayoutBasis::PerHour => {
                    times_n(a.sum_insured, i64::from(spec.max_hours.unwrap_or(0)))
                }
                _ => a.sum_insured,
            };
            for zone in zones_of(
                cover,
                &p.commute_routes,
                &p.work_areas,
                &p.upcoming_events,
                cfg,
            ) {
                let total = out.entry(zone).or_default();
                *total = Money(total.0.saturating_add(benefit.0));
            }
        }
    }
    out
}

/// The zones whose benefit in force exceeds `zone_accumulation_limit` (R51).
pub fn full_zones(
    by_zone: &BTreeMap<TriggerZone, Money>,
    cfg: &EngineConfig,
) -> BTreeSet<TriggerZone> {
    by_zone
        .iter()
        .filter(|(_, b)| **b > cfg.accumulation.zone_accumulation_limit)
        .map(|(z, _)| z.clone())
        .collect()
}
