//! Demo mode (`DEMO_MODE=1`): the API runs offline on a synthetic book, with saved AI replies
//! and a clock that starts at 15:00 Hong Kong time on 18 November 2026, when the organiser's
//! saved notice cancels a concert. It seeds three customers with policies (a delivery rider, a
//! traveller with a December trip, a concert-goer), a prospect with no policy yet, sixty
//! synthetic riders through the delivery partner (so the partner's aggregates have a cohort),
//! and the partner reward offers. `GET /demo/sessions` hands out the synthetic principals'
//! tokens; `POST /demo/replay/{source}` pushes a source's saved response as if the source had
//! sent it; `GET /demo/evidence/{file}` serves the synthetic evidence images. None of this
//! exists outside demo mode, and no real customer's data is used.
use crate::auth::Principal;
use crate::bodies::{CustomerRecord, EvidenceFile, FeedOutcome, PolicyRecord, RatingAnswers};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, Inner};
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, NaiveDate, Utc};
use microinsurance_engine::money::Money;
use microinsurance_engine::optimiser;
use microinsurance_engine::rewards::RewardOffer;
use microinsurance_engine::types::{
    DeclaredItem, Policy, PolicyPeriod, Profile, UpcomingEvent, WorkWindow, add_policy_periods,
};
use microinsurance_store::{Keys, collections as c};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path as FsPath;

/// Demo mode's signing secret. Never used outside demo mode.
pub const TOKEN_SECRET: &[u8] = b"demo-mode-only-token-secret-not-for-production";
/// Demo mode's audit seed. Live, the seed is the insurer's secret (R14).
pub const AUDIT_SEED: u64 = 2026;

/// 15:00 Hong Kong time on Wednesday 18 November 2026.
pub fn start() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-18T07:00:00Z")
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_default()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap_or_default()
}

fn weekdays(start: u16, end: u16) -> Vec<WorkWindow> {
    (1..=5)
        .map(|weekday| WorkWindow {
            weekday,
            start_minute: start,
            end_minute: end,
        })
        .collect()
}

struct Seed {
    customer_id: String,
    partner_id: Option<&'static str>,
    profile: Profile,
    /// The plan held, and the day its covers started; none for a prospect.
    plan: Option<(&'static str, NaiveDate)>,
    rating: Option<(&'static str, &'static str)>,
}

/// The synthetic book's amounts and references in the market's currency: the base market's, or
/// the international demonstration market's (`config/markets/global`, US dollars).
struct Book {
    rider_phone: i64,
    rider_earnings: i64,
    traveller_phone: i64,
    traveller_earnings: i64,
    flight: &'static str,
    concert_ticket: i64,
    new_earnings: i64,
    new_routes: &'static [&'static str],
    cohort_earnings: i64,
}

fn book(currency: &str) -> Book {
    match currency {
        "USD" => Book {
            rider_phone: 600,
            rider_earnings: 120,
            traveller_phone: 900,
            traveller_earnings: 150,
            flight: "ZZ204/2026-12-20",
            concert_ticket: 120,
            new_earnings: 110,
            new_routes: &[],
            cohort_earnings: 100,
        },
        _ => Book {
            rider_phone: 4_000,
            rider_earnings: 700,
            traveller_phone: 6_000,
            traveller_earnings: 900,
            flight: "CX500/2026-12-20",
            concert_ticket: 880,
            new_earnings: 650,
            new_routes: &["mtr.tsuen_wan"],
            cohort_earnings: 600,
        },
    }
}

fn seeds(currency: &str) -> Vec<Seed> {
    let b = book(currency);
    let profile = |id: &str| Profile {
        customer_id: id.into(),
        ..Profile::default()
    };
    let mut out = vec![
        Seed {
            customer_id: "cust-demo-rider".into(),
            partner_id: Some("delivery_app"),
            profile: Profile {
                occupation: Some("delivery rider".into()),
                work_windows: weekdays(9 * 60, 18 * 60),
                declared_items: vec![DeclaredItem {
                    kind: "device.phone".into(),
                    description: "work phone".into(),
                    declared_value: Some(Money::from_major(b.rider_phone)),
                }],
                typical_daily_earnings: Some(Money::from_major(b.rider_earnings)),
                consents: ["photos".to_string()].into(),
                ..profile("cust-demo-rider")
            },
            plan: Some(("plus", date(2026, 6, 1))),
            rating: Some(("declared_work_hours", "day")),
        },
        // As the workers' prevention demo: the saved proposal and verdict replay.
        Seed {
            customer_id: "cust-demo-traveller".into(),
            partner_id: None,
            profile: Profile {
                consents: ["photos".to_string()].into(),
                declared_items: vec![DeclaredItem {
                    kind: "device.phone".into(),
                    description: "new phone".into(),
                    declared_value: Some(Money::from_major(b.traveller_phone)),
                }],
                upcoming_events: vec![UpcomingEvent {
                    kind: "trip.air".into(),
                    starts: date(2026, 12, 20),
                    ends: Some(date(2026, 12, 27)),
                    reference: Some(b.flight.into()),
                    value: None,
                }],
                work_windows: weekdays(9 * 60, 18 * 60),
                typical_daily_earnings: Some(Money::from_major(b.traveller_earnings)),
                ..profile("cust-demo-traveller")
            },
            plan: Some(("plus", date(2026, 9, 1))),
            rating: Some(("device_value", "3000_to_8000")),
        },
        Seed {
            customer_id: "cust-demo-fan".into(),
            partner_id: None,
            profile: Profile {
                upcoming_events: vec![UpcomingEvent {
                    kind: "event.concert".into(),
                    starts: date(2026, 11, 21),
                    ends: None,
                    reference: Some("EVT-2026-1121-ARENA".into()),
                    value: Some(Money::from_major(b.concert_ticket)),
                }],
                ..profile("cust-demo-fan")
            },
            plan: Some(("starter", date(2026, 11, 1))),
            rating: None,
        },
        Seed {
            customer_id: "cust-demo-new".into(),
            partner_id: Some("delivery_app"),
            profile: Profile {
                occupation: Some("delivery rider".into()),
                work_windows: weekdays(10 * 60, 20 * 60),
                commute_routes: b.new_routes.iter().map(|r| r.to_string()).collect(),
                typical_daily_earnings: Some(Money::from_major(b.new_earnings)),
                ..profile("cust-demo-new")
            },
            plan: None,
            rating: None,
        },
    ];
    for i in 1..=60 {
        let id = format!("cust-demo-cohort-{i:02}");
        out.push(Seed {
            profile: Profile {
                work_windows: weekdays(8 * 60, 17 * 60),
                typical_daily_earnings: Some(Money::from_major(b.cohort_earnings)),
                ..profile(&id)
            },
            customer_id: id,
            partner_id: Some("delivery_app"),
            plan: Some(("essential", date(2026, 8, 1))),
            rating: None,
        });
    }
    out
}

/// Seeds the synthetic book (see the module docs).
pub async fn seed(state: &AppState, root: &FsPath) -> Result<(), String> {
    let november = PolicyPeriod {
        index: 6,
        starts: date(2026, 11, 1),
        ends: add_policy_periods(date(2026, 11, 1), 1),
    };
    let created = start();
    let err = |e: microinsurance_store::StoreError| e.to_string();
    for s in seeds(&state.cfg.currency) {
        let keys = Keys::customer(&s.customer_id).with_partner(s.partner_id);
        let record = CustomerRecord {
            profile: s.profile.clone(),
            partner_id: s.partner_id.map(str::to_string),
            created_at: created,
            cancelled: None,
        };
        state
            .store
            .insert(c::CUSTOMERS, &s.customer_id, keys.clone(), record)
            .await
            .map_err(err)?;
        if let Some((factor, answer)) = s.rating {
            let answers = RatingAnswers {
                answers: [(factor.to_string(), answer.to_string())].into(),
            };
            state
                .store
                .insert(c::RATING_ANSWERS, &s.customer_id, keys.clone(), answers)
                .await
                .map_err(err)?;
        }
        let Some((plan_id, since)) = s.plan else {
            continue;
        };
        let plan = state
            .cfg
            .plan(plan_id)
            .ok_or_else(|| format!("plan {plan_id}"))?
            .clone();
        // The engine's own suggestion for the customer, so the sums insured match the rates.
        let rated = crate::handlers::rated(state, &plan, &s.profile, None, november)
            .await
            .map_err(|e| e.body.detail)?;
        let schedule = optimiser::suggest(&plan, &s.profile, None, &rated.ctx(&state.cfg))
            .map_err(|e| e.to_string())?
            .outcome
            .schedule;
        let policy_id = format!("pol-{}", s.customer_id.trim_start_matches("cust-"));
        let policy = Policy {
            policy_id: policy_id.clone(),
            customer_id: s.customer_id.clone(),
            plan: plan.clone(),
            cover_starts: schedule
                .allocations
                .iter()
                .map(|a| (a.cover_id.clone(), since))
                .collect(),
            schedule,
            period: november,
            work_windows: s.profile.work_windows.clone(),
            commute_routes: s.profile.commute_routes.clone(),
            work_areas: s.profile.work_areas.clone(),
            upcoming_events: s.profile.upcoming_events.clone(),
            subscribed_at: DateTime::parse_from_rfc3339("2026-06-01T02:00:00Z")
                .map(|t| t.with_timezone(&Utc))
                .unwrap_or_default(),
        };
        let record = PolicyRecord {
            policy,
            partner_id: s.partner_id.map(str::to_string),
            premium_due: november.starts,
            premium_paid_on: Some(november.starts),
            quote_id: "seeded".into(),
        };
        state
            .store
            .insert(c::POLICIES, &policy_id, keys, record)
            .await
            .map_err(err)?;
    }
    let path = root.join("fixtures/reward_offers_synthetic.json");
    let text = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let offers: Vec<RewardOffer> =
        serde_json::from_slice(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    for o in offers {
        let keys = Keys {
            customer_id: None,
            partner_id: o.partner_id.clone(),
        };
        state
            .store
            .insert(c::REWARD_OFFERS, &o.offer_id.clone(), keys, o)
            .await
            .map_err(err)?;
    }
    Ok(())
}

/// A synthetic principal and its token.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DemoSession {
    pub label: String,
    pub principal: Principal,
    pub token: String,
}

fn principals() -> Vec<(&'static str, Principal)> {
    let customer = |id: &str, partner: Option<&str>| Principal::Customer {
        customer_id: id.into(),
        partner_id: partner.map(str::to_string),
    };
    vec![
        (
            "Delivery rider (through delivery_app)",
            customer("cust-demo-rider", Some("delivery_app")),
        ),
        (
            "Traveller with a December trip",
            customer("cust-demo-traveller", None),
        ),
        (
            "Concert-goer with a ticket for 21 November",
            customer("cust-demo-fan", None),
        ),
        (
            "New customer through delivery_app",
            customer("cust-demo-new", Some("delivery_app")),
        ),
        (
            "Partner: delivery_app",
            Principal::Partner {
                partner_id: "delivery_app".into(),
            },
        ),
        (
            "Partner: ticketing_partner",
            Principal::Partner {
                partner_id: "ticketing_partner".into(),
            },
        ),
        (
            "Claims handler: handler-amy",
            Principal::Staff {
                staff_id: "handler-amy".into(),
            },
        ),
        (
            "Claims handler: handler-ben",
            Principal::Staff {
                staff_id: "handler-ben".into(),
            },
        ),
        (
            "Feed: organiser_notices",
            Principal::Feed {
                source: "organiser_notices".into(),
            },
        ),
    ]
}

/// The synthetic principals' tokens, valid for a day of demo time.
pub async fn sessions(State(state): State<AppState>) -> ApiResult<Vec<DemoSession>> {
    let expires = state.clock.now() + chrono::Duration::days(1);
    Ok(Json(
        principals()
            .into_iter()
            .map(|(label, principal)| DemoSession {
                label: label.into(),
                token: state.keys.issue(&principal, expires),
                principal,
            })
            .collect(),
    ))
}

/// Pushes a source's saved response (`demo_mode_fixture` in its config file) as if the source
/// had sent it.
pub async fn replay(
    State(state): State<AppState>,
    Path(source): Path<String>,
) -> ApiResult<FeedOutcome> {
    let access = state
        .config
        .source_access(&source)
        .map_err(ApiError::not_found)?;
    let fixture = access
        .demo_mode_fixture
        .ok_or_else(|| ApiError::not_found(format!("{source} has no saved response")))?;
    let path = state.root.join(fixture);
    let body = std::fs::read_to_string(&path)
        .map_err(|e| ApiError::internal(format!("{}: {e}", path.display())))?;
    crate::handlers::feeds::push_payload(&state, &source, body)
        .await
        .map(Json)
}

fn evidence_dir(state: &Inner) -> Option<std::path::PathBuf> {
    Some(state.root.join("fixtures/evidence"))
}

/// The synthetic evidence images, by file name.
pub async fn evidence_list(State(state): State<AppState>) -> ApiResult<Vec<String>> {
    let dir = evidence_dir(&state).ok_or_else(|| ApiError::internal("fixtures"))?;
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with("_synthetic.png"))
        .collect();
    names.sort();
    Ok(Json(names))
}

/// One synthetic evidence image, ready to upload.
pub async fn evidence_file(
    State(state): State<AppState>,
    Path(file): Path<String>,
) -> ApiResult<EvidenceFile> {
    if !file.ends_with("_synthetic.png") || file.contains(['/', '\\']) || file.contains("..") {
        return Err(ApiError::not_found(file));
    }
    let dir = evidence_dir(&state).ok_or_else(|| ApiError::internal("fixtures"))?;
    let bytes = std::fs::read(dir.join(&file)).map_err(|_| ApiError::not_found(file))?;
    Ok(Json(EvidenceFile {
        media_type: "image/png".into(),
        data_base64: STANDARD.encode(bytes),
    }))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/demo/sessions", get(sessions))
        .route("/demo/replay/{source}", post(replay))
        .route("/demo/evidence", get(evidence_list))
        .route("/demo/evidence/{file}", get(evidence_file))
}
