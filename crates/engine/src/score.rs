//! Protection Score. Earned only from verified prevention activities (R10); gives a premium
//! rebate or a bonus sum insured (customer chooses), never worse terms and never a different plan
//! price, capped per points pool (R11, R36); points fade (R12) but not while lapsed (R53). The
//! rebate is a product rule, identical for everyone who earns it (R50).
use crate::config::{EngineConfig, ScoreConfig};
use crate::money::{Bps, Money};
use crate::types::{ClassId, PoolId, Schedule};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ScoreMode {
    /// Credited after the policy period; the plan price is unchanged.
    PremiumRebate,
    /// More cover at the same price.
    BonusSumInsured,
}

/// A prevention activity that passed verification. Constructed only by
/// `prevention::complete`, so self-reported activity can never earn points (R10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedActivity {
    on: NaiveDate,
    points: u32,
    /// The class of business of the cover it protects (R36).
    class_of_business: ClassId,
    /// The prevention measure verified, for its repeat window (R10).
    task_type: String,
}

impl VerifiedActivity {
    pub(crate) fn new(
        on: NaiveDate,
        points: u32,
        class_of_business: ClassId,
        task_type: String,
    ) -> VerifiedActivity {
        VerifiedActivity {
            on,
            points,
            class_of_business,
            task_type,
        }
    }

    pub fn task_type(&self) -> &str {
        &self.task_type
    }

    pub fn on(&self) -> NaiveDate {
        self.on
    }

    pub fn points(&self) -> u32 {
        self.points
    }

    pub fn class_of_business(&self) -> &str {
        &self.class_of_business
    }
}

/// A lapse: from its first day, until the day cover was reinstated (exclusive). Points do not
/// fade while lapsed (R53).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lapse {
    pub from: NaiveDate,
    pub until: NaiveDate,
}

/// 2^(−k/64) in 32-bit fixed point, k = 0..=64.
const HALVING: [u64; 65] = [
    4_294_967_296,
    4_248_701_965,
    4_202_935_003,
    4_157_661_043,
    4_112_874_773,
    4_068_570_940,
    4_024_744_348,
    3_981_389_855,
    3_938_502_376,
    3_896_076_880,
    3_854_108_391,
    3_812_591_987,
    3_771_522_796,
    3_730_896_002,
    3_690_706_840,
    3_650_950_594,
    3_611_622_603,
    3_572_718_252,
    3_534_232_978,
    3_496_162_267,
    3_458_501_653,
    3_421_246_719,
    3_384_393_094,
    3_347_936_457,
    3_311_872_529,
    3_276_197_082,
    3_240_905_930,
    3_205_994_934,
    3_171_459_999,
    3_137_297_074,
    3_103_502_151,
    3_070_071_267,
    3_037_000_500,
    3_004_285_971,
    2_971_923_842,
    2_939_910_317,
    2_908_241_642,
    2_876_914_102,
    2_845_924_021,
    2_815_267_765,
    2_784_941_738,
    2_754_942_382,
    2_725_266_179,
    2_695_909_648,
    2_666_869_345,
    2_638_141_863,
    2_609_723_834,
    2_581_611_923,
    2_553_802_834,
    2_526_293_303,
    2_499_080_105,
    2_472_160_047,
    2_445_529_972,
    2_419_186_755,
    2_393_127_307,
    2_367_348_571,
    2_341_847_524,
    2_316_621_173,
    2_291_666_561,
    2_266_980_759,
    2_242_560_872,
    2_218_404_036,
    2_194_507_417,
    2_170_868_212,
    2_147_483_648,
];

/// `points × 2^(−age/half_life)`, with integer arithmetic: whole half-lives by shifting, the
/// rest from the table, interpolated linearly.
fn decayed(points: u32, age_days: u64, half_life_days: u32) -> u32 {
    let hl = u64::from(half_life_days.max(1));
    let whole = age_days / hl;
    if whole >= 32 {
        return 0;
    }
    let pos = (age_days % hl) * 64;
    let (idx, frac) = (pos / hl, pos % hl);
    let idx = usize::try_from(idx).unwrap_or(63).min(63);
    let (a, b) = (HALVING[idx], HALVING[idx + 1]);
    let factor = a - (a - b) * frac / hl;
    let shift = 32 + whole;
    let v = (u64::from(points) * factor + (1 << (shift - 1))) >> shift;
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// Days of fading between `on` and `today`, leaving out lapses.
fn fading_days(on: NaiveDate, today: NaiveDate, lapses: &[Lapse]) -> u64 {
    let total = (today - on).num_days().max(0);
    let paused: i64 = lapses
        .iter()
        .map(|l| (l.until.min(today) - l.from.max(on)).num_days().max(0))
        .sum();
    u64::try_from((total - paused).max(0)).unwrap_or(0)
}

/// Decayed points with integer arithmetic (R12). Activities after `today` do not count yet.
pub fn points(
    history: &[VerifiedActivity],
    today: NaiveDate,
    half_life_days: u32,
    lapses: &[Lapse],
) -> u32 {
    history
        .iter()
        .filter(|a| a.on <= today)
        .map(|a| decayed(a.points, fading_days(a.on, today, lapses), half_life_days))
        .fold(0_u32, u32::saturating_add)
}

/// Points per points pool (R36): an activity counts only in the pool of the class it protects.
pub fn points_by_pool(
    history: &[VerifiedActivity],
    today: NaiveDate,
    lapses: &[Lapse],
    cfg: &EngineConfig,
) -> BTreeMap<PoolId, u32> {
    let mut by_pool: BTreeMap<PoolId, Vec<VerifiedActivity>> = BTreeMap::new();
    for a in history {
        by_pool
            .entry(cfg.lines.pool_of(&a.class_of_business))
            .or_default()
            .push(a.clone());
    }
    by_pool
        .into_iter()
        .map(|(pool, acts)| {
            let p = points(&acts, today, cfg.score.point_half_life_days, lapses);
            (pool, p)
        })
        .collect()
}

/// Monotone in points, within 0..=max_effect (R11).
pub fn effect(points: u32, cfg: &ScoreConfig) -> Bps {
    let full = u64::from(cfg.points_for_max_effect.max(1));
    let scaled =
        u64::from(cfg.max_effect.0) * u64::from(points.min(cfg.points_for_max_effect)) / full;
    Bps(u32::try_from(scaled)
        .unwrap_or(cfg.max_effect.0)
        .min(cfg.max_effect.0))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ScoreOutcome {
    /// Premiums are those of the rated schedule; sums insured include any bonus.
    pub schedule: Schedule,
    /// Credited after the policy period, funded by the insurer (R50).
    pub premium_rebate: Money,
}

/// Applies each pool's effect only to covers in that pool (R36), capped at `max_effect` (R11),
/// on top of the period's rated schedule. Terms are never worse than that base, and the plan
/// price is never changed.
pub fn apply(
    effects: &BTreeMap<PoolId, Bps>,
    mode: ScoreMode,
    schedule: &Schedule,
    cfg: &EngineConfig,
) -> ScoreOutcome {
    let mut out = schedule.clone();
    let mut premium_rebate = Money::ZERO;
    for a in &mut out.allocations {
        let Ok(cover) = cfg.cover(&a.cover_id) else {
            continue;
        };
        let pool = cfg.lines.pool_of(&cover.class_of_business);
        let eff = effects
            .get(&pool)
            .copied()
            .unwrap_or(Bps::ZERO)
            .min(cfg.score.max_effect);
        match mode {
            ScoreMode::PremiumRebate => premium_rebate += a.premium.mul_bps(eff),
            ScoreMode::BonusSumInsured => {
                let bonus = a.sum_insured.mul_bps(eff).floor_major();
                let limit = cover.product_limit.unwrap_or(Money(i64::MAX));
                a.sum_insured = (a.sum_insured + bonus).min(limit.max(a.sum_insured));
            }
        }
    }
    ScoreOutcome {
        schedule: out,
        premium_rebate,
    }
}
