//! M1 Pick-your-price plans. The plan price is the same for everyone and changes only by the
//! customer's choice (R01, R45) or a promotional discount (R44).
use crate::Result;
use crate::config::{EngineConfig, Promotion};
use crate::consent::{self, CustomerTap, Endorsement, Offer, OfferKind};
use crate::money::Money;
use crate::rewards::Funder;
use crate::types::{Plan, PlanId, Policy};
use crate::{EngineError, catalogue};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// The configured plans: the same for every customer, never personalised or recommended (R01,
/// R45). The caller shows all of them in configured order.
pub fn available_plans(cfg: &EngineConfig) -> &[Plan] {
    &cfg.plans
}

/// Who is buying, for promotion eligibility only. Deliberately holds no risk class and no
/// protected attribute (R44).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PurchaseContext {
    pub channel: Option<String>,
    pub new_customer: bool,
}

/// What the customer pays for a plan on a day. The plan price itself, which is allocated across
/// covers, stays at `list_price`; the discount is paid by its funder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PlanPrice {
    pub plan_id: PlanId,
    pub list_price: Money,
    pub promotion_id: Option<String>,
    pub discount: Money,
    pub funded_by: Option<Funder>,
    pub payable: Money,
}

fn promotion_applies(p: &Promotion, plan: &Plan, who: &PurchaseContext, on: NaiveDate) -> bool {
    p.starts <= on
        && on <= p.ends
        && p.plans.contains(&plan.id)
        && (!p.new_customers_only || who.new_customer)
        && p.channel
            .as_ref()
            .is_none_or(|c| who.channel.as_ref() == Some(c))
}

/// R44: every eligible customer gets the same discount, deterministically. Where campaigns
/// overlap, the largest discount applies; ties go to the lowest promotion id.
pub fn price(plan: &Plan, who: &PurchaseContext, on: NaiveDate, cfg: &EngineConfig) -> PlanPrice {
    let best = cfg
        .promotions
        .iter()
        .filter(|p| promotion_applies(p, plan, who, on))
        .min_by(|a, b| b.discount.cmp(&a.discount).then_with(|| a.id.cmp(&b.id)));
    let discount = best.map_or(Money::ZERO, |p| plan.price.mul_bps(p.discount));
    PlanPrice {
        plan_id: plan.id.clone(),
        list_price: plan.price,
        promotion_id: best.map(|p| p.id.clone()),
        discount,
        funded_by: best.map(|p| p.funded_by),
        payable: plan.price - discount,
    }
}

/// The covers the optimiser may use for a plan: the launch-phase catalogue plus the plan's
/// privileges, all approved by the carrier (R24, R45).
pub fn covers_for(plan: &Plan, cfg: &EngineConfig) -> Vec<String> {
    let mut covers: Vec<String> = catalogue::enabled(cfg, cfg.launch_phase)
        .into_iter()
        .map(|c| c.id.clone())
        .collect();
    for extra in &plan.privileges.extra_covers {
        if cfg.approved_products.contains(extra) && !covers.contains(extra) {
            covers.push(extra.clone());
        }
    }
    covers.sort();
    covers
}

/// Needs the customer's tap (R03), applies next policy period (R04), respects the minimum term
/// (R05). The offer must be a plan change the customer asked for; the system never proposes a
/// plan (R45).
pub fn change_plan(
    policy: &Policy,
    offer: &Offer,
    tap: &CustomerTap,
    cfg: &EngineConfig,
) -> Result<Endorsement> {
    let OfferKind::PlanChange { plan } = &offer.change else {
        return Err(EngineError::Rule {
            rule: "R03",
            detail: format!("offer {} is not a plan change", offer.offer_id),
        });
    };
    if cfg.plan(plan).is_none() {
        return Err(EngineError::Rule {
            rule: "R01",
            detail: format!("plan {plan} is not a configured plan"),
        });
    }
    let subscribed = policy.subscribed_at.date_naive();
    if !consent::served_min_periods(subscribed, policy.period.ends, cfg) {
        return Err(EngineError::Rule {
            rule: "R05",
            detail: format!(
                "the plan can change once {} policy periods are served",
                cfg.policy_period.min_policy_periods
            ),
        });
    }
    consent::accept(offer, tap, policy, cfg)
}
