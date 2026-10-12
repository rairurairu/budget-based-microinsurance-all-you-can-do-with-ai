//! The claims jobs (ADR-010): glue between the AI assessor and the engine. `NotifyClaim`
//! registers the notification and returns the triage at once, so an urgent case's first response
//! goes out before anyone is asked (R46). `AssessClaim` runs once the evidence is complete: the
//! assessor recommends approve or refer and the engine decides, straight through within the STP
//! authority or to a person (R13, R57). Every decision is logged with a hash of its inputs.
//! Nothing here decides an amount.
use chrono::{DateTime, Utc};
use microinsurance_ai::assess::{self, AssessOutcome};
use microinsurance_ai::client::Model;
use microinsurance_ai::harness::{Harness, Harnessed, Source};
use microinsurance_ai::verify::{Evidence, fingerprint};
use microinsurance_engine::claims::{
    self, Claim, ClaimContext, ClaimNotice, ClaimPayment, Decided, PersonOutcome, StpOutcome,
};
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::guardrails::human_care::Triage;
use microinsurance_engine::guardrails::{Actor, DecisionRecord};
use serde::Serialize;
use std::collections::BTreeSet;

/// What the claims jobs need, for one run.
pub struct ClaimsDesk<'a, M: Model> {
    pub ai: &'a Harness<M>,
    pub cfg: &'a EngineConfig,
    /// The audit seed: the insurer's secret, recorded with each sample (R14).
    pub seed: u64,
}

/// A claim assessed and decided.
#[derive(Debug, Clone)]
pub struct Assessed {
    pub decided: Decided,
    pub assessment: Harnessed<AssessOutcome>,
    pub record: DecisionRecord,
}

/// SHA-256 of a decision's inputs, so it can be replayed.
pub fn inputs_hash(inputs: &impl Serialize) -> String {
    fingerprint(&serde_json::to_vec(inputs).unwrap_or_default())
}

fn outcome_text(outcome: &StpOutcome) -> (String, String) {
    match outcome {
        StpOutcome::Approved { payment } => ("approved".into(), payment.idempotency_key.clone()),
        StpOutcome::Referred { referral } => ("referred".into(), format!("{referral:?}")),
    }
}

impl<M: Model> ClaimsDesk<'_, M> {
    /// `NotifyClaim`: the claim as registered, and its triage.
    pub fn notify(
        &self,
        notice: ClaimNotice,
        ctx: &ClaimContext<'_>,
    ) -> Result<(Claim, Triage), String> {
        claims::notify(notice, ctx, self.cfg).map_err(|e| e.to_string())
    }

    /// `AssessClaim`: the assessor's recommendation on the evidence, then the engine's decision.
    /// Evidence the fingerprint check has seen before counts as an anomaly indicator (R14).
    #[allow(clippy::too_many_arguments)]
    pub async fn assess(
        &self,
        claim: Claim,
        description: &str,
        evidence: &[Evidence],
        seen: &BTreeSet<String>,
        language: Option<&str>,
        ctx: ClaimContext<'_>,
        now: DateTime<Utc>,
    ) -> Result<Assessed, String> {
        let assessment = assess::assess(
            self.ai,
            &claim,
            description,
            evidence,
            seen,
            language,
            self.cfg,
        )
        .await;
        let ctx = ClaimContext {
            evidence_reused: ctx.evidence_reused || assessment.value.evidence_reused(),
            ..ctx
        };
        let review = assessment.value.review();
        let hash = inputs_hash(&(&claim, &assessment.value));
        let decided = claims::decide(claim, review, &ctx, now, self.seed, self.cfg)
            .map_err(|e| e.to_string())?;
        let (decision, reason) = outcome_text(&decided.outcome);
        let record = DecisionRecord {
            decision_id: format!("decision/{}/stp", decided.claim.claim_id),
            actor: if assessment.source == Source::Model {
                Actor::Ai
            } else {
                Actor::Rules
            },
            reviewer: None,
            subject_type: "claim".into(),
            subject_id: decided.claim.claim_id.clone(),
            customer_id: decided.claim.customer_id.clone(),
            decision,
            inputs_hash: hash,
            reason,
            at: now,
        };
        Ok(Assessed {
            decided,
            assessment,
            record,
        })
    }

    /// A person's decision on a claim with them, logged under the person's name.
    pub fn person_decides(
        &self,
        claim: Claim,
        reviewer: &str,
        outcome: PersonOutcome,
        ctx: &ClaimContext<'_>,
        now: DateTime<Utc>,
    ) -> Result<(Claim, Option<ClaimPayment>, DecisionRecord), String> {
        let hash = inputs_hash(&(&claim, &outcome));
        let n = match &claim.state {
            claims::ClaimState::WithPerson {
                referral: claims::ClaimReferral::Appeal { .. },
                ..
            } => "review",
            _ => "person",
        };
        let decision = match &outcome {
            PersonOutcome::Approve { .. } => "approved".to_string(),
            PersonOutcome::Decline { reason } => format!("declined: {reason:?}"),
            PersonOutcome::RequestEvidence { kinds } => format!("evidence requested: {kinds:?}"),
        };
        let (claim, payment) =
            claims::person_decision(claim, reviewer, outcome, ctx, now, self.cfg)
                .map_err(|e| e.to_string())?;
        let record = DecisionRecord {
            decision_id: format!("decision/{}/{n}", claim.claim_id),
            actor: Actor::Person,
            reviewer: Some(reviewer.to_string()),
            subject_type: "claim".into(),
            subject_id: claim.claim_id.clone(),
            customer_id: claim.customer_id.clone(),
            decision,
            inputs_hash: hash,
            reason: String::new(),
            at: now,
        };
        Ok((claim, payment, record))
    }
}
