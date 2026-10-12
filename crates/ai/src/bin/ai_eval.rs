//! The golden-set runner for `just eval` (python/evals). Reads cases as JSON lines on stdin,
//! `{"case", "component", "input"}`, runs each through its component exactly as production does
//! (harness, checks, fallback) and writes one JSON line per case to stdout with the result, the
//! request the model was sent (after protected attributes were stripped) and its replay key.
//!
//! By default the model is the saved replies in `fixtures/ai_replies/` and nothing leaves the
//! machine. With `EVAL_LIVE=1` it is the configured provider (`ANTHROPIC_API_KEY`), and each
//! line also carries the model's raw reply so it can be saved as a replay.
use chrono::NaiveDate;
use microinsurance_ai::client::{AnthropicClient, AnyModel, Model, ModelRequest};
use microinsurance_ai::harness::{AiError, Harness, Harnessed};
use microinsurance_ai::language::Languages;
use microinsurance_ai::prompts::Prompts;
use microinsurance_ai::replay::{self, ReplayModel};
use microinsurance_ai::settings::AiSettings;
use microinsurance_ai::understand::ProposalForm;
use microinsurance_ai::verify::Evidence;
use microinsurance_ai::vocab::Vocabulary;
use microinsurance_ai::{assess, explain, foresee, prevention, rewards, understand, verify};
use microinsurance_engine::claims::Claim;
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::consent::SumInsuredChange;
use microinsurance_engine::optimiser::SuggestionBasis;
use microinsurance_engine::prevention::PreventionTaskDraft;
use microinsurance_engine::rewards::RewardOffer;
use microinsurance_engine::types::{Profile, Schedule, UpcomingEvent};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What one call sent and got back.
#[derive(Default)]
struct Seen {
    user: Option<String>,
    key: Option<String>,
    reply: Option<String>,
}

/// The model, recording each request and reply for the report.
struct Recording {
    inner: AnyModel,
    seen: Mutex<Seen>,
}

impl Model for Recording {
    async fn complete(&self, req: &ModelRequest) -> Result<String, AiError> {
        if let Ok(mut s) = self.seen.lock() {
            s.user = Some(req.user.clone());
            s.key = Some(replay::key(req));
        }
        let out = self.inner.complete(req).await;
        if let (Ok(text), Ok(mut s)) = (&out, self.seen.lock()) {
            s.reply = Some(text.clone());
        }
        out
    }
}

#[derive(Deserialize)]
struct Case {
    case: String,
    component: String,
    input: Value,
}

#[derive(Deserialize)]
struct EvidenceFile {
    media_type: String,
    /// Relative to the repository root.
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

fn field<T: DeserializeOwned>(input: &Value, name: &str) -> Result<T, String> {
    serde_json::from_value(input.get(name).cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("input.{name}: {e}"))
}

/// The evidence files of a case, read from the repository or given inline.
fn read_evidence(files: Vec<EvidenceFile>, root: &Path) -> Result<Vec<Evidence>, String> {
    files
        .into_iter()
        .map(|f| {
            let bytes = match (f.path, f.text) {
                (Some(p), _) => std::fs::read(root.join(&p)).map_err(|e| format!("{p}: {e}"))?,
                (None, Some(t)) => t.into_bytes(),
                (None, None) => return Err("evidence needs a path or a text".into()),
            };
            Ok(Evidence {
                media_type: f.media_type,
                bytes,
            })
        })
        .collect()
}

fn report<T: serde::Serialize>(out: Harnessed<T>) -> Result<Value, String> {
    serde_json::to_value(out).map_err(|e| e.to_string())
}

async fn run_case(
    h: &Harness<Recording>,
    case: &Case,
    cfg: &EngineConfig,
    vocab: &Vocabulary,
    root: &Path,
) -> Result<Value, String> {
    let i = &case.input;
    match case.component.as_str() {
        understand::FORM => {
            let channel: String = field(i, "channel")?;
            let language: Option<String> = field(i, "language")?;
            report(understand::proposal_form(h, &channel, language.as_deref(), vocab).await)
        }
        understand::FACTS => {
            let form: ProposalForm = field(i, "form")?;
            let answers: Value = field(i, "answers")?;
            report(understand::map_answers(h, &form, &answers, vocab).await)
        }
        foresee::COMPONENT => {
            let text: String = field(i, "text")?;
            let today: NaiveDate = field(i, "today")?;
            let months: u8 = field(i, "horizon_months")?;
            report(foresee::extract_upcoming_events(h, &text, today, months, vocab).await)
        }
        explain::SCHEDULE => {
            let schedule: Schedule = field(i, "schedule")?;
            let profile: Profile = field(i, "profile")?;
            report(explain::explain_schedule(h, &schedule, &profile, cfg).await)
        }
        explain::CHANGE => {
            let change: SumInsuredChange = field(i, "change")?;
            let basis: Option<SuggestionBasis> = field(i, "basis")?;
            let language: Option<String> = field(i, "language")?;
            report(
                explain::explain_change(h, &change, basis.as_ref(), language.as_deref(), cfg).await,
            )
        }
        prevention::COMPONENT => {
            let event: UpcomingEvent = field(i, "event")?;
            let cover_id: String = field(i, "cover_id")?;
            let profile: Profile = field(i, "profile")?;
            report(prevention::propose(h, &event, &cover_id, &profile, cfg, vocab).await)
        }
        verify::COMPONENT => {
            let task: PreventionTaskDraft = field(i, "task")?;
            let evidence = read_evidence(field(i, "evidence")?, root)?;
            let seen: BTreeSet<String> = field::<Option<_>>(i, "seen")?.unwrap_or_default();
            let language: Option<String> = field(i, "language")?;
            report(verify::verdict(h, &task, &evidence, &seen, language.as_deref()).await)
        }
        assess::COMPONENT => {
            let claim: Claim = field(i, "claim")?;
            let description: String = field(i, "description")?;
            let evidence = read_evidence(field(i, "evidence")?, root)?;
            let seen: BTreeSet<String> = field::<Option<_>>(i, "seen")?.unwrap_or_default();
            let language: Option<String> = field(i, "language")?;
            report(
                assess::assess(
                    h,
                    &claim,
                    &description,
                    &evidence,
                    &seen,
                    language.as_deref(),
                    cfg,
                )
                .await,
            )
        }
        rewards::COMPONENT => {
            let profile: Profile = field(i, "profile")?;
            let offers: Vec<RewardOffer> = field(i, "offers")?;
            report(rewards::rank(h, &profile, &offers).await)
        }
        other => Err(format!("unknown component {other:?}")),
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let root = std::env::var("REPO_ROOT").map_or_else(|_| PathBuf::from("."), PathBuf::from);
    let config = microinsurance_config::ConfigDir::from_env(&root)?;
    let cfg = config.load()?;
    let settings = AiSettings::from_insurer_yaml(&config.insurer_text()?)?;
    let vocab = Vocabulary::from_config(&cfg, config.declared_routes()?);
    let languages = Languages::load(&config.language_files()?, &cfg)?;
    let live = std::env::var("EVAL_LIVE").is_ok_and(|v| v == "1");
    let inner = if live {
        AnyModel::Live(AnthropicClient::from_env(settings.clone()).map_err(|e| e.to_string())?)
    } else {
        AnyModel::Replay(ReplayModel::load(
            &root.join("fixtures").join("ai_replies"),
        )?)
    };
    let h = Harness::new(
        Recording {
            inner,
            seen: Mutex::new(Seen::default()),
        },
        Prompts::in_repo(&root),
        settings,
        languages,
    );
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let case: Case = serde_json::from_str(&line).map_err(|e| format!("case: {e}"))?;
        if let Ok(mut s) = h.model.seen.lock() {
            *s = Seen::default();
        }
        let result = run_case(&h, &case, &cfg, &vocab, &root).await;
        let seen = h
            .model
            .seen
            .lock()
            .map(|mut s| std::mem::take(&mut *s))
            .unwrap_or_default();
        let mut line = json!({
            "case": case.case,
            "component": case.component,
            "request": { "user": seen.user, "key": seen.key },
        });
        match result {
            Ok(out) => line["result"] = out,
            Err(e) => line["error"] = json!(e),
        }
        if live {
            line["reply"] = json!(seen.reply);
        }
        writeln!(stdout, "{line}").map_err(|e| e.to_string())?;
    }
    Ok(())
}
