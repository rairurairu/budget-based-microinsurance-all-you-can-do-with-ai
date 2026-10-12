//! A load test of the API's non-AI routes against `docs/performance.md` (p99 < 50 ms), over HTTP
//! against a running API: demo mode (the in-memory store) or the production set-up with
//! PostgreSQL (ADR-012).
//!
//! It first binds `CUSTOMERS` new customers as a partner's embedded journey does: the partner
//! exchanges its reference for a customer token, the customer declares their working hours, asks
//! for a quote and taps to accept it (writes, timed per step). Then `CONCURRENCY` clients call a
//! mix of the routes customers, the partner and people at the insurer read most, for `SECONDS`,
//! and it reports p50, p95 and p99 per route.
//!
//! ```text
//! cargo run --release -p microinsurance-api --example load_test
//! ```
//!
//! `BASE_URL` (default `http://localhost:8080`). Tokens: in demo mode from `/demo/sessions`;
//! otherwise signed with `API_TOKEN_SECRET`, the API's own, as the insurer's identity provider
//! would. `PARTNER` (default `delivery_app`, a licensed agency that binds), `STAFF` (default
//! `handler-amy`), `CUSTOMERS` (default 200), `CONCURRENCY` (default 16), `SECONDS` (default 30).
//! Routes that ask the AI layer (the policy view, the renewal view, prevention tasks, reward
//! ranking) are left out: they wait on the model within the harness's timeout (R22).
use chrono::{Duration as Days, Utc};
use microinsurance_api::auth::{Principal, TokenKeys};
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// The target for every non-AI route (docs/performance.md).
const P99_TARGET_MS: f64 = 50.0;

fn var(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn number(name: &str, default: u64) -> Result<u64, String> {
    var(name, &default.to_string())
        .parse()
        .map_err(|_| format!("{name} is a number"))
}

/// Latencies per route, in microseconds, and errors.
#[derive(Default)]
struct Timings {
    by_route: BTreeMap<&'static str, Vec<u64>>,
    errors: BTreeMap<&'static str, u64>,
}

struct Load {
    client: Client,
    base: String,
    timings: Mutex<Timings>,
    /// Off while warming up: connection pools fill before anything is timed.
    recording: std::sync::atomic::AtomicBool,
}

impl Load {
    /// One call, timed under `route`; the JSON reply when it succeeds.
    async fn call(
        &self,
        route: &'static str,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> Option<Value> {
        let mut req = self.client.request(method, format!("{}{path}", self.base));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let started = Instant::now();
        let resp = req.send().await;
        let (ok, value) = match resp {
            Ok(r) => {
                let ok = r.status() == StatusCode::OK;
                let value = r.json::<Value>().await.ok();
                (ok, value)
            }
            Err(_) => (false, None),
        };
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        if self.recording.load(Ordering::Relaxed) {
            let mut t = self.timings.lock().await;
            t.by_route.entry(route).or_default().push(micros);
            if !ok {
                *t.errors.entry(route).or_default() += 1;
            }
        }
        ok.then_some(value).flatten()
    }
}

/// The partner's and the person's tokens: demo sessions, or signed with the API's secret.
async fn principals(load: &Load, partner: &str, staff: &str) -> Result<(String, String), String> {
    if let Ok(secret) = std::env::var("API_TOKEN_SECRET") {
        let keys = TokenKeys::new(secret.as_bytes());
        let until = Utc::now() + Days::hours(2);
        return Ok((
            keys.issue(
                &Principal::Partner {
                    partner_id: partner.to_string(),
                },
                until,
            ),
            keys.issue(
                &Principal::Staff {
                    staff_id: staff.to_string(),
                },
                until,
            ),
        ));
    }
    let sessions: Vec<Value> = load
        .client
        .get(format!("{}/demo/sessions", load.base))
        .send()
        .await
        .map_err(|e| format!("{}: {e}", load.base))?
        .json()
        .await
        .map_err(|_| "no demo sessions: set API_TOKEN_SECRET outside demo mode".to_string())?;
    let find = |kind: &str, field: &str, id: &str| {
        sessions
            .iter()
            .find(|s| s["principal"]["kind"] == kind && s["principal"][field] == id)
            .and_then(|s| s["token"].as_str())
            .map(str::to_string)
            .ok_or_else(|| format!("no demo session for {kind} {id}"))
    };
    Ok((
        find("partner", "partner_id", partner)?,
        find("staff", "staff_id", staff)?,
    ))
}

/// A partner's embedded journey for one new customer: their token, their working hours, a quote
/// and the tap that binds it. The customer's token, when they were bound.
async fn bind(load: &Load, partner: &str, run: &str, i: u64) -> Option<String> {
    let issued = load
        .call(
            "POST /partners/me/customer-tokens",
            Method::POST,
            "/partners/me/customer-tokens",
            Some(partner),
            Some(json!({ "customer_ref": format!("load-{run}-{i}") })),
        )
        .await?;
    let token = issued["token"].as_str()?.to_string();
    let hours: Vec<Value> = (1..=5)
        .map(|d| json!({ "weekday": d, "start_minute": 540, "end_minute": 1080 }))
        .collect();
    load.call(
        "PATCH /me/profile",
        Method::PATCH,
        "/me/profile",
        Some(&token),
        Some(json!({ "work_windows": hours })),
    )
    .await?;
    let quote = load
        .call(
            "POST /me/quotes",
            Method::POST,
            "/me/quotes",
            Some(&token),
            Some(json!({ "plan_id": "essential" })),
        )
        .await?;
    let id = quote["offer"]["offer_id"].as_str()?.to_string();
    load.call(
        "POST /me/quotes/{quote_id}/accept",
        Method::POST,
        &format!("/me/quotes/{}/accept", id.replace('/', "%2F")),
        Some(&token),
        None,
    )
    .await?;
    Some(token)
}

/// The read mix, by weight: what customers, the partner and people at the insurer open most.
const MIX: &[(&str, u64)] = &[
    ("GET /plans", 20),
    ("GET /registry", 10),
    ("GET /me", 15),
    ("GET /me/payouts", 15),
    ("GET /me/claims", 10),
    ("GET /me/decisions", 10),
    ("GET /partners/me/results", 10),
    ("GET /ops/queue", 5),
    ("GET /ops/jobs", 5),
];

fn pick(n: u64) -> &'static str {
    let total: u64 = MIX.iter().map(|(_, w)| w).sum();
    let mut at = n % total;
    for (route, w) in MIX {
        if at < *w {
            return route;
        }
        at -= w;
    }
    MIX[0].0
}

fn percentile(sorted: &[u64], pct: usize) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (pct * sorted.len()).div_ceil(100).max(1) - 1;
    sorted[rank.min(sorted.len() - 1)] as f64 / 1000.0
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let base = var("BASE_URL", "http://localhost:8080");
    let partner_id = var("PARTNER", "delivery_app");
    let staff_id = var("STAFF", "handler-amy");
    let customers = number("CUSTOMERS", 200)?;
    let concurrency = number("CONCURRENCY", 16)?;
    let seconds = number("SECONDS", 30)?;
    let client = Client::builder()
        .pool_max_idle_per_host(usize::try_from(concurrency).unwrap_or(16))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let load = Arc::new(Load {
        client,
        base,
        timings: Mutex::default(),
        recording: std::sync::atomic::AtomicBool::new(false),
    });
    let (partner, staff) = principals(&load, &partner_id, &staff_id).await?;
    let run = Utc::now().format("%Y%m%d%H%M%S").to_string();

    // Warm-up, not timed: one journey per client, so every pool has its connections.
    let mut warm = Vec::new();
    for i in 0..concurrency {
        let (load, partner, run) = (Arc::clone(&load), partner.clone(), format!("{run}-warm"));
        warm.push(tokio::spawn(
            async move { bind(&load, &partner, &run, i).await },
        ));
    }
    for w in warm {
        w.await.map_err(|e| e.to_string())?;
    }
    load.recording.store(true, Ordering::Relaxed);

    // Binding, `concurrency` journeys at a time.
    let started = Instant::now();
    let next = Arc::new(AtomicU64::new(0));
    let mut tasks = Vec::new();
    for _ in 0..concurrency {
        let (load, next, partner, run) = (
            Arc::clone(&load),
            Arc::clone(&next),
            partner.clone(),
            run.clone(),
        );
        tasks.push(tokio::spawn(async move {
            let mut tokens = Vec::new();
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= customers {
                    return tokens;
                }
                if let Some(t) = bind(&load, &partner, &run, i).await {
                    tokens.push(t);
                }
            }
        }));
    }
    let mut tokens = Vec::new();
    for t in tasks {
        tokens.extend(t.await.map_err(|e| e.to_string())?);
    }
    println!(
        "bound   {} of {customers} customers in {:.1} s",
        tokens.len(),
        started.elapsed().as_secs_f64()
    );
    if tokens.is_empty() {
        return Err("no customer was bound: see the errors above the report".into());
    }

    // Reads, for `seconds`.
    let tokens = Arc::new(tokens);
    let until = Instant::now() + Duration::from_secs(seconds);
    let calls = Arc::new(AtomicU64::new(0));
    let mut tasks = Vec::new();
    for w in 0..concurrency {
        let (load, tokens, calls, partner, staff) = (
            Arc::clone(&load),
            Arc::clone(&tokens),
            Arc::clone(&calls),
            partner.clone(),
            staff.clone(),
        );
        tasks.push(tokio::spawn(async move {
            let mut n = w;
            while Instant::now() < until {
                n = n
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let route = pick(n >> 33);
                let customer = &tokens[usize::try_from(n % tokens.len() as u64).unwrap_or(0)];
                let (path, token) = match route {
                    "GET /plans" => ("/plans?channel=delivery_app", None),
                    "GET /registry" => ("/registry", None),
                    "GET /me" => ("/me", Some(customer.as_str())),
                    "GET /me/payouts" => ("/me/payouts", Some(customer.as_str())),
                    "GET /me/claims" => ("/me/claims", Some(customer.as_str())),
                    "GET /me/decisions" => ("/me/decisions", Some(customer.as_str())),
                    "GET /partners/me/results" => ("/partners/me/results", Some(partner.as_str())),
                    "GET /ops/queue" => ("/ops/queue", Some(staff.as_str())),
                    _ => ("/ops/jobs", Some(staff.as_str())),
                };
                load.call(route, Method::GET, path, token, None).await;
                calls.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for t in tasks {
        t.await.map_err(|e| e.to_string())?;
    }
    let total = calls.load(Ordering::Relaxed);

    let t = load.timings.lock().await;
    println!(
        "read    {total} calls in {seconds} s ({:.0}/s), {concurrency} clients",
        total as f64 / seconds as f64
    );
    println!(
        "{:<36} {:>7} {:>6} {:>9} {:>9} {:>9}  target",
        "route", "calls", "errors", "p50 ms", "p95 ms", "p99 ms"
    );
    let mut missed = 0;
    for (route, micros) in &t.by_route {
        let mut sorted = micros.clone();
        sorted.sort_unstable();
        let p99 = percentile(&sorted, 99);
        let met = p99 < P99_TARGET_MS;
        missed += usize::from(!met);
        println!(
            "{route:<36} {:>7} {:>6} {:>9.1} {:>9.1} {:>9.1}  {}",
            sorted.len(),
            t.errors.get(route).copied().unwrap_or(0),
            percentile(&sorted, 50),
            percentile(&sorted, 95),
            p99,
            if met { "met" } else { "MISSED" }
        );
    }
    println!(
        "p99 < {P99_TARGET_MS} ms: {} of {} routes",
        t.by_route.len() - missed,
        t.by_route.len()
    );
    Ok(())
}
