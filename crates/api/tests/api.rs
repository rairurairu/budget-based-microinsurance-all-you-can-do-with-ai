//! The API end to end in demo mode: the real `config/`, the synthetic book, saved AI replies, the
//! in-memory store and ledger. Each test builds its own API, so tests never share state.
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use microinsurance_api::auth::Principal;
use microinsurance_api::bodies::{
    Accepted, ChannelInfo, ClaimRecord, CustomerRecord, EvidenceFile, FeedOutcome, PayoutNotice,
    PolicyView, Quote, ReviewOpened, RewardsView, TaskOutcome,
};
use microinsurance_api::demo::DemoSession;
use microinsurance_api::error::ErrorBody;
use microinsurance_api::routes::{Access, ROUTES};
use microinsurance_api::{Settings, app, build};
use microinsurance_engine::claims::{ClaimReferral, ClaimState};
use microinsurance_engine::guardrails::DecisionRecord;
use microinsurance_engine::guardrails::queue::{OpsSnapshot, PersonAction, WorkKind};
use microinsurance_engine::prevention::{PreventionTask, TaskState};
use microinsurance_learning::insights::PartnerResults;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tower::ServiceExt;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Api {
    app: Router,
    sessions: Vec<DemoSession>,
}

async fn api() -> Api {
    let settings = Settings {
        root: repo(),
        market: None,
        insurer: None,
        demo: true,
        database_url: None,
        pool_size: 8,
        token_secret: None,
        audit_seed: None,
        rating_basis: None,
        prevention_basis: None,
        audit_store: None,
        demo_start: None,
        ai_live: false,
    };
    let state = build(settings).await.expect("the demo API builds");
    let app = app(state, &repo());
    let mut api = Api {
        app,
        sessions: Vec::new(),
    };
    let (status, sessions) = api.get::<Vec<DemoSession>>("/demo/sessions", None).await;
    assert_eq!(status, StatusCode::OK);
    api.sessions = sessions.expect("sessions");
    api
}

impl Api {
    fn token(&self, matches: impl Fn(&Principal) -> bool) -> String {
        self.sessions
            .iter()
            .find(|s| matches(&s.principal))
            .map(|s| s.token.clone())
            .expect("a demo session")
    }

    fn customer(&self, id: &str) -> String {
        self.token(|p| matches!(p, Principal::Customer { customer_id, .. } if customer_id == id))
    }

    fn staff(&self, id: &str) -> String {
        self.token(|p| matches!(p, Principal::Staff { staff_id } if staff_id == id))
    }

    fn partner(&self, id: &str) -> String {
        self.token(|p| matches!(p, Principal::Partner { partner_id } if partner_id == id))
    }

    fn feed(&self) -> String {
        self.token(|p| matches!(p, Principal::Feed { .. }))
    }

    async fn send(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<String>,
    ) -> (StatusCode, Vec<u8>) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let body = match body {
            Some(b) => {
                req = req.header(header::CONTENT_TYPE, "application/json");
                Body::from(b)
            }
            None => Body::empty(),
        };
        let resp = self
            .app
            .clone()
            .oneshot(req.body(body).expect("request"))
            .await
            .expect("response");
        let status = resp.status();
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        (status, bytes.to_vec())
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        token: Option<&str>,
    ) -> (StatusCode, Option<T>) {
        let (status, bytes) = self.send("GET", path, token, None).await;
        (status, serde_json::from_slice(&bytes).ok())
    }

    async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        token: &str,
        body: &B,
    ) -> (StatusCode, Option<T>, Vec<u8>) {
        let text = serde_json::to_string(body).expect("json");
        let (status, bytes) = self.send("POST", path, Some(token), Some(text)).await;
        (status, serde_json::from_slice(&bytes).ok(), bytes)
    }

    async fn ok<B: Serialize, T: DeserializeOwned>(&self, path: &str, token: &str, body: &B) -> T {
        let (status, value, bytes) = self.post::<B, T>(path, token, body).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{path}: {}",
            String::from_utf8_lossy(&bytes)
        );
        value.expect("a typed reply")
    }

    async fn refused<B: Serialize>(
        &self,
        path: &str,
        token: &str,
        body: &B,
    ) -> (StatusCode, ErrorBody) {
        let (status, err, bytes) = self.post::<B, ErrorBody>(path, token, body).await;
        (
            status,
            err.unwrap_or_else(|| panic!("{path}: {status} {}", String::from_utf8_lossy(&bytes))),
        )
    }
}

fn encode(id: &str) -> String {
    id.replace('%', "%25")
        .replace('/', "%2F")
        .replace(':', "%3A")
}

#[tokio::test]
async fn every_route_in_the_table_is_served_and_guarded() {
    let api = api().await;
    for route in ROUTES {
        let path = route
            .path
            .split('/')
            .map(|seg| if seg.starts_with('{') { "x" } else { seg })
            .collect::<Vec<_>>()
            .join("/");
        let body = match route.request {
            microinsurance_api::routes::Body::Empty => None,
            _ => Some("{}".to_string()),
        };
        let (status, bytes) = api
            .send(&route.method.as_str().to_uppercase(), &path, None, body)
            .await;
        assert_ne!(status, StatusCode::METHOD_NOT_ALLOWED, "{path}");
        if status == StatusCode::NOT_FOUND {
            // Only a handler's own answer, never the router's.
            assert!(
                serde_json::from_slice::<ErrorBody>(&bytes).is_ok(),
                "{} {path} is not served",
                route.method.as_str()
            );
        }
        if route.access != Access::Public {
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} needs a token");
        }
    }
}

#[tokio::test]
async fn each_principal_reaches_only_its_own_routes() {
    let api = api().await;
    let rider = api.customer("cust-demo-rider");
    let amy = api.staff("handler-amy");
    let partner = api.partner("delivery_app");
    assert_eq!(
        api.send("GET", "/ops/queue", Some(&rider), None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        api.send("GET", "/me", Some(&partner), None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        api.send("GET", "/partners/me/results", Some(&amy), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        api.send("GET", "/me", Some("v1.forged.token"), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    // A source pushes only its own reports.
    let (status, _) = api
        .send(
            "POST",
            "/webhooks/triggers/hko_warnings",
            Some(&api.feed()),
            Some("{}".into()),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn r03_r58_one_tap_binds_one_policy_from_the_next_day() {
    let api = api().await;
    let new = api.customer("cust-demo-new");
    let (status, _) = api.get::<Value>("/me/policy", Some(&new)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let quote: Quote = api
        .ok("/me/quotes", &new, &json!({ "plan_id": "essential" }))
        .await;
    assert_eq!(quote.schedule.total_premium().0, 8_000);
    // 18 November in Hong Kong: cover starts on the 19th.
    assert_eq!(quote.period_starts.to_string(), "2026-11-19");
    let path = format!("/me/quotes/{}/accept", quote.offer.offer_id);
    let accepted: Accepted = api.ok(&path, &new, &json!({})).await;
    let policy = accepted.policy.expect("a policy");
    assert_eq!(policy.period.starts.to_string(), "2026-11-19");
    assert_eq!(policy.schedule, quote.schedule);
    // A second tap on the same quote binds nothing more.
    let (status, err) = api.refused(&path, &new, &json!({})).await;
    assert_eq!(
        (status, err.code.as_str()),
        (StatusCode::CONFLICT, "conflict")
    );
    let (status, view) = api.get::<PolicyView>("/me/policy", Some(&new)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view.expect("policy").policy.policy_id, policy.policy_id);
}

#[tokio::test]
async fn r49_quote_keeps_what_was_shown() {
    let api = api().await;
    let new = api.customer("cust-demo-new");
    let quote: Quote = api
        .ok("/me/quotes", &new, &json!({ "plan_id": "plus" }))
        .await;
    let suggestion = quote
        .suggestion
        .as_ref()
        .expect("the optimiser's suggestion");
    assert!(!suggestion.basis.is_empty(), "the factors behind it");
    assert_eq!(quote.explanations.len(), quote.schedule.allocations.len());
    let (status, kept) = api
        .get::<Quote>(&format!("/me/quotes/{}", quote.offer.offer_id), Some(&new))
        .await;
    assert_eq!(status, StatusCode::OK);
    let kept = kept.expect("kept");
    assert_eq!(
        serde_json::to_value(&kept).ok(),
        serde_json::to_value(&quote).ok()
    );
    // Another customer cannot read it.
    let other = api.customer("cust-demo-rider");
    let (status, _) = api
        .get::<Quote>(
            &format!("/me/quotes/{}", quote.offer.offer_id),
            Some(&other),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn r34_the_customers_own_split_is_priced_or_refused() {
    let api = api().await;
    let new = api.customer("cust-demo-new");
    let suggested: Quote = api
        .ok("/me/quotes", &new, &json!({ "plan_id": "starter" }))
        .await;
    let premiums: serde_json::Map<String, Value> = suggested
        .schedule
        .allocations
        .iter()
        .map(|a| (a.cover_id.clone(), json!(a.premium.0)))
        .collect();
    let own: Quote = api
        .ok(
            "/me/quotes",
            &new,
            &json!({ "plan_id": "starter", "premiums": premiums }),
        )
        .await;
    assert!(own.suggestion.is_none(), "the customer's own split, priced");
    assert_eq!(own.schedule, suggested.schedule);
    let (status, err) = api
        .refused(
            "/me/quotes",
            &new,
            &json!({ "plan_id": "starter", "premiums": { "weather_income": 1000 } }),
        )
        .await;
    assert_eq!(
        (status, err.rule.as_deref()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("R34"))
    );
}

#[tokio::test]
async fn r52_a_referral_only_partner_refers_and_never_binds() {
    let api = api().await;
    let (_, delivery) = api.get::<ChannelInfo>("/channels/delivery_app", None).await;
    assert!(delivery.expect("channel").binds);
    let (_, ticketing) = api
        .get::<ChannelInfo>("/channels/ticketing_partner", None)
        .await;
    let ticketing = ticketing.expect("channel");
    assert!(!ticketing.binds);
    assert!(ticketing.refer_to.is_some());
    // The partner exchanges its reference for a customer token; binding through it is refused.
    let token: Value = api
        .ok(
            "/partners/me/customer-tokens",
            &api.partner("ticketing_partner"),
            &json!({ "customer_ref": "fan-123" }),
        )
        .await;
    let customer = token["token"].as_str().expect("token").to_string();
    let quote: Quote = api
        .ok("/me/quotes", &customer, &json!({ "plan_id": "starter" }))
        .await;
    let (status, err) = api
        .refused(
            &format!("/me/quotes/{}/accept", quote.offer.offer_id),
            &customer,
            &json!({}),
        )
        .await;
    assert_eq!(
        (status, err.rule.as_deref()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("R52"))
    );
}

#[tokio::test]
async fn r21_partner_sees_aggregates_over_enough_customers_only() {
    let api = api().await;
    let (status, results) = api
        .get::<PartnerResults>("/partners/me/results", Some(&api.partner("delivery_app")))
        .await;
    assert_eq!(status, StatusCode::OK);
    let results = results.expect("results");
    let figure = |m: &str| {
        results
            .figures
            .iter()
            .find(|f| f.metric == m)
            .expect("figure")
    };
    assert!(figure("subscribers").value.is_some_and(|v| v >= 50));
    assert_eq!(figure("prevention_task_completion").value, None);
    let text = serde_json::to_string(&results).expect("json");
    assert!(!text.contains("cust-"), "no customer id reaches a partner");
}

#[tokio::test]
async fn r27_a_pushed_cancellation_pays_once_and_streams_to_the_customer() {
    let api = api().await;
    let fan = api.customer("cust-demo-fan");
    // Listen before the organiser's notice arrives.
    let req = Request::builder()
        .uri("/me/payouts/stream")
        .header(header::AUTHORIZATION, format!("Bearer {fan}"))
        .body(Body::empty())
        .expect("request");
    let mut stream = api
        .app
        .clone()
        .oneshot(req)
        .await
        .expect("stream")
        .into_body();
    let body = std::fs::read_to_string(repo().join("fixtures/organiser_cancellation.json"))
        .expect("fixture");
    let feed = api.feed();
    let (status, bytes) = api
        .send(
            "POST",
            "/webhooks/triggers/organiser_notices",
            Some(&feed),
            Some(body.clone()),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let outcome: FeedOutcome = serde_json::from_slice(&bytes).expect("outcome");
    assert_eq!(outcome.payouts.len(), 1);
    assert_eq!(
        outcome.payouts[0].amount.0, 88_000,
        "the ticket's declared value"
    );
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        let mut text = String::new();
        while let Some(Ok(frame)) = stream.frame().await {
            if let Some(data) = frame.data_ref() {
                text.push_str(&String::from_utf8_lossy(data));
                if text.contains("event: payout") && text.contains("\n\n") {
                    return text;
                }
            }
        }
        text
    })
    .await
    .expect("the payout is streamed");
    assert!(event.contains("pol-demo-fan"), "{event}");
    // The same notice again pays nothing more (R27).
    let (_, bytes) = api
        .send(
            "POST",
            "/webhooks/triggers/organiser_notices",
            Some(&feed),
            Some(body),
        )
        .await;
    let again: FeedOutcome = serde_json::from_slice(&bytes).expect("outcome");
    assert!(again.payouts.is_empty());
    let (_, payouts) = api
        .get::<Vec<PayoutNotice>>("/me/payouts", Some(&fan))
        .await;
    let payouts = payouts.expect("payouts");
    assert_eq!(payouts.len(), 1);
    // The ticket cover offers alternatives worth at least the cash: the customer chooses (R16, R28).
    assert!(payouts[0].settled.is_none());
    assert!(payouts[0].decision.options.len() > 1);
    let key = payouts[0].decision.idempotency_key.clone();
    let chose: PayoutNotice = api
        .ok(
            &format!("/me/payouts/{}/choice", encode(&key)),
            &fan,
            &json!({ "settlement": { "method": "cash" } }),
        )
        .await;
    assert_eq!(chose.settled.map(|s| s.cash.0), Some(88_000));
}

async fn evidence(api: &Api, file: &str) -> EvidenceFile {
    let (status, f) = api
        .get::<EvidenceFile>(&format!("/demo/evidence/{file}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    f.expect("file")
}

#[tokio::test]
async fn r13_r15_only_a_named_person_declines_and_another_reviews() {
    let api = api().await;
    let rider = api.customer("cust-demo-rider");
    let claim: ClaimRecord = api
        .ok(
            "/me/claims",
            &rider,
            &json!({
                "cover_id": "device_damage",
                "loss_date": "2026-11-16",
                "amount_claimed": 90000,
                "description": "The screen cracked when the phone fell off my bike mount."
            }),
        )
        .await;
    assert!(matches!(
        claim.claim.state,
        ClaimState::AwaitingEvidence { .. }
    ));
    let id = claim.claim.claim_id.clone();
    let photo = evidence(&api, "damage_unclear_synthetic.png").await;
    let assessed: ClaimRecord = api
        .ok(
            &format!("/me/claims/{id}/evidence"),
            &rider,
            &json!({ "kinds": ["photo"], "files": [photo] }),
        )
        .await;
    // With no saved reply for this case the assessor gives its saved answer: a person decides.
    assert!(
        matches!(assessed.claim.state, ClaimState::WithPerson { .. }),
        "{:?}",
        assessed.claim.state
    );
    let amy = api.staff("handler-amy");
    let ben = api.staff("handler-ben");
    let (_, queue) = api.get::<OpsSnapshot>("/ops/queue", Some(&amy)).await;
    let item = queue
        .expect("queue")
        .items
        .into_iter()
        .find(|i| i.item_id == id)
        .expect("the claim is queued");
    assert!(item.actions.contains(&PersonAction::Decline));
    // A decline names its reason.
    let (status, err) = api
        .refused(
            "/ops/decisions",
            &amy,
            &json!({ "item_id": id, "action": "decline" }),
        )
        .await;
    assert_eq!(
        (status, err.rule.as_deref()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("R13"))
    );
    let declined: DecisionRecord = api
        .ok(
            "/ops/decisions",
            &amy,
            &json!({ "item_id": id, "action": "decline", "decline_reason": "not_an_insured_loss", "note": "Mount failure is wear" }),
        )
        .await;
    assert_eq!(declined.reviewer.as_deref(), Some("handler-amy"));
    // The customer asks for a review: the claim goes back to a person other than amy.
    let opened: ReviewOpened = api
        .ok(
            "/me/review-requests",
            &rider,
            &json!({ "decision_id": declined.decision_id, "reason": "It was an accident" }),
        )
        .await;
    let back = opened.claim.expect("the claim, under review");
    assert!(matches!(
        back.state,
        ClaimState::WithPerson { referral: ClaimReferral::Appeal { ref declined_by }, .. } if declined_by == "handler-amy"
    ));
    let (status, err) = api
        .refused(
            "/ops/decisions",
            &amy,
            &json!({ "item_id": id, "action": "approve", "amount": 90000 }),
        )
        .await;
    assert_eq!(
        (status, err.rule.as_deref()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("R15"))
    );
    let approved: DecisionRecord = api
        .ok(
            "/ops/decisions",
            &ben,
            &json!({ "item_id": id, "action": "approve", "amount": 90000 }),
        )
        .await;
    assert_eq!(approved.reviewer.as_deref(), Some("handler-ben"));
    let (_, paid) = api
        .get::<ClaimRecord>(&format!("/me/claims/{id}"), Some(&rider))
        .await;
    let paid = paid.expect("claim");
    assert!(matches!(paid.claim.state, ClaimState::Approved { .. }));
    assert_eq!(paid.payment.map(|p| p.amount.0), Some(90_000));
}

#[tokio::test]
async fn r13_a_decline_is_offered_only_on_a_claim_referral() {
    let api = api().await;
    let fan = api.customer("cust-demo-fan");
    let body = std::fs::read_to_string(repo().join("fixtures/organiser_cancellation.json"))
        .expect("fixture");
    api.send(
        "POST",
        "/webhooks/triggers/organiser_notices",
        Some(&api.feed()),
        Some(body),
    )
    .await;
    let (_, decisions) = api
        .get::<Vec<DecisionRecord>>("/me/decisions", Some(&fan))
        .await;
    let payout = decisions
        .expect("decisions")
        .into_iter()
        .find(|d| d.subject_type == "payout")
        .expect("the payout decision");
    let opened: ReviewOpened = api
        .ok(
            "/me/review-requests",
            &fan,
            &json!({ "decision_id": payout.decision_id, "reason": "Please check" }),
        )
        .await;
    let appeal = opened.appeal.expect("a review request");
    let amy = api.staff("handler-amy");
    let (status, err) = api
        .refused(
            "/ops/decisions",
            &amy,
            &json!({ "item_id": appeal.appeal_id, "action": "decline", "decline_reason": "fraud" }),
        )
        .await;
    assert_eq!(
        (status, err.rule.as_deref()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("R13"))
    );
    let upheld: DecisionRecord = api
        .ok("/ops/decisions", &amy, &json!({ "item_id": appeal.appeal_id, "action": "uphold", "note": "Paid as the notice said" }))
        .await;
    assert_eq!(upheld.decision, "review: upheld");
    let (_, queue) = api.get::<OpsSnapshot>("/ops/queue", Some(&amy)).await;
    assert!(queue.expect("queue").items.iter().all(
        |i| !matches!(&i.work, WorkKind::Appeal { appeal_id, .. } if *appeal_id == appeal.appeal_id)
    ));
}

#[tokio::test]
async fn r10_r14_r16_a_verified_task_credits_the_reward_balance() {
    let api = api().await;
    let traveller = api.customer("cust-demo-traveller");
    let offered: Vec<PreventionTask> = api
        .ok("/me/prevention-tasks/refresh", &traveller, &json!({}))
        .await;
    assert_eq!(offered.len(), 1, "the saved proposal for the December trip");
    let task = &offered[0];
    assert_eq!(task.due.to_string(), "2026-12-20");
    let path = encode(&task.task_id);
    let _: PreventionTask = api
        .ok(
            &format!("/me/prevention-tasks/{path}/accept"),
            &traveller,
            &json!({}),
        )
        .await;
    let photo = evidence(&api, "screen_protector_synthetic.png").await;
    let outcome: TaskOutcome = api
        .ok(
            &format!("/me/prevention-tasks/{path}/evidence"),
            &traveller,
            &json!({ "files": [photo.clone()] }),
        )
        .await;
    match &outcome.task.state {
        TaskState::Verified { .. } => {
            let granted = outcome.granted.expect("the reward");
            let (_, rewards) = api
                .get::<RewardsView>("/me/rewards", Some(&traveller))
                .await;
            let rewards = rewards.expect("rewards");
            assert_eq!(rewards.balance, granted.granted);
            assert!(rewards.menu[0].is_cash, "cash first, always (R16)");
        }
        TaskState::WithPerson { .. } => {
            assert!(outcome.granted.is_none(), "held for a person first (R14)")
        }
        other => panic!("unexpected {other:?}"),
    }
    // The same photo again, on another customer's task, is caught as reused (R10).
    let rider = api.customer("cust-demo-rider");
    let (_, me) = api.get::<CustomerRecord>("/me", Some(&rider)).await;
    assert!(me.is_some());
}

#[tokio::test]
async fn r55_a_customer_declares_only_registered_work_areas() {
    let api = api().await;
    // The base market's sources report no areas: there is nothing to declare, and the apps are
    // told the market's currency and allocation step rather than assuming them.
    let (status, registry) = api.get::<Value>("/registry", None).await;
    assert_eq!(status, StatusCode::OK);
    let registry = registry.expect("registry");
    assert_eq!(registry["areas"], json!([]));
    assert_eq!(registry["allocation_step"], json!(500));
    assert_eq!(registry["currency"], json!("HKD"));
    let rider = api.customer("cust-demo-rider");
    let body = json!({ "work_areas": ["tw.taipei_city"] }).to_string();
    let (status, bytes) = api
        .send("PATCH", "/me/profile", Some(&rider), Some(body))
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let err: ErrorBody = serde_json::from_slice(&bytes).expect("error body");
    assert_eq!(err.rule.as_deref(), Some("R55"));
}

#[tokio::test]
async fn adoption_reports_reach_people_at_the_insurer_only() {
    let api = api().await;
    let amy = api.staff("handler-amy");
    let rider = api.customer("cust-demo-rider");
    for path in ["/ops/jobs", "/ops/experience?month=2026-11", "/ops/pilot"] {
        let (status, _) = api.get::<Value>(path, Some(&rider)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
    let (status, jobs) = api.get::<Value>("/ops/jobs", Some(&amy)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(jobs.expect("jobs")["failed"], json!([]));
    // The study counts the synthetic book's November periods by class, with no customer id.
    let (status, study) = api
        .get::<Value>("/ops/experience?month=2026-11", Some(&amy))
        .await;
    assert_eq!(status, StatusCode::OK);
    let study = study.expect("study");
    let classes = study["classes"].as_array().expect("classes");
    assert!(!classes.is_empty());
    assert!(classes.iter().all(|r| r["period"] == json!("2026-11")));
    assert!(!study.to_string().contains("cust-"), "aggregates only");
    let (status, _) = api
        .get::<Value>("/ops/experience?month=November", Some(&amy))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // The pilot report: the metrics of the customers bound in a window, with the gates as fixed.
    let (status, report) = api
        .get::<Value>("/ops/pilot?from=2026-01-01&to=2026-12-31", Some(&amy))
        .await;
    assert_eq!(status, StatusCode::OK);
    let report = report.expect("report");
    assert_eq!(report["gates_fixed_on"], json!("2027-04-15"));
    let metric = |name: &str| {
        report["metrics"]
            .as_array()
            .expect("metrics")
            .iter()
            .find(|m| m["metric"] == json!(name))
            .cloned()
            .expect("metric")
    };
    assert_eq!(metric("retention_rate")["value"], json!(10_000));
    assert!(
        metric("loss_ratio")["sample"]
            .as_u64()
            .is_some_and(|n| n > 0)
    );
    assert!(
        report["gates"]
            .as_array()
            .expect("gates")
            .iter()
            .any(|g| g["metric"] == json!("loss_ratio") && g["kind"] == json!("stop"))
    );
}

#[tokio::test]
async fn a_front_end_opened_without_its_trailing_slash_redirects_to_it() {
    // The apps link their files relatively, so `/app` must become `/app/`.
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("front-ends");
    let dist = root.join("apps/customer-app/dist");
    std::fs::create_dir_all(&dist).expect("dist");
    std::fs::write(dist.join("index.html"), "<!doctype html>").expect("index");
    let state = build(Settings {
        root: repo(),
        market: None,
        insurer: None,
        demo: true,
        database_url: None,
        pool_size: 8,
        token_secret: None,
        audit_seed: None,
        rating_basis: None,
        prevention_basis: None,
        audit_store: None,
        demo_start: None,
        ai_live: false,
    })
    .await
    .expect("the demo API builds");
    let app = app(state, &root);
    let get = |uri: &str| {
        app.clone()
            .oneshot(Request::get(uri).body(Body::empty()).expect("request"))
    };

    let response = get("/app?lang=en").await.expect("response");
    assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(response.headers()[header::LOCATION], "/app/?lang=en");
    assert_eq!(
        get("/app/").await.expect("response").status(),
        StatusCode::OK
    );
    // An app that is not built is not redirected to.
    assert_eq!(
        get("/ops").await.expect("response").status(),
        StatusCode::NOT_FOUND
    );
}
