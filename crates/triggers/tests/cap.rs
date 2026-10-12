//! The Common Alerting Protocol adapter (ADR-012) on saved (synthetic) Taiwan feeds: only actual
//! alerts count, an update replaces what it references, a cancel withdraws it, one alert in two
//! languages is one report, and each listed area is a zone in the id a customer declares.
use chrono::{DateTime, Utc};
use microinsurance_engine::triggers::SourceReport;
use microinsurance_triggers::adapters::cap::CapAlerts;
use microinsurance_triggers::{Feed, TriggerSource};
use std::path::PathBuf;

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .expect("time")
        .with_timezone(&Utc)
}

fn source(id: &str, fixture: &str) -> CapAlerts {
    let text = std::fs::read_to_string(repo(&format!("config/markets/tw/triggers/{id}.yaml")))
        .expect("source config");
    CapAlerts::from_source_yaml(
        id,
        Feed::Fixture(repo("fixtures/markets/tw").join(fixture)),
        &text,
    )
    .expect("cap section")
}

fn zones(reports: &[SourceReport]) -> Vec<(&str, &str)> {
    reports
        .iter()
        .map(|r| (r.code.as_str(), r.zone.as_deref().unwrap_or("all")))
        .collect()
}

#[tokio::test]
async fn r55_work_suspensions_are_reported_per_declared_area() {
    let feed = source("dgpa_work_suspension", "cap_work_suspension.xml");
    assert_eq!(feed.id(), "dgpa_work_suspension");
    let reports = feed.poll().await.expect("parses");
    // The exercise is dropped; the alert in two languages is one report per area; the
    // classes-only code is reported for the engine to ignore (it maps no event type).
    assert_eq!(
        zones(&reports),
        [
            ("WORK_SCHOOL_SUSPENDED", "tw.new_taipei_city"),
            ("WORK_SCHOOL_SUSPENDED", "tw.taipei_city"),
            ("WORK_SCHOOL_SUSPENDED", "tw.hualien_county"),
            ("SCHOOL_SUSPENDED", "tw.taichung_city"),
        ]
    );
    let taipei = &reports[1];
    assert_eq!(taipei.source, "dgpa_work_suspension");
    assert_eq!(taipei.starts, utc("2026-09-13T16:00:00Z"), "from the onset");
    assert_eq!(
        taipei.ends,
        Some(utc("2026-09-14T16:00:00Z")),
        "to the expiry"
    );
    assert!(
        reports
            .iter()
            .all(|r| r.zone.as_deref() != Some("tw.kaohsiung_city"))
    );
}

#[tokio::test]
async fn r55_an_update_replaces_the_alert_it_references() {
    let reports = source("dgpa_work_suspension", "cap_work_suspension_day2.xml")
        .poll()
        .await
        .expect("parses");
    let ids: Vec<&str> = reports.iter().map(|r| r.report_id.as_str()).collect();
    assert!(
        ids.iter().all(|i| !i.starts_with("DGPA-20260913-001:")),
        "the update replaces it: {ids:?}"
    );
    assert!(ids.contains(&"DGPA-20260914-001:WORK_SCHOOL_SUSPENDED:tw.taipei_city"));
    // Hualien on Monday, and again on Tuesday.
    let hualien: Vec<DateTime<Utc>> = reports
        .iter()
        .filter(|r| r.zone.as_deref() == Some("tw.hualien_county"))
        .map(|r| r.starts)
        .collect();
    assert_eq!(
        hualien,
        [utc("2026-09-13T16:00:00Z"), utc("2026-09-14T16:00:00Z")]
    );
}

#[tokio::test]
async fn r55_a_territory_wide_source_reports_without_a_zone() {
    let reports = source("cwa_typhoon_warnings", "cap_typhoon_warning.xml")
        .poll()
        .await
        .expect("parses");
    assert_eq!(zones(&reports), [("TYPHOON_SEA_LAND", "all")]);
    assert_eq!(reports[0].ends, None, "in force until lifted");
    let none = source("cwa_typhoon_warnings", "cap_empty.xml")
        .poll()
        .await
        .expect("parses");
    assert!(none.is_empty());
}

#[test]
fn r55_a_cancel_withdraws_the_alert_and_tests_never_count() {
    let alert = |id: &str, status: &str, msg: &str, refs: &str| {
        format!(
            "<alert xmlns=\"urn:oasis:names:tc:emergency:cap:1.2\"><identifier>{id}</identifier>\
             <sent>2026-09-13T20:00:00+08:00</sent><status>{status}</status><msgType>{msg}</msgType>\
             <references>{refs}</references><info><eventCode><valueName>profile:CAP-TWP:Event:1.0</valueName>\
             <value>WORK_SUSPENDED</value></eventCode><onset>2026-09-14T00:00:00+08:00</onset>\
             <area><geocode><valueName>Taiwan_Geocode_103</valueName><value>63000</value></geocode></area>\
             </info></alert>"
        )
    };
    let feed = |alerts: &[String]| format!("<feed>{}</feed>", alerts.concat());
    let cap = source("dgpa_work_suspension", "cap_empty.xml");
    let issued = alert("A1", "Actual", "Alert", "");
    assert_eq!(
        cap.parse(&feed(std::slice::from_ref(&issued)))
            .expect("parses")
            .len(),
        1
    );
    let cancel = alert(
        "A2",
        "Actual",
        "Cancel",
        "x@example,A1,2026-09-13T20:00:00+08:00",
    );
    assert!(
        cap.parse(&feed(&[issued.clone(), cancel]))
            .expect("parses")
            .is_empty()
    );
    let test = alert(
        "A3",
        "Test",
        "Cancel",
        "x@example,A1,2026-09-13T20:00:00+08:00",
    );
    assert_eq!(
        cap.parse(&feed(&[issued, test])).expect("parses").len(),
        1,
        "a test message changes nothing"
    );
    assert!(
        cap.parse("<feed><alert></feed>").is_err(),
        "a garbled feed is an error"
    );
}
