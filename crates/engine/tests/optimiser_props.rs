//! Property tests for the optimiser (R02, R09, R34, R42).
mod common;

use common::*;
use microinsurance_engine::catalogue;
use microinsurance_engine::money::Money;
use microinsurance_engine::optimiser;
use microinsurance_engine::types::{DeclaredItem, Profile, UpcomingEvent};
use proptest::prelude::*;

fn profile(
    earnings: Option<i64>,
    route: bool,
    device: Option<i64>,
    event: Option<(u32, i64)>,
) -> Profile {
    let mut p = rider();
    p.typical_daily_earnings = earnings.map(Money::from_major);
    if !route {
        p.commute_routes.clear();
    }
    p.declared_items = device
        .map(|v| DeclaredItem {
            kind: "device.phone".into(),
            description: "phone".into(),
            declared_value: Some(Money::from_major(v)),
        })
        .into_iter()
        .collect();
    p.upcoming_events = event
        .map(|(d, v)| UpcomingEvent {
            kind: "event.concert".into(),
            starts: day(2026, 10, d),
            ends: None,
            reference: None,
            value: Some(Money::from_major(v)),
        })
        .into_iter()
        .collect();
    p
}

proptest! {
    #[test]
    fn r02_any_profile_allocates_in_steps_within_the_rules(
        earnings in proptest::option::of(0_i64..3_000),
        route in any::<bool>(),
        device in proptest::option::of(100_i64..20_000),
        event in proptest::option::of((1_u32..=31, 50_i64..5_000)),
        plan_ix in 0_usize..3,
    ) {
        let p = profile(earnings, route, device, event);
        let w = World::new(&p, &basis(), period(1, 2026, 10));
        let plan = w.cfg.plans[plan_ix].clone();
        let out = optimiser::suggest(&plan, &p, None, &w.ctx()).expect("suggestion").outcome;
        let step = w.cfg.policy_period.allocation_step;

        // The whole plan price is placed unless no split can take it (R42).
        prop_assert_eq!(out.schedule.total_premium() + out.unallocated, plan.price);
        prop_assert_eq!(out.unallocated, out.beyond_capacity);
        for a in &out.schedule.allocations {
            let cover = w.cfg.cover(&a.cover_id).expect("cover");
            prop_assert!(a.premium.is_multiple_of(step));
            prop_assert!(Some(a.sum_insured) <= cover.product_limit);
            if let Some(cap) = catalogue::max_parametric_payout(cover, &p) {
                prop_assert!(a.sum_insured <= cap);
            }
        }

        // Same input, same output.
        let again = optimiser::suggest(&plan, &p, None, &w.ctx()).expect("again").outcome;
        prop_assert_eq!(&again, &out);
        // Renewing the unchanged default keeps it (R34).
        let renewed = optimiser::renew(&out.schedule, &plan, &p, &w.ctx()).expect("renewal");
        prop_assert_eq!(renewed.schedule, out.schedule);
    }
}
