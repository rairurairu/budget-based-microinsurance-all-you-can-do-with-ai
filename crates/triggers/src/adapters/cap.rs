//! Public warnings in the Common Alerting Protocol (CAP 1.2, OASIS; ITU-T X.1303), the
//! international standard many national weather and civil-protection services publish in. One
//! adapter serves every CAP source; each source's config file says which of the alert's codes
//! and geocodes it reads (ADR-012).
//!
//! Live: a CAP document: one `<alert>`, or a feed (Atom or RSS) whose entries carry their alert
//! inline as XML. Every `alert` element is read, whatever wraps it.
//!
//! - Only `status` `Actual` counts: an exercise, a test or a draft never pays.
//! - An `Update` or a `Cancel` replaces the alerts it lists in `references`, so a cancelled alert
//!   is no longer reported and the engine closes its trigger event (R55).
//! - Each `info` reports the value of its `eventCode` named `cap.event_code` in the source's
//!   config as the source's code; the source maps the codes that pay (`codes`) and the engine
//!   ignores the rest. An `info` without that code is skipped. One alert in several languages
//!   (one `info` each) is one report.
//! - In force from `onset` (or `effective`, or the alert's `sent`) until `expires`: an alert
//!   announced for later is not in force until its onset (`engine::triggers::track`).
//! - Zones: a source with `areas` reports once per `area` whose `geocode` (named `cap.geocode`)
//!   the config lists, in the area id a customer declares; an area it does not list is skipped.
//!   A source without `areas` reports territory-wide.
//!
//! History: none from the feed (`history`: a source is priced from an export of its record, or
//! on the insurer's filed rate).
use crate::{Feed, SourceError, TriggerSource};
use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_engine::triggers::SourceReport;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// Which of an alert's codes a source reads: the `cap` section of its config file.
#[derive(Debug, Clone, Deserialize)]
pub struct CapProfile {
    /// The `eventCode` `valueName` whose `value` is the source's code.
    pub event_code: String,
    /// The `geocode` `valueName` whose `value` names an area; any geocode when unset.
    #[serde(default)]
    pub geocode: Option<String>,
}

#[derive(Deserialize)]
struct SourceFile {
    cap: CapProfile,
    /// The feed's geocodes to the area ids a customer declares.
    #[serde(default)]
    areas: BTreeMap<String, String>,
}

/// A CAP source, as its config file sets it up.
pub struct CapAlerts {
    /// The registered data source id (`config/triggers/<id>.yaml`).
    pub source: String,
    pub live: Feed,
    pub profile: CapProfile,
    /// Geocode to area id; empty for a territory-wide source.
    pub areas: BTreeMap<String, String>,
}

impl CapAlerts {
    /// The adapter for the source `id`, from its config file's `cap` and `areas` sections.
    pub fn from_source_yaml(id: &str, live: Feed, text: &str) -> Result<CapAlerts, SourceError> {
        let file: SourceFile = serde_saphyr::from_str(text)
            .map_err(|e| SourceError::Payload(format!("{id}.yaml cap: {e}")))?;
        Ok(CapAlerts {
            source: id.to_string(),
            live,
            profile: file.cap,
            areas: file.areas,
        })
    }

    /// The document as reports.
    pub fn parse(&self, xml: &str) -> Result<Vec<SourceReport>, SourceError> {
        let alerts = read_alerts(xml)?;
        let actual: Vec<&Alert> = alerts.iter().filter(|a| a.status == "Actual").collect();
        let replaced: BTreeSet<&str> = actual
            .iter()
            .filter(|a| a.msg_type == "Update" || a.msg_type == "Cancel")
            .flat_map(|a| a.references.iter().map(String::as_str))
            .collect();
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for alert in actual {
            if !(alert.msg_type == "Alert" || alert.msg_type == "Update")
                || replaced.contains(alert.identifier.as_str())
            {
                continue;
            }
            for info in &alert.infos {
                let Some(code) = info
                    .event_codes
                    .iter()
                    .find(|(name, _)| *name == self.profile.event_code)
                    .map(|(_, v)| v.trim().to_string())
                else {
                    continue;
                };
                let starts = time(
                    info.onset
                        .as_deref()
                        .or(info.effective.as_deref())
                        .unwrap_or(&alert.sent),
                )?;
                let ends = info.expires.as_deref().map(time).transpose()?;
                for zone in self.zones(info) {
                    if !seen.insert((code.clone(), zone.clone(), starts, ends)) {
                        continue;
                    }
                    out.push(SourceReport {
                        report_id: format!(
                            "{}:{code}:{}",
                            alert.identifier,
                            zone.as_deref().unwrap_or("all")
                        ),
                        source: self.source.clone(),
                        code: code.clone(),
                        starts,
                        ends,
                        zone,
                        delay_minutes: None,
                        reference: None,
                    });
                }
            }
        }
        Ok(out)
    }

    /// The zones an `info` reports in: none for a territory-wide source, otherwise each listed
    /// area it names.
    fn zones(&self, info: &Info) -> Vec<Option<String>> {
        if self.areas.is_empty() {
            return vec![None];
        }
        let named: BTreeSet<&String> = info
            .areas
            .iter()
            .flatten()
            .filter(|(name, _)| self.profile.geocode.as_ref().is_none_or(|g| g == name))
            .filter_map(|(_, value)| self.areas.get(value.trim()))
            .collect();
        named.into_iter().map(|a| Some(a.clone())).collect()
    }
}

fn time(s: &str) -> Result<DateTime<Utc>, SourceError> {
    DateTime::parse_from_rfc3339(s.trim())
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| SourceError::Payload(format!("time {s:?}: {e}")))
}

/// The parts of a CAP alert the adapter reads.
#[derive(Debug, Default)]
struct Alert {
    identifier: String,
    sent: String,
    status: String,
    msg_type: String,
    /// The identifiers of the alerts it replaces.
    references: Vec<String>,
    infos: Vec<Info>,
}

#[derive(Debug, Default)]
struct Info {
    /// (valueName, value)
    event_codes: Vec<(String, String)>,
    effective: Option<String>,
    onset: Option<String>,
    expires: Option<String>,
    /// Each area's geocodes, (valueName, value).
    areas: Vec<Vec<(String, String)>>,
}

/// `sender,identifier,sent` triples, space-separated: the identifiers.
fn referenced(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|t| t.split(',').nth(1))
        .map(str::to_string)
        .collect()
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

/// Every `alert` element of the document, in document order.
fn read_alerts(xml: &str) -> Result<Vec<Alert>, SourceError> {
    let bad = |e: &dyn std::fmt::Display| SourceError::Payload(format!("CAP: {e}"));
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut alerts = Vec::new();
    let mut alert: Option<Alert> = None;
    let mut info: Option<Info> = None;
    let mut area: Option<Vec<(String, String)>> = None;
    let mut pair = (String::new(), String::new());
    loop {
        match reader.read_event().map_err(|e| bad(&e))? {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                match name.as_str() {
                    "alert" => alert = Some(Alert::default()),
                    "info" if alert.is_some() => info = Some(Info::default()),
                    "area" if info.is_some() => area = Some(Vec::new()),
                    "eventCode" | "geocode" => pair = (String::new(), String::new()),
                    _ => {}
                }
                path.push(name);
                text.clear();
            }
            Event::Text(t) => text.push_str(&t.decode().map_err(|e| bad(&e))?),
            Event::CData(t) => text.push_str(&t.decode().map_err(|e| bad(&e))?),
            Event::GeneralRef(r) => {
                if let Some(c) = r.resolve_char_ref().map_err(|e| bad(&e))? {
                    text.push(c);
                } else if let Some(c) = entity(&r.decode().map_err(|e| bad(&e))?) {
                    text.push(c);
                }
            }
            Event::End(_) => {
                let name = path.pop().unwrap_or_default();
                let parent = path.last().map_or("", String::as_str);
                let value = std::mem::take(&mut text).trim().to_string();
                match (parent, name.as_str()) {
                    ("alert", field) => {
                        if let Some(a) = alert.as_mut() {
                            match field {
                                "identifier" => a.identifier = value,
                                "sent" => a.sent = value,
                                "status" => a.status = value,
                                "msgType" => a.msg_type = value,
                                "references" => a.references = referenced(&value),
                                "info" => a.infos.extend(info.take()),
                                _ => {}
                            }
                        }
                    }
                    ("info", field) => {
                        if let Some(i) = info.as_mut() {
                            match field {
                                "effective" => i.effective = Some(value),
                                "onset" => i.onset = Some(value),
                                "expires" => i.expires = Some(value),
                                "eventCode" => i.event_codes.push(std::mem::take(&mut pair)),
                                "area" => i.areas.extend(area.take()),
                                _ => {}
                            }
                        }
                    }
                    ("area", "geocode") => {
                        if let Some(a) = area.as_mut() {
                            a.push(std::mem::take(&mut pair));
                        }
                    }
                    ("eventCode" | "geocode", "valueName") => pair.0 = value,
                    ("eventCode" | "geocode", "value") => pair.1 = value,
                    (_, "alert") => alerts.extend(alert.take()),
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(alerts)
}

impl TriggerSource for CapAlerts {
    fn id(&self) -> &str {
        &self.source
    }

    async fn poll(&self) -> Result<Vec<SourceReport>, SourceError> {
        self.parse(&self.live.text().await?)
    }

    async fn history(
        &self,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> Result<Vec<SourceReport>, SourceError> {
        Err(SourceError::Unavailable(format!(
            "{}: the feed carries no history; price from an export of the record",
            self.source
        )))
    }
}
