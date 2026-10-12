//! Hong Kong Observatory weather warnings.
//!
//! Live: the warning summary (`warnsum`), one entry per warning type in force, keyed by type
//! (`WTCSGNL`, `WRAIN`, …) with the sub-code (`TC8NE`, `WRAINB`) that the source's config maps to
//! an event type and stage. A cancelled warning is listed for a while with `actionCode: CANCEL`;
//! it is reported with its end. Times carry their offset (+08:00).
//!
//! History: the HKO Warnings and Signals Database, exported as CSV with one row per warning
//! issued: `code,issued,cancelled` in local time with offset. Decide: confirm the export format
//! with HKO; the adapter reads only these three columns.
use crate::{Feed, SourceError, TriggerSource};
use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_engine::triggers::SourceReport;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const SOURCE_ID: &str = "hko_warnings";

pub struct HkoWarnings {
    pub live: Feed,
    /// The CSV export of the warnings database, when available.
    pub history: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    action_code: Option<String>,
    issue_time: String,
    #[serde(default)]
    update_time: Option<String>,
}

fn time(s: &str) -> Result<DateTime<Utc>, SourceError> {
    DateTime::parse_from_rfc3339(s)
        .or_else(|_| DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M%:z"))
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| SourceError::Payload(format!("time {s:?}: {e}")))
}

/// The warning summary as reports, ordered by warning type.
pub fn parse_warnsum(json: &str) -> Result<Vec<SourceReport>, SourceError> {
    let entries: BTreeMap<String, Entry> =
        serde_json::from_str(json).map_err(|e| SourceError::Payload(e.to_string()))?;
    entries
        .into_iter()
        .map(|(kind, e)| {
            let starts = time(&e.issue_time)?;
            let ends = match (e.action_code.as_deref(), &e.update_time) {
                (Some("CANCEL"), Some(t)) => Some(time(t)?),
                (Some("CANCEL"), None) => Some(starts),
                _ => None,
            };
            Ok(SourceReport {
                report_id: format!("{kind}:{}", e.issue_time),
                source: SOURCE_ID.into(),
                code: e.code.unwrap_or(kind),
                starts,
                ends,
                zone: None,
                delay_minutes: None,
                reference: None,
            })
        })
        .collect()
}

/// The warnings database export: one complete report per row that started in `[from, to]`
/// (local dates of the row's own offset).
pub fn parse_history(
    csv: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<SourceReport>, SourceError> {
    let mut out = Vec::new();
    for (n, line) in csv.lines().enumerate() {
        let line = line.trim();
        if n == 0 || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split(',').map(str::trim).collect();
        let [code, issued, cancelled] = cols.as_slice() else {
            return Err(SourceError::Payload(format!("line {}: {line:?}", n + 1)));
        };
        let local_day = DateTime::parse_from_rfc3339(issued)
            .or_else(|_| DateTime::parse_from_str(issued, "%Y-%m-%dT%H:%M%:z"))
            .map(|t| t.date_naive())
            .map_err(|e| SourceError::Payload(format!("line {}: {e}", n + 1)))?;
        if local_day < from || local_day > to {
            continue;
        }
        out.push(SourceReport {
            report_id: format!("{code}:{issued}"),
            source: SOURCE_ID.into(),
            code: (*code).to_string(),
            starts: time(issued)?,
            ends: Some(time(cancelled)?),
            zone: None,
            delay_minutes: None,
            reference: None,
        });
    }
    Ok(out)
}

impl TriggerSource for HkoWarnings {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    async fn poll(&self) -> Result<Vec<SourceReport>, SourceError> {
        parse_warnsum(&self.live.text().await?)
    }

    async fn history(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<SourceReport>, SourceError> {
        let path = self.history.as_ref().ok_or_else(|| {
            SourceError::Unavailable("no warnings database export configured".into())
        })?;
        let csv = std::fs::read_to_string(path)
            .map_err(|e| SourceError::Fetch(format!("{}: {e}", path.display())))?;
        parse_history(&csv, from, to)
    }
}
