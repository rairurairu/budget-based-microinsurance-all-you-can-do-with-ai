//! Every store passes the same tests. The PostgreSQL store runs when `DATABASE_URL` is set
//! (`just dev` starts one); without it those tests say so and pass.
use chrono::NaiveDate;
use microinsurance_engine::config::Tariff;
use microinsurance_engine::money::Money;
use microinsurance_engine::payouts::{
    PayoutDecision, PayoutLine, PayoutStage, Settlement, idempotency_key, settle,
};
use microinsurance_engine::rewards::{EarnedFor, Grant, Redeemed, Redemption, RewardEarned};
use microinsurance_engine::suppliers::{
    InKindOrder, OrderSource, OrderState, SupplierStatement, statement,
};
use microinsurance_ledger::{
    Account, LedgerError, LedgerStore, MemoryLedger, PgLedger, PostOutcome, Posting,
    check_balanced, postings,
};

fn payout(cents: i64) -> [Posting; 2] {
    [
        Posting {
            account: Account::InsurerRisk,
            amount: Money(cents),
        },
        Posting {
            account: Account::PayoutsPayable,
            amount: Money(-cents),
        },
    ]
}

fn decision(policy: &str, cents: i64) -> PayoutDecision {
    PayoutDecision {
        idempotency_key: idempotency_key(
            policy,
            "windstorm_and_rain:all:-:20260914T000000Z",
            PayoutStage::Guaranteed,
        ),
        policy_id: policy.into(),
        occurrence_id: "windstorm_and_rain:all:-:20260914T000000Z".into(),
        stage: PayoutStage::Guaranteed,
        lines: vec![PayoutLine {
            cover_id: "weather_income".into(),
            amount: Money(cents),
        }],
        amount: Money(cents),
        options: vec![Settlement::Cash],
    }
}

/// A fresh, isolated key space for each run against a shared database.
fn unique(key: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{key}/{nanos}")
}

async fn r27_second_post_with_same_key_is_already_posted(ledger: &impl LedgerStore) {
    let key = unique(&idempotency_key(
        "pol-1",
        "hko-TC8NE-2026-09-14",
        PayoutStage::Guaranteed,
    ));
    let before = ledger
        .balance(Account::PayoutsPayable)
        .await
        .expect("balance");
    assert_eq!(
        ledger.post(&key, &payout(40_000)).await.ok(),
        Some(PostOutcome::Posted)
    );
    assert_eq!(
        ledger.post(&key, &payout(40_000)).await.ok(),
        Some(PostOutcome::AlreadyPosted)
    );
    let paid = ledger
        .balance(Account::PayoutsPayable)
        .await
        .expect("balance");
    assert_eq!(paid - before, Money(-40_000), "paid once");
    // A different amount under the same key is a bug upstream, never a second payout.
    assert!(matches!(
        ledger.post(&key, &payout(50_000)).await,
        Err(LedgerError::KeyReused(_))
    ));
}

async fn unbalanced_post_changes_nothing(ledger: &impl LedgerStore) {
    let before = ledger.balance(Account::Cash).await.expect("balance");
    let entries = [Posting {
        account: Account::Cash,
        amount: Money(1),
    }];
    assert!(ledger.post(&unique("k"), &entries).await.is_err());
    assert_eq!(ledger.balance(Account::Cash).await.ok(), Some(before));
}

async fn r27_batch_retries_post_once(ledger: &impl LedgerStore) {
    let before = ledger.balance(Account::InsurerRisk).await.expect("balance");
    let batch: Vec<(String, Vec<Posting>)> = (0..50)
        .map(|i| (unique(&format!("batch-{i}")), payout(1_000).to_vec()))
        .collect();
    let first = ledger.post_many(&batch).await;
    assert!(first.iter().all(|r| matches!(r, Ok(PostOutcome::Posted))));
    let again = ledger.post_many(&batch).await;
    assert!(
        again
            .iter()
            .all(|r| matches!(r, Ok(PostOutcome::AlreadyPosted)))
    );
    // One reused key fails alone.
    let mut mixed = batch[..2].to_vec();
    mixed[1].1 = payout(2_000).to_vec();
    let results = ledger.post_many(&mixed).await;
    assert!(matches!(results[0], Ok(PostOutcome::AlreadyPosted)));
    assert!(matches!(results[1], Err(LedgerError::KeyReused(_))));
    // A key repeated within one batch posts once.
    let twice = unique("twice");
    let repeated = vec![
        (twice.clone(), payout(700).to_vec()),
        (twice.clone(), payout(700).to_vec()),
        (twice, payout(900).to_vec()),
    ];
    let results = ledger.post_many(&repeated).await;
    assert!(matches!(results[0], Ok(PostOutcome::Posted)));
    assert!(matches!(results[1], Ok(PostOutcome::AlreadyPosted)));
    assert!(matches!(results[2], Err(LedgerError::KeyReused(_))));
    let after = ledger.balance(Account::InsurerRisk).await.expect("balance");
    assert_eq!(after - before, Money(50_700));
}

async fn r53_settlement_postings_balance(ledger: &impl LedgerStore) {
    let d = decision(&unique("pol-grace"), 50_000);
    let decided = postings::payout_decided(&d);
    let settled = postings::payout_settled(&settle(&d, None, Money(8_000)).expect("cash"));
    for (key, entries) in [&decided, &settled] {
        assert!(check_balanced(entries).is_ok());
        assert_eq!(
            ledger.post(key, entries).await.ok(),
            Some(PostOutcome::Posted)
        );
    }
    assert!(settled.1.contains(&Posting {
        account: Account::Cash,
        amount: Money(-42_000)
    }));
    assert!(settled.1.contains(&Posting {
        account: Account::CustomerPremiums,
        amount: Money(-8_000)
    }));
}

async fn every_test(ledger: &impl LedgerStore) {
    r27_second_post_with_same_key_is_already_posted(ledger).await;
    unbalanced_post_changes_nothing(ledger).await;
    r27_batch_retries_post_once(ledger).await;
    r53_settlement_postings_balance(ledger).await;
}

#[test]
fn balanced_postings_pass() {
    assert!(check_balanced(&payout(5_000)).is_ok());
}

#[test]
fn unbalanced_postings_fail() {
    let entries = [Posting {
        account: Account::Cash,
        amount: Money(1),
    }];
    assert!(check_balanced(&entries).is_err());
}

#[test]
fn accounts_round_trip_their_names() {
    for a in Account::ALL {
        assert_eq!(Account::parse(a.as_str()), Some(a));
    }
}

#[tokio::test]
async fn memory_store() {
    every_test(&MemoryLedger::new()).await;
}

#[tokio::test]
async fn postgres_store() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL not set: PostgreSQL ledger tests skipped");
        return;
    };
    let ledger = PgLedger::connect(&url).await.expect("connects");
    ledger.migrate().await.expect("migrates");
    ledger
        .migrate()
        .await
        .expect("migrating twice changes nothing");
    every_test(&ledger).await;
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("date")
}

fn order(id: &str, source: OrderSource, tariff: Tariff, reserved: i64) -> InKindOrder {
    InKindOrder {
        order_id: id.into(),
        supplier_id: "lounges".into(),
        option: "lounge_access".into(),
        source,
        tariff,
        reserved: Money(reserved),
        issued_on: day(1),
        expires_on: day(30),
        state: OrderState::Issued,
    }
}

async fn post_all(ledger: &MemoryLedger, sets: &[(String, Vec<Posting>)]) {
    for (key, entries) in sets {
        assert!(check_balanced(entries).is_ok(), "{key}");
        assert_eq!(
            ledger.post(key, entries).await.ok(),
            Some(PostOutcome::Posted),
            "{key}"
        );
    }
}

async fn balances(ledger: &MemoryLedger) -> Vec<(Account, i64)> {
    let mut out = Vec::new();
    for a in Account::ALL {
        out.push((a, ledger.balance(a).await.expect("balance").0));
    }
    out
}

/// R56: a HK$600 payout taken as lounge access at a HK$200 tariff, an HK$600 payout taken as an
/// invoiced seat at HK$420, an order never used, and a HK$12 reward spent HK$5 on a partner
/// reward: suppliers are paid what they delivered and nothing else; the customer gets the rest.
#[tokio::test]
async fn r56_in_kind_and_reward_postings_conserve_money() {
    let ledger = MemoryLedger::new();
    let delivered = |mut o: InKindOrder, charge: i64, to_customer: i64| {
        o.state = OrderState::Delivered {
            on: day(5),
            charge: Money(charge),
            to_customer: Money(to_customer),
        };
        o
    };
    let lounge = delivered(
        order(
            "lounge",
            OrderSource::Payout,
            Tariff::Fixed(Money(20_000)),
            60_000,
        ),
        20_000,
        0,
    );
    let seat = delivered(
        order("seat", OrderSource::Payout, Tariff::Invoiced, 60_000),
        42_000,
        18_000,
    );
    let mut unused = order(
        "unused",
        OrderSource::Payout,
        Tariff::Fixed(Money(20_000)),
        60_000,
    );
    unused.state = OrderState::Expired { on: day(31) };
    let grant = Grant {
        idempotency_key: "reward/task-1".into(),
        reward: RewardEarned {
            reward_id: "task-1".into(),
            customer_id: "c".into(),
            pool: "property".into(),
            earned_for: EarnedFor::VerifiedActivity {
                task_id: "task-1".into(),
            },
            amount: Money(1_200),
            on: day(3),
        },
        granted: Money(1_200),
        over_cap: Money::ZERO,
    };
    let spent = Redemption {
        redemption_id: "redeem/1".into(),
        customer_id: "c".into(),
        offer_id: "phone_mount".into(),
        amount: Money(500),
        method: Redeemed::InKind {
            supplier_id: "lounges".into(),
        },
    };
    let mount = delivered(
        order(
            "redeem/1",
            OrderSource::Reward,
            Tariff::Fixed(Money(500)),
            500,
        ),
        500,
        0,
    );

    // Each payout decided and settled in kind: the whole amount reserved.
    let mut sets = Vec::new();
    for id in ["lounge", "seat", "unused"] {
        sets.push((format!("{id}/decided"), payout(60_000).to_vec()));
        sets.push((
            format!("{id}/settled"),
            vec![
                Posting {
                    account: Account::PayoutsPayable,
                    amount: Money(60_000),
                },
                Posting {
                    account: Account::InKindPayable,
                    amount: Money(-60_000),
                },
            ],
        ));
    }
    sets.push(postings::reward_granted(&grant));
    sets.push(postings::reward_redeemed(&spent));
    sets.extend(postings::in_kind_delivered(&lounge));
    sets.extend(postings::in_kind_delivered(&seat));
    assert!(
        postings::in_kind_delivered(&mount).is_none(),
        "a reward is charged its price"
    );
    sets.extend(postings::in_kind_returned(&unused));
    assert!(postings::in_kind_returned(&lounge).is_none());
    let orders = [lounge, seat, unused, mount];
    let paid: SupplierStatement = statement("lounges", day(1), day(31), &orders);
    assert_eq!(paid.total, Money(20_000 + 42_000 + 500));
    sets.push(postings::supplier_paid(&paid));
    post_all(&ledger, &sets).await;

    let b = balances(&ledger).await;
    let get = |a: Account| b.iter().find(|(x, _)| *x == a).map_or(0, |(_, v)| *v);
    assert_eq!(
        get(Account::InKindPayable),
        0,
        "every order delivered, returned or paid"
    );
    assert_eq!(get(Account::PayoutsPayable), 0);
    // Insurer: HK$1,800 of payouts, HK$400 back from the lounge's lower tariff.
    assert_eq!(get(Account::InsurerRisk), 180_000 - 40_000);
    assert_eq!(get(Account::RewardExpense), 1_200);
    assert_eq!(
        get(Account::RewardsPayable),
        -700,
        "HK$7 left in the reward balance"
    );
    // Out of the bank: suppliers HK$625, the seat's rest HK$180 and the unused HK$600.
    assert_eq!(get(Account::Cash), -(62_500 + 18_000 + 60_000));
}

/// A HK$600 claim payment settled in the grace period, HK$80 premium set off (R53), and a HK$12
/// reward voided for fraud after HK$5 of it was spent (ADR-010): every set balances and the
/// accounts end where the decisions say.
#[tokio::test]
async fn claim_payment_and_voided_reward_postings_balance() {
    use microinsurance_engine::claims::{self, ClaimPayment};
    use microinsurance_engine::rewards::{FraudFinding, void};
    let ledger = MemoryLedger::new();
    let payment = ClaimPayment {
        idempotency_key: "claim/c-1/payment".into(),
        claim_id: "c-1".into(),
        policy_id: "pol-1".into(),
        cover_id: "device_damage".into(),
        amount: Money(60_000),
        options: vec![Settlement::Cash],
    };
    let settled = claims::settle(&payment, None, Money(8_000)).expect("settled");
    let grant = Grant {
        idempotency_key: "reward/task-1".into(),
        reward: RewardEarned {
            reward_id: "task-1".into(),
            customer_id: "c".into(),
            pool: "property".into(),
            earned_for: EarnedFor::VerifiedActivity {
                task_id: "task-1".into(),
            },
            amount: Money(1_200),
            on: day(3),
        },
        granted: Money(1_200),
        over_cap: Money::ZERO,
    };
    let spent = Redemption {
        redemption_id: "redeem/1".into(),
        customer_id: "c".into(),
        offer_id: "cash".into(),
        amount: Money(500),
        method: Redeemed::Cash,
    };
    let finding = FraudFinding {
        reviewer: "fraud-dee".into(),
        on: day(20),
        note: "Photo reused".into(),
    };
    let voided = void(&grant, finding, Money(700)).expect("voided");
    let mut sets = vec![
        postings::claim_approved(&payment),
        postings::payout_settled(&settled),
        postings::reward_granted(&grant),
        postings::reward_redeemed(&spent),
    ];
    sets.extend(postings::reward_voided(&voided));
    post_all(&ledger, &sets).await;
    let b = balances(&ledger).await;
    let get = |a: Account| b.iter().find(|(x, _)| *x == a).map_or(0, |(_, v)| *v);
    assert_eq!(get(Account::PayoutsPayable), 0);
    assert_eq!(get(Account::InsurerRisk), 60_000);
    assert_eq!(get(Account::CustomerPremiums), -8_000);
    assert_eq!(get(Account::RewardsPayable), 0);
    // HK$7 taken back; the HK$5 already paid out stays an expense.
    assert_eq!(get(Account::RewardExpense), 500);
    assert_eq!(get(Account::Cash), -(52_000 + 500));
}
