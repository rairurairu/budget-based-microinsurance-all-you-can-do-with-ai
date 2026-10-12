//! M3 Micro-cover catalogue: what may be offered (R24), when new cover is refused (R06), waiting
//! periods (R07, R47), the parametric payout ceiling (R09) and zone accumulation (R51).
use crate::config::EngineConfig;
use crate::money::Money;
use crate::optimiser::underwriting::ClassStatus;
use crate::triggers::{self, Occurrence, TriggerZone};
use crate::types::{CoverSpec, LaunchPhase, Profile};
use crate::{EngineError, Result};
use chrono::{Days, NaiveDate};
use std::collections::BTreeSet;

/// Covers offered in this launch phase and approved by the carrier (R24), ordered by id.
pub fn enabled(cfg: &EngineConfig, phase: u8) -> Vec<&CoverSpec> {
    cfg.catalogue
        .values()
        .filter(|c| matches!(c.launch_phase, LaunchPhase::Phase(p) if p <= phase))
        .filter(|c| cfg.approved_products.contains(&c.id))
        .collect()
}

/// R24: only pre-approved products of the licensed insurer are bundled.
pub fn check_bundled(cover_id: &str, cfg: &EngineConfig) -> Result<()> {
    if cfg.approved_products.contains(cover_id) {
        Ok(())
    } else {
        Err(EngineError::Rule {
            rule: "R24",
            detail: format!("cover {cover_id} is not an approved product of the insurer"),
        })
    }
}

/// R06: refuses new or increased cover while an event type and stage the cover lists (a
/// tropical cyclone signal from Signal 1) is in force anywhere.
pub fn check_new_cover(cover: &CoverSpec, in_force: &BTreeSet<Occurrence>) -> Result<()> {
    match in_force
        .iter()
        .find(|o| cover.block_new_while.iter().any(|c| triggers::meets(o, c)))
    {
        Some(o) => Err(EngineError::Rule {
            rule: "R06",
            detail: format!("no new {} while {} is in force", cover.id, o.event_type),
        }),
        None => Ok(()),
    }
}

/// True while the loss date is inside the cover's waiting period (R07: 14 days for devices).
pub fn in_waiting_period(cover: &CoverSpec, cover_start: NaiveDate, loss_date: NaiveDate) -> bool {
    (loss_date - cover_start).num_days() < i64::from(cover.waiting_period_days)
}

/// The first day a loss is covered: after the cover's own waiting period (R07) and, after a
/// postponement, the re-entry waiting period (R47).
pub fn covered_from(cover: &CoverSpec, cover_start: NaiveDate, status: &ClassStatus) -> NaiveDate {
    let own = cover_start
        .checked_add_days(Days::new(u64::from(cover.waiting_period_days)))
        .unwrap_or(NaiveDate::MAX);
    match status {
        ClassStatus::Open {
            reentry_waiting_until: Some(until),
        } => own.max(*until),
        _ => own,
    }
}

/// R09: a parametric benefit amount stays below typical daily earnings. `None` when the cover
/// has no earnings cap; zero when it has one but the customer has not declared earnings, so the
/// cover cannot be sized for them.
pub fn max_parametric_payout(cover: &CoverSpec, profile: &Profile) -> Option<Money> {
    let ratio = cover.payout.as_ref()?.cap_ratio_of_daily_earnings?;
    Some(
        profile
            .typical_daily_earnings
            .map_or(Money::ZERO, |e| e.mul_bps(ratio).floor_major()),
    )
}

/// R51: no new or increased premium into a trigger zone whose benefit in force exceeds
/// `zone_accumulation_limit` (`triggers::full_zones`). Existing cover is unaffected.
pub fn check_accumulation(
    cover: &CoverSpec,
    profile: &Profile,
    full: &BTreeSet<TriggerZone>,
    cfg: &EngineConfig,
) -> Result<()> {
    match triggers::zones_of(
        cover,
        &profile.commute_routes,
        &profile.work_areas,
        &profile.upcoming_events,
        cfg,
    )
    .into_iter()
    .find(|z| full.contains(z))
    {
        Some((event_type, zone)) => Err(EngineError::Rule {
            rule: "R51",
            detail: format!(
                "no new {} while benefit in force for {event_type} in {} exceeds the zone \
                 accumulation limit",
                cover.id,
                zone.as_deref().unwrap_or("the territory")
            ),
        }),
        None => Ok(()),
    }
}
