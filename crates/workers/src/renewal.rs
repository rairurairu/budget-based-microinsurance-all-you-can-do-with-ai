//! Monthly renewal. `ProposeRenewal` runs `renewal_notice_days` before a policy period: the
//! unchanged allocation renews re-rated (R34) and the notice shows the new sums insured (R35).
//! `OpenPolicyPeriod` runs on the period's first day: the customer's accepted reallocation for
//! it, or the default from the notice (R03, R04), becomes the schedule, and the premium is billed
//! to the ledger. The policy renews every period until the customer cancels (R41).
use microinsurance_engine::Result;
use microinsurance_engine::consent::{self, Endorsement, RenewalNotice};
use microinsurance_engine::optimiser::{self, AllocationOutcome, PeriodContext};
use microinsurance_engine::types::{Policy, PolicyPeriod, Profile, Schedule, add_policy_periods};
use microinsurance_ledger::Posting;
use microinsurance_ledger::postings;
use std::collections::BTreeMap;

/// The next policy period after `current`.
pub fn next_period(current: &PolicyPeriod) -> PolicyPeriod {
    PolicyPeriod {
        index: current.index + 1,
        starts: current.ends,
        ends: add_policy_periods(current.ends, 1),
    }
}

#[derive(Debug, Clone)]
pub struct Proposal {
    /// The optimiser's default for the next period: applies unless the customer updates it.
    pub default: AllocationOutcome,
    /// Sent to the customer by `notice.issue_by`; `ai::explain` adds the sentences (R23).
    pub notice: RenewalNotice,
}

/// `ProposeRenewal`. `next` is the context of the coming period: its rates, class statuses and
/// what is in force (R06, R31, R39, R41, R51).
pub fn propose(policy: &Policy, profile: &Profile, next: &PeriodContext) -> Result<Proposal> {
    let default = optimiser::renew(&policy.schedule, &policy.plan, profile, next)?;
    let notice =
        consent::renewal_notice(&policy.schedule, &default, profile, &next.period, next.cfg);
    Ok(Proposal { default, notice })
}

/// `OpenPolicyPeriod`: the policy for the next period and its premium posting. `default` is the
/// schedule the renewal notice issued (R35), or the current one when no notice was issued in
/// time; the customer's accepted change for the period replaces it (R03, R04). Covers that
/// continue keep their start date (waiting periods served stay served); covers new to the
/// schedule start with the period.
pub fn open_period(
    policy: &Policy,
    default: &Schedule,
    update: Option<&Endorsement>,
) -> (Policy, (String, Vec<Posting>)) {
    let period = next_period(&policy.period);
    let schedule = consent::schedule_for_period(default, update, period.index);
    let cover_starts: BTreeMap<_, _> = schedule
        .allocations
        .iter()
        .map(|a| {
            let start = policy
                .cover_starts
                .get(&a.cover_id)
                .copied()
                .unwrap_or(period.starts);
            (a.cover_id.clone(), start)
        })
        .collect();
    let billed =
        postings::premium_billed(&policy.policy_id, period.index, schedule.total_premium());
    let renewed = Policy {
        schedule,
        period,
        cover_starts,
        ..policy.clone()
    };
    (renewed, billed)
}
