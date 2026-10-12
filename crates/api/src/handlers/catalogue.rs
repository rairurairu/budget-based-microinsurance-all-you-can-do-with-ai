//! Public routes: what anyone may read before becoming a customer. The plans are listed the same
//! for everyone, in configured order, and none is recommended (R01, R45).
use crate::bodies::{ChannelInfo, CoverInfo, RatingQuestion, Registry};
use crate::copy::CopyPack;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use microinsurance_engine::config::{FactorData, FactorLevel, FactorStatus, LicenceType};
use microinsurance_engine::consent;
use microinsurance_engine::plans::{self, PlanPrice, PurchaseContext};
use microinsurance_engine::types::PayoutBasis;
use serde::Deserialize;
use std::collections::BTreeSet;

pub async fn health() -> &'static str {
    "ok"
}

#[derive(Debug, Deserialize)]
pub struct ChannelQuery {
    pub channel: Option<String>,
}

/// Every plan with what a new customer pays for it today on `channel` (R01, R44, R45).
pub async fn plans(
    State(state): State<AppState>,
    Query(q): Query<ChannelQuery>,
) -> ApiResult<Vec<PlanPrice>> {
    let who = PurchaseContext {
        channel: q.channel,
        new_customer: true,
    };
    let today = state.today();
    Ok(Json(
        plans::available_plans(&state.cfg)
            .iter()
            .map(|p| plans::price(p, &who, today, &state.cfg))
            .collect(),
    ))
}

/// The covers any plan can include today, with their key features (R23, R24).
pub async fn covers(State(state): State<AppState>) -> ApiResult<Vec<CoverInfo>> {
    let ids: BTreeSet<String> = plans::available_plans(&state.cfg)
        .iter()
        .flat_map(|p| plans::covers_for(p, &state.cfg))
        .collect();
    let mut out = Vec::new();
    for id in ids {
        let c = state.cfg.cover(&id)?;
        out.push(CoverInfo {
            cover_id: c.id.clone(),
            name: c.name.clone(),
            cover_type: c.cover_type,
            class_of_business: c.class_of_business.clone(),
            key_features: c.key_features.clone(),
            exposure: c.exposure.clone(),
            waiting_period_days: c.waiting_period_days,
            product_limit: c.product_limit,
            daily_benefit: c
                .payout
                .as_ref()
                .is_some_and(|p| p.basis == PayoutBasis::FixedDaily),
        });
    }
    Ok(Json(out))
}

/// R52: whether a partner's channel binds cover today, or refers to the insurer's own journey.
pub async fn channel(
    State(state): State<AppState>,
    Path(partner_id): Path<String>,
) -> ApiResult<ChannelInfo> {
    let partner = state
        .cfg
        .distribution_partners
        .get(&partner_id)
        .ok_or_else(|| ApiError::not_found(format!("{partner_id} is not a recorded partner")))?;
    let binds =
        consent::check_binding_partner(Some(&partner.id), state.today(), &state.cfg).is_ok();
    Ok(Json(ChannelInfo {
        partner_id,
        binds,
        refer_to: (!binds || partner.licence_type == LicenceType::ReferralOnly)
            .then(|| state.channels.own_journey_url.clone()),
    }))
}

/// The registered factors a customer may answer for the covers on offer, with the answers each
/// factor's filed table rates (R33). Protected attributes appear only where registered (R19).
pub async fn rating_questions(State(state): State<AppState>) -> ApiResult<Vec<RatingQuestion>> {
    let classes: BTreeSet<String> = plans::available_plans(&state.cfg)
        .iter()
        .flat_map(|p| plans::covers_for(p, &state.cfg))
        .filter_map(|id| state.cfg.cover(&id).ok())
        .map(|c| c.class_of_business.clone())
        .collect();
    let mut out = Vec::new();
    for f in state.cfg.rating_factors.values() {
        let asked = f.level == FactorLevel::Individual
            && f.status == FactorStatus::Approved
            && f.data == Some(FactorData::Declared)
            && classes.iter().any(|c| f.rates(c));
        if !asked {
            continue;
        }
        let answers: BTreeSet<String> = state
            .rating
            .factor_tables
            .get(&f.id)
            .into_iter()
            .flat_map(|tables| tables.iter())
            .filter(|(class, _)| {
                classes
                    .iter()
                    .any(|c| microinsurance_engine::config::LinesOfBusiness::contains(class, c))
            })
            .flat_map(|(_, t)| t.relativities.keys().cloned())
            .collect();
        if !answers.is_empty() {
            out.push(RatingQuestion {
                factor_id: f.id.clone(),
                formula: f.formula.clone(),
                answers: answers.into_iter().collect(),
            });
        }
    }
    Ok(Json(out))
}

/// The app copy in a language: that language when offered, otherwise the default.
pub async fn copy(
    State(state): State<AppState>,
    Path(language): Path<String>,
) -> ApiResult<CopyPack> {
    state
        .copy
        .pack(Some(&language))
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::not_found("no copy"))
}

/// The registries the apps build their forms from (R20, R55).
pub async fn registry(State(state): State<AppState>) -> ApiResult<Registry> {
    let cfg = &state.cfg;
    let v = &state.vocab;
    Ok(Json(Registry {
        currency: cfg.currency.clone(),
        utc_offset_minutes: cfg.utc_offset_minutes,
        default_language: cfg.languages.default.clone(),
        languages: cfg.languages.offered.iter().cloned().collect(),
        data_types: cfg.data_types.data_types.iter().cloned().collect(),
        evidence_kinds: cfg.data_types.evidence_kinds.keys().cloned().collect(),
        upcoming_event_kinds: v.upcoming_event_kinds.iter().cloned().collect(),
        declared_item_kinds: v.declared_item_kinds.iter().cloned().collect(),
        routes: v.routes.iter().cloned().collect(),
        areas: state.areas.iter().cloned().collect(),
        allocation_step: cfg.policy_period.allocation_step,
        urgent_circumstances: cfg.guardrails.human_care.urgent.iter().cloned().collect(),
    }))
}
