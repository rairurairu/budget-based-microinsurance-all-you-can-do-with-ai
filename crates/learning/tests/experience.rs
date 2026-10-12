//! Experience studies and pilot metrics (ADR-012): aggregates for the actuary and the pilot's
//! gates, with credibility on the claims behind them and nothing per customer.
use microinsurance_learning::experience::{
    ExposureFacts, FULL_CREDIBILITY_CLAIMS, PreventionFacts, credibility_bps, prevention_study,
    study,
};
use microinsurance_learning::pilot::{PilotBook, PilotCustomer, metrics, percentile};

fn period(class: &str, month: &str, claims: u32, incurred: i64) -> ExposureFacts {
    ExposureFacts {
        class_of_business: class.into(),
        period: month.into(),
        premium_minor: 4_000,
        sum_insured_minor: 48_000,
        expected_minor: 1_000,
        claims,
        incurred_minor: incurred,
    }
}

#[test]
fn credibility_follows_the_square_root_rule_up_to_full() {
    assert_eq!(credibility_bps(0, FULL_CREDIBILITY_CLAIMS), 0);
    assert_eq!(credibility_bps(1_082, FULL_CREDIBILITY_CLAIMS), 10_000);
    assert_eq!(credibility_bps(5_000, FULL_CREDIBILITY_CLAIMS), 10_000);
    // A quarter of the claims for full credibility: half the weight.
    assert_eq!(credibility_bps(25, 100), 5_000);
    assert!(credibility_bps(100, 1_082) < credibility_bps(200, 1_082));
}

#[test]
fn r31_actual_against_expected_by_class_and_period() {
    let facts = [
        period("pecuniary.income_weather", "2026-09", 1, 48_000),
        period("pecuniary.income_weather", "2026-09", 0, 0),
        period("pecuniary.income_weather", "2026-10", 0, 0),
        period("property.device", "2026-09", 1, 1_500),
    ];
    let rows = study(&facts, FULL_CREDIBILITY_CLAIMS);
    assert_eq!(rows.len(), 3);
    let typhoon = &rows[0];
    assert_eq!(
        (typhoon.class_of_business.as_str(), typhoon.period.as_str()),
        ("pecuniary.income_weather", "2026-09")
    );
    assert_eq!(typhoon.policy_periods, 2);
    assert_eq!(typhoon.expected_minor, 2_000);
    // A typhoon month: 24 times the expected cost, but one claim is barely credible.
    assert_eq!(typhoon.actual_to_expected_bps, Some(240_000));
    assert_eq!(typhoon.loss_ratio_bps, Some(60_000));
    assert!(typhoon.credibility_bps < 400);
    assert_eq!(rows[1].actual_to_expected_bps, Some(0));
}

#[test]
fn r18_prevention_frequency_with_and_without_the_measure() {
    let mut facts = Vec::new();
    for i in 0..100 {
        facts.push(PreventionFacts {
            class_of_business: "property.device".into(),
            measure: "screen_protector_fitted".into(),
            verified: i < 40,
            claims: u32::from(if i < 40 { i < 1 } else { i < 43 }),
        });
    }
    let rows = prevention_study(&facts, FULL_CREDIBILITY_CLAIMS);
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!((r.periods_with, r.claims_with), (40, 1));
    assert_eq!((r.periods_without, r.claims_without), (60, 3));
    assert_eq!(r.frequency_with_bps, Some(250));
    assert_eq!(r.frequency_without_bps, Some(500));
}

#[test]
fn r59_pilot_metrics_compare_the_control_group() {
    let customer = |control: bool, claims: u32| PilotCustomer {
        control,
        quoted: true,
        bound: true,
        loss_periods: 10,
        loss_claims: claims,
        ..PilotCustomer::default()
    };
    let mut customers: Vec<PilotCustomer> = (0..80).map(|_| customer(false, 1)).collect();
    customers.extend((0..20).map(|_| customer(true, 2)));
    customers.extend((0..100).map(|_| PilotCustomer {
        quoted: true,
        ..PilotCustomer::default()
    }));
    let book = PilotBook {
        customers,
        premium_minor: 1_000_000,
        paid_minor: 550_000,
        claims_decided: 10,
        claims_straight_through: 7,
        claim_payment_hours: vec![1, 2, 3, 30, 50],
        ..PilotBook::default()
    };
    let m = metrics(&book);
    let get = |name: &str| m.iter().find(|x| x.metric == name).expect(name).value;
    assert_eq!(get("conversion_rate"), Some(5_000));
    assert_eq!(get("claim_frequency_treatment"), Some(1_000));
    assert_eq!(get("claim_frequency_control"), Some(2_000));
    assert_eq!(get("claims_avoided_vs_control"), Some(5_000));
    assert_eq!(get("straight_through_rate"), Some(7_000));
    assert_eq!(get("loss_ratio"), Some(5_500));
    assert_eq!(get("claim_payment_hours_p95"), Some(50));
    // Nothing to measure yet: no value, so no gate on it can pass.
    assert_eq!(get("audit_disagreement_rate"), None);
    assert_eq!(get("renewal_rate"), None);
    assert_eq!(percentile(&[5, 1, 3], 50), Some(3));
    assert_eq!(percentile(&[], 95), None);
}
