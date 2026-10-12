use microinsurance_learning::insights::{
    CustomerFacts, SegmentRow, Unit, partner_results, suppress_small_cohorts,
};

const MIN_COHORT: u32 = 50;

#[test]
fn r21_small_cohorts_are_suppressed() {
    let row = |n| SegmentRow {
        segment: "travellers".into(),
        metric: "upgrade_pref".into(),
        value_bps: 6_000,
        cohort_size: n,
    };
    let kept = suppress_small_cohorts(vec![row(MIN_COHORT - 1), row(MIN_COHORT)], MIN_COHORT);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].cohort_size, MIN_COHORT);
}

fn customer(tasks: u32, verified: u32, payouts: u32) -> CustomerFacts {
    CustomerFacts {
        through_partner: true,
        in_force: true,
        tasks_offered: tasks,
        tasks_verified: verified,
        payouts,
        payout_minor: i64::from(payouts) * 48_000,
        ..CustomerFacts::default()
    }
}

#[test]
fn r21_partner_sees_aggregates_over_enough_customers_only() {
    // 60 customers came through the partner; 40 had prevention tasks, 10 had a payout.
    let mut facts: Vec<CustomerFacts> = (0..60)
        .map(|i| customer(u32::from(i < 40), u32::from(i < 30), u32::from(i < 10)))
        .collect();
    // Another channel's customers are not the partner's.
    facts.extend((0..100).map(|_| CustomerFacts {
        through_partner: false,
        ..customer(1, 1, 1)
    }));
    let r = partner_results("delivery_app", &facts, MIN_COHORT);
    let get = |m: &str| r.figures.iter().find(|f| f.metric == m).expect("figure");
    assert_eq!(get("subscribers").value, Some(60));
    assert_eq!(get("in_force").value, Some(60));
    // 40 customers with tasks: below the cohort, withheld, never rounded or estimated.
    let completion = get("prevention_task_completion");
    assert_eq!((completion.value, completion.cohort_size), (None, 40));
    assert_eq!(completion.unit, Unit::Bps);
    assert_eq!(get("payouts").value, None);
    assert_eq!(get("payouts_amount").value, None);
    assert_eq!(get("redemptions").cohort_size, 0);

    let r = partner_results("delivery_app", &facts, 40);
    let get = |m: &str| r.figures.iter().find(|f| f.metric == m).expect("figure");
    assert_eq!(get("prevention_task_completion").value, Some(7_500));
}

#[test]
fn r21_partner_sees_its_funnel_and_retention_over_enough_customers() {
    // 80 came through the partner and were quoted; 60 bound; of 50 whose first period ended, 40
    // renewed; 50 were granted rewards, two each, and took 1 each from their balance.
    let facts: Vec<CustomerFacts> = (0..80)
        .map(|i| CustomerFacts {
            through_partner: true,
            quoted: true,
            bound: i < 60,
            due_to_renew: i < 50,
            renewed: i < 40,
            rewards_granted: if i < 50 { 2 } else { 0 },
            rewards_redeemed: u32::from(i < 50),
            ..CustomerFacts::default()
        })
        .collect();
    let r = partner_results("delivery_app", &facts, MIN_COHORT);
    let get = |m: &str| r.figures.iter().find(|f| f.metric == m).expect("figure");
    assert_eq!(get("quoted").value, Some(80));
    assert_eq!(get("conversion").value, Some(7_500));
    assert_eq!(get("renewal").value, Some(8_000));
    assert_eq!(get("reward_redemption").value, Some(5_000));
    // Fewer than the cohort: withheld.
    let r = partner_results("delivery_app", &facts[..45], MIN_COHORT);
    let get = |m: &str| r.figures.iter().find(|f| f.metric == m).expect("figure");
    assert_eq!(get("conversion").value, None);
}

#[test]
fn r21_a_zero_cohort_setting_still_needs_one_customer() {
    let r = partner_results("delivery_app", &[], 0);
    assert!(r.figures.iter().all(|f| f.value.is_none()));
}
