//! What people work on (ADR-010): the ops console's snapshot, the weekly `FairnessReport` and
//! the claims part of demo mode. Demo mode runs four synthetic claims on saved AI replies: one
//! approved straight through and paid; one in its waiting period, declined by a person and put
//! to a second person on the customer's request; an urgent accident claim whose first response
//! goes out at once and which waits for a person above the STP limit; and a claim reusing an
//! earlier photo, held for risk review. With a review request on a payout and the payouts drawn
//! for audit, they make the queue the ops console shows.
use crate::claims::ClaimsDesk;
use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_ai::client::Model;
use microinsurance_ai::verify::{Evidence, fingerprint};
use microinsurance_engine::claims::{
    self, ClaimContext, ClaimNotice, ClaimState, DeclineReason, PersonOutcome, StpOutcome,
};
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::consent::PremiumStatus;
use microinsurance_engine::guardrails::appeals::{self, Appeal};
use microinsurance_engine::guardrails::audit::{self, AuditFinding};
use microinsurance_engine::guardrails::fairness::{self, Observation};
use microinsurance_engine::guardrails::queue::{self, OpsSnapshot, WorkItem};
use microinsurance_engine::guardrails::{Actor, DecisionRecord};
use microinsurance_engine::money::Money;
use microinsurance_engine::optimiser::underwriting::ClassStatus;
use microinsurance_engine::types::{Allocation, Policy, PolicyPeriod, Schedule};
use microinsurance_ledger::{LedgerStore, postings};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::Path;

/// Decisions and approvals of one audit group, as the audit store counts them.
#[derive(Debug, Clone, Deserialize)]
pub struct GroupCount {
    pub kind: String,
    pub attribute: String,
    pub group: String,
    pub decisions: u32,
    pub approved: u32,
}

/// The audit store's synthetic contents for demo mode (`fixtures/guardrails_synthetic.json`).
#[derive(Debug, Clone, Deserialize)]
pub struct AuditStore {
    pub fairness: Vec<GroupCount>,
    pub audit_findings: Vec<AuditFinding>,
}

/// One observation per decision.
pub fn observations(counts: &[GroupCount]) -> Vec<Observation> {
    counts
        .iter()
        .flat_map(|c| {
            (0..c.decisions).map(move |i| Observation {
                kind: c.kind.clone(),
                attribute: c.attribute.clone(),
                group: c.group.clone(),
                approved: i < c.approved,
            })
        })
        .collect()
}

/// The ops console's snapshot: the queue in order, the fairness audit (R19) and the audit
/// overturn rates (R14).
pub fn snapshot(
    as_of: DateTime<Utc>,
    items: Vec<WorkItem>,
    store: &AuditStore,
    cfg: &EngineConfig,
) -> Result<OpsSnapshot, String> {
    let fairness =
        fairness::parity_reports(&observations(&store.fairness), &cfg.guardrails.fairness)
            .map_err(|e| e.to_string())?;
    let audit = audit::overturn_rates(&store.audit_findings);
    Ok(OpsSnapshot::new(as_of, items, fairness, audit))
}

fn utc(s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| e.to_string())
}

fn date(y: i32, m: u32, d: u32) -> Result<NaiveDate, String> {
    NaiveDate::from_ymd_opt(y, m, d).ok_or_else(|| "date".to_string())
}

fn hkd(m: Money) -> String {
    format!("HK${}.{:02}", m.0 / 100, m.0 % 100)
}

/// A synthetic policy for October 2026 with one indemnity cover, held since `since`.
fn policy(
    cfg: &EngineConfig,
    id: &str,
    cover: &str,
    sum_insured: i64,
    since: NaiveDate,
) -> Result<Policy, String> {
    let plan = cfg.plan("essential").ok_or("plan essential")?.clone();
    Ok(Policy {
        policy_id: id.into(),
        customer_id: format!("cust-{id}"),
        schedule: Schedule {
            plan_id: plan.id.clone(),
            allocations: vec![Allocation {
                cover_id: cover.into(),
                premium: Money::from_major(20),
                sum_insured: Money::from_major(sum_insured),
            }],
        },
        plan,
        period: PolicyPeriod {
            index: 8,
            starts: date(2026, 10, 1)?,
            ends: date(2026, 11, 1)?,
        },
        cover_starts: [(cover.to_string(), since)].into(),
        work_windows: Vec::new(),
        commute_routes: Vec::new(),
        work_areas: Vec::new(),
        upcoming_events: Vec::new(),
        subscribed_at: utc("2026-03-01T00:00:00Z")?,
    })
}

fn notice(id: &str, cover: &str, amount: i64) -> Result<ClaimNotice, String> {
    Ok(ClaimNotice {
        claim_id: id.into(),
        cover_id: cover.into(),
        loss_date: date(2026, 10, 18)?,
        notified_at: utc("2026-10-20T09:00:00Z")?,
        amount_claimed: Money::from_major(amount),
        circumstance: None,
        asked_for_person: false,
        guarantee_requested: Money::ZERO,
    })
}

fn image(root: &Path, file: &str) -> Result<Evidence, String> {
    let path = root.join("fixtures/evidence").join(file);
    Ok(Evidence {
        media_type: "image/png".into(),
        bytes: std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
    })
}

/// What every demo claim shares: cover open, premium paid, the repair network available.
struct DemoBook {
    open: ClassStatus,
    paid: PremiumStatus,
    suitable: BTreeSet<String>,
}

impl DemoBook {
    fn ctx<'a>(&'a self, policy: &'a Policy) -> ClaimContext<'a> {
        ClaimContext {
            policy,
            class_status: &self.open,
            premium: &self.paid,
            claims_in_class: 1,
            evidence_reused: false,
            monthly_earnings: None,
            suitable: &self.suitable,
        }
    }
}

/// The claims demo's result: the lines it prints and the ops console's snapshot.
pub struct ClaimsDemo {
    pub lines: Vec<String>,
    pub snapshot: OpsSnapshot,
    pub records: Vec<DecisionRecord>,
}

/// Four synthetic claims on saved AI replies (see the module docs), then the snapshot. `extra`
/// are other people's items, such as payouts drawn for audit after payment.
pub async fn claims_demo<M: Model>(
    desk: &ClaimsDesk<'_, M>,
    ledger: &impl LedgerStore,
    root: &Path,
    extra: Vec<WorkItem>,
) -> Result<ClaimsDemo, String> {
    let cfg = desk.cfg;
    let mut lines = Vec::new();
    let mut records = Vec::new();
    let mut items = extra;
    let book = DemoBook {
        open: ClassStatus::default(),
        paid: PremiumStatus::Paid,
        suitable: ["partner_repair_with_loaner".to_string()].into(),
    };
    let ctx = |policy| book.ctx(policy);
    let decided_at = utc("2026-10-20T09:05:00Z")?;
    let mut seen: BTreeSet<String> = BTreeSet::new();

    // 1. A cracked screen: approved straight through, settled in cash.
    let phone = policy(
        cfg,
        "demo-phone-owner",
        "device_damage",
        3_000,
        date(2026, 6, 1)?,
    )?;
    let cracked = image(root, "cracked_screen_synthetic.png")?;
    let (claim, _) = desk.notify(
        notice("claim-demo-1", "device_damage", 1_050)?,
        &ctx(&phone),
    )?;
    let claim = claims::add_evidence(claim, &["photo".to_string()].into(), cfg)
        .map_err(|e| e.to_string())?;
    let a = desk
        .assess(
            claim,
            "I dropped my phone on the pavement and the screen cracked across the middle.",
            std::slice::from_ref(&cracked),
            &seen,
            None,
            ctx(&phone),
            decided_at,
        )
        .await?;
    seen.insert(fingerprint(&cracked.bytes));
    records.push(a.record.clone());
    match &a.decided.outcome {
        StpOutcome::Approved { payment } => {
            let shown: Vec<String> = payment.options.iter().map(|o| format!("{o:?}")).collect();
            lines.push(format!(
                "claim   {} approved straight through: {} ({})",
                payment.claim_id,
                hkd(payment.amount),
                shown.join(" | ")
            ));
            let settled = claims::settle(payment, None, Money::ZERO).map_err(|e| e.to_string())?;
            for (key, entries) in [
                postings::claim_approved(payment),
                postings::payout_settled(&settled),
            ] {
                ledger
                    .post(&key, &entries)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            lines.push(format!("paid    {} in cash", hkd(settled.cash)));
        }
        StpOutcome::Referred { referral } => {
            lines.push(format!("claim   claim-demo-1 with a person: {referral:?}"));
            items.extend(queue::claim_item(&a.decided.claim));
        }
    }

    // 2. A loss in the waiting period: a person declines; the customer asks for a review, which
    // a different person takes.
    let new_phone = policy(
        cfg,
        "demo-new-phone",
        "device_damage",
        2_000,
        date(2026, 10, 10)?,
    )?;
    let (claim, _) = desk.notify(
        notice("claim-demo-2", "device_damage", 900)?,
        &ctx(&new_phone),
    )?;
    let claim = claims::add_evidence(claim, &["photo".to_string()].into(), cfg)
        .map_err(|e| e.to_string())?;
    let a = desk
        .assess(
            claim,
            "The screen broke yesterday when I sat on it.",
            &[image(root, "damage_unclear_synthetic.png")?],
            &seen,
            None,
            ctx(&new_phone),
            decided_at,
        )
        .await?;
    records.push(a.record.clone());
    lines.push(format!(
        "claim   claim-demo-2 with a person: {}",
        a.record.reason
    ));
    let (claim, _, record) = desk.person_decides(
        a.decided.claim,
        "handler-amy",
        PersonOutcome::Decline {
            reason: DeclineReason::WaitingPeriod,
        },
        &ctx(&new_phone),
        utc("2026-10-20T15:00:00Z")?,
    )?;
    records.push(record);
    lines.push("claim   claim-demo-2 declined by handler-amy: waiting period (R13)".into());
    let claim =
        claims::appeal(claim, utc("2026-10-21T01:00:00Z")?, cfg).map_err(|e| e.to_string())?;
    lines.push("review  claim-demo-2 requested; a different person decides (R15)".into());
    items.extend(queue::claim_item(&claim));

    // 3. An injury: the first response at once, then a person above the STP limit.
    let rider = policy(
        cfg,
        "demo-cyclist",
        "personal_accident",
        50_000,
        date(2026, 3, 1)?,
    )?;
    let mut n = notice("claim-demo-3", "personal_accident", 3_800)?;
    n.circumstance = Some("injury".into());
    n.guarantee_requested = Money::from_major(8_000);
    let (claim, triage) = desk.notify(n, &ctx(&rider))?;
    if let Some(first) = &triage.first_response {
        let actions: Vec<String> = first.actions.iter().map(|a| format!("{a:?}")).collect();
        lines.push(format!(
            "urgent  claim-demo-3 first response by {}: {}",
            first.respond_by.format("%H:%M:%S UTC"),
            actions.join(", ")
        ));
        if let Some(g) = first.guarantee {
            lines.push(format!(
                "        guarantee of payment {} automatic, {} for the assistance provider to confirm (R46)",
                hkd(g.automatic),
                hkd(g.to_confirm)
            ));
        }
        lines.push(format!("        a person is offered: {:?}", triage.person));
    }
    let kinds = [
        "hospital_receipt".to_string(),
        "medical_certificate".to_string(),
    ]
    .into();
    let claim = claims::add_evidence(claim, &kinds, cfg).map_err(|e| e.to_string())?;
    let text = |t: &str| Evidence {
        media_type: "text/plain".into(),
        bytes: t.as_bytes().to_vec(),
    };
    let documents = [
        text("瑪嘉烈醫院 收據 2026-10-19 急症室及住院一晚 骨科 診斷：右腕骨折 合共 HK$3,800"),
        text("醫生證明書 2026-10-19 右腕骨折 建議休息六星期"),
    ];
    let a = desk
        .assess(
            claim,
            "我踩單車時跌倒，手腕骨折，在醫院住了一晚，現在仍然很痛。",
            &documents,
            &seen,
            Some("zh-Hant"),
            ctx(&rider),
            decided_at,
        )
        .await?;
    records.push(a.record.clone());
    lines.push(format!(
        "claim   claim-demo-3 with a person: {}",
        a.record.reason
    ));
    items.extend(queue::claim_item(&a.decided.claim));

    // 4. The first claim's photo sent again: caught by its fingerprint, held for risk review.
    let (claim, _) = desk.notify(
        notice("claim-demo-4", "device_damage", 1_050)?,
        &ctx(&phone),
    )?;
    let claim = claims::add_evidence(claim, &["photo".to_string()].into(), cfg)
        .map_err(|e| e.to_string())?;
    let a = desk
        .assess(
            claim,
            "My phone screen cracked again.",
            &[cracked],
            &seen,
            None,
            ctx(&phone),
            decided_at,
        )
        .await?;
    records.push(a.record.clone());
    let indicators: Vec<String> = a
        .decided
        .anomaly
        .indicators
        .iter()
        .map(|i| format!("{i:?}"))
        .collect();
    lines.push(format!(
        "claim   claim-demo-4 with a person: {} (anomaly {} bps: {})",
        a.record.reason,
        a.decided.anomaly.score.0,
        indicators.join(", ")
    ));
    if let ClaimState::WithPerson { .. } = a.decided.claim.state {
        items.extend(queue::claim_item(&a.decided.claim));
    }

    // A review request on a payout the rules did not make: the stall works at weekends.
    let unpaid = DecisionRecord {
        decision_id: "decision/demo-weekend-stall/windstorm_and_rain/not-matched".into(),
        actor: Actor::Rules,
        reviewer: None,
        subject_type: "payout".into(),
        subject_id: "demo-weekend-stall/windstorm_and_rain".into(),
        customer_id: "cust-demo-weekend-stall".into(),
        decision: "not matched".into(),
        inputs_hash: String::new(),
        reason: "No declared working hours under the signal".into(),
        at: utc("2026-09-15T23:00:00Z")?,
    };
    let review: Appeal = appeals::open(
        &unpaid,
        "cust-demo-weekend-stall",
        "I also open the stall on Monday mornings",
        utc("2026-10-20T03:00:00Z")?,
        &cfg.guardrails,
    )
    .map_err(|e| e.to_string())?;
    lines.push(format!("review  {} requested (R15)", review.decision_id));
    items.extend(queue::appeal_item(&review));
    records.push(unpaid);

    let store: AuditStore = {
        let path = root.join("fixtures/guardrails_synthetic.json");
        let text = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_slice(&text).map_err(|e| format!("{}: {e}", path.display()))?
    };
    let snapshot = snapshot(utc("2026-10-21T02:00:00Z")?, items, &store, cfg)?;
    for r in &snapshot.fairness {
        if !r.flagged.is_empty() {
            lines.push(format!(
                "fairness {} by {}: {} below {} bps of the best group (R19)",
                r.kind,
                r.attribute,
                r.flagged.join(", "),
                cfg.guardrails.fairness.min_parity_ratio.0
            ));
        }
    }
    lines.push(format!(
        "queue   {} items for people, urgent first",
        snapshot.items.len()
    ));
    Ok(ClaimsDemo {
        lines,
        snapshot,
        records,
    })
}
