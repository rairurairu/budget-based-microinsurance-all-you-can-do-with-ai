//! `just gen`: JSON Schemas for every type that crosses a boundary, and an OpenAPI 3.1 document
//! carrying them as components, into contracts/. The TypeScript client is generated from it.
//! The HTTP paths come from the API's route table (`api::routes::ROUTES`, ADR-011), so a route
//! cannot be served without being in the contract. The reply schemas the AI layer asks models for
//! are in contracts/schemas/ai/. Never edit contracts/ by hand.
use microinsurance_ai::assess::{AssessOutcome, AssessReply};
use microinsurance_ai::explain::{CoverExplanation, ExplainReply, SentenceReply};
use microinsurance_ai::foresee::ForeseeReply;
use microinsurance_ai::prevention::PreventionReply;
use microinsurance_ai::rewards::RankReply;
use microinsurance_ai::understand::{FormReply, ProfileFacts, ProposalForm};
use microinsurance_ai::verify::{VerifyOutcome, VerifyReply};
use microinsurance_ai::{
    assess, explain, foresee, prevention, rewards, schema, understand, verify,
};
use microinsurance_api::auth::Principal;
use microinsurance_api::bodies::{
    Accepted, AiSource, AuditRecord, ChangeSentence, ChannelInfo, ClaimEvidence, ClaimRecord,
    ClaimRequest, ConsentsUpdate, CoverInfo, CustomerRecord, EvidenceFile, EvidenceUpload,
    FeedOutcome, GuaranteeRecord, IdentityCheck, IssuedToken, JobsView, PayoutNotice,
    PersonDecision, PilotReport, PolicyRecord, PolicyView, PoolScore, PremiumPayment,
    ProfileFactsView, ProfileUpdate, ProposalAnswers, ProposalFormRequest, ProposalFormView, Quote,
    QuoteRequest, RatingAnswers, RatingQuestion, RedeemRequest, Registry, RenewalView,
    ReviewOpened, ReviewRequest, RewardOffers, RewardsView, ScoreView, SettlementChoice,
    TaskOutcome, TaskRecord, TokenRequest,
};
use microinsurance_api::copy::CopyPack;
use microinsurance_api::demo::DemoSession;
use microinsurance_api::error::ErrorBody;
use microinsurance_api::routes::{Access, Body, ROUTES};
use microinsurance_engine::claims::{
    Assessment, Claim, ClaimNotice, ClaimPayment, PersonOutcome, StpOutcome,
};
use microinsurance_engine::consent::{
    CancellationData, CustomerTap, Endorsement, Offer, PremiumStatus, RenewalNotice,
};
use microinsurance_engine::guardrails::DecisionRecord;
use microinsurance_engine::guardrails::appeals::Appeal;
use microinsurance_engine::guardrails::audit::AuditFinding;
use microinsurance_engine::guardrails::human_care::Triage;
use microinsurance_engine::guardrails::queue::OpsSnapshot;
use microinsurance_engine::optimiser::Suggestion;
use microinsurance_engine::optimiser::underwriting::ClassStatus;
use microinsurance_engine::payouts::{PayoutDecision, Settled};
use microinsurance_engine::plans::{PlanPrice, PurchaseContext};
use microinsurance_engine::prevention::{
    PreventionTask, PreventionTaskDraft, PreventionTaskProposal, Verdict,
};
use microinsurance_engine::rewards::{FraudFinding, Grant, Redemption, RewardOffer, Voided};
use microinsurance_engine::score::{ScoreMode, ScoreOutcome};
use microinsurance_engine::suppliers::{InKindOrder, SupplierStatement};
use microinsurance_engine::types::{Plan, Policy, Profile, Schedule};
use microinsurance_learning::experience::ExperienceStudy;
use microinsurance_learning::insights::PartnerResults;
use schemars::JsonSchema;
use schemars::generate::{SchemaGenerator, SchemaSettings};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

type Render = fn(&mut SchemaGenerator) -> Value;

fn root<T: JsonSchema>(g: &mut SchemaGenerator) -> Value {
    g.root_schema_for::<T>().to_value()
}

fn component<T: JsonSchema>(g: &mut SchemaGenerator) -> Value {
    g.subschema_for::<T>().to_value()
}

/// Every boundary type, by the name it is published under.
fn boundary_types() -> Vec<(&'static str, Render, Render)> {
    macro_rules! types {
        ($($t:ident),* $(,)?) => {
            vec![$((stringify!($t), root::<$t> as Render, component::<$t> as Render)),*]
        };
    }
    types!(
        // AI recommendations: crates/ai parses model output into these (R22).
        Assessment,
        AssessOutcome,
        Verdict,
        VerifyOutcome,
        PreventionTaskProposal,
        PreventionTaskDraft,
        ProposalForm,
        ProfileFacts,
        CoverExplanation,
        // Customer journey.
        Plan,
        PlanPrice,
        PurchaseContext,
        Profile,
        Schedule,
        Suggestion,
        Offer,
        CustomerTap,
        Endorsement,
        RenewalNotice,
        ClassStatus,
        PremiumStatus,
        ScoreMode,
        ScoreOutcome,
        StpOutcome,
        // Automatic payouts: shown to the customer, who may choose an alternative (R16, R28).
        PayoutDecision,
        Settled,
        // Prevention and rewards: the task lifecycle, the reward balance and its menu (R10, R16,
        // R17, ADR-009).
        PreventionTask,
        Grant,
        RewardOffer,
        Redemption,
        // Supplier settlement: in-kind orders and monthly statements (R56).
        InKindOrder,
        SupplierStatement,
        // Claims: the notification of loss, the claim and its payment; straight-through approval or
        // referral, never a decline; a person's decision; an urgent case's triage (R13, R46, R57).
        ClaimNotice,
        Claim,
        ClaimPayment,
        PersonOutcome,
        Triage,
        // Guardrails: the decision log, review requests, audit findings, voided rewards and the
        // ops console's snapshot (R14, R15, R19; ADR-010).
        DecisionRecord,
        Appeal,
        AuditFinding,
        FraudFinding,
        Voided,
        OpsSnapshot,
        // The API (ADR-011): who is calling, errors, and each route's request and response.
        Principal,
        ErrorBody,
        AiSource,
        Policy,
        CancellationData,
        CoverInfo,
        ChannelInfo,
        Registry,
        RatingQuestion,
        CopyPack,
        CustomerRecord,
        ProfileUpdate,
        ConsentsUpdate,
        RatingAnswers,
        ProposalFormRequest,
        ProposalFormView,
        ProposalAnswers,
        ProfileFactsView,
        QuoteRequest,
        Quote,
        Accepted,
        PolicyRecord,
        PolicyView,
        RenewalView,
        ChangeSentence,
        EvidenceFile,
        EvidenceUpload,
        TaskRecord,
        TaskOutcome,
        PoolScore,
        ScoreView,
        RewardsView,
        RedeemRequest,
        RewardOffers,
        PayoutNotice,
        SettlementChoice,
        ClaimRequest,
        ClaimRecord,
        ClaimEvidence,
        GuaranteeRecord,
        ReviewRequest,
        ReviewOpened,
        IdentityCheck,
        TokenRequest,
        IssuedToken,
        PremiumPayment,
        PartnerResults,
        PersonDecision,
        AuditRecord,
        FeedOutcome,
        DemoSession,
        // Adoption (ADR-012): the job queue, the experience study and the pilot report.
        JobsView,
        ExperienceStudy,
        PilotReport,
    )
}

/// The reply each AI component asks its model for (structured outputs), by component.
fn ai_replies() -> Vec<(&'static str, Value)> {
    vec![
        (
            understand::FORM,
            schema::published::<FormReply>(understand::FORM),
        ),
        (
            understand::FACTS,
            schema::published::<ProfileFacts>(understand::FACTS),
        ),
        (
            foresee::COMPONENT,
            schema::published::<ForeseeReply>(foresee::COMPONENT),
        ),
        (
            explain::SCHEDULE,
            schema::published::<ExplainReply>(explain::SCHEDULE),
        ),
        (
            explain::CHANGE,
            schema::published::<SentenceReply>(explain::CHANGE),
        ),
        (
            prevention::COMPONENT,
            schema::published::<PreventionReply>(prevention::COMPONENT),
        ),
        (
            verify::COMPONENT,
            schema::published::<VerifyReply>(verify::COMPONENT),
        ),
        (
            assess::COMPONENT,
            schema::published::<AssessReply>(assess::COMPONENT),
        ),
        (
            rewards::COMPONENT,
            schema::published::<RankReply>(rewards::COMPONENT),
        ),
    ]
}

fn json_text(value: &Value) -> Result<String, Box<dyn Error>> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(text)
}

fn schema_ref(name: &str) -> Value {
    json!({ "$ref": format!("#/components/schemas/{name}") })
}

/// A body's media type and schema; none for an empty body.
fn body(b: Body) -> Option<(&'static str, Value)> {
    match b {
        Body::Empty => None,
        Body::Text => Some(("text/plain", json!({ "type": "string" }))),
        Body::Json(name) => Some(("application/json", schema_ref(name))),
        Body::JsonList(name) => Some((
            "application/json",
            json!({ "type": "array", "items": schema_ref(name) }),
        )),
        Body::Events(name) => Some((
            "text/event-stream",
            json!({
                "type": "string",
                "description": format!("Server-sent events, each carrying a {name} as JSON")
            }),
        )),
    }
}

fn schema_name(b: Body) -> Option<&'static str> {
    match b {
        Body::Json(n) | Body::JsonList(n) | Body::Events(n) => Some(n),
        Body::Empty | Body::Text => None,
    }
}

fn caller(access: Access) -> &'static str {
    match access {
        Access::Public => "public",
        Access::Customer => "customer",
        Access::Partner => "partner",
        Access::Staff => "staff",
        Access::Feed => "feed",
    }
}

/// Every route of the API's table, as OpenAPI paths. A route naming a schema that is not
/// generated fails the generation.
fn paths(known: &BTreeSet<&str>) -> Result<Value, Box<dyn Error>> {
    let mut paths = serde_json::Map::new();
    for r in ROUTES {
        let method = r.method.as_str();
        for name in [r.request, r.response].into_iter().filter_map(schema_name) {
            if !known.contains(name) {
                return Err(
                    format!("{method} {} names {name}, which is not generated", r.path).into(),
                );
            }
        }
        let mut parameters: Vec<Value> = r
            .path
            .split('/')
            .filter_map(|seg| seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
            .map(|name| json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" } }))
            .collect();
        parameters.extend(r.query.iter().map(
            |name| json!({ "name": name, "in": "query", "required": false, "schema": { "type": "string" } }),
        ));
        let operation_id: String = format!("{method}{}", r.path)
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>()
            .split('_')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("_");
        let ok = match body(r.response) {
            Some((media, schema)) => {
                json!({ "description": "OK", "content": { media: { "schema": schema } } })
            }
            None => json!({ "description": "OK" }),
        };
        let mut op = json!({
            "operationId": operation_id,
            "summary": r.summary,
            "description": format!(
                "Caller: {}. Rules: {}.",
                caller(r.access),
                if r.rules.is_empty() { "none".to_string() } else { r.rules.join(", ") }
            ),
            "tags": [caller(r.access)],
            "responses": {
                "200": ok,
                "4XX": {
                    "description": "Refused: `rule` names the business rule that refused it, when one did",
                    "content": { "application/json": { "schema": schema_ref("ErrorBody") } }
                }
            }
        });
        if !parameters.is_empty() {
            op["parameters"] = json!(parameters);
        }
        if let Some((media, schema)) = body(r.request) {
            op["requestBody"] =
                json!({ "required": true, "content": { media: { "schema": schema } } });
        }
        if r.access != Access::Public {
            op["security"] = json!([{ "bearer": [] }]);
        }
        let item = paths.entry(r.path.to_string()).or_insert_with(|| json!({}));
        item[method] = op;
    }
    Ok(Value::Object(paths))
}

/// Every generated file, by path relative to contracts/.
fn render() -> Result<BTreeMap<PathBuf, String>, Box<dyn Error>> {
    let mut files = BTreeMap::new();
    let mut components = SchemaSettings::draft2020_12()
        .with(|s| {
            s.definitions_path = "/components/schemas".into();
            s.meta_schema = None;
        })
        .into_generator();
    for (name, standalone, as_component) in boundary_types() {
        let path = Path::new("schemas").join(format!("{name}.json"));
        files.insert(
            path,
            json_text(&standalone(
                &mut SchemaSettings::draft2020_12().into_generator(),
            ))?,
        );
        as_component(&mut components);
    }
    for (component, reply) in ai_replies() {
        let path = Path::new("schemas")
            .join("ai")
            .join(format!("{component}.json"));
        files.insert(path, json_text(&reply)?);
    }
    let schemas = components.take_definitions(true);
    let known: BTreeSet<&str> = schemas.keys().map(String::as_str).collect();
    let openapi = json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Budget-Based Microinsurance API",
            "version": "0.0.0",
            "description": "Generated by `just gen` from the Rust types and the API's route table. Do not edit by hand. Path parameters are percent-encoded: some ids contain `/`."
        },
        "paths": paths(&known)?,
        "components": {
            "schemas": schemas,
            "securitySchemes": {
                "bearer": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "A token naming one principal: a customer, a distribution partner, a person at the insurer or a data source (ADR-011)."
                }
            }
        }
    });
    files.insert(PathBuf::from("openapi.json"), json_text(&openapi)?);
    Ok(files)
}

/// Every file under `dir`, at any depth.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out
}

/// Writes contracts/, or with `--check` fails if it differs from the Rust types (CI drift check).
fn main() -> Result<(), Box<dyn Error>> {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts");
    let files = render()?;
    let on_disk = files_under(&out.join("schemas"));

    if std::env::args().any(|a| a == "--check") {
        let mut drift: Vec<String> = files
            .iter()
            .filter(|(path, text)| fs::read_to_string(out.join(path)).ok().as_ref() != Some(*text))
            .map(|(path, _)| path.display().to_string())
            .collect();
        drift.extend(
            on_disk
                .iter()
                .filter_map(|p| p.strip_prefix(&out).ok())
                .filter(|p| !files.contains_key(*p))
                .map(|p| format!("{} (no longer generated)", p.display())),
        );
        if drift.is_empty() {
            return Ok(());
        }
        return Err(format!(
            "contracts/ is out of date; run `just gen`: {}",
            drift.join(", ")
        )
        .into());
    }

    for stale in on_disk {
        fs::remove_file(stale)?;
    }
    for (path, text) in &files {
        if let Some(dir) = out.join(path).parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(out.join(path), text)?;
    }
    println!("wrote {} files to contracts/", files.len());
    Ok(())
}
