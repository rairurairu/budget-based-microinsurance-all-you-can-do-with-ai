//! Every AI component, with a scripted model: what reaches the model, which replies are used and
//! which fall back (R02, R16, R19, R20, R22, R23, R38).
use chrono::NaiveDate;
use microinsurance_ai::client::{AnthropicClient, Model, ModelRequest, reply_text};
use microinsurance_ai::harness::{AiError, Harness, Source};
use microinsurance_ai::language::{Languages, MAX_QUESTIONS};
use microinsurance_ai::prompts::Prompts;
use microinsurance_ai::settings::{AiSettings, Tier};
use microinsurance_ai::understand::ProposalForm;
use microinsurance_ai::verify::{Evidence, VerifyOutcome};
use microinsurance_ai::vocab::Vocabulary;
use microinsurance_ai::{
    assess, explain, foresee, prevention, replay, rewards, schema, understand, verify,
};
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::consent::SumInsuredChange;
use microinsurance_engine::money::Rate;
use microinsurance_engine::money::{Bps, Money};
use microinsurance_engine::optimiser::SuggestionBasis;
use microinsurance_engine::optimiser::rating::AppliedFactor;
use microinsurance_engine::prevention::PreventionTaskDraft;
use microinsurance_engine::rewards::{Funder, RewardOffer};
use microinsurance_engine::types::{
    Allocation, DeclaredItem, Profile, Schedule, UpcomingEvent, WorkWindow,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn cfg() -> EngineConfig {
    microinsurance_config::load(&root().join("config"), "insurer.example.yaml").expect("config")
}

fn settings() -> AiSettings {
    let text =
        std::fs::read_to_string(root().join("config/insurer.example.yaml")).expect("insurer");
    AiSettings::from_insurer_yaml(&text).expect("ai section")
}

fn languages(cfg: &EngineConfig) -> Languages {
    let files = microinsurance_config::ConfigDir::base(&root().join("config"))
        .language_files()
        .expect("packs");
    Languages::load(&files, cfg).expect("packs are valid")
}

fn vocab(cfg: &EngineConfig) -> Vocabulary {
    Vocabulary::from_config(
        cfg,
        microinsurance_config::ConfigDir::base(&root().join("config"))
            .declared_routes()
            .expect("routes"),
    )
}

/// Replies with fixed text, or waits, and keeps every request it is sent.
struct Scripted {
    reply: Result<String, String>,
    delay: Duration,
    sent: Mutex<Vec<ModelRequest>>,
}

impl Model for Scripted {
    async fn complete(&self, req: &ModelRequest) -> Result<String, AiError> {
        self.sent.lock().expect("lock").push(req.clone());
        tokio::time::sleep(self.delay).await;
        self.reply.clone().map_err(AiError::Call)
    }
}

fn harness(reply: Value) -> Harness<Scripted> {
    harness_text(&reply.to_string())
}

fn harness_text(reply: &str) -> Harness<Scripted> {
    Harness::new(
        Scripted {
            reply: Ok(reply.to_string()),
            delay: Duration::ZERO,
            sent: Mutex::new(Vec::new()),
        },
        Prompts::in_repo(&root()),
        settings(),
        languages(&cfg()),
    )
}

fn sent(h: &Harness<Scripted>) -> Vec<ModelRequest> {
    h.model.sent.lock().expect("lock").clone()
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("date")
}

fn rider() -> Profile {
    Profile {
        customer_id: "cust-synthetic-1".into(),
        occupation: Some("food delivery rider".into()),
        work_windows: vec![WorkWindow {
            weekday: 1,
            start_minute: 9 * 60,
            end_minute: 18 * 60,
        }],
        declared_items: vec![DeclaredItem {
            kind: "device.phone".into(),
            description: "work phone".into(),
            declared_value: Some(Money::from_major(4_000)),
        }],
        commute_routes: vec!["mtr.tsuen_wan".into()],
        work_areas: vec!["tw.taipei_city".into()],
        upcoming_events: Vec::new(),
        typical_daily_earnings: Some(Money::from_major(900)),
        consents: ["photos".to_string()].into(),
        language: None,
    }
}

fn schedule() -> Schedule {
    Schedule {
        plan_id: "essential".into(),
        allocations: vec![
            Allocation {
                cover_id: "commute_disruption".into(),
                premium: Money::from_major(25),
                sum_insured: Money::from_major(150),
            },
            Allocation {
                cover_id: "weather_income".into(),
                premium: Money::from_major(55),
                sum_insured: Money::from_major(480),
            },
        ],
    }
}

#[tokio::test]
async fn r23_explain_uses_one_checked_sentence_per_cover() {
    let cfg = cfg();
    let h = harness(json!({"explanations": [
        {"cover_id": "weather_income", "sentence": "You get paid when a typhoon signal stops your deliveries during your working hours."},
        {"cover_id": "commute_disruption", "sentence": "You get paid when your train line is suspended on your way to work."}
    ]}));
    let out = explain::explain_schedule(&h, &schedule(), &rider(), &cfg).await;
    assert_eq!(out.source, Source::Model);
    assert_eq!(out.value[0].cover_id, "commute_disruption");
    let req = &sent(&h)[0];
    assert_eq!(req.component, "explain");
    assert_eq!(req.tier, Tier::Fast);
    // R02: the model is never told an amount.
    for amount in ["480", "150", "4000", "900", "48000", "15000"] {
        assert!(!req.user.contains(amount), "{amount} reached the model");
    }
    assert!(!req.user.contains("cust-synthetic-1"));
}

#[tokio::test]
async fn r23_two_sentences_fall_back_to_the_saved_sentence() {
    let cfg = cfg();
    let h = harness(json!({"explanations": [
        {"cover_id": "weather_income", "sentence": "You get paid in a typhoon. It is automatic."},
        {"cover_id": "commute_disruption", "sentence": "You get paid when your line stops."}
    ]}));
    let out = explain::explain_schedule(&h, &schedule(), &rider(), &cfg).await;
    assert_eq!(out.source, Source::Fallback);
    assert!(out.reason.as_deref().is_some_and(|r| r.contains("R23")));
    let pack = h.languages.pack(None);
    assert_eq!(out.value[1].sentence, pack.saved.covers["weather_income"]);
    assert!(
        out.value
            .iter()
            .all(|e| h.languages.check_sentence(pack, &e.sentence, &[]).is_ok())
    );
}

#[tokio::test]
async fn r02_explain_never_shows_an_amount() {
    let cfg = cfg();
    for sentence in [
        "You get HK$480 when a typhoon signal stops your work.",
        "You get 480 when a typhoon signal stops your work.",
        "You get paid 480 dollars when a typhoon signal stops your work.",
    ] {
        let h = harness(json!({"explanations": [
            {"cover_id": "weather_income", "sentence": sentence},
            {"cover_id": "commute_disruption", "sentence": "You get paid when your line stops."}
        ]}));
        let out = explain::explain_schedule(&h, &schedule(), &rider(), &cfg).await;
        assert_eq!(out.source, Source::Fallback, "{sentence}");
    }
    // A number that is not an amount is fine.
    let h = harness(json!({"explanations": [
        {"cover_id": "weather_income", "sentence": "You get paid when Signal 8 stops your work."},
        {"cover_id": "commute_disruption", "sentence": "You get paid when your line stops."}
    ]}));
    let out = explain::explain_schedule(&h, &schedule(), &rider(), &cfg).await;
    assert_eq!(out.source, Source::Model);
}

#[tokio::test]
async fn r22_missing_or_extra_covers_fall_back() {
    let cfg = cfg();
    let h = harness(json!({"explanations": [
        {"cover_id": "weather_income", "sentence": "You get paid in a typhoon."}
    ]}));
    let out = explain::explain_schedule(&h, &schedule(), &rider(), &cfg).await;
    assert_eq!(out.source, Source::Fallback);
    assert_eq!(out.value.len(), 2);
}

#[tokio::test]
async fn r19_renewal_reason_sees_market_factors_only() {
    let cfg = cfg();
    let h = harness(
        json!({"sentence": "Typhoon season is coming, so the same premium buys less weather cover next month."}),
    );
    let change = SumInsuredChange {
        cover_id: "weather_income".into(),
        premium: Money::from_major(55),
        before: Money::from_major(480),
        after: Money::from_major(400),
        declared_value: None,
        below_declared_value: false,
    };
    let factor = |id: &str, relativity: u32| AppliedFactor {
        factor_id: id.into(),
        relativity: Bps(relativity),
        withheld: false,
        shadow: false,
    };
    let basis = SuggestionBasis {
        cover_id: "weather_income".into(),
        need: Money::from_major(480),
        rate: Rate(1),
        risk_class: "standard".into(),
        factors: vec![
            factor("hazard_frequency", 13_000),
            factor("age_band", 9_000),
        ],
    };
    let out = explain::explain_change(&h, &change, Some(&basis), None, &cfg).await;
    assert_eq!(out.source, Source::Model);
    let user = &sent(&h)[0].user;
    assert!(user.contains("hazard_frequency") && user.contains("less_cover_same_premium"));
    assert!(
        !user.contains("age_band"),
        "a personal factor reached the model"
    );
    assert!(!user.contains("480") && !user.contains("400"));
}

#[tokio::test]
async fn r19_protected_answers_never_reach_the_model() {
    let cfg = cfg();
    let v = vocab(&cfg);
    let h = harness(
        json!({"occupation": "courier", "devices": ["device.phone"], "commute_routes": ["mtr.tsuen_wan"]}),
    );
    let form = understand::fallback_form("delivery_app", h.languages.pack(None));
    let answers =
        json!({"occupation": "courier", "age": 45, "extra": {"gender": "f", "nationality": "x"}});
    let out = understand::map_answers(&h, &form, &answers, &v).await;
    assert_eq!(out.source, Source::Model);
    assert_eq!(out.value.commute_routes, ["mtr.tsuen_wan"]);
    let user = &sent(&h)[0].user;
    for field in ["\"age\"", "\"gender\"", "\"nationality\""] {
        assert!(!user.contains(field), "{field} reached the model");
    }
}

#[tokio::test]
async fn r55_mapped_facts_use_registered_ids_only() {
    let cfg = cfg();
    let v = vocab(&cfg);
    let packs = languages(&cfg);
    let form = understand::fallback_form("delivery_app", packs.pack(None));
    for reply in [
        json!({"occupation": null, "devices": ["phone"], "commute_routes": []}),
        json!({"occupation": null, "devices": [], "commute_routes": ["Tsuen Wan Line"]}),
        json!({"occupation": "a 45 year-old woman", "devices": [], "commute_routes": []}),
    ] {
        let h = harness(reply.clone());
        let out = understand::map_answers(&h, &form, &json!({}), &v).await;
        assert_eq!(out.source, Source::Fallback, "{reply}");
        assert_eq!(out.value, understand::ProfileFacts::default());
    }
}

#[tokio::test]
async fn r19_form_never_asks_for_a_protected_attribute() {
    let cfg = cfg();
    let v = vocab(&cfg);
    let good = json!({"questions": [
        {"id": "work_hours", "text": "When do you usually work?", "options": ["Days", "Nights"]}
    ]});
    let h = harness(good);
    let out = understand::proposal_form(&h, "delivery_app", None, &v).await;
    assert_eq!(out.source, Source::Model);
    assert_eq!(out.value.questions.len(), 1);
    for bad in [
        json!({"questions": [{"id": "age_band", "text": "How old are you?", "options": []}]}),
        json!({"questions": [{"id": "home", "text": "Which district do you live in?", "options": []}]}),
        json!({"questions": [{"id": "income", "text": "How many dollars do you earn?", "options": []}]}),
        json!({"questions": []}),
    ] {
        let h = harness(bad.clone());
        let out = understand::proposal_form(&h, "delivery_app", None, &v).await;
        assert_eq!(out.source, Source::Fallback, "{bad}");
        assert_eq!(
            out.value,
            understand::fallback_form("delivery_app", h.languages.pack(None))
        );
    }
    let h = harness(json!({}));
    let form: ProposalForm = understand::fallback_form("ticketing", h.languages.pack(None));
    assert!(form.questions.len() <= MAX_QUESTIONS);
}

#[tokio::test]
async fn r02_foresee_returns_registered_events_without_a_value() {
    let cfg = cfg();
    let v = vocab(&cfg);
    let today = day(2026, 10, 9);
    let h = harness(json!({"events": [
        {"kind": "event.concert", "starts": "2026-11-21", "ends": null, "reference": "EVT-2026-1121-ARENA"},
        {"kind": "trip.air", "starts": "2026-12-20", "ends": "2026-12-27", "reference": "CX880/2026-12-20"}
    ]}));
    let out =
        foresee::extract_upcoming_events(&h, "Concert on 21 Nov; Tokyo 20-27 Dec", today, 3, &v)
            .await;
    assert_eq!(out.source, Source::Model);
    assert_eq!(out.value.len(), 2);
    assert!(out.value.iter().all(|e| e.value.is_none()));
    for bad in [
        json!({"events": [{"kind": "event.party", "starts": "2026-11-21", "ends": null, "reference": null}]}),
        json!({"events": [{"kind": "trip.air", "starts": "2027-03-01", "ends": null, "reference": null}]}),
        json!({"events": [{"kind": "trip.air", "starts": "2026-10-01", "ends": null, "reference": null}]}),
        json!({"events": [{"kind": "trip.air", "starts": "2026-12-20", "ends": "2026-12-01", "reference": null}]}),
        json!({"events": [{"kind": "event.concert", "starts": "2026-11-21", "ends": null, "reference": null, "value": 88000}]}),
    ] {
        let h = harness(bad.clone());
        let out = foresee::extract_upcoming_events(&h, "text", today, 3, &v).await;
        assert_eq!(out.source, Source::Fallback, "{bad}");
        assert!(out.value.is_empty());
    }
}

fn trip() -> UpcomingEvent {
    UpcomingEvent {
        kind: "trip.air".into(),
        starts: day(2026, 12, 20),
        ends: Some(day(2026, 12, 27)),
        reference: Some("CX880/2026-12-20".into()),
        value: None,
    }
}

#[tokio::test]
async fn r38_parametric_covers_get_no_prevention_task() {
    let cfg = cfg();
    let v = vocab(&cfg);
    let h = harness(json!({"task": null}));
    let out = prevention::propose(&h, &trip(), "flight_delay", &rider(), &cfg, &v).await;
    assert_eq!(out.source, Source::Rule);
    assert!(out.value.is_none());
    assert!(sent(&h).is_empty(), "the model was called");
}

#[tokio::test]
async fn r20_prevention_evidence_must_be_consented() {
    let cfg = cfg();
    let v = vocab(&cfg);
    let reply = json!({"task": {
        "task_type": "screen_protector_fitted",
        "action": "Fit a screen protector on your phone before you travel.",
        "evidence": "photo"
    }});
    let h = harness(reply.clone());
    let out = prevention::propose(&h, &trip(), "device_damage", &rider(), &cfg, &v).await;
    assert_eq!(out.source, Source::Model);
    assert_eq!(
        out.value.as_ref().map(|t| t.evidence.as_str()),
        Some("photo")
    );
    assert_eq!(sent(&h)[0].tier, Tier::Strong);
    // Without the photos opt-in, a photo is not evidence this customer can send.
    let diagnostics_only = Profile {
        consents: ["device_diagnostics".to_string()].into(),
        ..rider()
    };
    let h = harness(reply.clone());
    let out = prevention::propose(&h, &trip(), "device_damage", &diagnostics_only, &cfg, &v).await;
    assert_eq!(out.source, Source::Fallback);
    assert!(out.value.is_none());
    assert!(!sent(&h)[0].user.contains("\"photo\""));
    // With nothing the customer can prove, the model is not asked.
    let nothing = Profile {
        consents: BTreeSet::new(),
        ..rider()
    };
    let h = harness(reply);
    let out = prevention::propose(&h, &trip(), "device_damage", &nothing, &cfg, &v).await;
    assert_eq!(out.source, Source::Rule);
    assert!(sent(&h).is_empty(), "the model was called");
}

#[tokio::test]
async fn r18_prevention_task_type_must_be_a_registered_measure() {
    let cfg = cfg();
    let v = vocab(&cfg);
    let h = harness(json!({"task": {
        "task_type": "phone_kept_in_pocket",
        "action": "Keep your phone in a zipped pocket on the plane.",
        "evidence": "photo"
    }}));
    let out = prevention::propose(&h, &trip(), "device_damage", &rider(), &cfg, &v).await;
    assert_eq!(out.source, Source::Fallback);
    assert!(out.value.is_none());
    let asked = &sent(&h)[0].user;
    assert!(
        asked.contains("screen_protector_fitted"),
        "the registered measures are listed"
    );
    assert!(
        !asked.contains("smoke_alarm_fitted"),
        "only the cover's own class"
    );
}

#[tokio::test]
async fn r02_prevention_task_never_names_a_reward() {
    let cfg = cfg();
    let v = vocab(&cfg);
    let h = harness(json!({"task": {
        "task_type": "screen_protector_fitted",
        "action": "Fit a screen protector and earn $20 back.",
        "evidence": "photo"
    }}));
    let out = prevention::propose(&h, &trip(), "device_damage", &rider(), &cfg, &v).await;
    assert_eq!(out.source, Source::Fallback);
}

fn task() -> PreventionTaskDraft {
    PreventionTaskDraft {
        task_type: "screen_protector_fitted".into(),
        cover_id: "device_damage".into(),
        event_reference: None,
        action: "Fit a screen protector on your phone before you travel.".into(),
        evidence: "photo".into(),
        claim_prob_before: Bps(300),
        claim_prob_after: Bps(200),
        avg_claim_cost: Money::from_major(1_200),
        reward_cost: Money::from_major(5),
    }
}

fn photo(bytes: &[u8]) -> Evidence {
    Evidence {
        media_type: "image/png".into(),
        bytes: bytes.to_vec(),
    }
}

#[tokio::test]
async fn r10_verify_flags_reused_evidence_without_a_model_call() {
    let h = harness(json!({}));
    let seen: BTreeSet<String> = [verify::fingerprint(b"same photo")].into();
    let out = verify::verdict(&h, &task(), &[photo(b"same photo")], &seen, None).await;
    assert_eq!(out.source, Source::Rule);
    let VerifyOutcome::Verdict(v) = out.value else {
        panic!("a reused photo gets a verdict");
    };
    assert!(!v.passed && v.reuse_suspected);
    assert!(sent(&h).is_empty());
}

#[tokio::test]
async fn r10_verify_edited_evidence_never_passes() {
    let h = harness(
        json!({"passed": true, "reuse_suspected": false, "edited_suspected": true,
        "reason": "The photo shows a screen protector but looks edited."}),
    );
    let out = verify::verdict(&h, &task(), &[photo(b"new photo")], &BTreeSet::new(), None).await;
    assert_eq!(out.source, Source::Model);
    let VerifyOutcome::Verdict(v) = out.value else {
        panic!("verdict expected");
    };
    assert!(!v.passed && v.reuse_suspected);
    let req = &sent(&h)[0];
    assert_eq!(req.images.len(), 1);
    assert_eq!(req.tier, Tier::Strong);
}

#[tokio::test]
async fn r22_verify_without_a_valid_verdict_goes_to_a_person() {
    let h = harness_text("I think it passed");
    let out = verify::verdict(
        &h,
        &task(),
        &[photo(b"another photo")],
        &BTreeSet::new(),
        None,
    )
    .await;
    assert_eq!(out.source, Source::Fallback);
    assert!(matches!(out.value, VerifyOutcome::ReferToPerson { .. }));
}

fn offer(id: &str, value: i64, is_cash: bool) -> RewardOffer {
    RewardOffer {
        offer_id: id.into(),
        family: if is_cash { "insurance" } else { "partner" }.into(),
        partner_id: None,
        description: format!("offer {id}"),
        perceived_value: Money::from_major(value),
        price: Money::from_major(1),
        funded_by: Funder::Insurer,
        inventory: None,
        is_cash,
    }
}

#[tokio::test]
async fn r16_rewards_ranking_keeps_every_offer_and_cash() {
    let offers = [
        offer("cash_10", 10, true),
        offer("phone_mount", 25, false),
        offer("lounge", 60, false),
    ];
    let h = harness(json!({"offer_ids": ["phone_mount", "cash_10", "lounge"]}));
    let out = rewards::rank(&h, &rider(), &offers).await;
    assert_eq!(out.source, Source::Model);
    assert_eq!(out.value, ["phone_mount", "cash_10", "lounge"]);
    assert!(
        !sent(&h)[0].user.contains("\"price\""),
        "the model never sees what an offer costs"
    );
    // Dropping cash is not a ranking: the saved order puts cash first.
    let h = harness(json!({"offer_ids": ["phone_mount", "lounge"]}));
    let out = rewards::rank(&h, &rider(), &offers).await;
    assert_eq!(out.source, Source::Fallback);
    assert_eq!(out.value, ["cash_10", "lounge", "phone_mount"]);
}

#[tokio::test]
async fn r22_a_slow_or_failed_model_uses_the_saved_answer() {
    let cfg = cfg();
    let v = vocab(&cfg);
    let mut s = settings();
    s.timeout_ms = 20;
    let slow = Harness::new(
        Scripted {
            reply: Ok(json!({"questions": []}).to_string()),
            delay: Duration::from_millis(200),
            sent: Mutex::new(Vec::new()),
        },
        Prompts::in_repo(&root()),
        s,
        languages(&cfg),
    );
    let out = understand::proposal_form(&slow, "direct", None, &v).await;
    assert_eq!(out.source, Source::Fallback);
    assert!(
        out.reason
            .as_deref()
            .is_some_and(|r| r.contains("timed out"))
    );
    let failed = Harness::new(
        Scripted {
            reply: Err("HTTP 500".into()),
            delay: Duration::ZERO,
            sent: Mutex::new(Vec::new()),
        },
        Prompts::in_repo(&root()),
        settings(),
        languages(&cfg),
    );
    let out = rewards::rank(
        &failed,
        &rider(),
        &[offer("a", 1, true), offer("b", 2, false)],
    )
    .await;
    assert_eq!(out.source, Source::Fallback);
    // A missing prompt file is a failed call too.
    let no_prompts = Harness::new(
        Scripted {
            reply: Ok("{}".into()),
            delay: Duration::ZERO,
            sent: Mutex::new(Vec::new()),
        },
        Prompts::new(root().join("no-such-dir")),
        settings(),
        languages(&cfg),
    );
    let out = foresee::extract_upcoming_events(&no_prompts, "x", day(2026, 10, 9), 3, &v).await;
    assert_eq!(out.source, Source::Fallback);
    assert!(sent(&no_prompts).is_empty());
}

#[test]
fn every_component_has_its_prompt() {
    let prompts = Prompts::in_repo(&root());
    for c in [
        understand::FORM,
        understand::FACTS,
        foresee::COMPONENT,
        explain::SCHEDULE,
        explain::CHANGE,
        prevention::COMPONENT,
        verify::COMPONENT,
        assess::COMPONENT,
        rewards::COMPONENT,
    ] {
        let text = prompts.load(c).unwrap_or_else(|e| panic!("{c}: {e}"));
        assert!(
            text.contains(&format!("contracts/schemas/ai/{c}.json")),
            "{c}"
        );
        assert!(!text.contains("Decide"), "{c} is still a placeholder");
    }
}

#[test]
fn reply_schemas_are_closed_and_fully_required() {
    fn walk(v: &Value) {
        if let Some(map) = v.as_object() {
            for k in ["minimum", "maximum", "minLength", "format"] {
                if k == "format" {
                    assert!(
                        map.get(k).is_none_or(|f| f == "date"),
                        "format {:?}",
                        map.get(k)
                    );
                } else {
                    assert!(!map.contains_key(k), "{k} left in the schema");
                }
            }
            if let Some(props) = map.get("properties").and_then(Value::as_object) {
                assert_eq!(map["additionalProperties"], json!(false));
                let required: BTreeSet<&str> = map["required"]
                    .as_array()
                    .expect("required")
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                assert_eq!(required, props.keys().map(String::as_str).collect());
            }
            map.values().for_each(walk);
        } else if let Some(items) = v.as_array() {
            items.iter().for_each(walk);
        }
    }
    walk(&schema::output_schema::<foresee::ForeseeReply>());
    walk(&schema::output_schema::<verify::VerifyReply>());
    walk(&schema::output_schema::<assess::AssessReply>());
    walk(&schema::output_schema::<prevention::PreventionReply>());
    walk(&schema::output_schema::<understand::FormReply>());
}

#[test]
fn the_live_request_asks_for_the_schema_and_reads_only_a_finished_reply() {
    let client = AnthropicClient {
        http: reqwest::Client::new(),
        api_key: "test".into(),
        base_url: "http://127.0.0.1:9".into(),
        settings: settings(),
        max_retries: 0,
    };
    let req = ModelRequest {
        component: "explain",
        tier: Tier::Fast,
        system: "prompt".into(),
        user: "{}".into(),
        images: vec![microinsurance_ai::client::Image {
            media_type: "image/png".into(),
            bytes: vec![1, 2, 3],
        }],
        schema: schema::output_schema::<explain::ExplainReply>(),
    };
    let body = client.body(&req);
    assert_eq!(body["model"], "claude-haiku-5-5");
    assert_eq!(body["output_config"]["format"]["type"], "json_schema");
    assert_eq!(body["messages"][0]["content"][0]["type"], "image");
    assert!(
        body.get("fallbacks").is_none(),
        "no server-side fallback on the fast model"
    );
    let strong = client.body(&ModelRequest {
        tier: Tier::Strong,
        ..req
    });
    assert_eq!(strong["model"], "claude-sonnet-5-5");
    assert_eq!(strong["fallbacks"], "default");

    let done = json!({"stop_reason": "end_turn", "content": [
        {"type": "thinking", "thinking": ""}, {"type": "text", "text": "{\"a\":1}"}]});
    assert_eq!(reply_text(&done).expect("text"), "{\"a\":1}");
    for stop in ["refusal", "max_tokens"] {
        let body = json!({"stop_reason": stop, "content": [{"type": "text", "text": "{"}]});
        assert!(reply_text(&body).is_err(), "{stop}");
    }
}

#[test]
fn replay_keys_depend_on_component_input_and_images() {
    let req = |component: &'static str, user: &str, image: Option<&[u8]>| ModelRequest {
        component,
        tier: Tier::Fast,
        system: "any prompt".into(),
        user: user.into(),
        images: image
            .map(|b| microinsurance_ai::client::Image {
                media_type: "image/png".into(),
                bytes: b.to_vec(),
            })
            .into_iter()
            .collect(),
        schema: json!({}),
    };
    let a = replay::key(&req("explain", "{\"x\":1}", None));
    let prompt_edited = ModelRequest {
        system: "another prompt".into(),
        ..req("explain", "{\"x\":1}", None)
    };
    assert_eq!(a, replay::key(&prompt_edited));
    assert_ne!(a, replay::key(&req("rewards", "{\"x\":1}", None)));
    assert_ne!(a, replay::key(&req("explain", "{\"x\":2}", None)));
    assert_ne!(a, replay::key(&req("explain", "{\"x\":1}", Some(b"photo"))));
    assert!(a.starts_with("explain:") && a.len() == "explain:".len() + 16);
}

fn chinese_rider() -> Profile {
    Profile {
        language: Some("zh-Hant".into()),
        ..rider()
    }
}

#[tokio::test]
async fn text_follows_the_customers_language_setting() {
    let cfg = cfg();
    let h = harness(json!({"explanations": [
        {"cover_id": "weather_income", "sentence": "颱風訊號令你停工時，你會收到當天的賠償。"},
        {"cover_id": "commute_disruption", "sentence": "你的路綫受阻時，你會自動收到賠償。"}
    ]}));
    let out = explain::explain_schedule(&h, &schedule(), &chinese_rider(), &cfg).await;
    assert_eq!(out.source, Source::Model);
    assert!(sent(&h)[0].user.contains("\"language\":\"zh-Hant\""));
    // An English reply to a Chinese-speaking customer is not their language: saved answer.
    let h = harness(json!({"explanations": [
        {"cover_id": "weather_income", "sentence": "You get paid in a typhoon."},
        {"cover_id": "commute_disruption", "sentence": "You get paid when your line stops."}
    ]}));
    let out = explain::explain_schedule(&h, &schedule(), &chinese_rider(), &cfg).await;
    assert_eq!(out.source, Source::Fallback);
    let pack = h.languages.pack(Some("zh-Hant"));
    assert_eq!(out.value[1].sentence, pack.saved.covers["weather_income"]);
    // Money and protected attributes are caught in every offered language.
    for sentence in [
        "颱風時你會收到港幣480元。",
        "按你的年齡，颱風時你會收到賠償。",
    ] {
        let h = harness(json!({"explanations": [
            {"cover_id": "weather_income", "sentence": sentence},
            {"cover_id": "commute_disruption", "sentence": "你的路綫受阻時，你會自動收到賠償。"}
        ]}));
        let out = explain::explain_schedule(&h, &schedule(), &chinese_rider(), &cfg).await;
        assert_eq!(out.source, Source::Fallback, "{sentence}");
    }
    // No setting, or one not offered: the market default.
    assert_eq!(h.languages.pack(None).tag, cfg.languages.default);
    assert_eq!(h.languages.pack(Some("fr")).tag, cfg.languages.default);
}

#[tokio::test]
async fn saved_answers_come_in_the_customers_language() {
    let cfg = cfg();
    let v = vocab(&cfg);
    let h = harness_text("not json");
    let form = understand::proposal_form(&h, "delivery_app", Some("zh-Hant"), &v).await;
    assert_eq!(form.source, Source::Fallback);
    assert_eq!(form.value.language, "zh-Hant");
    let verdict = verify::verdict(
        &h,
        &task(),
        &[photo(b"zh photo")],
        &BTreeSet::new(),
        Some("zh-Hant"),
    )
    .await;
    let VerifyOutcome::ReferToPerson { reason } = verdict.value else {
        panic!("no valid verdict goes to a person");
    };
    assert_eq!(reason, h.languages.pack(Some("zh-Hant")).saved.verify.refer);
    let change = SumInsuredChange {
        cover_id: "weather_income".into(),
        premium: Money::from_major(55),
        before: Money::from_major(480),
        after: Money::from_major(400),
        declared_value: None,
        below_declared_value: false,
    };
    let out = explain::explain_change(&h, &change, None, Some("zh-Hant"), &cfg).await;
    assert_eq!(
        out.value,
        "下個月同樣的保費可為你的天氣收入保障買到較少保障。"
    );
}

#[test]
fn language_packs_are_checked_when_loaded() {
    let cfg = cfg();
    let files = microinsurance_config::ConfigDir::base(&root().join("config"))
        .language_files()
        .expect("packs");
    let with = |tag: &str, from: &str, to: &str| -> Vec<(String, String)> {
        files
            .iter()
            .map(|(t, text)| {
                (
                    t.clone(),
                    if t == tag {
                        text.replace(from, to)
                    } else {
                        text.clone()
                    },
                )
            })
            .collect()
    };
    // A saved answer that breaks the rules it guards: the process does not start.
    for (from, to) in [
        (
            "You get paid when your regular route is disrupted.",
            "You get HK$50 when your route is disrupted.",
        ),
        (
            "You get paid when your regular route is disrupted.",
            "Paid. Automatically.",
        ),
        (
            "  scam_loss: You get part of your money back after an online scam.\n",
            "",
        ),
        (
            "Next month your {cover} stays the same.",
            "Next month nothing changes.",
        ),
        (
            "      text: When do you usually work?",
            "      text: How old are you, and what is your age?",
        ),
    ] {
        let err = Languages::load(&with("en", from, to), &cfg).expect_err(to);
        assert!(err.contains("languages/en.yaml"), "{err}");
    }
    // Every offered language needs its pack.
    let only_en: Vec<(String, String)> = files.iter().filter(|(t, _)| t == "en").cloned().collect();
    assert!(Languages::load(&only_en, &cfg).is_err());
    // A customer's setting must be an offered language.
    let mut p = rider();
    p.language = Some("fr".into());
    assert!(cfg.check_profile(&p).is_err());
    p.language = Some("zh-Hant".into());
    assert!(cfg.check_profile(&p).is_ok());
}

fn claim() -> microinsurance_engine::claims::Claim {
    use microinsurance_engine::claims::{Claim, ClaimState};
    Claim {
        claim_id: "claim-1".into(),
        policy_id: "pol-1".into(),
        customer_id: "cust-1".into(),
        cover_id: "device_damage".into(),
        class_of_business: "property.device".into(),
        loss_date: day(2026, 10, 18),
        notified_at: chrono::DateTime::from_timestamp(1_792_400_000, 0).expect("time"),
        amount_claimed: Money::from_major(1_234),
        circumstance: None,
        asked_for_person: false,
        guarantee_requested: Money::ZERO,
        evidence: ["photo".to_string()].into(),
        urgent: false,
        state: ClaimState::Assessing,
    }
}

async fn assessed(reply: Value) -> (Harness<Scripted>, assess::AssessOutcome, Source) {
    let cfg = cfg();
    let h = harness(reply);
    let out = assess::assess(
        &h,
        &claim(),
        "My screen cracked when the phone fell.",
        &[photo(b"cracked")],
        &BTreeSet::new(),
        None,
        &cfg,
    )
    .await;
    (h, out.value, out.source)
}

#[tokio::test]
async fn r13_assess_reply_cannot_decline() {
    let (_, out, source) = assessed(
        json!({"recommend": "decline", "estimate": 0, "confidence": 99,
        "consistent": true, "urgent": false, "human_care": false,
        "reason": "This damage is not covered."}),
    )
    .await;
    assert_eq!(source, Source::Fallback);
    assert!(matches!(out, assess::AssessOutcome::ReferToPerson { .. }));
}

#[tokio::test]
async fn r02_assessor_never_sees_the_amount_claimed_or_sum_insured() {
    let (h, out, source) = assessed(json!({"recommend": "approve", "estimate": 1100,
        "confidence": 90, "consistent": true, "urgent": false, "human_care": false,
        "reason": "The photo shows a cracked phone screen."}))
    .await;
    assert_eq!(source, Source::Model);
    let assess::AssessOutcome::Assessed { assessment, .. } = out else {
        panic!("assessed");
    };
    assert_eq!(assessment.estimate, Money::from_major(1_100));
    assert_eq!(assessment.confidence, Bps(9_000));
    let req = &sent(&h)[0];
    assert_eq!(req.tier, Tier::Strong);
    for hidden in [
        "1234",
        "123400",
        "amount_claimed",
        "sum_insured",
        "cust-1",
        "pol-1",
    ] {
        assert!(!req.user.contains(hidden), "{hidden} sent to the model");
    }
}

#[tokio::test]
async fn r57_inconsistent_evidence_is_never_an_approval() {
    let (_, out, _) = assessed(
        json!({"recommend": "approve", "estimate": 900, "confidence": 95,
        "consistent": false, "urgent": false, "human_care": false,
        "reason": "The photo does not show the damage described."}),
    )
    .await;
    assert_eq!(
        out.review(),
        microinsurance_engine::claims::Review::Assessed(
            microinsurance_engine::claims::Assessment {
                estimate: Money::from_major(900),
                confidence: Bps(9_500),
                recommend: microinsurance_engine::claims::Recommendation::Refer,
                escalation: Default::default(),
            }
        )
    );
}

#[tokio::test]
async fn r46_assessor_escalation_is_carried_to_the_engine() {
    let (_, out, _) = assessed(
        json!({"recommend": "refer", "estimate": 0, "confidence": 50,
        "consistent": true, "urgent": true, "human_care": true,
        "reason": "A person from our team will contact you soon."}),
    )
    .await;
    let microinsurance_engine::claims::Review::Assessed(a) = out.review() else {
        panic!("assessed");
    };
    assert!(a.escalation.urgent && a.escalation.human_care);
}

#[tokio::test]
async fn r14_assess_flags_reused_evidence_without_a_model_call() {
    let cfg = cfg();
    let h = harness(json!({}));
    let seen: BTreeSet<String> = [verify::fingerprint(b"old photo")].into();
    let out = assess::assess(
        &h,
        &claim(),
        "Cracked screen.",
        &[photo(b"old photo")],
        &seen,
        None,
        &cfg,
    )
    .await;
    assert_eq!(out.source, Source::Rule);
    assert!(out.value.evidence_reused());
    assert_eq!(
        out.value.review(),
        microinsurance_engine::claims::Review::NoAssessment
    );
    assert!(sent(&h).is_empty());
}

#[tokio::test]
async fn r19_the_declared_work_area_never_reaches_the_model() {
    // The rider declared where they work (ADR-012): it decides where a trigger pays, and is the
    // insured location, never an AI input.
    let cfg = cfg();
    assert_eq!(rider().work_areas, ["tw.taipei_city"]);
    let h = harness(json!({"explanations": []}));
    explain::explain_schedule(&h, &schedule(), &rider(), &cfg).await;
    let offers = [offer("cash_10", 10, true), offer("lounge", 60, false)];
    let ranked = harness(json!({"offer_ids": ["lounge", "cash_10"]}));
    rewards::rank(&ranked, &rider(), &offers).await;
    for request in sent(&h).iter().chain(sent(&ranked).iter()) {
        assert!(
            !request.user.contains("taipei") && !request.user.contains("work_area"),
            "the work area reached the model"
        );
    }
}
