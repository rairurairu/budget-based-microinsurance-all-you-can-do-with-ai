//! Event organiser cancellation notices, matched to ticket references.
//!
//! The ticketing partner publishes notices in this engine's notice format, as a feed the worker
//! polls or as a webhook (`POST /webhooks/triggers/organiser_notices`, Phase 6), one notice per
//! event and status:
//!
//! ```json
//! { "notices": [ { "notice_id": "N-1", "event_id": "EVT-2026-1107",
//!                  "status": "cancelled", "announced_at": "2026-11-01T10:00:00+08:00",
//!                  "event_starts": "2026-11-07T20:00:00+08:00" } ] }
//! ```
//!
//! The zone is the partner's `event_id`, the reference a customer declares from the ticket
//! (`event.*` upcoming event); `ticket_reference`, when the partner sends one per ticket, is
//! reported as the report's reference. The status, upper-cased, is the source's code; the
//! source's config maps the codes that pay (`POSTPONED`, `CANCELLED`) and the engine ignores the
//! rest (R55). The disruption is in force from the announcement until the event's scheduled start
//! (`event_starts`), when the report ends; without it, while the partner lists the notice.
//!
//! History: none (the source's `history.available` is false).
use crate::{Feed, SourceError, TriggerSource};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use microinsurance_engine::triggers::SourceReport;
use serde::Deserialize;

pub const SOURCE_ID: &str = "organiser_notices";

pub struct OrganiserNotices {
    pub live: Feed,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notice {
    pub notice_id: String,
    pub event_id: String,
    pub status: String,
    pub announced_at: DateTime<FixedOffset>,
    /// When the event was scheduled to start.
    #[serde(default)]
    pub event_starts: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    pub ticket_reference: Option<String>,
    /// For the customer's notification only.
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Notices {
    notices: Vec<Notice>,
}

impl Notice {
    pub fn report(&self) -> SourceReport {
        let at = self.announced_at.with_timezone(&Utc);
        SourceReport {
            report_id: self.notice_id.clone(),
            source: SOURCE_ID.into(),
            code: self.status.trim().to_uppercase(),
            starts: at,
            ends: self
                .event_starts
                .map(|t| t.with_timezone(&Utc))
                .filter(|t| *t > at),
            zone: Some(self.event_id.trim().to_string()),
            delay_minutes: None,
            reference: self.ticket_reference.clone(),
        }
    }
}

/// The notice feed as reports.
pub fn parse_notices(json: &str) -> Result<Vec<SourceReport>, SourceError> {
    let n: Notices = serde_json::from_str(json).map_err(|e| SourceError::Payload(e.to_string()))?;
    Ok(n.notices.iter().map(Notice::report).collect())
}

/// One webhook body as a report.
pub fn parse_webhook(json: &str) -> Result<SourceReport, SourceError> {
    let n: Notice = serde_json::from_str(json).map_err(|e| SourceError::Payload(e.to_string()))?;
    Ok(n.report())
}

impl TriggerSource for OrganiserNotices {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    async fn poll(&self) -> Result<Vec<SourceReport>, SourceError> {
        parse_notices(&self.live.text().await?)
    }

    async fn history(
        &self,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> Result<Vec<SourceReport>, SourceError> {
        Err(SourceError::Unavailable(format!(
            "{SOURCE_ID}: no historical record; priced on the insurer's filed rate"
        )))
    }
}
