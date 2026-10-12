//! Targets in docs/performance.md.
#[path = "../tests/common/mod.rs"]
mod common;

use chrono::Duration;
use common::*;
use criterion::{Criterion, criterion_group, criterion_main};
use microinsurance_engine::money::{Bps, Money};
use microinsurance_engine::optimiser::{self, rating};
use microinsurance_engine::payouts::{self, OccurrencePayouts};
use microinsurance_engine::plans;
use microinsurance_engine::prevention::{self, PreventionTaskDraft};
use microinsurance_engine::triggers::{self, SourceReport};
use microinsurance_engine::types::{Allocation, Schedule, WorkWindow};
use std::collections::{BTreeMap, BTreeSet};
use std::hint::black_box;

fn prevention_test(c: &mut Criterion) {
    let m = PreventionTaskDraft {
        task_type: "case".into(),
        cover_id: "device_damage".into(),
        event_reference: None,
        action: "case photo".into(),
        evidence: "photo".into(),
        claim_prob_before: Bps(400),
        claim_prob_after: Bps(250),
        avg_claim_cost: Money::from_major(800),
        reward_cost: Money(300),
    };
    c.bench_function("prevention_test_admit", |b| {
        b.iter(|| prevention::admit(black_box(&m)).is_ok())
    });
}

/// One customer's policy period: rate every cover of the plan, then suggest an allocation.
/// Target: < 50 µs (docs/performance.md).
fn optimise_one_customer(c: &mut Criterion) {
    let w = World::new(&rider(), &basis(), period(1, 2026, 10));
    let plan = w.plan("max");
    let covers = plans::covers_for(&plan, &w.cfg);
    let (profile, inputs, basis) = (rider(), inputs(), basis());
    c.bench_function("optimise_one_customer", |b| {
        b.iter(|| {
            let ratings = rating::rate_covers(
                &covers,
                w.period.starts,
                &inputs,
                &profile.consents,
                &basis,
                &w.cfg,
            )
            .expect("rated");
            let ctx = optimiser::PeriodContext {
                ratings: &ratings,
                ..w.ctx()
            };
            optimiser::suggest(black_box(&plan), black_box(&profile), None, &ctx).expect("ok")
        })
    });
}

/// Signal 8 across 1,000,000 weather income policies: match every policy and decide every
/// guaranteed payout. Target: < 5 s on 4 vCPU (docs/performance.md); this runs on one core.
fn trigger_fanout(c: &mut Criterion) {
    let cfg = cfg();
    let template = policy(
        &cfg,
        Schedule {
            plan_id: "essential".into(),
            allocations: vec![Allocation {
                cover_id: "weather_income".into(),
                premium: Money::from_major(40),
                sum_insured: Money::from_major(500),
            }],
        },
        period(1, 2026, 10),
    );
    let policies: Vec<_> = (0..1_000_000_u32)
        .map(|i| {
            let mut p = template.clone();
            p.policy_id = format!("pol-{i}");
            p.work_windows = vec![WorkWindow {
                weekday: u8::try_from(i % 7 + 1).unwrap_or(1),
                start_minute: 9 * 60,
                end_minute: 18 * 60,
            }];
            p
        })
        .collect();
    let index = triggers::build_index(&policies, &cfg.catalogue);
    let starts = at(2026, 10, 5) - Duration::hours(4);
    let report = SourceReport {
        report_id: "bench".into(),
        source: "hko_warnings".into(),
        code: "TC8NE".into(),
        starts,
        ends: None,
        zone: None,
        delay_minutes: None,
        reference: None,
    };
    let mut events = BTreeMap::new();
    triggers::track(&mut events, "hko_warnings", &[report], starts, &cfg);
    let mut occurrences = BTreeMap::new();
    triggers::group(&mut occurrences, &events, starts, &cfg);
    let occurrence = occurrences.into_values().next().expect("occurrence");
    let as_of = starts + Duration::hours(6);
    let suitable = BTreeSet::new();
    let mut group = c.benchmark_group("trigger_fanout");
    group.sample_size(10);
    group.bench_function("signal_8_one_million_policies", |b| {
        b.iter(|| {
            let matches = triggers::match_occurrence(
                &occurrence,
                &events,
                black_box(&policies),
                &index,
                as_of,
                &cfg,
            );
            let mut book =
                OccurrencePayouts::open(&occurrence, triggers::benefit_in_force(&matches), &cfg)
                    .expect("limit");
            payouts::guaranteed(&mut book, &matches, &suitable, &cfg).len()
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    prevention_test,
    optimise_one_customer,
    trigger_fanout
);
criterion_main!(benches);
