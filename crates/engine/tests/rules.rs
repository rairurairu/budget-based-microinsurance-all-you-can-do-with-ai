//! One test per rule in docs/rules.md that the engine enforces. Rules enforced at a boundary have
//! their tests there: R21 in `crates/learning/tests`, R49 (the quote kept as shown) in
//! `crates/api/tests`.
mod common;

use chrono::{DateTime, Duration, Utc};
use common::*;
use microinsurance_engine::claims::{
    self, Assessment, Claim, ClaimContext, ClaimNotice, ClaimReferral, ClaimState, DecidedBy,
    DeclineReason, PersonOutcome, Recommendation, StpOutcome,
};
use microinsurance_engine::config::{self, ConfigSources, EngineConfig, RiskClass};
use microinsurance_engine::consent::{
    self, CustomerTap, LatePayment, Offer, OfferKind, PremiumStatus,
};
use microinsurance_engine::guardrails::anomaly::Indicator;
use microinsurance_engine::guardrails::appeals::{self, AppealState};
use microinsurance_engine::guardrails::audit::{self, AuditFinding, AuditReason};
use microinsurance_engine::guardrails::fairness::{self, Observation};
use microinsurance_engine::guardrails::human_care::{
    self, CaseFacts, Escalation, FirstAction, PersonRoute, UrgentBecause,
};
use microinsurance_engine::guardrails::queue::{self, PersonAction, WorkKind};
use microinsurance_engine::guardrails::{Actor, DecisionRecord};
use microinsurance_engine::money::{Bps, Money, Rate};
use microinsurance_engine::optimiser::rating::{
    self, ClaimStatus, ClaimsExperience, ClassHistory, ExperienceClaim, RatingInputs,
};
use microinsurance_engine::optimiser::underwriting::{self, ClassStatus};
use microinsurance_engine::optimiser::{self, AllocationOutcome};
use microinsurance_engine::payouts::{self, PayoutDecision, PayoutStage, Settled, Settlement};
use microinsurance_engine::pilot::{self, Cohort};
use microinsurance_engine::prevention::{
    self, HeldCover, MeasureClaims, PreventionBasis, PreventionTask, PreventionTaskDraft,
    PreventionTaskProposal, Referral, Review, TaskState, Verdict, VerifiedBy,
};
use microinsurance_engine::rewards::{
    self, EarnedFor, Funder, Redeemed, RewardEarned, RewardOffer,
};
use microinsurance_engine::score::{self, Lapse, ScoreMode, VerifiedActivity};
use microinsurance_engine::suppliers::{self, InKindOrder, OrderSource, OrderState};
use microinsurance_engine::triggers::{
    self, LossOccurrence, Occurrence, SourceReport, TriggerEvent,
};
use microinsurance_engine::types::{
    Allocation, ExposureKind, Policy, Profile, Schedule, WorkWindow, add_policy_periods,
};
use microinsurance_engine::{EngineError, catalogue, plans};
use std::collections::{BTreeMap, BTreeSet};

fn screen_task(reward_cents: i64) -> PreventionTaskDraft {
    PreventionTaskDraft {
        task_type: "case_and_screen_protector".into(),
        cover_id: "device_damage".into(),
        event_reference: None,
        action: "Photograph your phone in its case with a screen protector".into(),
        evidence: "photo".into(),
        claim_prob_before: Bps(400), // 4% a month
        claim_prob_after: Bps(250),  // 2.5% a month
        avg_claim_cost: Money::from_major(800),
        reward_cost: Money(reward_cents),
    }
}

fn in_force(cfg: &microinsurance_engine::config::EngineConfig, code: &str) -> BTreeSet<Occurrence> {
    let report = SourceReport {
        report_id: format!("hko-{code}"),
        source: "hko_warnings".into(),
        code: code.into(),
        starts: at(2026, 10, 1),
        ends: None,
        zone: None,
        delay_minutes: None,
        reference: None,
    };
    triggers::classify(&report, cfg).into_iter().collect()
}

fn assert_rule<T: std::fmt::Debug>(result: Result<T, EngineError>, rule: &str) {
    match result {
        Err(EngineError::Rule { rule: r, .. }) if r == rule => {}
        other => panic!("expected rule {rule} to block, got {other:?}"),
    }
}

fn oct() -> microinsurance_engine::types::PolicyPeriod {
    period(1, 2026, 10)
}

fn suggest(world: &World, plan_id: &str, profile: &Profile) -> AllocationOutcome {
    optimiser::suggest(&world.plan(plan_id), profile, None, &world.ctx())
        .expect("suggestion")
        .outcome
}

fn premiums(pairs: &[(&str, i64)]) -> BTreeMap<String, Money> {
    pairs
        .iter()
        .map(|(c, p)| ((*c).to_string(), Money::from_major(*p)))
        .collect()
}

fn offer(change: OfferKind) -> Offer {
    Offer {
        offer_id: "offer-1".into(),
        customer_id: "cust-rider".into(),
        change,
        explanation: "Your phone is covered from next month.".into(),
        created_at: at(2026, 10, 10),
    }
}

fn tap(offer_id: &str, customer_id: &str) -> CustomerTap {
    CustomerTap {
        offer_id: offer_id.into(),
        customer_id: customer_id.into(),
        at: at(2026, 10, 11),
    }
}

fn experience(paid: usize, under_review: usize, exposure: u32, peer_ppm: u32) -> ClaimsExperience {
    let claim = |i: usize, status| ExperienceClaim {
        period_starts: add_policy_periods(day(2025, 10, 1), u32::try_from(i % 12).unwrap_or(0)),
        status,
    };
    ClaimsExperience {
        claims: (0..paid)
            .map(|i| claim(i, ClaimStatus::Paid))
            .chain((0..under_review).map(|i| claim(i, ClaimStatus::UnderReview)))
            .collect(),
        exposure_periods: exposure,
        peer_frequency_ppm: peer_ppm,
    }
}

// ---- implemented

#[test]
fn r01_plans_same_for_every_customer() {
    let cfg = cfg();
    let prices: Vec<i64> = plans::available_plans(&cfg)
        .iter()
        .map(|p| p.price.0)
        .collect();
    assert_eq!(prices, vec![4_000, 8_000, 15_000]);
    // Pricing takes no customer or risk input beyond promotion eligibility.
    let essential = plan(&cfg, "essential");
    let a = plans::price(&essential, &Default::default(), day(2026, 10, 1), &cfg);
    let b = plans::price(&essential, &Default::default(), day(2026, 10, 1), &cfg);
    assert_eq!(a, b);
    assert_eq!(a.payable, Money::from_major(80));
}

#[test]
fn r02_optimiser_is_deterministic() {
    let w = World::new(&rider(), &basis(), oct());
    let first = optimiser::suggest(&w.plan("essential"), &rider(), None, &w.ctx());
    let again = World::new(&rider(), &basis(), oct());
    let second = optimiser::suggest(&again.plan("essential"), &rider(), None, &again.ctx());
    assert_eq!(first.expect("first"), second.expect("second"));
}

#[test]
fn r02_allocations_sum_to_plan_price_in_steps() {
    let w = World::new(&rider(), &basis(), oct());
    for plan in ["starter", "essential", "max"] {
        let out = suggest(&w, plan, &rider());
        let step = w.cfg.policy_period.allocation_step;
        assert_eq!(
            out.schedule.total_premium() + out.unallocated,
            w.plan(plan).price
        );
        assert!(
            out.schedule
                .allocations
                .iter()
                .all(|a| a.premium.is_multiple_of(step))
        );
    }
    // Within the rider's exposure, the smaller plans are placed in full.
    assert_eq!(suggest(&w, "essential", &rider()).unallocated, Money::ZERO);
    assert_eq!(suggest(&w, "starter", &rider()).unallocated, Money::ZERO);
}

#[test]
fn r03_no_change_without_tap() {
    let w = World::new(&rider(), &basis(), oct());
    let current = suggest(&w, "essential", &rider()).schedule;
    let pol = policy(&w.cfg, current.clone(), oct());
    let o = offer(OfferKind::AddCover {
        cover_id: "device_damage".into(),
    });
    assert_rule(
        consent::accept(&o, &tap("offer-2", "cust-rider"), &pol, &w.cfg),
        "R03",
    );
    assert_rule(
        consent::accept(&o, &tap("offer-1", "someone-else"), &pol, &w.cfg),
        "R03",
    );
    // With no tap, the previous allocation renews unchanged.
    let nov = World::new(&rider(), &basis(), period(2, 2026, 11));
    let renewed =
        optimiser::renew(&current, &nov.plan("essential"), &rider(), &nov.ctx()).expect("renewal");
    for a in &current.allocations {
        assert_eq!(renewed.schedule.premium(&a.cover_id), a.premium);
    }
}

#[test]
fn r04_change_applies_next_period() {
    let w = World::new(&rider(), &basis(), oct());
    let pol = policy(&w.cfg, suggest(&w, "essential", &rider()).schedule, oct());
    let o = offer(OfferKind::AddCover {
        cover_id: "device_damage".into(),
    });
    let e = consent::accept(&o, &tap("offer-1", "cust-rider"), &pol, &w.cfg).expect("accepted");
    assert_eq!(e.effective_period, pol.period.index + 1);
}

#[test]
fn r05_minimum_one_period() {
    let w = World::new(&rider(), &basis(), oct());
    let mut pol = policy(&w.cfg, suggest(&w, "essential", &rider()).schedule, oct());
    // Held since this period started: one period served by the next one.
    assert!(consent::can_remove("device_damage", &pol, &w.cfg));
    // Added for the next period: not removable from it.
    pol.cover_starts
        .insert("device_damage".into(), day(2026, 11, 1));
    assert!(!consent::can_remove("device_damage", &pol, &w.cfg));
    let o = offer(OfferKind::RemoveCover {
        cover_id: "device_damage".into(),
    });
    assert_rule(
        consent::accept(&o, &tap("offer-1", "cust-rider"), &pol, &w.cfg),
        "R05",
    );
}

#[test]
fn r06_no_new_weather_cover_when_signal_in_force() {
    let mut w = World::new(&rider(), &basis(), oct());
    let weather = w.cfg.cover("weather_income").expect("cover").clone();
    let signal3 = in_force(&w.cfg, "TC3");
    assert_rule(catalogue::check_new_cover(&weather, &signal3), "R06");
    assert!(catalogue::check_new_cover(&weather, &BTreeSet::new()).is_ok());
    // An amber rainstorm is not a tropical cyclone signal: new cover stays open.
    assert!(catalogue::check_new_cover(&weather, &in_force(&w.cfg, "WRAINA")).is_ok());

    w.in_force = signal3;
    let out = suggest(&w, "essential", &rider());
    assert_eq!(out.schedule.premium("weather_income"), Money::ZERO);
    let p = premiums(&[
        ("weather_income", 10),
        ("personal_accident", 15),
        ("device_damage", 55),
    ]);
    assert_rule(
        optimiser::price_allocation(&w.plan("essential"), &p, None, &rider(), &w.ctx()),
        "R06",
    );
}

#[test]
fn r07_device_cover_waits_14_days() {
    let cfg = cfg();
    let cover = cfg.cover("device_damage").expect("cover");
    assert!(catalogue::in_waiting_period(
        cover,
        day(2026, 10, 1),
        day(2026, 10, 14)
    ));
    assert!(!catalogue::in_waiting_period(
        cover,
        day(2026, 10, 1),
        day(2026, 10, 15)
    ));
}

#[test]
fn r09_payout_below_daily_earnings() {
    let w = World::new(&rider(), &basis(), oct());
    let weather = w.cfg.cover("weather_income").expect("cover");
    let cap = catalogue::max_parametric_payout(weather, &rider()).expect("capped cover");
    assert_eq!(cap, Money::from_major(480));
    assert!(cap < rider().typical_daily_earnings.expect("declared"));
    let out = suggest(&w, "max", &rider());
    let si = out
        .schedule
        .allocation("weather_income")
        .expect("allocated")
        .sum_insured;
    assert!(si <= cap);
    let p = premiums(&[
        ("weather_income", 25),
        ("personal_accident", 15),
        ("device_damage", 40),
    ]);
    assert_rule(
        optimiser::price_allocation(&w.plan("essential"), &p, None, &rider(), &w.ctx()),
        "R09",
    );
}

#[test]
fn r10_unverified_task_earns_nothing() {
    let cfg = cfg();
    let task = screen_task(300);
    let verdict = |passed, reuse_suspected| Verdict {
        passed,
        reason: "case visible".into(),
        reuse_suspected,
    };
    let on = day(2026, 10, 5);
    assert!(prevention::complete(&task, &verdict(false, false), on, &cfg).is_none());
    assert!(prevention::complete(&task, &verdict(true, true), on, &cfg).is_none());
    let done = prevention::complete(&task, &verdict(true, false), on, &cfg).expect("verified");
    assert_eq!(done.points(), 100);
    assert_eq!(done.class_of_business(), "property.device");
}

#[test]
fn r11_effect_capped_at_20_percent() {
    let cfg = cfg();
    assert_eq!(score::effect(0, &cfg.score), Bps(0));
    assert_eq!(score::effect(500, &cfg.score), Bps(1_000));
    assert_eq!(score::effect(1_000, &cfg.score), Bps(2_000));
    assert_eq!(score::effect(1_000_000, &cfg.score), Bps(2_000));
    let mut last = Bps(0);
    for p in (0..2_000).step_by(37) {
        let e = score::effect(p, &cfg.score);
        assert!(e >= last);
        last = e;
    }
}

#[test]
fn r11_never_worse_than_rated_base() {
    let w = World::new(&rider(), &basis(), oct());
    let base = suggest(&w, "essential", &rider()).schedule;
    let effects: BTreeMap<String, Bps> = [
        ("property".to_string(), Bps(2_000)),
        ("pecuniary".to_string(), Bps(1_000)),
    ]
    .into();
    for mode in [ScoreMode::BonusSumInsured, ScoreMode::PremiumRebate] {
        let out = score::apply(&effects, mode, &base, &w.cfg);
        for (b, a) in base.allocations.iter().zip(&out.schedule.allocations) {
            assert!(a.sum_insured >= b.sum_insured);
            assert_eq!(a.premium, b.premium);
        }
    }
}

#[test]
fn r11_premium_rebate_leaves_plan_price_unchanged() {
    let w = World::new(&rider(), &basis(), oct());
    let base = suggest(&w, "essential", &rider()).schedule;
    let effects: BTreeMap<String, Bps> = [("property".to_string(), Bps(2_000))].into();
    let out = score::apply(&effects, ScoreMode::PremiumRebate, &base, &w.cfg);
    assert_eq!(out.schedule, base);
    assert_eq!(out.schedule.total_premium(), w.plan("essential").price);
    assert_eq!(
        out.premium_rebate,
        base.premium("device_damage").mul_bps(Bps(2_000))
    );
}

#[test]
fn r11_cap_applies_per_points_pool() {
    let w = World::new(&rider(), &basis(), oct());
    let base = suggest(&w, "essential", &rider()).schedule;
    let effects: BTreeMap<String, Bps> = [("property".to_string(), Bps(5_000))].into();
    let out = score::apply(&effects, ScoreMode::PremiumRebate, &base, &w.cfg);
    assert_eq!(
        out.premium_rebate,
        base.premium("device_damage").mul_bps(Bps(2_000))
    );
}

#[test]
fn r12_points_fade() {
    let cfg = cfg();
    let task = screen_task(300);
    let ok = Verdict {
        passed: true,
        reason: "ok".into(),
        reuse_suspected: false,
    };
    let on = day(2026, 1, 1);
    let history = vec![prevention::complete(&task, &ok, on, &cfg).expect("verified")];
    let at_days = |d: u64| on.checked_add_days(chrono::Days::new(d)).expect("date");
    let pts = |d: u64, lapses: &[Lapse]| score::points(&history, at_days(d), 45, lapses);
    assert_eq!(pts(0, &[]), 100);
    assert_eq!(pts(45, &[]), 50);
    assert_eq!(pts(90, &[]), 25);
    let mut last = 100;
    for d in 0..200 {
        let p = pts(d, &[]);
        assert!(p <= last);
        last = p;
    }
    // R53: no fading while lapsed.
    let lapse = [Lapse {
        from: at_days(10),
        until: at_days(55),
    }];
    assert_eq!(pts(90, &lapse), 50);
}

#[test]
fn r13_stp_claim_outcomes_cannot_decline() {
    // Exhaustive match: adding a Declined variant breaks this test at compile time.
    fn outcome(o: &StpOutcome) -> &'static str {
        match o {
            StpOutcome::Approved { .. } => "approved",
            StpOutcome::Referred { .. } => "referred",
        }
    }
    let referred = StpOutcome::Referred {
        referral: ClaimReferral::AssessorReferred,
    };
    assert_eq!(outcome(&referred), "referred");
}

#[test]
fn r14_audit_samples_about_one_in_twenty() {
    let audited = (0..100_000)
        .filter(|i| audit::should_audit(&format!("decision-{i}"), Bps(500), 7))
        .count();
    assert!(
        (4_500..=5_500).contains(&audited),
        "audited {audited} of 100000"
    );
}

#[test]
fn r14_audit_selection_is_replayable() {
    let a = audit::should_audit("decision-42", Bps(500), 7);
    assert_eq!(a, audit::should_audit("decision-42", Bps(500), 7));
}

#[test]
fn r18_screen_task_saves_hk12_and_is_admitted() {
    let m = screen_task(300);
    assert_eq!(prevention::claims_avoided(&m), Money::from_major(12));
    assert!(prevention::admit(&m).is_ok());
}

#[test]
fn r18_task_failing_the_prevention_test_is_rejected() {
    let err = prevention::admit(&screen_task(1_200)).expect_err("reward equals claims avoided");
    assert!(err.to_string().contains("R18"));
}

#[test]
fn r19_banned_attribute_factor_rejected() {
    let factors = format!(
        "{FACTORS}  - {{ id: ethnicity_group, level: individual, basis: novel, status: candidate, data: declared, classes: [travel], formula: x }}\n"
    );
    assert_rule(load(INSURER, LINES, &factors, &CATALOGUE), "R19");
}

#[test]
fn r20_data_used_only_with_opt_in() {
    let cfg = cfg();
    let mut consents: BTreeSet<String> = BTreeSet::new();
    assert!(!consent::may_use(
        "device_diagnostics",
        "device_damage",
        &consents,
        &cfg
    ));
    consents.insert("device_diagnostics".into());
    assert!(consent::may_use(
        "device_diagnostics",
        "device_damage",
        &consents,
        &cfg
    ));
    // Purpose-bound: not for a cover it does not support.
    assert!(!consent::may_use(
        "device_diagnostics",
        "weather_income",
        &consents,
        &cfg
    ));
    consents.insert("location".into());
    assert!(!consent::may_use(
        "location",
        "device_damage",
        &consents,
        &cfg
    ));
}

#[test]
fn r24_unapproved_cover_rejected() {
    let cfg = cfg();
    assert_rule(catalogue::check_bundled("contents", &cfg), "R24");
    assert!(
        catalogue::enabled(&cfg, 9)
            .iter()
            .all(|c| c.id != "contents")
    );
    let insurer = INSURER.replace("extra_covers: [flight_delay]", "extra_covers: [contents]");
    assert_rule(load(&insurer, LINES, FACTORS, &CATALOGUE), "R24");
}

#[test]
fn r29_mul_bps_rounding() {
    assert_eq!(Money(5).mul_bps(Bps(5_000)), Money(3)); // 2.5 → 3, half-up
    assert_eq!(Money(-5).mul_bps(Bps(5_000)), Money(-3)); // away from zero
    assert_eq!(Money(4).mul_bps(Bps(5_000)), Money(2));
    assert_eq!(Money(1).mul_bps(Bps(4_999)), Money(0));
    assert_eq!(
        Money::from_major(80).mul_bps(Bps(2_000)),
        Money::from_major(16)
    );
    // Sums insured round down to whole HK$.
    assert_eq!(
        Rate(30_000).sum_insured(Money::from_major(10)),
        Money::from_major(333)
    );
}

#[test]
fn r29_config_decimals_are_exact_or_refused() {
    let cfg = cfg();
    assert_eq!(cfg.pricing.target_loss_ratio, Bps(5_000));
    assert_eq!(cfg.promotions[0].discount, Bps(2_000));
    let insurer = INSURER.replace("target_loss_ratio: 0.5", "target_loss_ratio: 0.55555");
    assert!(matches!(
        load(&insurer, LINES, FACTORS, &CATALOGUE),
        Err(EngineError::Config(_))
    ));
}

#[test]
fn r31_higher_risk_buys_less_cover_at_same_price() {
    let calm = World::new(&rider(), &basis(), oct());
    let mut risky_basis = basis();
    risky_basis
        .expected_cost
        .insert("weather_income".into(), flat_curve(40_000));
    let risky = World::new(&rider(), &risky_basis, oct());
    let p = premiums(&[
        ("weather_income", 10),
        ("personal_accident", 15),
        ("device_damage", 55),
    ]);
    let plan = calm.plan("essential");
    let a = optimiser::price_allocation(&plan, &p, None, &rider(), &calm.ctx()).expect("calm");
    let b = optimiser::price_allocation(&plan, &p, None, &rider(), &risky.ctx()).expect("risky");
    let si = |s: &Schedule| s.allocation("weather_income").map(|a| a.sum_insured);
    assert!(si(&b) < si(&a));
    assert_eq!(a.total_premium(), b.total_premium());
}

#[test]
fn r31_lower_risk_buys_more_cover_at_same_price() {
    let calm = World::new(&rider(), &basis(), oct());
    let mut safer_basis = basis();
    safer_basis
        .expected_cost
        .insert("device_damage".into(), flat_curve(2_000));
    let safer = World::new(&rider(), &safer_basis, oct());
    let p = premiums(&[("device_damage", 30), ("personal_accident", 10)]);
    let plan = calm.plan("starter");
    let a = optimiser::price_allocation(&plan, &p, None, &rider(), &calm.ctx()).expect("base");
    let b = optimiser::price_allocation(&plan, &p, None, &rider(), &safer.ctx()).expect("safer");
    let si = |s: &Schedule| s.allocation("device_damage").map(|a| a.sum_insured);
    assert!(si(&b) > si(&a));
}

fn peak_curve() -> BTreeMap<chrono::NaiveDate, Rate> {
    let mut c = flat_curve(10_000);
    c.insert(day(2026, 10, 1), Rate(40_000));
    c
}

#[test]
fn r32_new_customer_pays_max_of_period_and_horizon_average() {
    let cfg = cfg();
    let (own, avg) =
        rating::horizon_cost("weather_income", &peak_curve(), day(2026, 10, 1), 3).expect("costs");
    assert_eq!((own, avg), (Rate(40_000), Rate(20_000)));
    assert_eq!(rating::base_cost(own, avg, 0, &cfg), Rate(40_000));
    // Before the peak, the average already shows it (known future risk).
    let (_, sept_avg) =
        rating::horizon_cost("weather_income", &peak_curve(), day(2026, 9, 1), 3).expect("costs");
    assert_eq!(sept_avg, Rate(20_000));
}

#[test]
fn r31_period_starting_mid_month_takes_the_rate_in_force_that_day() {
    // Bound on 18 October, the policy period runs from the 19th: October's rate is in force,
    // then November's and December's for the next two periods (R32).
    let (own, avg) =
        rating::horizon_cost("weather_income", &peak_curve(), day(2026, 10, 19), 3).expect("costs");
    assert_eq!((own, avg), (Rate(40_000), Rate(20_000)));
    // A curve that has run out is an error, never a stale rate.
    let short: BTreeMap<_, _> = peak_curve()
        .into_iter()
        .filter(|(d, _)| *d <= day(2026, 11, 1))
        .collect();
    assert!(rating::horizon_cost("weather_income", &short, day(2026, 10, 19), 3).is_err());
    assert!(rating::horizon_cost("weather_income", &short, day(2026, 10, 19), 2).is_ok());
}

#[test]
fn r32_continuous_customer_pays_horizon_average() {
    let cfg = cfg();
    let (own, avg) =
        rating::horizon_cost("weather_income", &peak_curve(), day(2026, 10, 1), 3).expect("costs");
    assert_eq!(rating::base_cost(own, avg, 3, &cfg), Rate(20_000));
}

#[test]
fn r33_unregistered_factor_rejected() {
    let cfg = cfg();
    let cover = cfg.cover("personal_accident").expect("cover");
    for unregistered in ["religion", "hazard_frequency"] {
        let mut i = inputs();
        i.factor_values.insert(unregistered.into(), "x".into());
        assert_rule(
            rating::rate_cover(
                cover,
                day(2026, 10, 1),
                &i,
                &BTreeSet::new(),
                &basis(),
                &cfg,
            ),
            "R33",
        );
    }
}

#[test]
fn r33_withheld_factor_rated_at_undeclared_group_average() {
    let cfg = cfg();
    let rate = |i: &RatingInputs, cover: &str, consents: &BTreeSet<String>| {
        let c = cfg.cover(cover).expect("cover");
        rating::rate_cover(c, day(2026, 10, 1), i, consents, &basis(), &cfg).expect("rated")
    };
    let mut withheld = inputs();
    withheld.factor_values.remove("age_band");
    // Undeclared group 1.2 on half the credible exposure: blended to 1.1.
    let r = rate(&withheld, "personal_accident", &BTreeSet::new());
    assert_eq!(r.relativity, Bps(11_000));
    assert!(
        r.factors
            .iter()
            .any(|f| f.factor_id == "age_band" && f.withheld)
    );
    // A consented factor without its opt-in counts as withheld (R20).
    let mut trip = inputs();
    trip.factor_values
        .insert("trip_plan".into(), "short".into());
    let no_consent = rate(&trip, "flight_delay", &BTreeSet::new());
    assert_eq!(no_consent.relativity, Bps(11_000));
    let consented = rate(
        &trip,
        "flight_delay",
        &["calendar_plans".to_string()].into(),
    );
    assert_eq!(consented.relativity, Bps(10_000));
}

#[test]
fn r34_unchanged_allocation_carries_over_re_rated() {
    let w = World::new(&rider(), &basis(), oct());
    let current = suggest(&w, "essential", &rider()).schedule;
    let mut dearer = basis();
    dearer
        .expected_cost
        .insert("device_damage".into(), flat_curve(5_000));
    let nov = World::new(&rider(), &dearer, period(2, 2026, 11));
    let renewed =
        optimiser::renew(&current, &nov.plan("essential"), &rider(), &nov.ctx()).expect("renewal");
    assert_eq!(renewed.unallocated, Money::ZERO);
    for a in &current.allocations {
        assert_eq!(renewed.schedule.premium(&a.cover_id), a.premium);
    }
    let si = |s: &Schedule| s.allocation("device_damage").map(|a| a.sum_insured);
    assert!(si(&renewed.schedule) < si(&current));
}

#[test]
fn r34_whole_plan_price_to_one_cover_allowed() {
    let w = World::new(&rider(), &basis(), oct());
    let all_in = premiums(&[("device_damage", 40)]);
    let s = optimiser::price_allocation(&w.plan("starter"), &all_in, None, &rider(), &w.ctx())
        .expect("one cover is fine");
    assert_eq!(s.allocations.len(), 1);
    // Up to the product limit only.
    let over = premiums(&[("device_damage", 80)]);
    assert_rule(
        optimiser::price_allocation(&w.plan("essential"), &over, None, &rider(), &w.ctx()),
        "R34",
    );
    let short = premiums(&[("device_damage", 35)]);
    assert_rule(
        optimiser::price_allocation(&w.plan("starter"), &short, None, &rider(), &w.ctx()),
        "R34",
    );
}

#[test]
fn r35_new_cover_amounts_shown_before_period() {
    let w = World::new(&rider(), &basis(), oct());
    let current = suggest(&w, "essential", &rider()).schedule;
    let nov_period = period(2, 2026, 11);
    let nov = World::new(&rider(), &basis(), nov_period);
    let renewed =
        optimiser::renew(&current, &nov.plan("essential"), &rider(), &nov.ctx()).expect("renewal");
    let notice = consent::renewal_notice(&current, &renewed, &rider(), &nov_period, &nov.cfg);
    assert_eq!(notice.issue_by, day(2026, 10, 25));
    assert_eq!(notice.changes.len(), current.allocations.len());
    assert!(consent::check_notice_served(day(2026, 10, 25), nov_period.starts, &nov.cfg).is_ok());
    assert_rule(
        consent::check_notice_served(day(2026, 10, 26), nov_period.starts, &nov.cfg),
        "R35",
    );
}

#[test]
fn r36_points_never_cross_points_pool() {
    let cfg = cfg();
    let ok = Verdict {
        passed: true,
        reason: "ok".into(),
        reuse_suspected: false,
    };
    let device =
        prevention::complete(&screen_task(300), &ok, day(2026, 10, 1), &cfg).expect("verified");
    let by_pool = score::points_by_pool(&[device], day(2026, 10, 1), &[], &cfg);
    assert_eq!(by_pool.keys().collect::<Vec<_>>(), vec!["property"]);

    let w = World::new(&rider(), &basis(), oct());
    let base = suggest(&w, "essential", &rider()).schedule;
    let effects: BTreeMap<String, Bps> = [("property".to_string(), Bps(2_000))].into();
    let out = score::apply(&effects, ScoreMode::BonusSumInsured, &base, &w.cfg);
    for (b, a) in base.allocations.iter().zip(&out.schedule.allocations) {
        if a.cover_id == "device_damage" {
            assert!(a.sum_insured > b.sum_insured);
        } else {
            assert_eq!(
                a.sum_insured, b.sum_insured,
                "{} is outside the pool",
                a.cover_id
            );
        }
    }
}

#[test]
fn r37_cover_with_unknown_class_rejected() {
    let bad = DEVICE.replace("property.device", "property.gadgets");
    assert_rule(load(INSURER, LINES, FACTORS, &[bad.as_str()]), "R37");
}

#[test]
fn r37_cover_on_non_leaf_class_rejected() {
    let bad = DEVICE.replace(
        "class_of_business: property.device",
        "class_of_business: property",
    );
    let err = load(INSURER, LINES, FACTORS, &[bad.as_str()]).expect_err("non-leaf");
    assert!(err.to_string().contains("R37") && err.to_string().contains("property"));
}

#[test]
fn r37_class_tree_needs_parents_and_unique_ids() {
    let orphan = LINES.replace("  - { id: travel, name: Travel }\n", "");
    assert_rule(load(INSURER, &orphan, FACTORS, &[]), "R37");
    let dup = format!("{LINES}  - {{ id: travel.trip, name: Trip again }}\n");
    assert_rule(load(INSURER, &dup, FACTORS, &[]), "R37");
}

#[test]
fn r39_class_from_relativity_thresholds() {
    let cfg = cfg();
    let class = |bps| rating::risk_class(Bps(bps), &cfg).id;
    assert_eq!(class(8_000), "preferred");
    assert_eq!(class(8_001), "standard");
    assert_eq!(class(12_500), "standard");
    assert_eq!(class(20_000), "substandard");
    assert_eq!(class(20_001), "postponed");
}

#[test]
fn r39_common_shock_cancels_against_peers() {
    let cfg = cfg();
    // A typhoon season: 3 payouts in 12 periods, and peers had the same.
    let e = experience(3, 0, 12, 250_000);
    assert_eq!(
        rating::experience_factor(&e, day(2026, 10, 1), Bps::ONE, &cfg),
        Bps::ONE
    );
}

#[test]
fn r39_frequent_parametric_payouts_above_threshold_count() {
    let cfg = cfg();
    let mut i = inputs();
    i.classes.insert(
        "pecuniary.commute".into(),
        ClassHistory {
            continuous_periods: 12,
            experience: experience(6, 0, 12, 83_333),
        },
    );
    let cover = cfg.cover("commute_disruption").expect("cover");
    let r = rating::rate_cover(
        cover,
        day(2026, 10, 1),
        &i,
        &BTreeSet::new(),
        &basis(),
        &cfg,
    )
    .expect("rated");
    assert!(
        r.experience > Bps(50_000),
        "six times peers: {:?}",
        r.experience
    );
    assert!(r.risk_class.is_postponed());
    // Below the threshold it does not count.
    let mild = experience(1, 0, 12, 70_000);
    assert_eq!(
        rating::experience_factor(&mild, day(2026, 10, 1), Bps::ONE, &cfg),
        Bps::ONE
    );
}

#[test]
fn r39_no_cross_class_effect_without_link() {
    let cfg = cfg();
    let heavy = ClassHistory {
        continuous_periods: 12,
        experience: experience(6, 0, 12, 83_333),
    };
    let mut i = inputs();
    i.classes.insert("pecuniary.commute".into(), heavy.clone());
    let rate = |i: &RatingInputs, cover: &str| {
        let c = cfg.cover(cover).expect("cover");
        rating::rate_cover(c, day(2026, 10, 1), i, &BTreeSet::new(), &basis(), &cfg).expect("rated")
    };
    assert_eq!(rate(&i, "weather_income").experience, Bps::ONE);
    // The configured link moves flight delay by half of the trip experience.
    let mut linked = inputs();
    linked.classes.insert("travel.trip".into(), heavy);
    let flight = rate(&linked, "flight_delay");
    assert!(flight.experience > Bps::ONE);
    assert!(flight.experience < rate(&i, "commute_disruption").experience);
}

#[test]
fn r39_claims_under_review_count_only_once_decided() {
    let cfg = cfg();
    let pending = experience(0, 6, 12, 83_333);
    assert_eq!(
        rating::experience_factor(&pending, day(2026, 10, 1), Bps::ONE, &cfg),
        Bps::ONE
    );
    let decided = experience(6, 0, 12, 83_333);
    assert!(rating::experience_factor(&decided, day(2026, 10, 1), Bps::ONE, &cfg) > Bps::ONE);
}

#[test]
fn r41_postponed_class_takes_no_allocation() {
    let mut w = World::new(&rider(), &basis(), oct());
    w.statuses.insert(
        "property.device".into(),
        ClassStatus::Postponed {
            until: day(2027, 1, 1),
        },
    );
    let out = suggest(&w, "essential", &rider());
    assert_eq!(out.schedule.premium("device_damage"), Money::ZERO);
    let p = premiums(&[("device_damage", 25), ("personal_accident", 15)]);
    assert_rule(
        optimiser::price_allocation(&w.plan("starter"), &p, None, &rider(), &w.ctx()),
        "R41",
    );
}

#[test]
fn r41_postponement_starts_after_notice_and_ends_with_reassessment() {
    let cfg = cfg();
    let postponed = RiskClass {
        id: "postponed".into(),
        max_relativity: None,
    };
    let standard = rating::risk_class(Bps::ONE, &cfg);
    let review =
        |s: &ClassStatus, r: &RiskClass, y, m| underwriting::review(s, r, day(y, m, 1), &cfg);

    let notice = review(&ClassStatus::default(), &postponed, 2026, 11);
    assert_eq!(
        notice,
        ClassStatus::ExclusionNotice {
            excluded_from: day(2026, 12, 1)
        }
    );
    let off = review(&notice, &postponed, 2026, 12);
    assert_eq!(
        off,
        ClassStatus::Postponed {
            until: day(2027, 3, 1)
        }
    );
    // Not reassessed during the postponement, even if risk falls.
    assert_eq!(review(&off, &standard, 2027, 1), off);
    // Reassessed when it ends.
    assert_eq!(
        review(&off, &standard, 2027, 3),
        ClassStatus::Open {
            reentry_waiting_until: Some(day(2027, 3, 15))
        }
    );
    assert_eq!(
        review(&off, &postponed, 2027, 3),
        ClassStatus::Postponed {
            until: day(2027, 6, 1)
        }
    );
}

#[test]
fn r41_exclusion_applies_only_if_that_periods_underwriting_still_supports_it() {
    // The policy renews every period; each period's terms come from its own re-underwriting.
    let cfg = cfg();
    let standard = rating::risk_class(Bps::ONE, &cfg);
    let notice = ClassStatus::ExclusionNotice {
        excluded_from: day(2026, 12, 1),
    };
    assert_eq!(
        underwriting::review(&notice, &standard, day(2026, 12, 1), &cfg),
        ClassStatus::default()
    );
}

#[test]
fn r41_existing_cover_runs_to_period_end() {
    let w = World::new(&rider(), &basis(), oct());
    let current = suggest(&w, "essential", &rider()).schedule;
    let mut nov = World::new(&rider(), &basis(), period(2, 2026, 11));
    nov.statuses.insert(
        "property.device".into(),
        ClassStatus::ExclusionNotice {
            excluded_from: day(2026, 12, 1),
        },
    );
    // During the notice the cover continues as it was...
    let renewed =
        optimiser::renew(&current, &nov.plan("essential"), &rider(), &nov.ctx()).expect("renewal");
    assert_eq!(
        renewed.schedule.premium("device_damage"),
        current.premium("device_damage")
    );
    // ...but cannot grow.
    let mut more = BTreeMap::new();
    for a in &current.allocations {
        more.insert(a.cover_id.clone(), a.premium);
    }
    let shift = Money::from_major(5);
    let donor = current
        .allocations
        .iter()
        .find(|a| a.cover_id != "device_damage")
        .expect("another cover");
    more.insert(donor.cover_id.clone(), donor.premium - shift);
    more.insert(
        "device_damage".into(),
        current.premium("device_damage") + shift,
    );
    assert_rule(
        optimiser::price_allocation(
            &nov.plan("essential"),
            &more,
            Some(&current),
            &rider(),
            &nov.ctx(),
        ),
        "R41",
    );
}

#[test]
fn r41_freed_premium_renews_into_cover_and_is_shown() {
    let w = World::new(&rider(), &basis(), oct());
    let current = suggest(&w, "essential", &rider()).schedule;
    let mut dec = World::new(&rider(), &basis(), period(3, 2026, 12));
    dec.statuses.insert(
        "property.device".into(),
        ClassStatus::Postponed {
            until: day(2027, 3, 1),
        },
    );
    let renewed =
        optimiser::renew(&current, &dec.plan("essential"), &rider(), &dec.ctx()).expect("renewal");
    assert_eq!(renewed.schedule.premium("device_damage"), Money::ZERO);
    // Usable cover first (more commute cover), then where the limits allow (event ticket).
    assert!(renewed.schedule.premium("commute_disruption") > current.premium("commute_disruption"));
    assert_eq!(renewed.placed_beyond_need, Money::from_major(10));
    let notice = consent::renewal_notice(&current, &renewed, &rider(), &dec.period, &dec.cfg);
    assert_eq!(notice.placed_beyond_need, Money::from_major(10));
    assert!(notice.update_request.is_some());
}

#[test]
fn r42_premium_beyond_need_asks_customer_to_update_else_default_applies() {
    // A rider on the largest plan: usable cover runs out before the plan price does. Other
    // combinations remain, so this is not the R42 exit: the customer is asked to update in the
    // app or web, and the optimiser's default covers the period meanwhile.
    let w = World::new(&rider(), &basis(), oct());
    let out = suggest(&w, "max", &rider());
    assert_eq!(out.schedule.total_premium(), w.plan("max").price);
    assert!(out.placed_beyond_need > Money::ZERO);
    assert!(out.needs_customer_update());
    assert!(!out.may_change_plan_or_cancel_without_penalty());
    let flight = out
        .alternatives
        .iter()
        .find(|a| a.cover_id == "flight_delay")
        .expect("declaring a trip makes flight delay cover useful");
    assert_eq!(flight.exposure.kind, ExposureKind::UpcomingEvent);
    assert!(flight.exposure.of.contains("trip.air"));

    // At renewal: the notice asks for an update by the day before the period.
    let nov = period(2, 2026, 11);
    let notice = consent::renewal_notice(&out.schedule, &out, &rider(), &nov, &w.cfg);
    let request = notice.update_request.expect("update requested");
    assert_eq!(request.respond_by, day(2026, 10, 31));
    assert!(!request.alternatives.is_empty());

    // No update: the default applies. An accepted reallocation for the period replaces it.
    assert_eq!(
        consent::schedule_for_period(&out.schedule, None, nov.index),
        out.schedule
    );
    let mine = Schedule {
        plan_id: "max".into(),
        allocations: vec![],
    };
    let update = consent::Endorsement {
        policy_id: "pol-1".into(),
        effective_period: nov.index,
        change: OfferKind::Reallocation {
            schedule: mine.clone(),
        },
    };
    assert_eq!(
        consent::schedule_for_period(&out.schedule, Some(&update), nov.index),
        mine
    );
    assert_eq!(
        consent::schedule_for_period(&out.schedule, Some(&update), nov.index + 1),
        out.schedule
    );
}

#[test]
fn r42_usable_allocation_needs_no_update() {
    let w = World::new(&rider(), &basis(), oct());
    let out = suggest(&w, "essential", &rider());
    assert!(!out.needs_customer_update());
    let notice =
        consent::renewal_notice(&out.schedule, &out, &rider(), &period(2, 2026, 11), &w.cfg);
    assert!(notice.update_request.is_none());
}

#[test]
fn r42_plan_price_beyond_capacity_allows_change_or_cancel() {
    // After a postponement the remaining covers' limits cannot take the plan price in any split.
    let w = World::new(&rider(), &basis(), oct());
    let current = suggest(&w, "essential", &rider()).schedule;
    let mut dec = World::new(&rider(), &basis(), period(3, 2026, 12));
    dec.statuses.insert(
        "property.device".into(),
        ClassStatus::Postponed {
            until: day(2027, 3, 1),
        },
    );
    let renewed =
        optimiser::renew(&current, &dec.plan("essential"), &rider(), &dec.ctx()).expect("renewal");
    assert_eq!(renewed.beyond_capacity, Money::from_major(15));
    assert_eq!(renewed.unallocated, Money::from_major(15));
    assert!(renewed.may_change_plan_or_cancel_without_penalty());
    let notice = consent::renewal_notice(&current, &renewed, &rider(), &dec.period, &dec.cfg);
    assert!(notice.may_change_plan_or_cancel_without_penalty);
}

#[test]
fn r43_candidate_factor_never_affects_terms() {
    let cfg = cfg();
    let cover = cfg.cover("device_damage").expect("cover");
    let consents: BTreeSet<String> = ["device_diagnostics".to_string()].into();
    let mut poor = inputs();
    poor.factor_values
        .insert("device_condition".into(), "poor".into());
    let with = rating::rate_cover(cover, day(2026, 10, 1), &poor, &consents, &basis(), &cfg)
        .expect("rated");
    let without = rating::rate_cover(
        cover,
        day(2026, 10, 1),
        &inputs(),
        &consents,
        &basis(),
        &cfg,
    )
    .expect("rated");
    assert_eq!(with.rate, without.rate);
    assert_eq!(with.relativity, Bps::ONE);
    assert_eq!(with.shadow_relativity, Bps(15_000));
    // A candidate with no relativity table yet is skipped; an approved factor is not.
    let mut no_tables = basis();
    no_tables.factor_tables.remove("device_condition");
    assert!(
        rating::rate_cover(cover, day(2026, 10, 1), &poor, &consents, &no_tables, &cfg).is_ok()
    );
    no_tables.factor_tables.remove("age_band");
    let accident = cfg.cover("personal_accident").expect("cover");
    assert_rule(
        rating::rate_cover(
            accident,
            day(2026, 10, 1),
            &inputs(),
            &consents,
            &no_tables,
            &cfg,
        ),
        "R33",
    );
}

#[test]
fn r43_industry_factor_cannot_be_candidate() {
    let factors = FACTORS.replace(
        "id: trip_plan, level: individual, basis: industry, status: approved",
        "id: trip_plan, level: individual, basis: industry, status: candidate",
    );
    assert_rule(load(INSURER, LINES, &factors, &CATALOGUE), "R43");
}

#[test]
fn r44_promotion_applies_to_every_eligible_customer() {
    let cfg = cfg();
    let essential = plan(&cfg, "essential");
    let new_via_partner = plans::PurchaseContext {
        channel: Some("partner_app".into()),
        new_customer: true,
    };
    let jan = day(2027, 1, 15);
    let a = plans::price(&essential, &new_via_partner, jan, &cfg);
    let b = plans::price(&essential, &new_via_partner.clone(), jan, &cfg);
    assert_eq!(a, b);
    assert_eq!(a.discount, Money::from_major(16));
    assert_eq!(a.payable, Money::from_major(64));
    let existing = plans::PurchaseContext {
        new_customer: false,
        ..new_via_partner.clone()
    };
    assert_eq!(
        plans::price(&essential, &existing, jan, &cfg).discount,
        Money::ZERO
    );
    assert_eq!(
        plans::price(&essential, &new_via_partner, day(2027, 4, 1), &cfg).discount,
        Money::ZERO
    );
}

#[test]
fn r44_plan_price_changes_only_by_choice_or_promotion() {
    let w = World::new(&rider(), &basis(), oct());
    let mut riskier = basis();
    riskier
        .expected_cost
        .insert("personal_accident".into(), flat_curve(2_000));
    let r = World::new(&rider(), &riskier, oct());
    for world in [&w, &r] {
        let out = suggest(world, "essential", &rider());
        assert_eq!(out.schedule.total_premium(), Money::from_major(80));
    }
}

#[test]
fn r45_optimiser_never_changes_plan() {
    let w = World::new(&rider(), &basis(), oct());
    for plan in ["starter", "essential", "max"] {
        let out = suggest(&w, plan, &rider());
        assert_eq!(out.schedule.plan_id, plan);
        let renewed =
            optimiser::renew(&out.schedule, &w.plan(plan), &rider(), &w.ctx()).expect("renewal");
        assert_eq!(renewed.schedule.plan_id, plan);
    }
}

#[test]
fn r45_privileges_come_from_config() {
    let w = World::new(&rider(), &basis(), oct());
    assert!(plans::covers_for(&w.plan("max"), &w.cfg).contains(&"flight_delay".to_string()));
    assert!(!plans::covers_for(&w.plan("essential"), &w.cfg).contains(&"flight_delay".to_string()));
    let p = premiums(&[
        ("flight_delay", 10),
        ("personal_accident", 15),
        ("device_damage", 55),
    ]);
    assert_rule(
        optimiser::price_allocation(&w.plan("essential"), &p, None, &rider(), &w.ctx()),
        "R45",
    );
}

#[test]
fn r47_reentry_after_postponement_waits() {
    let cfg = cfg();
    let device = cfg.cover("device_damage").expect("cover");
    let reopened = ClassStatus::Open {
        reentry_waiting_until: Some(day(2027, 3, 15)),
    };
    assert_eq!(
        catalogue::covered_from(device, day(2027, 3, 1), &reopened),
        day(2027, 3, 15)
    );
    let accident = cfg.cover("personal_accident").expect("cover");
    assert_eq!(
        catalogue::covered_from(accident, day(2027, 3, 1), &reopened),
        day(2027, 3, 15)
    );
    assert_eq!(
        catalogue::covered_from(accident, day(2027, 3, 1), &ClassStatus::default()),
        day(2027, 3, 1)
    );
}

#[test]
fn r48_notice_flags_sum_insured_below_declared_value() {
    let w = World::new(&rider(), &basis(), oct());
    let p = premiums(&[("device_damage", 25), ("personal_accident", 15)]);
    let s = optimiser::price_allocation(&w.plan("starter"), &p, None, &rider(), &w.ctx())
        .expect("priced");
    let out = AllocationOutcome {
        schedule: s.clone(),
        unallocated: Money::ZERO,
        beyond_capacity: Money::ZERO,
        placed_beyond_need: Money::ZERO,
        alternatives: vec![],
    };
    let notice = consent::renewal_notice(&s, &out, &rider(), &oct(), &w.cfg);
    let device = notice
        .changes
        .iter()
        .find(|c| c.cover_id == "device_damage")
        .expect("device line");
    assert_eq!(device.declared_value, Some(Money::from_major(4_000)));
    assert!(device.below_declared_value);
}

#[test]
fn r50_rebate_identical_for_every_qualifying_customer() {
    let w = World::new(&rider(), &basis(), oct());
    let p = premiums(&[("device_damage", 25), ("personal_accident", 15)]);
    let s = optimiser::price_allocation(&w.plan("starter"), &p, None, &rider(), &w.ctx())
        .expect("priced");
    let effects: BTreeMap<String, Bps> = [("property".to_string(), Bps(1_500))].into();
    let a = score::apply(&effects, ScoreMode::PremiumRebate, &s, &w.cfg);
    let b = score::apply(&effects, ScoreMode::PremiumRebate, &s.clone(), &w.cfg);
    assert_eq!(a.premium_rebate, b.premium_rebate);
    assert_eq!(a.premium_rebate, Money::from_major(25).mul_bps(Bps(1_500)));
}

#[test]
fn r52_unlicensed_partner_cannot_bind() {
    let cfg = cfg();
    let on = day(2026, 10, 1);
    assert!(consent::check_binding_partner(None, on, &cfg).is_ok());
    assert!(consent::check_binding_partner(Some("delivery_app"), on, &cfg).is_ok());
    assert_rule(
        consent::check_binding_partner(Some("blog"), on, &cfg),
        "R52",
    );
    assert_rule(
        consent::check_binding_partner(Some("unknown_app"), on, &cfg),
        "R52",
    );
}

#[test]
fn r52_expired_licence_cannot_bind() {
    let cfg = cfg();
    assert!(consent::check_binding_partner(Some("delivery_app"), day(2027, 12, 31), &cfg).is_ok());
    assert_rule(
        consent::check_binding_partner(Some("delivery_app"), day(2028, 1, 1), &cfg),
        "R52",
    );
}

fn new_business(partner: Option<&str>, created_at: DateTime<Utc>) -> (World, Offer) {
    let w = World::new(&rider(), &basis(), oct());
    let schedule = suggest(&w, "essential", &rider()).schedule;
    let o = Offer {
        created_at,
        ..offer(OfferKind::NewPolicy {
            schedule,
            partner_id: partner.map(str::to_string),
        })
    };
    (w, o)
}

fn tap_at(at: DateTime<Utc>) -> CustomerTap {
    CustomerTap {
        at,
        ..tap("offer-1", "cust-rider")
    }
}

#[test]
fn r58_tap_binds_from_the_next_local_day() {
    // 23:30 local on 9 October is 15:30 UTC: cover starts on 10 October, local time.
    let made = DateTime::parse_from_rfc3339("2026-10-09T15:00:00Z")
        .expect("time")
        .with_timezone(&Utc);
    let (w, o) = new_business(Some("delivery_app"), made);
    let tapped = made + Duration::minutes(30);
    let p = consent::bind(&o, &tap_at(tapped), "pol-new", &rider(), &w.cfg).expect("bound");
    assert_eq!(p.period.index, 1);
    assert_eq!(p.period.starts, day(2026, 10, 10));
    assert_eq!(p.period.ends, day(2026, 11, 10));
    assert_eq!(p.schedule.total_premium(), Money::from_major(80));
    assert!(p.cover_starts.values().all(|d| *d == day(2026, 10, 10)));
    assert_eq!(p.commute_routes, rider().commute_routes);
    assert_eq!(p.subscribed_at, tapped);
}

#[test]
fn r58_no_policy_without_the_customers_tap() {
    let (w, o) = new_business(None, at(2026, 10, 10));
    let later = at(2026, 10, 10) + Duration::hours(1);
    assert_rule(
        consent::bind(
            &o,
            &CustomerTap {
                customer_id: "someone-else".into(),
                ..tap_at(later)
            },
            "pol-new",
            &rider(),
            &w.cfg,
        ),
        "R03",
    );
    // A change to an existing policy is not new business, and new business is not a change.
    assert_rule(
        consent::bind(
            &offer(OfferKind::AddCover {
                cover_id: "device_damage".into(),
            }),
            &tap_at(later),
            "pol-new",
            &rider(),
            &w.cfg,
        ),
        "R58",
    );
    let pol = policy(&w.cfg, suggest(&w, "essential", &rider()).schedule, oct());
    assert_rule(consent::accept(&o, &tap_at(later), &pol, &w.cfg), "R58");
}

#[test]
fn r58_quote_is_valid_on_the_day_it_was_made() {
    // Made 23:00 local on 10 October, tapped 00:30 local on 11 October.
    let made = DateTime::parse_from_rfc3339("2026-10-10T15:00:00Z")
        .expect("time")
        .with_timezone(&Utc);
    let (w, o) = new_business(None, made);
    let tapped = made + Duration::minutes(90);
    assert_rule(
        consent::bind(&o, &tap_at(tapped), "pol-new", &rider(), &w.cfg),
        "R58",
    );
}

#[test]
fn r52_binding_needs_a_licence_on_the_day_of_the_tap() {
    for (partner, made, bound) in [
        (None, at(2026, 10, 10), true),
        (Some("delivery_app"), at(2026, 10, 10), true),
        (Some("delivery_app"), at(2028, 1, 1), false),
        (Some("blog"), at(2026, 10, 10), false),
        (Some("unknown_app"), at(2026, 10, 10), false),
    ] {
        let (w, o) = new_business(partner, made);
        let result = consent::bind(&o, &tap_at(made), "pol-new", &rider(), &w.cfg);
        if bound {
            assert!(result.is_ok(), "{partner:?}: {result:?}");
        } else {
            assert_rule(result, "R52");
        }
    }
}

#[test]
fn r58_schedule_totals_the_plan_price() {
    let (w, o) = new_business(None, at(2026, 10, 10));
    let OfferKind::NewPolicy { mut schedule, .. } = o.change.clone() else {
        panic!("new business");
    };
    schedule.allocations.pop();
    let short = Offer {
        change: OfferKind::NewPolicy {
            schedule,
            partner_id: None,
        },
        ..o
    };
    assert_rule(
        consent::bind(
            &short,
            &tap_at(at(2026, 10, 10)),
            "pol-new",
            &rider(),
            &w.cfg,
        ),
        "R34",
    );
}

#[test]
fn r53_claim_in_grace_paid_net_of_premium() {
    let cfg = cfg();
    let due = day(2026, 11, 1);
    assert_eq!(
        consent::premium_status(due, None, day(2026, 11, 8), &cfg),
        PremiumStatus::InGrace {
            last_day: day(2026, 11, 8)
        }
    );
    assert_eq!(
        consent::net_of_premium_due(Money::from_major(500), Money::from_major(80)),
        Money::from_major(420)
    );
}

#[test]
fn r53_lapse_after_grace() {
    let cfg = cfg();
    let due = day(2026, 11, 1);
    assert_eq!(
        consent::premium_status(due, None, day(2026, 11, 9), &cfg),
        PremiumStatus::Lapsed {
            since: day(2026, 11, 9),
            reinstate_by: day(2026, 12, 9)
        }
    );
    assert_eq!(
        consent::premium_status(due, Some(day(2026, 11, 5)), day(2026, 11, 20), &cfg),
        PremiumStatus::Paid
    );
}

#[test]
fn r53_reinstatement_within_window_keeps_waiting_periods_served() {
    let cfg = cfg();
    let due = day(2026, 11, 1);
    assert_eq!(
        consent::late_payment(due, day(2026, 11, 8), &cfg),
        LatePayment::InTime
    );
    assert_eq!(
        consent::late_payment(due, day(2026, 12, 9), &cfg),
        LatePayment::Reinstated
    );
    assert_eq!(
        consent::late_payment(due, day(2026, 12, 10), &cfg),
        LatePayment::NewPolicy
    );
}

#[test]
fn r54_optional_data_deleted_on_cancellation() {
    let cfg = cfg();
    let mut p = rider();
    consent_to(&mut p, "trip_data");
    consent_to(&mut p, "photos");
    let out = consent::on_cancellation(&p, day(2026, 10, 31), &cfg);
    assert_eq!(out.delete, p.consents);
    assert_eq!(out.retain_records_until, day(2033, 10, 31));
    let insurer = INSURER.replace(
        "delete_on_cancellation: true",
        "delete_on_cancellation: false",
    );
    assert_rule(load(&insurer, LINES, FACTORS, &CATALOGUE), "R54");
}

const PILOT: &str = "pilot:\n  id: test_pilot\n  starts: 2027-05-01\n  ends: 2027-10-31\n  gates_fixed_on: 2027-04-15\n  control: { share: 0.25, periods: 3 }\n  gates: [{ metric: loss_ratio, kind: stop, at_most: 0.75 }]\n";

#[test]
fn r59_a_wait_list_control_group_is_drawn_and_offered_tasks_later() {
    let cfg = config::from_yaml(&ConfigSources {
        pilot: Some(PILOT),
        ..sources()
    })
    .expect("the pilot loads");
    let p = cfg.pilot.as_ref();
    let bound = day(2027, 6, 1);
    let cohorts: Vec<Cohort> = (0..400)
        .map(|i| pilot::cohort(p, &format!("cust-{i}"), bound, 1, 7))
        .collect();
    let waiting = cohorts.iter().filter(|c| **c == Cohort::WaitList).count();
    assert!((70..=130).contains(&waiting), "about a quarter: {waiting}");
    // Replayable from the seed; another seed draws another group.
    let again: Vec<Cohort> = (0..400)
        .map(|i| pilot::cohort(p, &format!("cust-{i}"), bound, 1, 7))
        .collect();
    assert_eq!(cohorts, again);
    let other: Vec<Cohort> = (0..400)
        .map(|i| pilot::cohort(p, &format!("cust-{i}"), bound, 1, 8))
        .collect();
    assert_ne!(cohorts, other);
    let waiter = (0..400)
        .map(|i| format!("cust-{i}"))
        .find(|c| pilot::cohort(p, c, bound, 1, 7) == Cohort::WaitList)
        .expect("someone waits");
    // They join after `periods`; customers bound outside the pilot are never drawn; with no
    // pilot everyone is offered tasks.
    assert_eq!(pilot::cohort(p, &waiter, bound, 4, 7), Cohort::Offered);
    assert_eq!(
        pilot::cohort(p, &waiter, day(2027, 11, 1), 1, 7),
        Cohort::Offered
    );
    assert_eq!(pilot::cohort(None, &waiter, bound, 1, 7), Cohort::Offered);
    // A wait-listed customer is offered no task.
    assert_rule(
        prevention::offer(
            sized(&cfg),
            &waiter,
            "pol",
            day(2027, 6, 5),
            day(2027, 6, 20),
            &[],
            Cohort::WaitList,
            &cfg,
        ),
        "R59",
    );
    // Gates are fixed before the pilot starts, and a control group leaves someone to compare.
    for bad in [
        PILOT.replace("gates_fixed_on: 2027-04-15", "gates_fixed_on: 2027-05-01"),
        PILOT.replace("share: 0.25", "share: 1"),
        PILOT.replace("periods: 3", "periods: 0"),
    ] {
        assert_rule(
            config::from_yaml(&ConfigSources {
                pilot: Some(&bad),
                ..sources()
            }),
            "R59",
        );
    }
}

#[test]
fn r54_cancellation_erases_answers_from_optional_data_only() {
    let cfg = cfg();
    let mut p = rider();
    consent_to(&mut p, "calendar_plans");
    let out = consent::on_cancellation(&p, day(2026, 10, 31), &cfg);
    // The trip plan came from the calendar the customer opted into; the device condition from
    // diagnostics they never shared; the age band they declared stays with the records.
    let erased = consent::answers_to_erase(
        ["trip_plan", "device_condition", "age_band", "not_a_factor"],
        &out,
        &cfg,
    );
    assert_eq!(erased, BTreeSet::from(["trip_plan".to_string()]));
}

#[test]
fn r54_returning_customer_claims_count_only_within_window() {
    let cfg = cfg();
    let claims_at = |y, m| ClaimsExperience {
        claims: (0..6)
            .map(|_| ExperienceClaim {
                period_starts: day(y, m, 1),
                status: ClaimStatus::Paid,
            })
            .collect(),
        exposure_periods: 12,
        peer_frequency_ppm: 83_333,
    };
    let back_on = day(2026, 10, 1);
    assert!(rating::experience_factor(&claims_at(2025, 11), back_on, Bps::ONE, &cfg) > Bps::ONE);
    assert_eq!(
        rating::experience_factor(&claims_at(2025, 9), back_on, Bps::ONE, &cfg),
        Bps::ONE
    );
}

#[test]
fn r55_source_codes_classify_to_registered_event_types_and_stages() {
    let cfg = cfg();
    let signal8 = in_force(&cfg, "TC8NE");
    let o = signal8.iter().next().expect("classified");
    assert_eq!(
        (o.event_type.as_str(), o.stage),
        ("weather.tropical_cyclone", 2)
    );
    let weather = cfg.cover("weather_income").expect("cover");
    let pays = |code: &str| {
        in_force(&cfg, code).iter().any(|o| {
            weather
                .trigger
                .as_ref()
                .is_some_and(|t| t.events.iter().any(|c| triggers::meets(o, c)))
        })
    };
    assert!(pays("TC8NE") && pays("TC10") && pays("WRAINB"));
    assert!(!pays("TC3") && !pays("WRAINA"));
    // An unregistered code is ignored, never guessed.
    assert!(in_force(&cfg, "WXYZ").is_empty());
}

#[test]
fn r55_cover_names_only_registered_event_types_and_stages() {
    let unknown_type = WEATHER.replace(
        "weather.rainstorm, from_stage: black",
        "weather.hail, from_stage: black",
    );
    assert_rule(
        load(INSURER, LINES, FACTORS, &[unknown_type.as_str()]),
        "R55",
    );
    let unknown_stage = WEATHER.replace("from_stage: black", "from_stage: purple");
    assert_rule(
        load(INSURER, LINES, FACTORS, &[unknown_stage.as_str()]),
        "R55",
    );
    // A delay is meaningless without its minutes.
    let no_minutes = FLIGHT.replace(", min_minutes: 120", "");
    assert_rule(load(INSURER, LINES, FACTORS, &[no_minutes.as_str()]), "R55");
}

#[test]
fn r55_source_codes_map_to_registered_event_types() {
    let bad = HKO.replace(
        "weather.rainstorm, stage: black",
        "weather.hail, stage: black",
    );
    let sources = [bad.as_str(), TRANSIT, FLIGHTS, ORGANISERS, HEAT];
    assert_rule(
        load_registries(
            INSURER,
            LINES,
            FACTORS,
            EVENT_TYPES,
            &sources,
            EXPOSURES,
            &CATALOGUE,
        ),
        "R55",
    );
}

#[test]
fn r55_offered_cover_needs_an_available_source_and_a_pricing_basis() {
    // The ticketing feed is not yet agreed: the event ticket cover cannot be offered.
    let pending = ORGANISERS.replace("status: available", "status: pending_agreement");
    let sources = [HKO, TRANSIT, FLIGHTS, pending.as_str(), HEAT];
    let err = load_registries(
        INSURER,
        LINES,
        FACTORS,
        EVENT_TYPES,
        &sources,
        EXPOSURES,
        &CATALOGUE,
    )
    .expect_err("no available source");
    assert!(err.to_string().contains("R55") && err.to_string().contains("event_ticket"));
    // Priced on the historical record, but no source has history.
    let no_history = HKO.replace("history: { available: true, since: 1946 }", "");
    let sources = [no_history.as_str(), TRANSIT, FLIGHTS, ORGANISERS, HEAT];
    assert_rule(
        load_registries(
            INSURER,
            LINES,
            FACTORS,
            EVENT_TYPES,
            &sources,
            EXPOSURES,
            &CATALOGUE,
        ),
        "R55",
    );
}

#[test]
fn r55_new_kinds_are_config_only_and_profiles_use_registered_ids() {
    let cfg = cfg();
    let event = cfg.cover("event_ticket").expect("cover");
    // A sports match counts for event ticket cover through its parent kind `event`.
    let mut fan = rider();
    fan.upcoming_events[0].kind = "event.sports".into();
    assert!(optimiser::need(event, &fan, &oct()) > Money::ZERO);
    assert!(cfg.check_profile(&fan).is_ok());
    // A kind no one registered is refused.
    fan.upcoming_events[0].kind = "event.unicorn_race".into();
    assert_rule(cfg.check_profile(&fan), "R55");
    let mut unknown_data = rider();
    consent_to(&mut unknown_data, "brainwaves");
    assert_rule(cfg.check_profile(&unknown_data), "R55");
    // A new kind is a config entry, not code.
    let more = EXPOSURES.replacen(
        "  - { id: event.concert, name: Concert }",
        "  - { id: event.concert, name: Concert }\n  - { id: event.unicorn_race, name: Unicorn race }",
        1,
    );
    let extended = load_registries(
        INSURER,
        LINES,
        FACTORS,
        EVENT_TYPES,
        &SOURCES,
        &more,
        &CATALOGUE,
    )
    .expect("loads");
    assert!(extended.check_profile(&fan).is_ok());
}

#[test]
fn r55_tiered_payout_follows_the_event_types_stages() {
    let heat = r#"
id: heat_stress_income
name: Heat stress income cover
type: parametric
class_of_business: pecuniary.income_weather
launch_phase: concept
key_features: "Paid when heat stress cuts your hours"
exposure: { kind: declared_hours }
trigger:
  events: [{ event_type: workplace.heat_stress, from_stage: red }]
  requires_declared_hours: true
payout: { basis: tiered_by_stage, tiers: { red: 0.5, black: 1.0 } }
"#;
    let cfg = load(
        INSURER,
        LINES,
        FACTORS,
        &[WEATHER, COMMUTE, EVENT, DEVICE, ACCIDENT, FLIGHT, heat],
    )
    .expect("tiered cover loads");
    let tiers = &cfg
        .cover("heat_stress_income")
        .expect("cover")
        .payout
        .as_ref()
        .expect("payout")
        .tiers;
    assert_eq!(tiers.get(&1), Some(&Bps(5_000)));
    assert_eq!(tiers.get(&2), Some(&Bps(10_000)));
    for bad in [
        heat.replace("{ red: 0.5, black: 1.0 }", "{ amber: 0.2, black: 1.0 }"),
        heat.replace("{ red: 0.5, black: 1.0 }", "{ red: 1.0, black: 0.5 }"),
    ] {
        assert_rule(
            load(
                INSURER,
                LINES,
                FACTORS,
                &[WEATHER, COMMUTE, EVENT, DEVICE, ACCIDENT, FLIGHT, &bad],
            ),
            "R55",
        );
    }
}

// ---- M5 automatic trigger payouts

/// Hong Kong local time (UTC+8) on a day of October 2026, as UTC.
fn hk(d: u32, h: u32, m: u32) -> DateTime<Utc> {
    day(2026, 10, d)
        .and_hms_opt(h, m, 0)
        .expect("valid time")
        .and_utc()
        - Duration::hours(8)
}

fn report(source: &str, code: &str, starts: DateTime<Utc>, zone: Option<&str>) -> SourceReport {
    SourceReport {
        report_id: format!("{source}-{code}-{starts}"),
        source: source.into(),
        code: code.into(),
        starts,
        ends: None,
        zone: zone.map(Into::into),
        delay_minutes: None,
        reference: None,
    }
}

type Events = BTreeMap<String, TriggerEvent>;

/// Polls of one source: each `(time, codes in force)`, folded into `events`.
fn poll_into(
    events: &mut Events,
    cfg: &EngineConfig,
    source: &str,
    steps: &[(DateTime<Utc>, &[&str])],
) -> Vec<triggers::Tracked> {
    steps
        .iter()
        .map(|(now, codes)| {
            let reports: Vec<SourceReport> = codes
                .iter()
                .map(|c| report(source, c, *now, None))
                .collect();
            triggers::track(events, source, &reports, *now, cfg)
        })
        .collect()
}

fn polls(cfg: &EngineConfig, source: &str, steps: &[(DateTime<Utc>, &[&str])]) -> Events {
    let mut events = BTreeMap::new();
    poll_into(&mut events, cfg, source, steps);
    events
}

/// The loss occurrences of closed events, grouped after every aggregation window has passed.
fn occurrences(events: &Events, cfg: &EngineConfig) -> Vec<LossOccurrence> {
    let mut out = BTreeMap::new();
    triggers::group(&mut out, events, hk(31, 0, 0), cfg);
    out.into_values().collect()
}

fn only(occurrences: &[LossOccurrence]) -> &LossOccurrence {
    assert_eq!(occurrences.len(), 1, "one loss occurrence: {occurrences:?}");
    &occurrences[0]
}

/// A weather income policy: HK$`si` benefit a day, working the given ISO weekdays 09:00–18:00.
fn weather_policy(cfg: &EngineConfig, id: &str, si: i64, weekdays: &[u8]) -> Policy {
    let schedule = Schedule {
        plan_id: "essential".into(),
        allocations: vec![Allocation {
            cover_id: "weather_income".into(),
            premium: Money::from_major(40),
            sum_insured: Money::from_major(si),
        }],
    };
    let mut p = policy(cfg, schedule, oct());
    p.policy_id = id.into();
    p.work_windows = weekdays
        .iter()
        .map(|d| WorkWindow {
            weekday: *d,
            start_minute: 9 * 60,
            end_minute: 18 * 60,
        })
        .collect();
    p
}

fn matches_at(
    occurrence: &LossOccurrence,
    events: &Events,
    policies: &[Policy],
    as_of: DateTime<Utc>,
    cfg: &EngineConfig,
) -> Vec<triggers::Match> {
    let index = triggers::build_index(policies, &cfg.catalogue);
    triggers::match_occurrence(occurrence, events, policies, &index, as_of, cfg)
}

/// What each policy is owed on the only occurrence in `events`, once it has closed.
fn owed(events: &Events, policies: &[Policy], cfg: &EngineConfig) -> BTreeMap<String, Money> {
    let occs = occurrences(events, cfg);
    matches_at(only(&occs), events, policies, hk(31, 0, 0), cfg)
        .into_iter()
        .map(|m| (m.policy_id, m.entitlement))
        .collect()
}

#[test]
fn r08_pays_only_in_declared_hours() {
    let cfg = cfg();
    let monday = [weather_policy(&cfg, "pol-mon", 500, &[1])];
    // 2026-10-05 is a Monday, 2026-10-04 a Sunday. The cover counts two hours to resume work
    // after the signal and needs an hour of declared work disrupted for the day to count.
    type Case = (u32, (u32, u32), (u32, u32), i64, &'static str);
    let cases: [Case; 6] = [
        (5, (10, 0), (14, 0), 500, "inside working hours"),
        (5, (19, 0), (23, 0), 0, "after work"),
        (
            5,
            (6, 0),
            (9, 0),
            500,
            "lowered at 09:00: two hours to resume work",
        ),
        (5, (6, 0), (7, 30), 0, "resumed by 09:30: half an hour lost"),
        (5, (17, 30), (23, 0), 0, "the last half hour of the shift"),
        (4, (10, 0), (14, 0), 0, "not a working day"),
    ];
    for (d, (fh, fm), (th, tm), expect, why) in cases {
        let events = polls(
            &cfg,
            "hko_warnings",
            &[(hk(d, fh, fm), &["TC8NE"]), (hk(d, th, tm), &[])],
        );
        assert_eq!(
            owed(&events, &monday, &cfg).get("pol-mon"),
            Some(&Money::from_major(expect)),
            "{why}"
        );
    }
    // A night shift (22:00–06:00) belongs to the day it starts.
    let mut night = weather_policy(&cfg, "pol-night", 500, &[1]);
    night.work_windows[0] = WorkWindow {
        weekday: 1,
        start_minute: 22 * 60,
        end_minute: 6 * 60,
    };
    let events = polls(
        &cfg,
        "hko_warnings",
        &[(hk(6, 2, 0), &["TC8NE"]), (hk(6, 5, 0), &[])],
    );
    assert_eq!(
        owed(&events, &[night], &cfg).get("pol-night"),
        Some(&Money::from_major(500))
    );
}

#[test]
fn r08_signal_9_and_10_match() {
    let cfg = cfg();
    let mut events = BTreeMap::new();
    let tracked = poll_into(
        &mut events,
        &cfg,
        "hko_warnings",
        &[
            (hk(5, 8, 0), &["TC3"]),
            (hk(5, 10, 0), &["TC8NE"]),
            (hk(5, 12, 0), &["TC9"]),
            (hk(5, 14, 0), &["TC10"]),
            (hk(6, 11, 0), &["TC3"]),
            (hk(6, 16, 0), &[]),
        ],
    );
    // One typhoon, raised and lowered: one trigger event.
    assert_eq!(events.len(), 1);
    assert_eq!(tracked[0].opened.len(), 1);
    assert!(tracked[1..].iter().all(|t| t.opened.is_empty()));
    let typhoon = events.values().next().expect("event");
    assert_eq!(typhoon.peak(), 4);
    assert!(typhoon.is_closed());
    // Signal 8 to 10 from Monday 10:00 to Tuesday 11:00: one day each for working days in it.
    let policies = [
        weather_policy(&cfg, "pol-mon", 500, &[1]),
        weather_policy(&cfg, "pol-mon-tue", 500, &[1, 2]),
    ];
    let paid = owed(&events, &policies, &cfg);
    assert_eq!(paid.get("pol-mon"), Some(&Money::from_major(500)));
    assert_eq!(paid.get("pol-mon-tue"), Some(&Money::from_major(1_000)));
    // Signal 3 alone pays nothing.
    let events = polls(
        &cfg,
        "hko_warnings",
        &[(hk(5, 10, 0), &["TC3"]), (hk(5, 16, 0), &[])],
    );
    assert_eq!(
        owed(&events, &policies, &cfg).get("pol-mon"),
        Some(&Money::ZERO)
    );
}

#[test]
fn r51_typhoon_and_its_rain_are_one_loss_occurrence() {
    let cfg = cfg();
    let policies = [
        weather_policy(&cfg, "pol-mon", 500, &[1]),
        weather_policy(&cfg, "pol-mon-tue", 500, &[1, 2]),
    ];
    // Signal 8 Monday 10:00–14:00, black rain Monday 15:00–17:00 and Tuesday 09:00–12:00.
    let mut events = BTreeMap::new();
    poll_into(
        &mut events,
        &cfg,
        "hko_warnings",
        &[
            (hk(5, 10, 0), &["TC8NE"]),
            (hk(5, 14, 0), &[]),
            (hk(5, 15, 0), &["WRAINB"]),
            (hk(5, 17, 0), &[]),
            (hk(6, 9, 0), &["WRAINB"]),
            (hk(6, 12, 0), &[]),
        ],
    );
    assert_eq!(events.len(), 3, "three trigger events");
    let occ = occurrences(&events, &cfg);
    let storm = only(&occ);
    assert_eq!(storm.peril, "windstorm_and_rain");
    assert_eq!(storm.events.len(), 3);
    // One daily benefit per day, however many of the peril's triggers touched it.
    let paid = owed(&events, &policies, &cfg);
    assert_eq!(paid.get("pol-mon"), Some(&Money::from_major(500)));
    assert_eq!(paid.get("pol-mon-tue"), Some(&Money::from_major(1_000)));
    // Black rain more than a day after the storm went quiet is a new occurrence.
    poll_into(
        &mut events,
        &cfg,
        "hko_warnings",
        &[(hk(9, 10, 0), &["WRAINB"]), (hk(9, 12, 0), &[])],
    );
    assert_eq!(occurrences(&events, &cfg).len(), 2);
    // The open occurrence closes only after its aggregation window passes quietly.
    let mut open = BTreeMap::new();
    let first = polls(
        &cfg,
        "hko_warnings",
        &[(hk(5, 10, 0), &["TC8NE"]), (hk(5, 14, 0), &[])],
    );
    let grouped = triggers::group(&mut open, &first, hk(5, 20, 0), &cfg);
    assert_eq!((grouped.opened.len(), grouped.closed.len()), (1, 0));
    let grouped = triggers::group(&mut open, &first, hk(6, 14, 0), &cfg);
    assert_eq!(grouped.closed.len(), 1);
}

/// A cover loaded beside the synthetic catalogue.
fn with_cover(yaml: &str) -> EngineConfig {
    load(
        INSURER,
        LINES,
        FACTORS,
        &[WEATHER, COMMUTE, EVENT, DEVICE, ACCIDENT, FLIGHT, yaml],
    )
    .expect("loads")
}

fn holding(cfg: &EngineConfig, cover: &str, si: i64) -> Policy {
    let mut p = weather_policy(cfg, "pol-1", si, &[1]);
    p.schedule.allocations[0].cover_id = cover.into();
    p.cover_starts = [(cover.to_string(), oct().starts)].into();
    p
}

#[test]
fn r08_hourly_benefit_counts_minutes() {
    let cfg = with_cover(
        r#"
id: rain_hours
name: Rain hours cover
type: parametric
class_of_business: pecuniary.income_weather
launch_phase: concept
key_features: "Paid by the hour of black rain in your working hours"
exposure: { kind: declared_hours }
trigger:
  events: [{ event_type: weather.rainstorm, from_stage: black }]
  requires_declared_hours: true
payout: { basis: per_hour, max_hours: 4 }
"#,
    );
    let p = [holding(&cfg, "rain_hours", 100)];
    let rain = |from: (u32, u32), to: (u32, u32)| {
        let events = polls(
            &cfg,
            "hko_warnings",
            &[
                (hk(5, from.0, from.1), &["WRAINB"]),
                (hk(5, to.0, to.1), &[]),
            ],
        );
        owed(&events, &p, &cfg).get("pol-1").copied()
    };
    // Pro rata to the minute: 90 minutes at HK$100 an hour.
    assert_eq!(rain((10, 0), (11, 30)), Some(Money::from_major(150)));
    assert_eq!(
        rain((10, 0), (10, 20)),
        Some(Money(3_333)),
        "down to the cent"
    );
    assert_eq!(
        rain((9, 0), (17, 0)),
        Some(Money::from_major(400)),
        "up to max_hours"
    );
}

#[test]
fn r55_minutes_in_force_are_consecutive() {
    let cfg = with_cover(
        r#"
id: rain_hour
name: Long black rain cover
type: parametric
class_of_business: pecuniary.income_weather
launch_phase: concept
key_features: "Paid when black rain lasts an hour"
exposure: { kind: everyone }
trigger:
  events: [{ event_type: weather.rainstorm, from_stage: black, min_minutes: 60 }]
payout: { basis: fixed_per_event }
"#,
    );
    let p = [holding(&cfg, "rain_hour", 300)];
    // Black for 40 minutes, red for 10, black for 40: never an hour unbroken.
    let broken = polls(
        &cfg,
        "hko_warnings",
        &[
            (hk(5, 10, 0), &["WRAINB"]),
            (hk(5, 10, 40), &["WRAINA"]),
            (hk(5, 10, 50), &["WRAINB"]),
            (hk(5, 11, 30), &[]),
        ],
    );
    assert_eq!(owed(&broken, &p, &cfg).get("pol-1"), Some(&Money::ZERO));
    let unbroken = polls(
        &cfg,
        "hko_warnings",
        &[(hk(5, 10, 0), &["WRAINB"]), (hk(5, 11, 5), &[])],
    );
    assert_eq!(
        owed(&unbroken, &p, &cfg).get("pol-1"),
        Some(&Money::from_major(300))
    );
}

#[test]
fn r55_route_and_ticket_triggers_pay_only_matching_declarations() {
    let cfg = cfg();
    let schedule = Schedule {
        plan_id: "essential".into(),
        allocations: vec![
            Allocation {
                cover_id: "commute_disruption".into(),
                premium: Money::from_major(40),
                sum_insured: Money::from_major(300),
            },
            Allocation {
                cover_id: "event_ticket".into(),
                premium: Money::from_major(40),
                sum_insured: Money::from_major(2_000),
            },
        ],
    };
    let rider_policy = [policy(&cfg, schedule, oct())];
    let rail = |line: &str, minutes: i64| {
        let mut events = BTreeMap::new();
        let r = report(
            "transit_disruptions",
            "RAIL_SUSPENDED",
            hk(5, 8, 0),
            Some(line),
        );
        triggers::track(&mut events, "transit_disruptions", &[r], hk(5, 8, 0), &cfg);
        let end = hk(5, 8, 0) + Duration::minutes(minutes);
        triggers::track(&mut events, "transit_disruptions", &[], end, &cfg);
        owed(&events, &rider_policy, &cfg)
    };
    assert_eq!(
        rail("Tsuen Wan line", 45).get("pol-1"),
        Some(&Money::from_major(300))
    );
    assert_eq!(rail("Tsuen Wan line", 20).get("pol-1"), Some(&Money::ZERO));
    assert!(rail("Island line", 45).is_empty(), "not the declared route");
    // The concert is cancelled: the declared ticket price, within the benefit amount.
    let mut events = BTreeMap::new();
    let r = report(
        "organiser_notices",
        "CANCELLED",
        hk(9, 12, 0),
        Some("TICKET-1"),
    );
    triggers::track(&mut events, "organiser_notices", &[r], hk(9, 12, 0), &cfg);
    triggers::track(&mut events, "organiser_notices", &[], hk(9, 13, 0), &cfg);
    assert_eq!(
        owed(&events, &rider_policy, &cfg).get("pol-1"),
        Some(&Money::from_major(800))
    );
    // A cover on an event reported per line must say which declaration it matches.
    let unmatched = COMMUTE.replace("  match: declared_route\n", "");
    assert_rule(
        load(
            INSURER,
            LINES,
            FACTORS,
            &[WEATHER, &unmatched, EVENT, DEVICE, ACCIDENT, FLIGHT],
        ),
        "R55",
    );
}

/// A weather income policy in the area market, working Mondays 09:00–18:00 in `areas`.
fn area_policy(cfg: &EngineConfig, id: &str, areas: &[&str]) -> Policy {
    let mut p = weather_policy(cfg, id, 500, &[1]);
    p.work_areas = areas.iter().map(|a| (*a).to_string()).collect();
    p
}

/// A work suspension for the whole local day `d` in `area`, as announced at `announced`.
fn suspension(area: &str, d: u32, announced: DateTime<Utc>) -> SourceReport {
    SourceReport {
        report_id: format!("suspension-{area}-{d}"),
        starts: hk(d, 0, 0),
        ends: Some(hk(d + 1, 0, 0)),
        ..report("work_suspensions", "WORK_SUSPENDED", announced, Some(area))
    }
}

#[test]
fn r55_area_matched_cover_pays_only_in_declared_work_areas() {
    let cfg = area_cfg();
    let book = [
        area_policy(&cfg, "pol-taipei", &["tw.taipei_city"]),
        area_policy(&cfg, "pol-hualien", &["tw.hualien_county"]),
        area_policy(&cfg, "pol-both", &["tw.taipei_city", "tw.new_taipei_city"]),
        area_policy(&cfg, "pol-none", &[]),
    ];
    // Monday 5 October: Taipei and New Taipei suspend work for the day, announced the evening
    // before; the reports stay listed until the day ends.
    let reports = [
        suspension("tw.taipei_city", 5, hk(4, 20, 0)),
        suspension("tw.new_taipei_city", 5, hk(4, 20, 0)),
    ];
    let mut events = BTreeMap::new();
    for now in [hk(4, 20, 0), hk(5, 0, 1), hk(6, 0, 1)] {
        triggers::track(&mut events, "work_suspensions", &reports, now, &cfg);
    }
    assert_eq!(events.len(), 2, "one trigger event per area: {events:?}");
    // One typhoon, one loss occurrence: the peril is counted across areas (R51).
    let occs = occurrences(&events, &cfg);
    let occurrence = only(&occs);
    assert_eq!(occurrence.zone, None);
    let matches = matches_at(occurrence, &events, &book, hk(31, 0, 0), &cfg);
    let owed: BTreeMap<&str, Money> = matches
        .iter()
        .map(|m| (m.policy_id.as_str(), m.entitlement))
        .collect();
    assert_eq!(owed.get("pol-taipei"), Some(&Money::from_major(500)));
    assert_eq!(
        owed.get("pol-both"),
        Some(&Money::from_major(500)),
        "two areas suspended on one day pay one daily benefit"
    );
    assert_eq!(
        owed.get("pol-hualien"),
        Some(&Money::ZERO),
        "exposed to the occurrence, not suspended where they work"
    );
    assert!(!owed.contains_key("pol-none"), "no declared area, no cover");
    assert_eq!(
        triggers::benefit_in_force(&matches),
        Money::from_major(1_500),
        "every declared area counts toward the occurrence's benefit in force (R51)"
    );
    // A cover matched on area needs events reported per area.
    let territory = AREA_WEATHER.replace("work.suspension", "weather.tropical_cyclone");
    let territory = territory.replace("from_stage: suspended", "from_stage: signal_8");
    let insurer = area_insurer();
    assert_rule(
        config::from_yaml(&ConfigSources {
            insurer: &insurer,
            event_types: &[EVENT_TYPES, AREA_EVENT_TYPES],
            trigger_sources: &[HKO, TRANSIT, FLIGHTS, ORGANISERS, HEAT, SUSPENSIONS],
            catalogue: &[&territory],
            ..sources()
        }),
        "R55",
    );
}

#[test]
fn r08_an_announced_suspension_is_in_force_from_its_onset() {
    let cfg = area_cfg();
    let announced = suspension("tw.taipei_city", 5, hk(4, 20, 0));
    let mut events = BTreeMap::new();
    let first = triggers::track(
        &mut events,
        "work_suspensions",
        std::slice::from_ref(&announced),
        hk(4, 20, 0),
        &cfg,
    );
    assert!(first.opened.is_empty(), "announced, not yet in force");
    triggers::track(
        &mut events,
        "work_suspensions",
        &[announced],
        hk(5, 0, 5),
        &cfg,
    );
    let event = events.values().next().expect("in force from its onset");
    assert_eq!(event.starts(), hk(5, 0, 0));
    // An evening worker on the Sunday it was announced is not disrupted by Monday's suspension.
    let mut sunday_evening = area_policy(&cfg, "pol-sunday", &["tw.taipei_city"]);
    sunday_evening.work_windows = vec![WorkWindow {
        weekday: 7,
        start_minute: 18 * 60,
        end_minute: 23 * 60,
    }];
    triggers::track(&mut events, "work_suspensions", &[], hk(6, 0, 5), &cfg);
    assert_eq!(
        owed(&events, &[sunday_evening], &cfg).get("pol-sunday"),
        Some(&Money::ZERO)
    );
}

#[test]
fn r42_area_matched_cover_needs_a_declared_work_area() {
    let cfg = area_cfg();
    let cover = cfg.cover("weather_income").expect("cover");
    let mut profile = rider();
    assert_eq!(optimiser::need(cover, &profile, &oct()), Money::ZERO);
    profile.work_areas = vec!["tw.taipei_city".into()];
    assert_eq!(
        optimiser::need(cover, &profile, &oct()),
        Money::from_major(800)
    );
}

#[test]
fn r51_a_peril_mixes_zone_units_only_when_counted_across_zones() {
    let mixed = |across: &str| {
        INSURER.replace(
            "  zone_accumulation_limit: 5000000",
            &format!(
                "    - {{ id: suspension, event_types: [work.suspension, workplace.heat_stress_x], event_limit: 1000000, aggregation_hours: 24{across} }}\n  zone_accumulation_limit: 5000000"
            ),
        )
    };
    let heat = "  - { id: workplace.heat_stress_x, name: Heat, stages: [red], measure: in_force, zone: territory, pricing_basis: insurer_filed_rate }\n";
    let types = format!("{AREA_EVENT_TYPES}{heat}");
    let load = |insurer: &str| {
        config::from_yaml(&ConfigSources {
            insurer,
            event_types: &[EVENT_TYPES, &types],
            trigger_sources: &[HKO, TRANSIT, FLIGHTS, ORGANISERS, HEAT, SUSPENSIONS],
            catalogue: &[
                AREA_WEATHER,
                COMMUTE,
                EVENT,
                DEVICE,
                ACCIDENT,
                FLIGHT,
                CONTENTS,
            ],
            ..sources()
        })
    };
    assert_rule(load(&mixed("")), "R51");
    assert!(load(&mixed(", across_zones: true")).is_ok());
}

#[test]
fn r55_a_market_overlay_states_its_own_stages() {
    // A later event types file replaces an entry with the same id: a market's official scale.
    let overlay = "event_types:\n  - { id: weather.rainstorm, name: Heavy rain advisory, stages: [amber, red, black, extreme], measure: in_force, zone: territory, pricing_basis: historical_record }\n";
    let cfg = config::from_yaml(&ConfigSources {
        event_types: &[EVENT_TYPES, overlay],
        ..sources()
    })
    .expect("the overlay loads");
    assert_eq!(
        cfg.triggers.event_types["weather.rainstorm"].stages.len(),
        4
    );
    // Within one file, an id is still registered once.
    let twice = format!("{overlay}{}", overlay.trim_start_matches("event_types:\n"));
    assert_rule(
        config::from_yaml(&ConfigSources {
            event_types: &[EVENT_TYPES, &twice],
            ..sources()
        }),
        "R55",
    );
}

#[test]
fn r55_history_reports_form_trigger_events() {
    let cfg = cfg();
    let spell = |code: &str, from: DateTime<Utc>, to: DateTime<Utc>| SourceReport {
        ends: Some(to),
        ..report("hko_warnings", code, from, None)
    };
    let history = [
        spell("TC3", hk(5, 6, 0), hk(5, 10, 0)),
        spell("TC8NE", hk(5, 10, 0), hk(5, 15, 0)),
        spell("TC3", hk(5, 15, 0), hk(5, 18, 0)),
        spell("WRAINB", hk(9, 9, 0), hk(9, 12, 0)),
        spell("TC8NE", hk(20, 9, 0), hk(20, 11, 0)),
    ];
    let events = triggers::from_history(&history, &cfg);
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(events[0].peak(), 2);
    assert_eq!(events[0].stages.len(), 3);
    assert_eq!(events[1].event_type, "weather.rainstorm");
    let occs = triggers::group_history(&events, &cfg);
    assert_eq!(occs.len(), 3, "each is more than a day apart");
    assert!(occs.iter().all(|o| o.closed));
    // Replaying the record pays as the live feed would.
    let by_id: Events = events
        .iter()
        .map(|e| (e.event_id.clone(), e.clone()))
        .collect();
    let mon = [weather_policy(&cfg, "pol-mon", 500, &[1])];
    let first = matches_at(&occs[0], &by_id, &mon, hk(31, 0, 0), &cfg);
    assert_eq!(first[0].entitlement, Money::from_major(500));
}

#[test]
fn r51_perils_are_validated() {
    // A cover paying on two perils could pay twice for one storm.
    let split = INSURER.replace(
        "event_types: [weather.tropical_cyclone, weather.rainstorm], event_limit: 1000000, aggregation_hours: 24 }",
        "event_types: [weather.tropical_cyclone], event_limit: 1000000, aggregation_hours: 24 }\n    - { id: rain, event_types: [weather.rainstorm], event_limit: 1000000, aggregation_hours: 24 }",
    );
    assert_rule(load(&split, LINES, FACTORS, &CATALOGUE), "R51");
    // A daily benefit needs a day's quiet before a new occurrence can pay the same day again.
    let short = INSURER.replace(
        "event_limit: 1000000, aggregation_hours: 24",
        "event_limit: 1000000, aggregation_hours: 6",
    );
    assert_rule(load(&short, LINES, FACTORS, &CATALOGUE), "R51");
    let unlimited = INSURER.replace(
        "    - { id: rail, event_types: [transit.rail_disruption], event_limit: 100000, aggregation_hours: 1 }\n",
        "",
    );
    assert_rule(load(&unlimited, LINES, FACTORS, &CATALOGUE), "R51");
    let twice = INSURER.replace(
        "event_types: [workplace.heat_stress]",
        "event_types: [workplace.heat_stress, weather.rainstorm]",
    );
    assert_rule(load(&twice, LINES, FACTORS, &CATALOGUE), "R51");
    // Resumption time belongs to declared hours.
    let loose = COMMUTE.replace(
        "  match: declared_route\n",
        "  match: declared_route\n  resumption_minutes: 30\n",
    );
    assert_rule(
        load(
            INSURER,
            LINES,
            FACTORS,
            &[WEATHER, &loose, EVENT, DEVICE, ACCIDENT, FLIGHT],
        ),
        "R08",
    );
}

/// Three workers on a typhoon in force from Monday 08:00 to Tuesday 19:00, two working Monday
/// and Tuesday, one Monday only, with event limit `limit_cents`. Returns the closed occurrence.
fn typhoon_book(limit_cents: i64) -> (EngineConfig, Vec<Policy>, Events, LossOccurrence) {
    let mut cfg = cfg();
    if let Some(p) = cfg.accumulation.perils.get_mut("windstorm_and_rain") {
        p.event_limit = Money(limit_cents);
    }
    let policies = vec![
        weather_policy(&cfg, "pol-a", 500, &[1, 2]),
        weather_policy(&cfg, "pol-b", 500, &[1, 2]),
        weather_policy(&cfg, "pol-c", 500, &[1]),
    ];
    let events = polls(
        &cfg,
        "hko_warnings",
        &[(hk(5, 8, 0), &["TC8NE"]), (hk(6, 19, 0), &[])],
    );
    let occ = only(&occurrences(&events, &cfg)).clone();
    (cfg, policies, events, occ)
}

/// The occurrence and its events while still in force.
fn open_at(events: &Events, occ: &LossOccurrence) -> (Events, LossOccurrence) {
    let open: Events = events
        .iter()
        .map(|(k, e)| {
            (
                k.clone(),
                TriggerEvent {
                    ends: None,
                    ..e.clone()
                },
            )
        })
        .collect();
    let occ = LossOccurrence {
        closed: false,
        quiet_since: None,
        ..occ.clone()
    };
    (open, occ)
}

#[test]
fn r51_guaranteed_share_paid_at_once() {
    // Benefit in force HK$1,500; event limit HK$1,000: two thirds are guaranteed.
    let (cfg, policies, events, closed) = typhoon_book(100_000);
    let (open_events, open) = open_at(&events, &closed);
    let now = matches_at(&open, &open_events, &policies, hk(5, 12, 0), &cfg);
    let mut book = payouts::OccurrencePayouts::open(&open, triggers::benefit_in_force(&now), &cfg)
        .expect("limit");
    assert_eq!(book.benefit_in_force, Money::from_major(1_500));
    let none = BTreeSet::new();
    let paid = payouts::guaranteed(&mut book, &now, &none, &cfg);
    assert_eq!(paid.len(), 3);
    for p in &paid {
        assert_eq!(p.stage, PayoutStage::Guaranteed);
        assert_eq!(
            p.amount,
            Money(33_333),
            "two thirds of HK$500, down to the cent"
        );
    }
    assert!(book.guaranteed_total() <= book.event_limit);
    // A peril without a limit pays nothing.
    let mut no_limit = cfg.clone();
    no_limit.accumulation.perils.clear();
    assert_rule(
        payouts::OccurrencePayouts::open(&open, Money::ZERO, &no_limit),
        "R51",
    );
}

#[test]
fn r51_balance_reduced_pro_rata_above_event_limit() {
    // Limit HK$2,000.01; benefit in force HK$1,500: Monday is paid in full at once.
    let (cfg, policies, events, closed) = typhoon_book(200_001);
    let (open_events, open) = open_at(&events, &closed);
    let first = matches_at(&open, &open_events, &policies, hk(5, 12, 0), &cfg);
    let mut book =
        payouts::OccurrencePayouts::open(&open, triggers::benefit_in_force(&first), &cfg)
            .expect("limit");
    let none = BTreeSet::new();
    let guaranteed = payouts::guaranteed(&mut book, &first, &none, &cfg);
    assert!(
        guaranteed
            .iter()
            .all(|p| p.amount == Money::from_major(500))
    );
    // The balance is paid only once the occurrence has closed.
    assert_rule(payouts::balance(&book, &open, &first, &none, &cfg), "R51");
    // Tuesday adds HK$500 each for pol-a and pol-b: HK$1,000 owed, HK$500.01 left in the limit.
    let last = matches_at(&closed, &events, &policies, hk(31, 0, 0), &cfg);
    let balance = payouts::balance(&book, &closed, &last, &none, &cfg).expect("closed");
    let by_policy: BTreeMap<&str, Money> = balance
        .iter()
        .map(|p| (p.policy_id.as_str(), p.amount))
        .collect();
    assert_eq!(
        by_policy.get("pol-a"),
        Some(&Money(25_001)),
        "tie: the cent goes by policy id"
    );
    assert_eq!(by_policy.get("pol-b"), Some(&Money(25_000)));
    assert_eq!(by_policy.get("pol-c"), None, "already paid in full");
    let total: Money = guaranteed.iter().chain(&balance).map(|p| p.amount).sum();
    assert_eq!(total, book.event_limit, "the event limit, exactly");
    // Under the limit, the balance is what is owed in full.
    let (cfg, policies, events, closed) = typhoon_book(1_000_000);
    let (open_events, open) = open_at(&events, &closed);
    let first = matches_at(&open, &open_events, &policies, hk(5, 12, 0), &cfg);
    let mut book =
        payouts::OccurrencePayouts::open(&open, triggers::benefit_in_force(&first), &cfg)
            .expect("limit");
    payouts::guaranteed(&mut book, &first, &none, &cfg);
    let last = matches_at(&closed, &events, &policies, hk(31, 0, 0), &cfg);
    let balance = payouts::balance(&book, &closed, &last, &none, &cfg).expect("closed");
    assert!(balance.iter().all(|p| p.amount == Money::from_major(500)));
}

#[test]
fn r27_one_payout_per_policy_event() {
    let (cfg, policies, events, closed) = typhoon_book(1_000_000);
    let none = BTreeSet::new();
    let (open_events, open) = open_at(&events, &closed);
    let first = matches_at(&open, &open_events, &policies, hk(5, 12, 0), &cfg);
    let mut book =
        payouts::OccurrencePayouts::open(&open, triggers::benefit_in_force(&first), &cfg)
            .expect("limit");
    let once = payouts::guaranteed(&mut book, &first, &none, &cfg);
    assert_eq!(once.len(), 3);
    // Polled again later the same day, and again on Tuesday when more is owed: nothing new until
    // the balance.
    assert!(payouts::guaranteed(&mut book, &first, &none, &cfg).is_empty());
    let tuesday = matches_at(&open, &open_events, &policies, hk(6, 12, 0), &cfg);
    assert!(payouts::guaranteed(&mut book, &tuesday, &none, &cfg).is_empty());
    let balance = payouts::balance(
        &book,
        &closed,
        &matches_at(&closed, &events, &policies, hk(31, 0, 0), &cfg),
        &none,
        &cfg,
    )
    .expect("closed");
    // One key per (policy, loss occurrence, stage), and the same decision gives the same key.
    let keys: BTreeSet<&str> = once
        .iter()
        .chain(&balance)
        .map(|p| p.idempotency_key.as_str())
        .collect();
    assert_eq!(keys.len(), once.len() + balance.len());
    assert_eq!(
        once[0].idempotency_key,
        payouts::idempotency_key("pol-a", &closed.occurrence_id, PayoutStage::Guaranteed)
    );
    // The same storm polled again gets the same occurrence id, so the same keys.
    let again = polls(
        &cfg,
        "hko_warnings",
        &[(hk(5, 8, 0), &["TC8NE"]), (hk(6, 19, 0), &[])],
    );
    assert_eq!(
        only(&occurrences(&again, &cfg)).occurrence_id,
        closed.occurrence_id
    );
}

/// A flight delay payout for the rider's declared booking.
fn flight_payout(suitable: &[&str]) -> PayoutDecision {
    let cfg = cfg();
    let schedule = Schedule {
        plan_id: "max".into(),
        allocations: vec![Allocation {
            cover_id: "flight_delay".into(),
            premium: Money::from_major(50),
            sum_insured: Money::from_major(600),
        }],
    };
    let mut p = policy(&cfg, schedule, oct());
    p.upcoming_events[0].kind = "trip.air".into();
    p.upcoming_events[0].reference = Some("CX100/2026-10-20".into());
    let mut events = BTreeMap::new();
    let delayed = SourceReport {
        delay_minutes: Some(150),
        ..report(
            "flight_status",
            "DELAYED",
            hk(20, 9, 0),
            Some("CX100/2026-10-20"),
        )
    };
    triggers::track(
        &mut events,
        "flight_status",
        &[delayed],
        hk(20, 11, 30),
        &cfg,
    );
    let mut occs = BTreeMap::new();
    triggers::group(&mut occs, &events, hk(20, 11, 30), &cfg);
    let occ = occs.into_values().next().expect("occurrence");
    let policies = [p];
    let found = matches_at(&occ, &events, &policies, hk(20, 11, 30), &cfg);
    let mut book = payouts::OccurrencePayouts::open(&occ, triggers::benefit_in_force(&found), &cfg)
        .expect("limit");
    let suitable: BTreeSet<String> = suitable.iter().map(|s| (*s).to_string()).collect();
    let mut paid = payouts::guaranteed(&mut book, &found, &suitable, &cfg);
    assert_eq!(paid.len(), 1);
    paid.remove(0)
}

#[test]
fn r16_cash_always_offered() {
    let with_lounge = flight_payout(&["lounge_access", "spa"]);
    assert_eq!(with_lounge.amount, Money::from_major(600));
    assert_eq!(
        with_lounge.options,
        vec![Settlement::Cash, Settlement::InKind("lounge_access".into())],
        "cash first; only alternatives the cover's terms permit"
    );
    let lounge_full = flight_payout(&[]);
    assert_eq!(lounge_full.options, vec![Settlement::Cash]);
    let (cfg, policies, events, closed) = typhoon_book(1_000_000);
    let first = matches_at(&closed, &events, &policies, hk(31, 0, 0), &cfg);
    let mut book =
        payouts::OccurrencePayouts::open(&closed, triggers::benefit_in_force(&first), &cfg)
            .expect("limit");
    let everything: BTreeSet<String> = ["lounge_access".to_string()].into();
    for p in payouts::guaranteed(&mut book, &first, &everything, &cfg) {
        assert_eq!(p.options, vec![Settlement::Cash]);
    }
}

#[test]
fn r28_alternative_needs_acceptance() {
    let payout = flight_payout(&["lounge_access"]);
    let cash = payouts::settle(&payout, None, Money::ZERO).expect("cash");
    assert_eq!((cash.method, cash.cash), (Settlement::Cash, payout.amount));
    let lounge = Settlement::InKind("lounge_access".into());
    let in_kind = payouts::settle(&payout, Some(&lounge), Money::ZERO).expect("accepted");
    assert_eq!((in_kind.method, in_kind.cash), (lounge, Money::ZERO));
    assert_rule(
        payouts::settle(
            &payout,
            Some(&Settlement::InKind("spa".into())),
            Money::ZERO,
        ),
        "R28",
    );
    // Not suitable now (the lounge is full): not offered, so it cannot be accepted.
    let full = flight_payout(&[]);
    assert_rule(
        payouts::settle(
            &full,
            Some(&Settlement::InKind("lounge_access".into())),
            Money::ZERO,
        ),
        "R28",
    );
}

#[test]
fn r53_payout_in_grace_paid_net_of_premium_due() {
    let payout = flight_payout(&["lounge_access"]);
    let due = Money::from_major(80);
    let cash = payouts::settle(&payout, None, due).expect("cash");
    assert_eq!(cash.premium_set_off, due);
    assert_eq!(cash.cash, payout.amount - due);
    let lounge = Settlement::InKind("lounge_access".into());
    let in_kind = payouts::settle(&payout, Some(&lounge), due).expect("accepted");
    assert_eq!(in_kind.premium_set_off, Money::ZERO, "premium stays due");
}

#[test]
fn r51_zone_over_accumulation_limit_takes_no_new_allocation() {
    let cfg = cfg();
    // HK$5,000,000 zone limit; 10,001 workers at HK$500 a day put the territory over it.
    let book: Vec<Policy> = (0..10_001)
        .map(|i| weather_policy(&cfg, &format!("pol-{i}"), 500, &[1]))
        .collect();
    let by_zone = triggers::benefit_in_force_by_zone(&book, &cfg);
    let full = triggers::full_zones(&by_zone, &cfg);
    let storms = ("windstorm_and_rain".to_string(), None);
    assert!(full.contains(&storms));
    assert_eq!(
        by_zone.get(&storms),
        Some(&Money::from_major(5_000_500)),
        "a cover paying on a typhoon and its rain counts once"
    );
    let weather = cfg.cover("weather_income").expect("cover");
    assert_rule(
        catalogue::check_accumulation(weather, &rider(), &full, &cfg),
        "R51",
    );
    // The optimiser places no new premium there; existing cover renews unchanged.
    let mut w = World::new(&rider(), &basis(), oct());
    w.full_zones = full;
    let fresh = suggest(&w, "essential", &rider());
    assert_eq!(fresh.schedule.premium("weather_income"), Money::ZERO);
    let current = Schedule {
        plan_id: "essential".into(),
        allocations: vec![Allocation {
            cover_id: "weather_income".into(),
            premium: Money::from_major(10),
            sum_insured: Money::from_major(250),
        }],
    };
    let renewed =
        optimiser::renew(&current, &w.plan("essential"), &rider(), &w.ctx()).expect("renews");
    assert_eq!(
        renewed.schedule.premium("weather_income"),
        Money::from_major(10)
    );
    // A route zone fills on its own: the territory stays open.
    let commute = cfg.cover("commute_disruption").expect("cover");
    let line_full: BTreeSet<_> = [("rail".to_string(), Some("Tsuen Wan line".to_string()))].into();
    assert_rule(
        catalogue::check_accumulation(commute, &rider(), &line_full, &cfg),
        "R51",
    );
    assert!(catalogue::check_accumulation(weather, &rider(), &line_full, &cfg).is_ok());
}

// ---- Phase 4: prevention task lifecycle, audit sampling, rewards, supplier settlement (ADR-009)

fn prevention_basis() -> PreventionBasis {
    PreventionBasis {
        measures: [(
            "case_and_screen_protector".to_string(),
            MeasureClaims {
                claim_prob_without: Bps(400),
                claim_prob_with: Bps(250),
                avg_claim_cost: Money::from_major(800),
            },
        )]
        .into(),
    }
}

fn screen_proposal() -> PreventionTaskProposal {
    PreventionTaskProposal {
        task_type: "case_and_screen_protector".into(),
        action: "Fit a case and a screen protector on your phone before you fly.".into(),
        evidence: "photo".into(),
    }
}

/// The rider, opted into photos (R20).
fn photo_rider() -> Profile {
    Profile {
        consents: ["photos".to_string()].into(),
        ..rider()
    }
}

fn held(relativity: u32, sum_insured: i64) -> HeldCover<'static> {
    HeldCover {
        cover_id: "device_damage",
        sum_insured: Money::from_major(sum_insured),
        relativity: Bps(relativity),
    }
}

fn sized(cfg: &EngineConfig) -> PreventionTaskDraft {
    prevention::size(
        &screen_proposal(),
        held(10_000, 8_000),
        Some("CX100/2026-10-20"),
        &photo_rider(),
        &prevention_basis(),
        cfg,
    )
    .expect("sized")
}

fn offered(cfg: &EngineConfig, history: &[VerifiedActivity]) -> PreventionTask {
    prevention::offer(
        sized(cfg),
        "cust-rider",
        "pol-rider",
        day(2026, 10, 5),
        day(2026, 10, 20),
        history,
        Cohort::Offered,
        cfg,
    )
    .expect("offered")
}

fn submitted(cfg: &EngineConfig) -> PreventionTask {
    let task = prevention::accept(offered(cfg, &[]), day(2026, 10, 6)).expect("accepted");
    prevention::submit(task, day(2026, 10, 7), vec!["sha256:a1".into()]).expect("submitted")
}

fn pass() -> Review {
    Review::Verdict(Verdict {
        passed: true,
        reason: "The case and screen protector are on.".into(),
        reuse_suspected: false,
    })
}

/// The first seed that does, or does not, draw `id` for audit.
fn seed_where(id: &str, drawn: bool, cfg: &EngineConfig) -> u64 {
    (0..10_000)
        .find(|s| audit::should_audit(id, cfg.guardrails.audit_rate, *s) == drawn)
        .expect("a seed")
}

#[test]
fn r40_reward_follows_claims_avoided_for_customer() {
    let cfg = cfg();
    let size = |relativity, sum_insured| {
        prevention::size(
            &screen_proposal(),
            held(relativity, sum_insured),
            None,
            &photo_rider(),
            &prevention_basis(),
            &cfg,
        )
        .expect("sized")
    };
    // 4% → 2.5% on an HK$800 repair: HK$12 avoided, a quarter of it (HK$3) as the reward.
    let average = size(10_000, 8_000);
    assert_eq!(prevention::claims_avoided(&average), Money::from_major(12));
    assert_eq!(average.reward_cost, Money::from_major(3));
    // A phone rated twice as likely to break: twice the claims avoided, twice the reward.
    let risky = size(20_000, 8_000);
    assert_eq!(
        (risky.claim_prob_before, risky.claim_prob_after),
        (Bps(800), Bps(500))
    );
    assert_eq!(risky.reward_cost, Money::from_major(6));
    // First loss: a claim costs this customer's insurer at most the sum insured (R48).
    let small = size(10_000, 500);
    assert_eq!(small.avg_claim_cost, Money::from_major(500));
    assert_eq!(
        small.reward_cost,
        Money(187),
        "a quarter of HK$7.50, rounded down"
    );
    assert!(
        small.reward_cost < prevention::claims_avoided(&small),
        "R18"
    );
    // No cover in force: no claims to avoid.
    assert_rule(
        prevention::size(
            &screen_proposal(),
            held(10_000, 0),
            None,
            &photo_rider(),
            &prevention_basis(),
            &cfg,
        ),
        "R40",
    );
}

#[test]
fn r38_task_for_parametric_cover_rejected() {
    let cfg = cfg();
    let flight = HeldCover {
        cover_id: "flight_delay",
        ..held(10_000, 600)
    };
    assert_rule(
        prevention::size(
            &screen_proposal(),
            flight,
            None,
            &photo_rider(),
            &prevention_basis(),
            &cfg,
        ),
        "R38",
    );
    // A measure for a class with no loss-based cover cannot be registered.
    let measures = format!(
        "{PREVENTION}  - {{ id: leave_early, class_of_business: pecuniary.commute, description: x, evidence: [photo], repeat_after_days: 1 }}\n"
    );
    assert_rule(
        config::from_yaml(&ConfigSources {
            prevention_measures: &measures,
            ..sources()
        }),
        "R38",
    );
}

#[test]
fn r18_only_registered_measures_of_the_cover_class_are_sized() {
    let cfg = cfg();
    let size = |proposal: &PreventionTaskProposal, profile: &Profile| {
        prevention::size(
            proposal,
            held(10_000, 8_000),
            None,
            profile,
            &prevention_basis(),
            &cfg,
        )
    };
    let invented = PreventionTaskProposal {
        task_type: "phone_kept_in_pocket".into(),
        ..screen_proposal()
    };
    assert_rule(size(&invented, &photo_rider()), "R18");
    let other_class = PreventionTaskProposal {
        task_type: "helmet_worn".into(),
        ..screen_proposal()
    };
    assert_rule(size(&other_class, &photo_rider()), "R36");
    let wrong_proof = PreventionTaskProposal {
        evidence: "hospital_receipt".into(),
        ..screen_proposal()
    };
    assert_rule(size(&wrong_proof, &photo_rider()), "R10");
    assert_rule(size(&screen_proposal(), &rider()), "R20");
    // A share of the claims avoided at or above them would fail the cost-benefit test.
    let free = PreventionTaskDraft {
        reward_cost: Money::from_major(12),
        ..sized(&cfg)
    };
    assert_rule(
        prevention::offer(
            free,
            "c",
            "p",
            day(2026, 10, 5),
            day(2026, 10, 20),
            &[],
            Cohort::Offered,
            &cfg,
        ),
        "R18",
    );
}

#[test]
fn r10_points_and_reward_only_through_the_lifecycle() {
    let cfg = cfg();
    let task = offered(&cfg, &[]);
    assert_eq!(
        task.task_id,
        "task/cust-rider/case_and_screen_protector/CX100/2026-10-20"
    );
    assert_eq!(task.class_of_business, "property.device");
    // Evidence before the customer took the task on is refused.
    assert!(prevention::submit(task.clone(), day(2026, 10, 6), vec!["x".into()]).is_err());
    let accepted = prevention::accept(task, day(2026, 10, 6)).expect("accepted");
    assert_rule(
        prevention::submit(accepted.clone(), day(2026, 10, 7), vec![]),
        "R10",
    );
    let sent =
        prevention::submit(accepted, day(2026, 10, 7), vec!["sha256:a1".into()]).expect("sent");
    let seed = seed_where(&sent.task_id, false, &cfg);
    let (done, credit) =
        prevention::review(sent, pass(), None, day(2026, 10, 8), seed, &cfg).expect("reviewed");
    assert_eq!(
        done.state,
        TaskState::Verified {
            on: day(2026, 10, 8),
            by: VerifiedBy::Verifier
        }
    );
    let credit = credit.expect("credited");
    assert_eq!(credit.activity.points(), 100);
    assert_eq!(credit.activity.task_type(), "case_and_screen_protector");
    assert_eq!(credit.reward.amount, Money::from_major(3));
    assert_eq!(credit.reward.pool, "property", "R36");
    // Verified once: it cannot be reviewed again.
    assert!(prevention::review(done, pass(), None, day(2026, 10, 9), seed, &cfg).is_err());
}

#[test]
fn r10_failed_or_reused_evidence_earns_nothing() {
    let cfg = cfg();
    let seed = 0;
    let fail = Review::Verdict(Verdict {
        passed: false,
        reason: "The screen protector is not visible.".into(),
        reuse_suspected: false,
    });
    let (task, credit) =
        prevention::review(submitted(&cfg), fail, None, day(2026, 10, 8), seed, &cfg)
            .expect("reviewed");
    assert!(credit.is_none());
    assert!(matches!(task.state, TaskState::NotVerified { .. }));
    // It can be sent again before the due date, not after.
    assert_rule(
        prevention::submit(task.clone(), day(2026, 10, 21), vec!["b".into()]),
        "R10",
    );
    assert!(prevention::submit(task, day(2026, 10, 9), vec!["b".into()]).is_ok());
    // Suspected reuse: only a person decides (R13).
    let reused = Review::Verdict(Verdict {
        passed: true,
        reason: "Seen before.".into(),
        reuse_suspected: true,
    });
    let (task, credit) =
        prevention::review(submitted(&cfg), reused, None, day(2026, 10, 8), seed, &cfg)
            .expect("reviewed");
    assert!(credit.is_none());
    assert_eq!(
        task.state,
        TaskState::WithPerson {
            on: day(2026, 10, 8),
            referral: Referral::SuspectedReuse
        }
    );
    let (task, credit) =
        prevention::person_decides(task, false, "Photo reused", day(2026, 10, 9), &cfg)
            .expect("decided");
    assert!(credit.is_none());
    assert!(matches!(task.state, TaskState::NotVerified { .. }));
    // No verdict from the verifier: a person, never a fail (R22).
    let (task, _) = prevention::review(
        submitted(&cfg),
        Review::NoVerdict,
        None,
        day(2026, 10, 8),
        seed,
        &cfg,
    )
    .expect("reviewed");
    assert!(matches!(
        task.state,
        TaskState::WithPerson {
            referral: Referral::NoVerdict,
            ..
        }
    ));
}

#[test]
fn r10_measure_earns_once_per_repeat_window() {
    let cfg = cfg();
    let (_, credit) = prevention::review(
        submitted(&cfg),
        pass(),
        None,
        day(2026, 10, 8),
        seed_where(&submitted(&cfg).task_id, false, &cfg),
        &cfg,
    )
    .expect("reviewed");
    let history = vec![credit.expect("credited").activity];
    let again = |on| {
        let draft = PreventionTaskDraft {
            event_reference: Some("CX200".into()),
            ..sized(&cfg)
        };
        prevention::offer(
            draft,
            "cust-rider",
            "pol",
            on,
            on,
            &history,
            Cohort::Offered,
            &cfg,
        )
    };
    // 90 days from 8 October: not before 6 January.
    assert_rule(again(day(2027, 1, 5)), "R10");
    assert!(again(day(2027, 1, 6)).is_ok());
}

#[test]
fn r10_task_without_evidence_expires() {
    let cfg = cfg();
    let task = offered(&cfg, &[]);
    assert_eq!(
        prevention::expire(task.clone(), day(2026, 10, 20)).state,
        TaskState::Offered
    );
    let late = prevention::expire(task.clone(), day(2026, 10, 21));
    assert_eq!(
        late.state,
        TaskState::Expired {
            on: day(2026, 10, 21)
        }
    );
    assert!(prevention::accept(task, day(2026, 10, 21)).is_err());
    // Evidence already sent is always decided.
    let sent = submitted(&cfg);
    assert_eq!(prevention::expire(sent.clone(), day(2026, 11, 30)), sent);
}

#[test]
fn r14_sampled_pass_held_for_a_person_before_credit() {
    let cfg = cfg();
    let task = submitted(&cfg);
    let seed = seed_where(&task.task_id, true, &cfg);
    let (held_task, credit) =
        prevention::review(task, pass(), None, day(2026, 10, 8), seed, &cfg).expect("reviewed");
    assert!(credit.is_none(), "nothing credited before the audit");
    assert_eq!(
        held_task.state,
        TaskState::WithPerson {
            on: day(2026, 10, 8),
            referral: Referral::Audit(AuditReason::RandomSample)
        }
    );
    let (task, credit) =
        prevention::person_decides(held_task, true, "Confirmed", day(2026, 10, 9), &cfg)
            .expect("decided");
    assert!(matches!(
        task.state,
        TaskState::Verified {
            by: VerifiedBy::Person,
            ..
        }
    ));
    assert_eq!(credit.expect("credited").reward.on, day(2026, 10, 9));
}

#[test]
fn r14_risk_review_always_goes_to_a_person() {
    let cfg = cfg();
    let task = submitted(&cfg);
    let seed = seed_where(&task.task_id, false, &cfg);
    let (task, credit) = prevention::review(
        task,
        pass(),
        Some(cfg.guardrails.risk_review_threshold),
        day(2026, 10, 8),
        seed,
        &cfg,
    )
    .expect("reviewed");
    assert!(credit.is_none());
    assert!(matches!(
        task.state,
        TaskState::WithPerson {
            referral: Referral::Audit(AuditReason::RiskReview),
            ..
        }
    ));
}

#[test]
fn r14_payout_audit_sample_is_replayable() {
    let cfg = cfg();
    let ids: Vec<String> = (0..20_000)
        .map(|i| format!("payout/pol-{i}/occ/guaranteed"))
        .collect();
    let decisions = || ids.iter().map(|i| (i.as_str(), None));
    let sample = audit::sample(decisions(), 42, &cfg.guardrails);
    assert!(
        (900..=1_100).contains(&sample.len()),
        "sampled {}",
        sample.len()
    );
    assert_eq!(sample, audit::sample(decisions(), 42, &cfg.guardrails));
    assert_ne!(sample, audit::sample(decisions(), 43, &cfg.guardrails));
}

#[test]
fn r15_task_not_verified_goes_to_a_person_on_appeal() {
    let cfg = cfg();
    let fail = Review::Verdict(Verdict {
        passed: false,
        reason: "Too dark to see.".into(),
        reuse_suspected: false,
    });
    let (task, _) = prevention::review(submitted(&cfg), fail, None, day(2026, 10, 8), 0, &cfg)
        .expect("reviewed");
    let task = prevention::appeal(task, day(2026, 10, 9)).expect("appealed");
    assert!(matches!(
        task.state,
        TaskState::WithPerson {
            referral: Referral::Appeal,
            ..
        }
    ));
    let (_, credit) =
        prevention::person_decides(task, true, "Visible", day(2026, 10, 10), &cfg).expect("ok");
    assert!(credit.is_some());
}

fn earned(id: &str, major: i64, on: chrono::NaiveDate) -> RewardEarned {
    RewardEarned {
        reward_id: id.into(),
        customer_id: "cust-rider".into(),
        pool: "property".into(),
        earned_for: EarnedFor::VerifiedActivity { task_id: id.into() },
        amount: Money::from_major(major),
        on,
    }
}

#[test]
fn r17_monthly_reward_cap() {
    let cfg = cfg();
    let mut grants = Vec::new();
    for (id, major, on, expect) in [
        ("a", 20, day(2026, 10, 3), 20),
        ("b", 20, day(2026, 10, 10), 10),
        ("c", 5, day(2026, 10, 31), 0),
        ("d", 20, day(2026, 11, 1), 20),
    ] {
        let g = rewards::grant(earned(id, major, on), &grants, &cfg);
        assert_eq!(g.granted, Money::from_major(expect), "{id}");
        assert_eq!(g.granted + g.over_cap, Money::from_major(major));
        assert_eq!(g.idempotency_key, format!("reward/{id}"));
        grants.push(g);
    }
    assert_eq!(
        rewards::month_to_date("cust-rider", day(2026, 10, 15), &grants),
        Money::from_major(30)
    );
    assert_eq!(
        rewards::month_to_date("someone-else", day(2026, 10, 15), &grants),
        Money::ZERO
    );
}

fn device_activity(cfg: &EngineConfig, on: chrono::NaiveDate) -> VerifiedActivity {
    let ok = Verdict {
        passed: true,
        reason: "ok".into(),
        reuse_suspected: false,
    };
    prevention::complete(&screen_task(300), &ok, on, cfg).expect("verified")
}

#[test]
fn r26_no_reward_conditional_on_not_claiming() {
    // Exhaustive match: a claim-free variant breaks this test at compile time.
    for e in [
        EarnedFor::VerifiedActivity {
            task_id: "t".into(),
        },
        EarnedFor::ParticipationStreak { periods: 3 },
    ] {
        match e {
            EarnedFor::VerifiedActivity { .. } | EarnedFor::ParticipationStreak { .. } => {}
        }
    }
    // Loyalty rewards sustained participation: three periods in a row with verified activity.
    let cfg = cfg();
    let periods = [period(1, 2026, 8), period(2, 2026, 9), period(3, 2026, 10)];
    let active: Vec<VerifiedActivity> = [day(2026, 8, 3), day(2026, 9, 9), day(2026, 10, 30)]
        .into_iter()
        .map(|d| device_activity(&cfg, d))
        .collect();
    let reward = rewards::loyalty("cust-rider", "property", &periods, &active, &cfg)
        .expect("a three-period streak");
    assert_eq!(reward.amount, Money::from_major(10));
    assert_eq!(
        reward.earned_for,
        EarnedFor::ParticipationStreak { periods: 3 }
    );
    assert_eq!(reward.reward_id, "loyalty/cust-rider/property/3");
    // Not in another pool, not after a gap, not on the fourth period.
    assert!(rewards::loyalty("cust-rider", "travel", &periods, &active, &cfg).is_none());
    assert!(rewards::loyalty("cust-rider", "property", &periods, &active[1..], &cfg).is_none());
    let gap = [period(1, 2026, 7), period(2, 2026, 9), period(3, 2026, 10)];
    let gap_active: Vec<VerifiedActivity> = [day(2026, 7, 3), day(2026, 9, 9), day(2026, 10, 1)]
        .into_iter()
        .map(|d| device_activity(&cfg, d))
        .collect();
    assert!(rewards::loyalty("cust-rider", "property", &gap, &gap_active, &cfg).is_none());
    let mut four = periods.to_vec();
    four.push(period(4, 2026, 11));
    let mut four_active = active.clone();
    four_active.push(device_activity(&cfg, day(2026, 11, 2)));
    assert!(rewards::loyalty("cust-rider", "property", &four, &four_active, &cfg).is_none());
}

fn partner_offer(id: &str, family: &str, partner: &str, price: i64) -> RewardOffer {
    RewardOffer {
        offer_id: id.into(),
        family: family.into(),
        partner_id: Some(partner.into()),
        description: format!("offer {id}"),
        perceived_value: Money::from_major(price * 4),
        price: Money::from_major(price),
        funded_by: Funder::Partner,
        inventory: None,
        is_cash: false,
    }
}

#[test]
fn r16_reward_menu_always_offers_cash_first() {
    let cfg = cfg();
    let essential = plan(&cfg, "essential");
    let offers = vec![
        partner_offer("phone_mount", "partner", "partner_shop", 5),
        partner_offer("unregistered", "partner", "street_stall", 5),
        partner_offer("too_dear", "partner", "partner_shop", 50),
        RewardOffer {
            inventory: Some(0),
            ..partner_offer("sold_out", "partner", "partner_shop", 5)
        },
        RewardOffer {
            is_cash: true,
            ..partner_offer("cash_lookalike", "partner", "partner_shop", 5)
        },
    ];
    let menu = rewards::menu(Money::from_major(12), &offers, &essential, &cfg);
    let ids: Vec<&str> = menu.iter().map(|o| o.offer_id.as_str()).collect();
    assert_eq!(ids, ["cash", "phone_mount"]);
    assert!(menu[0].is_cash);
    assert_eq!(
        menu[0].price,
        Money::from_major(12),
        "cash for the whole balance"
    );
    assert!(rewards::menu(Money::ZERO, &offers, &essential, &cfg).is_empty());
    // Only what was offered can be chosen.
    assert_rule(
        rewards::redeem("cust-rider", "redeem/1", "too_dear", &menu),
        "R16",
    );
    let cash = rewards::redeem("cust-rider", "redeem/1", "cash", &menu).expect("cash");
    assert_eq!(
        (cash.method, cash.amount),
        (Redeemed::Cash, Money::from_major(12))
    );
    let mount = rewards::redeem("cust-rider", "redeem/2", "phone_mount", &menu).expect("mount");
    assert_eq!(
        mount.method,
        Redeemed::InKind {
            supplier_id: "partner_shop".into()
        }
    );
    let grants = vec![rewards::grant(earned("a", 12, day(2026, 10, 3)), &[], &cfg)];
    assert_eq!(
        rewards::balance(&grants, &[mount], Money::ZERO, &[]),
        Money::from_major(7)
    );
}

#[test]
fn r45_reward_families_from_plan_privileges() {
    let cfg = cfg();
    let offers = vec![partner_offer("streak_badge", "loyalty", "partner_shop", 5)];
    let on = |id: &str| {
        rewards::menu(Money::from_major(10), &offers, &plan(&cfg, id), &cfg)
            .iter()
            .any(|o| o.offer_id == "streak_badge")
    };
    assert!(on("max"));
    assert!(!on("essential"));
    assert!(rewards::families_for(&plan(&cfg, "essential"), &cfg).contains("partner"));
}

#[test]
fn r16_config_must_offer_cash() {
    let insurer = INSURER.replace(
        "  reward_share_of_claims_avoided: 0.25\n",
        "  reward_share_of_claims_avoided: 0.25\n  always_offer_cash: false\n",
    );
    assert_rule(
        config::from_yaml(&ConfigSources {
            insurer: &insurer,
            ..sources()
        }),
        "R16",
    );
}

#[test]
fn r28_in_kind_offered_only_at_fair_value() {
    let cfg = cfg();
    // Lounge access: tariff HK$200, retail value HK$650.
    assert!(payouts::fair_value(
        "lounge_access",
        Money::from_major(600),
        &cfg
    ));
    assert!(
        !payouts::fair_value("lounge_access", Money::from_major(700), &cfg),
        "worth less than the cash"
    );
    assert!(
        !payouts::fair_value("lounge_access", Money::from_major(150), &cfg),
        "costs more than the cash"
    );
    assert!(
        payouts::fair_value("alternative_seat", Money::from_major(5_000), &cfg),
        "invoiced"
    );
    assert!(
        !payouts::fair_value("spa", Money::from_major(600), &cfg),
        "no supplier"
    );
    // Every in-kind option a cover names needs a supplier.
    let no_lounges = "suppliers: []\n";
    assert_rule(
        config::from_yaml(&ConfigSources {
            suppliers: no_lounges,
            ..sources()
        }),
        "R28",
    );
}

fn lounge_order(cfg: &EngineConfig) -> InKindOrder {
    let payout = flight_payout(&["lounge_access"]);
    let lounge = Settlement::InKind("lounge_access".into());
    let settled = payouts::settle(&payout, Some(&lounge), Money::ZERO).expect("accepted");
    suppliers::order_for_payout(&settled, day(2026, 10, 20), cfg).expect("order")
}

#[test]
fn r56_supplier_paid_its_tariff_only_for_what_it_delivered() {
    let cfg = cfg();
    let order = lounge_order(&cfg);
    assert_eq!(
        (order.supplier_id.as_str(), order.reserved),
        ("lounges", Money::from_major(600))
    );
    assert_eq!(order.expires_on, day(2026, 11, 18), "30 days to use it");
    let delivered = suppliers::deliver(order.clone(), day(2026, 10, 21), None).expect("delivered");
    assert_eq!(
        delivered.state,
        OrderState::Delivered {
            on: day(2026, 10, 21),
            charge: Money::from_major(200),
            to_customer: Money::ZERO
        }
    );
    assert!(
        suppliers::deliver(delivered.clone(), day(2026, 10, 22), None).is_err(),
        "once"
    );
    assert_rule(
        suppliers::deliver(order.clone(), day(2026, 11, 19), None),
        "R56",
    );
    let undelivered = suppliers::expire(order, day(2026, 11, 19));
    let october = suppliers::statement(
        "lounges",
        day(2026, 10, 1),
        day(2026, 11, 1),
        &[delivered.clone(), undelivered, delivered.clone()],
    );
    assert_eq!(october.statement_id, "supplier/lounges/2026-10-01");
    assert_eq!(
        october.lines.len(),
        1,
        "each delivery once; nothing for the expired order"
    );
    assert_eq!(october.total, Money::from_major(200));
    let november =
        suppliers::statement("lounges", day(2026, 11, 1), day(2026, 12, 1), &[delivered]);
    assert_eq!(november.total, Money::ZERO);
}

#[test]
fn r56_undelivered_in_kind_goes_back_to_the_customer() {
    let cfg = cfg();
    let order = lounge_order(&cfg);
    assert_eq!(
        suppliers::expire(order.clone(), day(2026, 11, 18)).state,
        OrderState::Issued
    );
    let expired = suppliers::expire(order, day(2026, 11, 19));
    assert_eq!(
        expired.state,
        OrderState::Expired {
            on: day(2026, 11, 19)
        }
    );
    // A partner reward: the supplier must be registered for rewards.
    let menu = rewards::menu(
        Money::from_major(10),
        &[partner_offer("phone_mount", "partner", "partner_shop", 5)],
        &plan(&cfg, "essential"),
        &cfg,
    );
    let mount = rewards::redeem("cust-rider", "redeem/1", "phone_mount", &menu).expect("mount");
    let order = suppliers::order_for_reward(&mount, day(2026, 10, 3), &cfg).expect("order");
    assert_eq!(order.source, OrderSource::Reward);
    assert_eq!(order.expires_on, day(2026, 11, 1));
    let cash = rewards::redeem("cust-rider", "redeem/2", "cash", &menu).expect("cash");
    assert_rule(
        suppliers::order_for_reward(&cash, day(2026, 10, 3), &cfg),
        "R56",
    );
}

#[test]
fn r56_invoiced_payout_charged_at_most_the_amount_rest_in_cash() {
    let cfg = cfg();
    let settled = Settled {
        idempotency_key: "payout/pol-1/occ-1/guaranteed/settled".into(),
        method: Settlement::InKind("alternative_seat".into()),
        amount: Money::from_major(600),
        premium_set_off: Money::ZERO,
        cash: Money::ZERO,
    };
    let order = suppliers::order_for_payout(&settled, day(2026, 10, 20), &cfg).expect("order");
    assert_rule(
        suppliers::deliver(order.clone(), day(2026, 10, 21), None),
        "R56",
    );
    assert_rule(
        suppliers::deliver(
            order.clone(),
            day(2026, 10, 21),
            Some(Money::from_major(601)),
        ),
        "R56",
    );
    let seat = suppliers::deliver(order, day(2026, 10, 21), Some(Money::from_major(420)))
        .expect("delivered");
    assert_eq!(
        seat.state,
        OrderState::Delivered {
            on: day(2026, 10, 21),
            charge: Money::from_major(420),
            to_customer: Money::from_major(180)
        }
    );
}

// ---- Phase 5: claims workflow, review requests, fairness audit, human care (ADR-010)

/// A device claim's world: one policy with device damage cover in October 2026.
struct ClaimWorld {
    cfg: EngineConfig,
    policy: Policy,
    status: ClassStatus,
    premium: PremiumStatus,
    suitable: BTreeSet<String>,
    claims_in_class: u32,
    evidence_reused: bool,
    monthly_earnings: Option<Money>,
}

impl ClaimWorld {
    /// Device damage cover with `sum_insured` (HK$), held since June so no waiting period or
    /// early-claim indicator applies.
    fn new(sum_insured: i64) -> ClaimWorld {
        let cfg = cfg();
        let schedule = Schedule {
            plan_id: "essential".into(),
            allocations: vec![Allocation {
                cover_id: "device_damage".into(),
                premium: Money::from_major(20),
                sum_insured: Money::from_major(sum_insured),
            }],
        };
        let mut policy = policy(&cfg, schedule, oct());
        policy
            .cover_starts
            .insert("device_damage".into(), day(2026, 6, 1));
        ClaimWorld {
            cfg,
            policy,
            status: ClassStatus::default(),
            premium: PremiumStatus::Paid,
            suitable: ["partner_repair".to_string()].into(),
            claims_in_class: 1,
            evidence_reused: false,
            monthly_earnings: None,
        }
    }

    fn ctx(&self) -> ClaimContext<'_> {
        ClaimContext {
            policy: &self.policy,
            class_status: &self.status,
            premium: &self.premium,
            claims_in_class: self.claims_in_class,
            evidence_reused: self.evidence_reused,
            monthly_earnings: self.monthly_earnings,
            suitable: &self.suitable,
        }
    }

    /// A claim notified and with its photo, ready for the assessor.
    fn ready(&self, notice: ClaimNotice) -> Claim {
        let (claim, _) = claims::notify(notice, &self.ctx(), &self.cfg).expect("notified");
        claims::add_evidence(claim, &["photo".to_string()].into(), &self.cfg).expect("evidence")
    }

    fn decide(&self, claim: Claim, review: claims::Review) -> claims::Decided {
        let seed = seed_where(&claim.claim_id, false, &self.cfg);
        claims::decide(
            claim,
            review,
            &self.ctx(),
            at(2026, 10, 21),
            seed,
            &self.cfg,
        )
        .expect("decided")
    }
}

fn notice(claim_id: &str, amount: i64, loss: chrono::NaiveDate) -> ClaimNotice {
    ClaimNotice {
        claim_id: claim_id.into(),
        cover_id: "device_damage".into(),
        loss_date: loss,
        notified_at: at(2026, 10, 20),
        amount_claimed: Money::from_major(amount),
        circumstance: None,
        asked_for_person: false,
        guarantee_requested: Money::ZERO,
    }
}

fn assessed(estimate: i64, confidence: u32, recommend: Recommendation) -> claims::Review {
    claims::Review::Assessed(Assessment {
        estimate: Money::from_major(estimate),
        confidence: Bps(confidence),
        recommend,
        escalation: Escalation::default(),
    })
}

fn approve(estimate: i64) -> claims::Review {
    assessed(estimate, 9_000, Recommendation::Approve)
}

fn referral(d: &claims::Decided) -> ClaimReferral {
    match &d.outcome {
        StpOutcome::Referred { referral } => referral.clone(),
        other => panic!("expected a referral, got {other:?}"),
    }
}

#[test]
fn r57_clean_claim_approved_straight_through() {
    let w = ClaimWorld::new(5_000);
    let claim = w.ready(notice("c-1", 600, day(2026, 10, 18)));
    let d = w.decide(claim, approve(650));
    let StpOutcome::Approved { payment } = &d.outcome else {
        panic!("expected approval, got {:?}", d.outcome);
    };
    assert_eq!(payment.idempotency_key, "claim/c-1/payment");
    assert_eq!(payment.amount, Money::from_major(600));
    // R16, R28: cash first; the invoiced repair the cover permits, suitable now.
    assert_eq!(
        payment.options,
        vec![
            Settlement::Cash,
            Settlement::InKind("partner_repair".into())
        ]
    );
    assert_eq!(
        d.claim.state,
        ClaimState::Approved {
            by: DecidedBy::Stp,
            on: at(2026, 10, 21),
            amount: Money::from_major(600)
        }
    );
    assert!(d.anomaly.indicators.is_empty());
}

#[test]
fn r57_claim_waits_for_the_evidence_its_cover_asks_for() {
    let w = ClaimWorld::new(5_000);
    let (claim, triage) =
        claims::notify(notice("c-2", 600, day(2026, 10, 18)), &w.ctx(), &w.cfg).expect("notified");
    assert!(!triage.is_urgent());
    assert_eq!(
        claim.state,
        ClaimState::AwaitingEvidence {
            missing: ["photo".to_string()].into()
        }
    );
    let early = claims::decide(
        claim.clone(),
        approve(650),
        &w.ctx(),
        at(2026, 10, 21),
        0,
        &w.cfg,
    );
    assert!(matches!(early, Err(EngineError::State(_))));
    assert_rule(
        claims::add_evidence(claim.clone(), &["selfie".to_string()].into(), &w.cfg),
        "R55",
    );
    let claim = claims::add_evidence(claim, &["photo".to_string()].into(), &w.cfg).expect("sent");
    assert_eq!(claim.state, ClaimState::Assessing);
}

#[test]
fn r08_parametric_cover_takes_no_claim() {
    let w = ClaimWorld::new(5_000);
    let mut n = notice("c-3", 300, day(2026, 10, 18));
    n.cover_id = "weather_income".into();
    assert_rule(claims::notify(n, &w.ctx(), &w.cfg), "R08");
}

#[test]
fn r07_claim_in_waiting_period_referred_then_declined_by_a_person() {
    let mut w = ClaimWorld::new(5_000);
    w.policy
        .cover_starts
        .insert("device_damage".into(), day(2026, 10, 1));
    let claim = w.ready(notice("c-4", 600, day(2026, 10, 10)));
    let d = w.decide(claim, approve(650));
    assert_eq!(referral(&d), ClaimReferral::WaitingPeriod);
    // R13: the automated path never declines; a person does, with a reason.
    assert_rule(
        claims::person_decision(
            d.claim.clone(),
            " ",
            PersonOutcome::Decline {
                reason: DeclineReason::WaitingPeriod,
            },
            &w.ctx(),
            at(2026, 10, 22),
            &w.cfg,
        ),
        "R13",
    );
    let (claim, paid) = claims::person_decision(
        d.claim,
        "handler-amy",
        PersonOutcome::Decline {
            reason: DeclineReason::WaitingPeriod,
        },
        &w.ctx(),
        at(2026, 10, 22),
        &w.cfg,
    )
    .expect("declined");
    assert!(paid.is_none());
    assert_eq!(
        claim.state,
        ClaimState::Declined {
            reviewer: "handler-amy".into(),
            on: at(2026, 10, 22),
            reason: DeclineReason::WaitingPeriod
        }
    );
}

#[test]
fn r47_claim_in_reentry_waiting_period_referred() {
    let mut w = ClaimWorld::new(5_000);
    w.status = ClassStatus::Open {
        reentry_waiting_until: Some(day(2026, 10, 15)),
    };
    let d = w.decide(w.ready(notice("c-5", 600, day(2026, 10, 12))), approve(650));
    assert_eq!(referral(&d), ClaimReferral::WaitingPeriod);
}

#[test]
fn r53_claim_after_lapse_referred_and_in_grace_paid_net() {
    let mut w = ClaimWorld::new(5_000);
    w.premium = PremiumStatus::Lapsed {
        since: day(2026, 10, 8),
        reinstate_by: day(2026, 11, 7),
    };
    let d = w.decide(w.ready(notice("c-6", 600, day(2026, 10, 18))), approve(650));
    assert_eq!(referral(&d), ClaimReferral::NotInForce);

    w.premium = PremiumStatus::InGrace {
        last_day: day(2026, 10, 7),
    };
    let d = w.decide(w.ready(notice("c-7", 600, day(2026, 10, 5))), approve(650));
    let StpOutcome::Approved { payment } = d.outcome else {
        panic!("expected approval");
    };
    let settled = claims::settle(&payment, None, Money::from_major(80)).expect("settled");
    assert_eq!(settled.cash, Money::from_major(520));
    assert_eq!(settled.premium_set_off, Money::from_major(80));
    assert_eq!(settled.idempotency_key, "claim/c-7/payment/settled");
    let in_kind = Settlement::InKind("partner_repair".into());
    let repaired = claims::settle(&payment, Some(&in_kind), Money::from_major(80)).expect("ok");
    assert_eq!(repaired.premium_set_off, Money::ZERO);
}

#[test]
fn r13_assessor_can_only_approve_or_refer() {
    let w = ClaimWorld::new(5_000);
    let refer = w.decide(
        w.ready(notice("c-8", 600, day(2026, 10, 18))),
        assessed(650, 9_000, Recommendation::Refer),
    );
    assert_eq!(referral(&refer), ClaimReferral::AssessorReferred);
    let unsure = w.decide(
        w.ready(notice("c-9", 600, day(2026, 10, 18))),
        assessed(650, 8_000, Recommendation::Approve),
    );
    assert_eq!(referral(&unsure), ClaimReferral::LowConfidence);
    // R22: no assessment is a person's, never a decline.
    let none = w.decide(
        w.ready(notice("c-10", 600, day(2026, 10, 18))),
        claims::Review::NoAssessment,
    );
    assert_eq!(referral(&none), ClaimReferral::NoAssessment);
    assert!(
        matches!(none.claim.state, ClaimState::WithPerson { due, .. } if due == at(2026, 10, 23))
    );
}

#[test]
fn r57_amount_above_estimate_tolerance_referred() {
    let w = ClaimWorld::new(5_000);
    // 15% over HK$650 is HK$747.50.
    let over = w.decide(
        w.ready(notice("c-11", 748, day(2026, 10, 18))),
        approve(650),
    );
    assert_eq!(referral(&over), ClaimReferral::AboveEstimate);
    let within = w.decide(
        w.ready(notice("c-12", 747, day(2026, 10, 18))),
        approve(650),
    );
    assert!(matches!(within.outcome, StpOutcome::Approved { .. }));
}

#[test]
fn r57_claim_above_stp_limit_referred() {
    let w = ClaimWorld::new(5_000);
    let d = w.decide(
        w.ready(notice("c-13", 2_500, day(2026, 10, 18))),
        approve(2_600),
    );
    assert_eq!(referral(&d), ClaimReferral::AboveStpLimit);
}

#[test]
fn r48_first_loss_no_average() {
    // A HK$8,000 phone insured for HK$3,000: a HK$2,500 loss is paid in full. An average clause
    // would pay 3,000 / 8,000 of it.
    assert_eq!(
        claims::first_loss(Money::from_major(2_500), Money::from_major(3_000)),
        Money::from_major(2_500)
    );
    assert_eq!(
        claims::first_loss(Money::from_major(4_000), Money::from_major(3_000)),
        Money::from_major(3_000)
    );
    let w = ClaimWorld::new(3_000);
    let d = w.decide(
        w.ready(notice("c-14", 2_500, day(2026, 10, 18))),
        approve(2_400),
    );
    assert_eq!(referral(&d), ClaimReferral::AboveStpLimit);
    let (claim, paid) = claims::person_decision(
        d.claim,
        "handler-amy",
        PersonOutcome::Approve {
            amount: Money::from_major(4_000),
        },
        &w.ctx(),
        at(2026, 10, 22),
        &w.cfg,
    )
    .expect("approved");
    assert_eq!(paid.expect("paid").amount, Money::from_major(3_000));
    assert!(matches!(
        claim.state,
        ClaimState::Approved {
            by: DecidedBy::Person,
            ..
        }
    ));
}

#[test]
fn r14_claim_drawn_for_audit_held_before_payment() {
    let w = ClaimWorld::new(5_000);
    let claim = w.ready(notice("c-15", 600, day(2026, 10, 18)));
    let seed = seed_where(&claim.claim_id, true, &w.cfg);
    let d = claims::decide(
        claim,
        approve(650),
        &w.ctx(),
        at(2026, 10, 21),
        seed,
        &w.cfg,
    )
    .expect("decided");
    assert_eq!(
        referral(&d),
        ClaimReferral::Audit {
            reason: AuditReason::RandomSample
        }
    );
    let (_, paid) = claims::person_decision(
        d.claim,
        "auditor-ben",
        PersonOutcome::Approve {
            amount: Money::from_major(600),
        },
        &w.ctx(),
        at(2026, 10, 22),
        &w.cfg,
    )
    .expect("approved");
    assert_eq!(
        paid.expect("paid after the audit").amount,
        Money::from_major(600)
    );
}

#[test]
fn r14_reused_evidence_always_reaches_risk_review() {
    let mut w = ClaimWorld::new(5_000);
    w.evidence_reused = true;
    let d = w.decide(
        w.ready(notice("c-16", 600, day(2026, 10, 18))),
        approve(650),
    );
    assert_eq!(
        referral(&d),
        ClaimReferral::Audit {
            reason: AuditReason::RiskReview
        }
    );
    assert!(d.anomaly.indicators.contains(&Indicator::EvidenceReused));
    assert!(d.anomaly.score >= w.cfg.guardrails.risk_review_threshold);
}

#[test]
fn r14_anomaly_score_names_its_indicators() {
    let mut w = ClaimWorld::new(1_000);
    w.policy
        .cover_starts
        .insert("device_damage".into(), day(2026, 10, 1));
    w.claims_in_class = 3;
    // Covered from 15 October; a loss on the 18th, HK$950 of HK$1,000, the third claim.
    let claim = w.ready(notice("c-17", 950, day(2026, 10, 18)));
    let score = claims::anomaly(&claim, &w.ctx(), &w.cfg).expect("score");
    assert_eq!(
        score.indicators,
        [
            Indicator::EarlyClaim,
            Indicator::FrequentClaims,
            Indicator::NearSumInsured
        ]
        .into()
    );
    assert_eq!(score.score, Bps(8_000));
    let d = w.decide(claim, approve(1_000));
    assert_eq!(
        referral(&d),
        ClaimReferral::Audit {
            reason: AuditReason::RiskReview
        }
    );
}

#[test]
fn r15_declined_claim_reviewed_by_a_different_person() {
    let w = ClaimWorld::new(5_000);
    let d = w.decide(
        w.ready(notice("c-18", 600, day(2026, 10, 18))),
        assessed(650, 9_000, Recommendation::Refer),
    );
    let decline = PersonOutcome::Decline {
        reason: DeclineReason::NotAnInsuredLoss,
    };
    let (claim, _) = claims::person_decision(
        d.claim,
        "handler-amy",
        decline.clone(),
        &w.ctx(),
        at(2026, 10, 22),
        &w.cfg,
    )
    .expect("declined");
    let claim = claims::appeal(claim, at(2026, 10, 23), &w.cfg).expect("review requested");
    assert_eq!(
        claim.state,
        ClaimState::WithPerson {
            since: at(2026, 10, 23),
            due: at(2026, 10, 26),
            referral: ClaimReferral::Appeal {
                declined_by: "handler-amy".into()
            }
        }
    );
    assert_rule(
        claims::person_decision(
            claim.clone(),
            "handler-amy",
            decline,
            &w.ctx(),
            at(2026, 10, 24),
            &w.cfg,
        ),
        "R15",
    );
    let (_, paid) = claims::person_decision(
        claim,
        "senior-chan",
        PersonOutcome::Approve {
            amount: Money::from_major(600),
        },
        &w.ctx(),
        at(2026, 10, 24),
        &w.cfg,
    )
    .expect("overturned");
    assert!(paid.is_some());
}

fn decision(actor: Actor, reviewer: Option<&str>) -> DecisionRecord {
    DecisionRecord {
        decision_id: format!("dec-{actor:?}"),
        actor,
        reviewer: reviewer.map(str::to_string),
        subject_type: "payout".into(),
        subject_id: "payout/pol-1/occ-1/guaranteed".into(),
        customer_id: "cust-rider".into(),
        decision: "not matched".into(),
        inputs_hash: "0".into(),
        reason: "Outside declared hours".into(),
        at: at(2026, 10, 1),
    }
}

#[test]
fn r15_any_ai_decision_appealable() {
    let cfg = cfg();
    let now = at(2026, 10, 2);
    for actor in [Actor::Ai, Actor::Rules] {
        let a = appeals::open(
            &decision(actor, None),
            "cust-rider",
            "I was working",
            now,
            &cfg.guardrails,
        )
        .expect("open");
        assert_eq!(a.due_at, now + Duration::hours(72));
        assert_eq!(a.state, AppealState::Open);
    }
    // A customer's own choice is changed by them, not reviewed.
    assert_rule(
        appeals::open(
            &decision(Actor::Customer, None),
            "cust-rider",
            "x",
            now,
            &cfg.guardrails,
        ),
        "R15",
    );
    // A person's decision is reviewed by someone else.
    let a = appeals::open(
        &decision(Actor::Person, Some("handler-amy")),
        "cust-rider",
        "Please look again",
        now,
        &cfg.guardrails,
    )
    .expect("open");
    assert_rule(
        appeals::decide(a.clone(), "handler-amy", true, "", now),
        "R15",
    );
    assert!(!appeals::overdue(&a, now + Duration::hours(72)));
    assert!(appeals::overdue(&a, now + Duration::hours(73)));
    let a =
        appeals::decide(a, "senior-chan", true, "Working hours confirmed", now).expect("decided");
    assert!(matches!(a.state, AppealState::Overturned { .. }));
    assert!(!appeals::overdue(&a, now + Duration::hours(100)));
}

fn observations(
    kind: &str,
    attribute: &str,
    group: &str,
    n: u32,
    approved: u32,
) -> Vec<Observation> {
    (0..n)
        .map(|i| Observation {
            kind: kind.into(),
            attribute: attribute.into(),
            group: group.into(),
            approved: i < approved,
        })
        .collect()
}

#[test]
fn r19_parity_report_flags_gaps() {
    let cfg = cfg();
    let mut obs = observations("claim", "age_band", "18-29", 100, 90);
    obs.extend(observations("claim", "age_band", "30-49", 100, 70));
    obs.extend(observations("claim", "age_band", "50+", 100, 60));
    // Too few decisions to compare.
    obs.extend(observations("claim", "age_band", "65+", 10, 1));
    obs.extend(observations("claim", "gender", "f", 50, 45));
    obs.extend(observations("claim", "gender", "m", 50, 44));
    let reports = fairness::parity_reports(&obs, &cfg.guardrails.fairness).expect("reports");
    assert_eq!(reports.len(), 2);
    let age = &reports[0];
    assert_eq!(age.attribute, "age_band");
    // 60% against 90%: 0.667 of the best, under four-fifths. 70% is 0.778: also flagged.
    assert_eq!(age.min_ratio, Some(Bps(6_667)));
    assert_eq!(age.flagged, vec!["30-49".to_string(), "50+".to_string()]);
    assert!(!age.groups["65+"].credible);
    let gender = &reports[1];
    assert!(gender.flagged.is_empty());
    // The audit store holds only the groups the insurer declared.
    let other = observations("claim", "ethnicity", "x", 1, 1);
    assert_rule(
        fairness::parity_reports(&other, &cfg.guardrails.fairness),
        "R19",
    );
}

fn case(circumstance: Option<&str>, amount: i64) -> CaseFacts<'_> {
    CaseFacts {
        notified_at: at(2026, 10, 20),
        circumstance,
        amount: Money::from_major(amount),
        monthly_earnings: None,
        asked_for_person: false,
        guarantee_requested: Money::ZERO,
        cover_active: true,
    }
}

#[test]
fn r46_urgent_first_response_before_person() {
    let cfg = cfg();
    let t = human_care::triage(&case(Some("injury"), 3_000), None, &cfg.guardrails).expect("ok");
    assert_eq!(
        t.urgent,
        [UrgentBecause::Circumstance("injury".into())].into()
    );
    let first = t.first_response.expect("first response");
    assert_eq!(first.respond_by, at(2026, 10, 20) + Duration::seconds(5));
    assert_eq!(
        first.actions,
        vec![
            FirstAction::EmergencyNumbers,
            FirstAction::AssistanceProviderConnect,
            FirstAction::GuaranteeOfPayment
        ]
    );
    // A loss of half the customer's monthly earnings is urgent too.
    let mut large = case(None, 6_000);
    large.monthly_earnings = Some(Money::from_major(12_000));
    let t = human_care::triage(&large, None, &cfg.guardrails).expect("ok");
    assert_eq!(t.urgent, [UrgentBecause::LargeLoss].into());
    // An unconfigured circumstance is refused, not guessed.
    assert_rule(
        human_care::triage(&case(Some("lost_keys"), 100), None, &cfg.guardrails),
        "R46",
    );
    // Earnings: HK$600 a day on one declared weekday, 52 weeks over 12 months.
    assert_eq!(
        human_care::monthly_earnings(&rider()),
        Some(Money::from_major(2_600))
    );
}

#[test]
fn r46_person_offered_after_first_response() {
    let cfg = cfg();
    let urgent = human_care::triage(&case(Some("hospitalisation"), 3_000), None, &cfg.guardrails)
        .expect("ok");
    assert!(urgent.first_response.is_some());
    assert_eq!(urgent.person, PersonRoute::Offered);
    let calm = human_care::triage(&case(None, 300), None, &cfg.guardrails).expect("ok");
    assert!(calm.first_response.is_none());
    assert_eq!(calm.person, PersonRoute::NotNeeded);
    let mut asked = case(None, 300);
    asked.asked_for_person = true;
    let asked = human_care::triage(&asked, None, &cfg.guardrails).expect("ok");
    assert_eq!(asked.person, PersonRoute::Assigned);
    // A claim notified with a request for a person goes straight to one.
    let w = ClaimWorld::new(5_000);
    let mut n = notice("c-19", 600, day(2026, 10, 18));
    n.asked_for_person = true;
    let (claim, _) = claims::notify(n, &w.ctx(), &w.cfg).expect("notified");
    assert!(matches!(
        claim.state,
        ClaimState::WithPerson {
            referral: ClaimReferral::HumanCare,
            ..
        }
    ));
}

#[test]
fn r46_auto_guarantee_capped() {
    let cfg = cfg();
    let mut facts = case(Some("hospitalisation"), 8_000);
    facts.guarantee_requested = Money::from_major(8_000);
    let t = human_care::triage(&facts, None, &cfg.guardrails).expect("ok");
    let g = t
        .first_response
        .and_then(|f| f.guarantee)
        .expect("guarantee");
    assert_eq!(g.automatic, Money::from_major(5_000));
    assert_eq!(g.to_confirm, Money::from_major(3_000));
    // No active cover in the class: no automatic guarantee, the first response still goes.
    facts.cover_active = false;
    let t = human_care::triage(&facts, None, &cfg.guardrails).expect("ok");
    let first = t.first_response.expect("first response");
    assert!(first.guarantee.is_none());
}

#[test]
fn r46_ai_escalates_never_deescalates() {
    let cfg = cfg();
    for circumstance in [None, Some("injury")] {
        for asked in [false, true] {
            let mut facts = case(circumstance, 300);
            facts.asked_for_person = asked;
            let rules = human_care::triage(&facts, None, &cfg.guardrails).expect("ok");
            for urgent in [false, true] {
                for care in [false, true] {
                    let ai = Escalation {
                        urgent,
                        human_care: care,
                    };
                    let t = human_care::triage(&facts, Some(ai), &cfg.guardrails).expect("ok");
                    assert!(t.is_urgent() >= rules.is_urgent());
                    assert!(t.person >= rules.person);
                    assert_eq!(t.is_urgent(), rules.is_urgent() || urgent);
                }
            }
        }
    }
    // An escalation from the assessor makes a calm claim urgent and sends its first response.
    let w = ClaimWorld::new(5_000);
    let review = claims::Review::Assessed(Assessment {
        estimate: Money::from_major(650),
        confidence: Bps(9_000),
        recommend: Recommendation::Approve,
        escalation: Escalation {
            urgent: true,
            human_care: true,
        },
    });
    let d = w.decide(w.ready(notice("c-20", 600, day(2026, 10, 18))), review);
    assert!(d.claim.urgent);
    assert!(
        d.newly_urgent
            .as_ref()
            .is_some_and(|t| t.first_response.is_some())
    );
    assert_eq!(referral(&d), ClaimReferral::HumanCare);
}

#[test]
fn r39_claim_counts_in_experience_once_decided() {
    let w = ClaimWorld::new(5_000);
    let claim = w.ready(notice("c-21", 600, day(2026, 10, 18)));
    assert_eq!(
        claims::experience(&claim, oct().starts).status,
        ClaimStatus::UnderReview
    );
    let d = w.decide(claim, approve(650));
    assert_eq!(
        claims::experience(&d.claim, oct().starts).status,
        ClaimStatus::Paid
    );
}

#[test]
fn r10_reward_voided_only_on_a_persons_fraud_finding() {
    let cfg = cfg();
    let earned = RewardEarned {
        reward_id: "task/cust-rider/case_and_screen_protector/2026-10-01".into(),
        customer_id: "cust-rider".into(),
        pool: "property".into(),
        earned_for: EarnedFor::VerifiedActivity {
            task_id: "task/cust-rider/case_and_screen_protector/2026-10-01".into(),
        },
        amount: Money::from_major(20),
        on: day(2026, 10, 2),
    };
    let grant = rewards::grant(earned, &[], &cfg);
    let finding = |reviewer: &str| rewards::FraudFinding {
        reviewer: reviewer.into(),
        on: day(2026, 10, 20),
        note: "Same photo as another customer".into(),
    };
    assert_rule(
        rewards::void(&grant, finding(""), Money::from_major(20)),
        "R13",
    );
    // HK$15 of the HK$20 is already spent: HK$5 is taken back, HK$15 is unrecovered.
    let voided = rewards::void(&grant, finding("fraud-dee"), Money::from_major(5)).expect("void");
    assert_eq!(voided.amount, Money::from_major(5));
    assert_eq!(voided.unrecovered, Money::from_major(15));
    assert_eq!(
        voided.idempotency_key,
        format!("{}/void", grant.idempotency_key)
    );
}

#[test]
fn r14_audit_overturn_rate_counts_the_random_sample_only() {
    let finding = |id: &str, kind: &str, reason, agreed| AuditFinding {
        decision_id: id.into(),
        kind: kind.into(),
        reason,
        reviewer: "auditor-ben".into(),
        agreed,
        note: String::new(),
        on: at(2026, 10, 20),
    };
    let findings = vec![
        finding("a", "claim", AuditReason::RandomSample, true),
        finding("b", "claim", AuditReason::RandomSample, false),
        finding("c", "claim", AuditReason::RandomSample, true),
        finding("d", "claim", AuditReason::RandomSample, true),
        finding("e", "claim", AuditReason::RiskReview, false),
        finding("f", "payout", AuditReason::RandomSample, true),
    ];
    let stats = audit::overturn_rates(&findings);
    assert_eq!(stats.len(), 2);
    assert_eq!((stats[0].kind.as_str(), stats[0].sampled), ("claim", 4));
    assert_eq!(stats[0].overturn_rate, Bps(2_500));
    assert_eq!(stats[1].overturn_rate, Bps::ZERO);
}

#[test]
fn r13_only_a_claim_referral_offers_a_decline() {
    let w = ClaimWorld::new(5_000);
    let mut urgent_notice = notice("c-23", 600, day(2026, 10, 18));
    urgent_notice.circumstance = Some("injury".into());
    let referred = |n| {
        let claim = w.ready(n);
        w.decide(claim, claims::Review::NoAssessment).claim
    };
    let calm = queue::claim_item(&referred(notice("c-22", 600, day(2026, 10, 18)))).expect("item");
    let urgent = queue::claim_item(&referred(urgent_notice)).expect("item");
    assert!(urgent.urgent);
    let audit = queue::audit_item(
        "payout/pol-1/occ-1/guaranteed",
        "payout",
        "cust-rider",
        AuditReason::RandomSample,
        at(2026, 10, 19),
        &w.cfg,
    );
    let appeal = appeals::open(
        &decision(Actor::Ai, None),
        "cust-rider",
        "Please look again",
        at(2026, 10, 19),
        &w.cfg.guardrails,
    )
    .expect("open");
    let appeal = queue::appeal_item(&appeal).expect("item");
    for item in [&calm, &audit, &appeal] {
        let declines = item.actions.contains(&PersonAction::Decline);
        assert_eq!(
            declines,
            matches!(item.work, WorkKind::ClaimReferral { .. })
        );
    }
    assert_eq!(
        audit.actions,
        vec![PersonAction::Agree, PersonAction::Overturn]
    );
    let mut items = vec![audit.clone(), calm.clone(), appeal.clone(), urgent.clone()];
    queue::order(&mut items);
    // Urgent first, then by due time.
    let ids: Vec<&str> = items.iter().map(|i| i.item_id.as_str()).collect();
    assert_eq!(ids[0], urgent.item_id);
    assert_eq!(ids[1], audit.item_id);
}
