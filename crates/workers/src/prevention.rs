//! `RefreshPreventionTasks` and the evidence check (ADR-009): glue between the AI layer and the
//! engine. The AI proposes a task for a registered measure and gives a verdict on the evidence;
//! the engine sizes the reward (R40), admits the task (R18), samples passes for audit (R14) and
//! credits points and the reward (R10). Nothing here decides an amount.
use chrono::NaiveDate;
use microinsurance_ai::client::Model;
use microinsurance_ai::harness::{Harness, Source};
use microinsurance_ai::language::Languages;
use microinsurance_ai::prompts::Prompts;
use microinsurance_ai::replay::ReplayModel;
use microinsurance_ai::settings::AiSettings;
use microinsurance_ai::verify::{self, Evidence, VerifyOutcome};
use microinsurance_ai::vocab::Vocabulary;
use microinsurance_config::ConfigDir;
use microinsurance_engine::config::EngineConfig;
use microinsurance_engine::pilot::Cohort;
use microinsurance_engine::prevention::{
    self, Completion, HeldCover, PreventionBasis, PreventionTask, Review,
};
use microinsurance_engine::score::VerifiedActivity;
use microinsurance_engine::types::{PolicyId, Profile, UpcomingEvent};
use std::collections::BTreeSet;
use std::path::Path;

/// The AI layer on saved replies (`fixtures/ai_replies`), for demo mode and tests: it never
/// calls the network, and an input with no saved reply gets the saved answer (R22).
pub fn replay_ai(
    root: &Path,
    config: &ConfigDir,
    cfg: &EngineConfig,
) -> Result<(Harness<ReplayModel>, Vocabulary), String> {
    let settings = AiSettings::from_insurer_yaml(&config.insurer_text()?)?;
    let languages = Languages::load(&config.language_files()?, cfg)?;
    let model = ReplayModel::load(&root.join("fixtures").join("ai_replies"))?;
    let vocab = Vocabulary::from_config(cfg, config.declared_routes()?);
    let harness = Harness::new(model, Prompts::in_repo(root), settings, languages);
    Ok((harness, vocab))
}

/// What the prevention jobs need, for one run.
pub struct PreventionDesk<'a, M: Model> {
    pub ai: &'a Harness<M>,
    pub vocab: &'a Vocabulary,
    pub cfg: &'a EngineConfig,
    /// Claims data of each measure's class for the policy period (`python/economics`).
    pub basis: &'a PreventionBasis,
    /// The audit seed: the insurer's secret, recorded with each sample (R14).
    pub seed: u64,
}

/// One customer's upcoming event and the cover it could protect.
pub struct TaskRequest<'a> {
    pub profile: &'a Profile,
    pub policy_id: &'a PolicyId,
    pub event: &'a UpcomingEvent,
    pub held: HeldCover<'a>,
    pub offered_on: NaiveDate,
    /// The customer's verified activities, for each measure's repeat window (R10).
    pub history: &'a [VerifiedActivity],
    /// Their group in a pilot (`pilot::cohort`, R59).
    pub cohort: Cohort,
}

/// Why no task was offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoTask {
    /// The AI layer proposed none (no fitting measure, or its saved answer).
    NoProposal {
        source: Source,
        reason: Option<String>,
    },
    /// The engine refused it: the rule and why.
    Refused(String),
}

impl<M: Model> PreventionDesk<'_, M> {
    /// A task for the event, due when it starts, or why there is none.
    pub async fn propose(&self, req: &TaskRequest<'_>) -> Result<PreventionTask, NoTask> {
        // A customer on a pilot's wait list is not offered one, so the AI is not asked (R59).
        if req.cohort == Cohort::WaitList {
            return Err(NoTask::Refused(
                "R59: in the pilot's wait-list control group: no prevention tasks yet".into(),
            ));
        }
        let proposed = microinsurance_ai::prevention::propose(
            self.ai,
            req.event,
            req.held.cover_id,
            req.profile,
            self.cfg,
            self.vocab,
        )
        .await;
        let Some(proposal) = proposed.value else {
            return Err(NoTask::NoProposal {
                source: proposed.source,
                reason: proposed.reason,
            });
        };
        let refused = |e: microinsurance_engine::EngineError| NoTask::Refused(e.to_string());
        let draft = prevention::size(
            &proposal,
            req.held,
            req.event.reference.as_deref(),
            req.profile,
            self.basis,
            self.cfg,
        )
        .map_err(refused)?;
        prevention::offer(
            draft,
            &req.profile.customer_id,
            req.policy_id,
            req.offered_on,
            req.event.starts,
            req.history,
            req.cohort,
            self.cfg,
        )
        .map_err(refused)
    }

    /// Takes the customer's evidence on an accepted task and records the verifier's review. A
    /// verdict the verifier could not give goes to a person (R22); passes are sampled for audit
    /// before they are credited (R14). `seen` holds every evidence fingerprint already used.
    pub async fn check(
        &self,
        task: PreventionTask,
        evidence: &[Evidence],
        seen: &BTreeSet<String>,
        language: Option<&str>,
        on: NaiveDate,
    ) -> Result<(PreventionTask, Option<Completion>), String> {
        let prints: Vec<String> = evidence
            .iter()
            .map(|e| verify::fingerprint(&e.bytes))
            .collect();
        let task = prevention::submit(task, on, prints).map_err(|e| e.to_string())?;
        let outcome = verify::verdict(self.ai, &task.draft, evidence, seen, language).await;
        let review = match outcome.value {
            VerifyOutcome::Verdict(v) => Review::Verdict(v),
            VerifyOutcome::ReferToPerson { .. } => Review::NoVerdict,
        };
        prevention::review(task, review, None, on, self.seed, self.cfg).map_err(|e| e.to_string())
    }
}
