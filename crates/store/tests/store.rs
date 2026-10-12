//! Every store passes the same tests. The PostgreSQL store runs when `DATABASE_URL` is set
//! (`just dev` starts one); without it those tests say so and pass.
use microinsurance_store::{By, Keys, PgStore, Store, StoreError};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Offer {
    plan: String,
    accepted: bool,
}

/// A fresh, isolated key space for each run against a shared database.
fn unique(key: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{key}-{nanos}")
}

async fn two_taps_on_one_offer_bind_once(store: &Store) {
    let id = unique("offer");
    let offer = Offer {
        plan: "essential".into(),
        accepted: false,
    };
    let read = store
        .insert("quotes", &id, Keys::customer("cust-1"), offer)
        .await
        .expect("insert");
    assert_eq!(read.version, 1);
    // Both taps read version 1; the first write wins, the second is told to read again.
    let mut first = read.clone();
    first.body.accepted = true;
    let mut second = read;
    second.body.accepted = true;
    let won = store.update("quotes", first).await.expect("first tap");
    assert_eq!(won.version, 2);
    assert!(matches!(
        store.update("quotes", second).await,
        Err(StoreError::Conflict { .. })
    ));
    // An id is written once.
    assert!(matches!(
        store
            .insert(
                "quotes",
                &id,
                Keys::customer("cust-1"),
                Offer {
                    plan: "max".into(),
                    accepted: false
                }
            )
            .await,
        Err(StoreError::Conflict { .. })
    ));
    let stored = store
        .get::<Offer>("quotes", &id)
        .await
        .expect("get")
        .expect("found");
    assert_eq!(stored.version, 2);
    assert_eq!(stored.body.plan, "essential");
}

async fn lists_by_customer_and_partner_in_id_order(store: &Store) {
    let collection = unique("policies");
    let partner = unique("delivery_app");
    for (id, customer, via) in [
        ("b", "cust-1", Some(partner.as_str())),
        ("a", "cust-1", None),
        ("c", "cust-2", Some(partner.as_str())),
    ] {
        store
            .insert(
                &collection,
                id,
                Keys::customer(customer).with_partner(via),
                Offer {
                    plan: id.into(),
                    accepted: false,
                },
            )
            .await
            .expect("insert");
    }
    let ids = |r: Vec<microinsurance_store::Record<Offer>>| -> Vec<String> {
        r.into_iter().map(|r| r.id).collect()
    };
    assert_eq!(
        ids(store.list(&collection, By::All).await.expect("all")),
        ["a", "b", "c"]
    );
    assert_eq!(
        ids(store
            .list(&collection, By::Customer("cust-1"))
            .await
            .expect("customer")),
        ["a", "b"]
    );
    assert_eq!(
        ids(store
            .list(&collection, By::Partner(&partner))
            .await
            .expect("partner")),
        ["b", "c"]
    );
    // Several customers in one query, as a partner's results read them.
    let both = [
        "cust-2".to_string(),
        "cust-1".to_string(),
        "cust-9".to_string(),
    ];
    assert_eq!(
        ids(store
            .list(&collection, By::Customers(&both))
            .await
            .expect("customers")),
        ["a", "b", "c"]
    );
    assert!(
        store
            .list::<Offer>(&collection, By::Customers(&[]))
            .await
            .expect("none")
            .is_empty()
    );
}

async fn subscribers_hear_every_write(store: &Store) {
    let mut changes = store.subscribe();
    let id = unique("payout");
    store
        .insert(
            "payouts",
            &id,
            Keys::customer("cust-9"),
            Offer {
                plan: "x".into(),
                accepted: false,
            },
        )
        .await
        .expect("insert");
    let heard = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match changes.recv().await {
                Ok(c) if c.id == id => return Some(c),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
    .expect("the write is heard");
    assert_eq!(heard.collection, "payouts");
    assert_eq!(heard.customer_id.as_deref(), Some("cust-9"));
}

async fn all(store: &Store) {
    two_taps_on_one_offer_bind_once(store).await;
    lists_by_customer_and_partner_in_id_order(store).await;
    subscribers_hear_every_write(store).await;
}

#[tokio::test]
async fn memory_store() {
    all(&Store::memory()).await;
}

#[tokio::test]
async fn postgres_store() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("DATABASE_URL not set: PostgreSQL store not tested");
        return;
    };
    let pg = PgStore::connect(&url).await.expect("connect");
    pg.migrate().await.expect("migrate");
    // Give the listener a moment to subscribe before the first write.
    tokio::time::sleep(Duration::from_millis(300)).await;
    all(&Store::Pg(pg)).await;
}
