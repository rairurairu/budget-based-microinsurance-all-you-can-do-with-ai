//! `PollTriggers`: a source's reports → trigger events (engine `track`) → loss occurrences
//! (`group`) → what each policy is owed (`match_occurrence`) → payouts in two stages within the
//! peril's event limit (R51) → ledger, idempotently (R27). A payout with only cash on offer is
//! settled at once, net of any premium due in the grace period (R53); one with alternatives waits
//! for the customer's choice (R16, R28).
//!
//! The desk's state is serialisable so it can be kept between runs (`state`). Postings that
//! failed to reach the ledger stay in the outbox and are retried on the next poll; their keys
//! make the retry safe.
use chrono::{DateTime, Utc};
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::money::Money;
use microinsurance_engine::payouts::{
    self, OccurrencePayouts, PayoutDecision, Settled, Settlement,
};
use microinsurance_engine::triggers::{
    self, Grouped, LossOccurrence, PolicyIndex, SourceReport, Tracked, TriggerEvent,
};
use microinsurance_engine::types::{Policy, PolicyId};
use microinsurance_ledger::{LedgerError, LedgerStore, PostOutcome, Posting, postings};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// What a poll needs besides the reports.
pub struct Book<'a> {
    pub cfg: &'a EngineConfig,
    pub policies: &'a [Policy],
    pub index: &'a PolicyIndex,
    /// Premium due and unpaid, by policy, for policies in the grace period (R53).
    pub premium_due: &'a BTreeMap<PolicyId, Money>,
    /// Settlement in kind that partners can provide now (R28).
    pub suitable: &'a BTreeSet<String>,
}

#[derive(Debug, Default)]
pub struct PollOutcome {
    pub tracked: Tracked,
    pub grouped: Grouped,
    pub decided: Vec<PayoutDecision>,
    pub settled: Vec<Settled>,
    /// Payouts offering alternatives: settled when the customer chooses (`settle_choice`).
    pub awaiting_choice: Vec<PayoutDecision>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TriggerDesk {
    pub events: BTreeMap<String, TriggerEvent>,
    pub occurrences: BTreeMap<String, LossOccurrence>,
    pub payouts: BTreeMap<String, OccurrencePayouts>,
    /// Ledger entries not yet confirmed, by idempotency key.
    pub outbox: BTreeMap<String, Vec<Posting>>,
}

impl TriggerDesk {
    /// Decides everything one poll of `source` changes, and closes any occurrence whose
    /// aggregation window has passed. Pure: no I/O; `flush` posts.
    pub fn poll(
        &mut self,
        source: &str,
        reports: &[SourceReport],
        now: DateTime<Utc>,
        book: &Book,
    ) -> Result<PollOutcome, String> {
        let cfg = book.cfg;
        let tracked = triggers::track(&mut self.events, source, reports, now, cfg);
        let grouped = triggers::group(&mut self.occurrences, &self.events, now, cfg);
        for id in &grouped.opened {
            let Some(occ) = self.occurrences.get(id) else {
                continue;
            };
            let exposed =
                triggers::match_occurrence(occ, &self.events, book.policies, book.index, now, cfg);
            let opened = OccurrencePayouts::open(occ, triggers::benefit_in_force(&exposed), cfg)
                .map_err(|e| e.to_string())?;
            self.payouts.insert(id.clone(), opened);
        }
        let mut out = PollOutcome::default();
        let mut due = book.premium_due.clone();
        let ids: Vec<String> = self.occurrences.keys().cloned().collect();
        for id in ids {
            let (Some(occ), Some(ledger)) = (self.occurrences.get(&id), self.payouts.get_mut(&id))
            else {
                continue;
            };
            let matches =
                triggers::match_occurrence(occ, &self.events, book.policies, book.index, now, cfg);
            let decided = if occ.closed {
                payouts::balance(ledger, occ, &matches, book.suitable, cfg)
                    .map_err(|e| e.to_string())?
            } else {
                payouts::guaranteed(ledger, &matches, book.suitable, cfg)
            };
            if occ.closed {
                for event in &occ.events {
                    self.events.remove(event);
                }
                self.occurrences.remove(&id);
                self.payouts.remove(&id);
            }
            for d in decided {
                let (key, entries) = postings::payout_decided(&d);
                self.outbox.insert(key, entries);
                if d.options == [Settlement::Cash] {
                    let owed = due.get(&d.policy_id).copied().unwrap_or(Money::ZERO);
                    let s = payouts::settle(&d, None, owed).map_err(|e| e.to_string())?;
                    if s.premium_set_off > Money::ZERO {
                        due.insert(d.policy_id.clone(), owed - s.premium_set_off);
                    }
                    let (key, entries) = postings::payout_settled(&s);
                    self.outbox.insert(key, entries);
                    out.settled.push(s);
                } else {
                    out.awaiting_choice.push(d.clone());
                }
                out.decided.push(d);
            }
        }
        // Events of a type no peril covers pay nothing: dropped once ended.
        let members: BTreeSet<&String> = self
            .occurrences
            .values()
            .flat_map(|o| o.events.iter())
            .collect();
        let idle: Vec<String> = self
            .events
            .values()
            .filter(|e| e.is_closed() && !members.contains(&e.event_id))
            .map(|e| e.event_id.clone())
            .collect();
        for id in idle {
            self.events.remove(&id);
        }
        out.tracked = tracked;
        out.grouped = grouped;
        Ok(out)
    }

    /// Settles a payout that offered alternatives, as the customer chose (R28).
    pub fn settle_choice(
        &mut self,
        decision: &PayoutDecision,
        choice: Option<&Settlement>,
        premium_due: Money,
    ) -> Result<Settled, String> {
        let s = payouts::settle(decision, choice, premium_due).map_err(|e| e.to_string())?;
        let (key, entries) = postings::payout_settled(&s);
        self.outbox.insert(key, entries);
        Ok(s)
    }

    /// Posts the outbox. Confirmed entries (posted now or before) leave it; anything else stays
    /// for the next poll and is reported.
    pub async fn flush(&mut self, ledger: &impl LedgerStore) -> Vec<String> {
        let batch: Vec<(String, Vec<Posting>)> = self
            .outbox
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let results = ledger.post_many(&batch).await;
        let mut errors = Vec::new();
        for ((key, _), result) in batch.iter().zip(results) {
            match result {
                Ok(PostOutcome::Posted | PostOutcome::AlreadyPosted) => {
                    self.outbox.remove(key);
                }
                Err(e @ LedgerError::KeyReused(_)) => errors.push(format!("{key}: {e}")),
                Err(e) => errors.push(format!("{key}: {e}")),
            }
        }
        errors
    }
}
