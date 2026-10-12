//! R46: an urgent case (injury, hospitalisation, an emergency abroad, or a loss large against the
//! customer's income) gets the configured first response at once, computed when the case is
//! notified and before anyone is asked; then a person is offered and the customer chooses. A
//! customer who needs human care, or asks for a person, gets one. The AI layer can only add an
//! escalation (`Escalation`): `triage` combines it with the rules by "or", so nothing the AI
//! says ever makes a case less urgent or takes a person away.
use crate::config::GuardrailConfig;
use crate::money::{Bps, Money};
use crate::types::Profile;
use crate::{EngineError, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// What the first response does (`guardrails.human_care.first_response`). A closed set: each is
/// carried out by the assistance channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum FirstAction {
    /// Local emergency numbers, shown at once.
    EmergencyNumbers,
    /// A call or chat with the assistance provider, opened at once.
    AssistanceProviderConnect,
    /// A guarantee of payment to the hospital or provider, automatic up to the cap.
    GuaranteeOfPayment,
}

/// Why a case is urgent.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "because", content = "circumstance", rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum UrgentBecause {
    /// The customer reported a configured urgent circumstance, such as `injury`.
    Circumstance(String),
    /// The loss is at least `large_loss_share_of_monthly_income` of the customer's monthly
    /// earnings.
    LargeLoss,
    /// The AI layer marked it urgent.
    AiEscalated,
}

/// What the AI layer may add to a case. It can raise, never lower: both flags only ever add to
/// what the rules found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Escalation {
    pub urgent: bool,
    /// Signs of distress or vulnerability: a person takes the case.
    pub human_care: bool,
}

/// The case as notified.
#[derive(Debug, Clone)]
pub struct CaseFacts<'a> {
    pub notified_at: DateTime<Utc>,
    /// One of `human_care.urgent`, when the customer reported one.
    pub circumstance: Option<&'a str>,
    pub amount: Money,
    /// The customer's monthly earnings, when known (`monthly_earnings`).
    pub monthly_earnings: Option<Money>,
    /// The customer asked for a person.
    pub asked_for_person: bool,
    /// The guarantee of payment the hospital or provider asks for; zero when none.
    pub guarantee_requested: Money,
    /// Cover in that class is in force for the loss (not lapsed, past any waiting period).
    pub cover_active: bool,
}

/// A guarantee of payment: automatic up to `auto_guarantee_cap`, the rest only once the
/// assistance provider confirms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Guarantee {
    pub automatic: Money,
    pub to_confirm: Money,
}

/// The automatic first response to an urgent case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct FirstResponse {
    /// Sent by then: `urgent_response_seconds` after notification.
    pub respond_by: DateTime<Utc>,
    pub actions: Vec<FirstAction>,
    /// None when no guarantee was asked for, or no cover in the class is active.
    pub guarantee: Option<Guarantee>,
}

/// Whether and how a person comes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum PersonRoute {
    /// The usual path; the customer can still ask for a person at any time.
    NotNeeded,
    /// After the first response a person is offered; the customer chooses.
    Offered,
    /// A person takes the case: the customer asked, or needs human care.
    Assigned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Triage {
    /// Empty when the case is not urgent.
    pub urgent: BTreeSet<UrgentBecause>,
    pub first_response: Option<FirstResponse>,
    pub person: PersonRoute,
}

impl Triage {
    pub fn is_urgent(&self) -> bool {
        !self.urgent.is_empty()
    }
}

/// Monthly earnings from the declared daily earnings and working days: the distinct weekdays
/// worked × 52 weeks ÷ 12 months, rounded down. None without both.
pub fn monthly_earnings(profile: &Profile) -> Option<Money> {
    let daily = profile.typical_daily_earnings?;
    let days: BTreeSet<u8> = profile.work_windows.iter().map(|w| w.weekday).collect();
    if days.is_empty() {
        return None;
    }
    let len = i64::try_from(days.len()).unwrap_or(0);
    Some(Money(daily.0 * len * 52 / 12))
}

/// The rules' view of a case, raised by the AI's escalation, if any (R46).
pub fn triage(
    facts: &CaseFacts<'_>,
    ai: Option<Escalation>,
    cfg: &GuardrailConfig,
) -> Result<Triage> {
    let hc = &cfg.human_care;
    let mut urgent = BTreeSet::new();
    if let Some(c) = facts.circumstance {
        if !hc.urgent.contains(c) {
            return Err(EngineError::Rule {
                rule: "R46",
                detail: format!("{c} is not a configured urgent circumstance"),
            });
        }
        urgent.insert(UrgentBecause::Circumstance(c.to_string()));
    }
    if facts.monthly_earnings.is_some_and(|m| {
        m > Money::ZERO
            && i128::from(facts.amount.0) * i128::from(Bps::ONE.0)
                >= i128::from(m.0) * i128::from(hc.large_loss_share_of_monthly_income.0)
    }) {
        urgent.insert(UrgentBecause::LargeLoss);
    }
    let ai = ai.unwrap_or_default();
    if ai.urgent {
        urgent.insert(UrgentBecause::AiEscalated);
    }
    let first_response = (!urgent.is_empty()).then(|| {
        let guarantee = (hc.first_response.contains(&FirstAction::GuaranteeOfPayment)
            && facts.cover_active
            && facts.guarantee_requested > Money::ZERO)
            .then(|| {
                let automatic = facts.guarantee_requested.min(hc.auto_guarantee_cap);
                Guarantee {
                    automatic,
                    to_confirm: facts.guarantee_requested - automatic,
                }
            });
        FirstResponse {
            respond_by: facts.notified_at
                + Duration::seconds(i64::from(hc.urgent_response_seconds)),
            actions: hc.first_response.clone(),
            guarantee,
        }
    });
    let person = if facts.asked_for_person || ai.human_care {
        PersonRoute::Assigned
    } else if first_response.is_some() {
        PersonRoute::Offered
    } else {
        PersonRoute::NotNeeded
    };
    Ok(Triage {
        urgent,
        first_response,
        person,
    })
}
