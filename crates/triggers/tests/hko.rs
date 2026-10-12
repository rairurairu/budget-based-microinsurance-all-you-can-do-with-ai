use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_triggers::adapters::hko::{self, HkoWarnings};
use microinsurance_triggers::{Feed, SourceError, TriggerSource};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .expect("time")
        .with_timezone(&Utc)
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("date")
}

#[tokio::test]
async fn warnsum_reports_each_warning_in_force_with_its_sub_code() {
    let source = HkoWarnings {
        live: Feed::Fixture(fixture("hko_warnsum_signal8.json")),
        history: None,
    };
    let reports = source.poll().await.expect("fixture parses");
    let codes: Vec<&str> = reports.iter().map(|r| r.code.as_str()).collect();
    assert_eq!(codes, ["TC8NE", "WTS"]);
    let signal8 = &reports[0];
    assert_eq!(signal8.source, "hko_warnings");
    assert_eq!(signal8.starts, utc("2026-09-14T01:40:00Z"));
    assert_eq!(signal8.ends, None);
}

#[tokio::test]
async fn cancelled_warning_carries_its_end() {
    let source = HkoWarnings {
        live: Feed::Fixture(fixture("hko_warnsum_all_clear.json")),
        history: None,
    };
    let reports = source.poll().await.expect("fixture parses");
    assert_eq!(reports[0].ends, Some(utc("2026-09-14T22:20:00Z")));
}

#[test]
fn nothing_in_force_is_no_reports() {
    assert!(hko::parse_warnsum("{}").expect("empty").is_empty());
    assert!(matches!(
        hko::parse_warnsum("[1, 2]"),
        Err(SourceError::Payload(_))
    ));
}

#[tokio::test]
async fn history_rows_in_range_with_their_ends() {
    let source = HkoWarnings {
        live: Feed::Fixture(fixture("hko_warnsum_signal8.json")),
        history: Some(fixture("hko_warnings_history_synthetic.csv")),
    };
    let all = source
        .history(day(2016, 1, 1), day(2025, 12, 31))
        .await
        .expect("history");
    assert!(all.len() > 100);
    assert!(all.iter().all(|r| r.ends.is_some_and(|e| e > r.starts)));
    let one_year = source
        .history(day(2020, 1, 1), day(2020, 12, 31))
        .await
        .expect("history");
    assert!(!one_year.is_empty() && one_year.len() < all.len());
    let none = HkoWarnings {
        live: Feed::Fixture(fixture("hko_warnsum_signal8.json")),
        history: None,
    };
    assert!(matches!(
        none.history(day(2020, 1, 1), day(2020, 12, 31)).await,
        Err(SourceError::Unavailable(_))
    ));
}

#[test]
fn history_rejects_malformed_rows() {
    let bad = "code,issued,cancelled\nTC8NE,2020-08-18T10:00+08:00\n";
    assert!(hko::parse_history(bad, day(2020, 1, 1), day(2020, 12, 31)).is_err());
}
