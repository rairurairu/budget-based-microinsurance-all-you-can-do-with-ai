use serde::{Deserialize, Serialize};
use std::iter::Sum;
use std::ops::{Add, AddAssign, Mul, Sub};

/// Money in minor units (cents). No floating point in money paths (R29).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Money(pub i64);

const MINOR_PER_MAJOR: i64 = 100;

impl Money {
    pub const ZERO: Money = Money(0);

    pub fn from_major(units: i64) -> Money {
        Money(units * MINOR_PER_MAJOR)
    }

    /// Multiply by a ratio, rounding half-up, away from zero (ADR-004).
    pub fn mul_bps(self, ratio: Bps) -> Money {
        Money(div_round_half_up(
            i128::from(self.0) * i128::from(ratio.0),
            i128::from(Bps::ONE.0),
        ))
    }

    /// Rounded down to whole major units. Sums insured are shown this way, so cover is never more
    /// than the premium buys (ADR-004).
    pub fn floor_major(self) -> Money {
        self.floor_to(Money(MINOR_PER_MAJOR))
    }

    /// The largest multiple of `step` not above this amount. `step` must be positive.
    pub fn floor_to(self, step: Money) -> Money {
        if step.0 <= 0 {
            return self;
        }
        Money(self.0.div_euclid(step.0) * step.0)
    }

    pub fn is_multiple_of(self, step: Money) -> bool {
        step.0 > 0 && self.0.rem_euclid(step.0) == 0
    }

    pub fn min(self, other: Money) -> Money {
        if self <= other { self } else { other }
    }

    pub fn max(self, other: Money) -> Money {
        if self >= other { self } else { other }
    }
}

impl Add for Money {
    type Output = Money;
    fn add(self, other: Money) -> Money {
        Money(self.0 + other.0)
    }
}

impl AddAssign for Money {
    fn add_assign(&mut self, other: Money) {
        self.0 += other.0;
    }
}

impl Sub for Money {
    type Output = Money;
    fn sub(self, other: Money) -> Money {
        Money(self.0 - other.0)
    }
}

impl Sum for Money {
    fn sum<I: Iterator<Item = Money>>(iter: I) -> Money {
        iter.fold(Money::ZERO, Add::add)
    }
}

/// A ratio in basis points: 10,000 = 100%.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Bps(pub u32);

impl Bps {
    pub const ZERO: Bps = Bps(0);
    pub const ONE: Bps = Bps(10_000);

    /// `numerator / denominator` as a ratio, rounded half-up; zero when the denominator is zero.
    pub fn ratio(numerator: u64, denominator: u64) -> Bps {
        if denominator == 0 {
            return Bps::ZERO;
        }
        Bps::saturating(div_round_half_up(
            i128::from(numerator) * i128::from(Bps::ONE.0),
            i128::from(denominator),
        ))
    }

    pub(crate) fn saturating(v: i64) -> Bps {
        Bps(u32::try_from(v.max(0)).unwrap_or(u32::MAX))
    }
}

/// Product of two ratios, rounded half-up. Saturates instead of overflowing.
impl Mul for Bps {
    type Output = Bps;
    fn mul(self, other: Bps) -> Bps {
        Bps::saturating(div_round_half_up(
            i128::from(self.0) * i128::from(other.0),
            i128::from(Bps::ONE.0),
        ))
    }
}

/// Premium per unit of sum insured, in parts per million: 1,000,000 = premium equals the sum
/// insured. Finer than `Bps` because micro-cover rates are small (ADR-004).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Rate(pub u32);

impl Rate {
    pub const PER_UNIT: u32 = 1_000_000;

    /// Scaled by a ratio, rounded half-up, never below 1 ppm so a sum insured is always finite.
    pub fn mul_bps(self, ratio: Bps) -> Rate {
        let v = div_round_half_up(
            i128::from(self.0) * i128::from(ratio.0),
            i128::from(Bps::ONE.0),
        );
        Rate(u32::try_from(v.max(1)).unwrap_or(u32::MAX))
    }

    /// Divided by a ratio (for example the target loss ratio), rounded half-up, never below 1 ppm.
    pub fn div_bps(self, ratio: Bps) -> Rate {
        if ratio.0 == 0 {
            return Rate(u32::MAX);
        }
        let v = div_round_half_up(
            i128::from(self.0) * i128::from(Bps::ONE.0),
            i128::from(ratio.0),
        );
        Rate(u32::try_from(v.max(1)).unwrap_or(u32::MAX))
    }

    /// The sum insured a premium buys at this rate, rounded down to whole major units.
    pub fn sum_insured(self, premium: Money) -> Money {
        if self.0 == 0 {
            return Money::ZERO;
        }
        let si = i128::from(premium.0) * i128::from(Rate::PER_UNIT) / i128::from(self.0);
        Money(i64::try_from(si).unwrap_or(i64::MAX)).floor_major()
    }

    /// The most premium whose sum insured stays within `limit`, rounded down to `step`.
    pub fn max_premium(self, limit: Money, step: Money) -> Money {
        let p = i128::from(limit.0) * i128::from(self.0) / i128::from(Rate::PER_UNIT);
        Money(i64::try_from(p).unwrap_or(i64::MAX)).floor_to(step)
    }
}

/// `n / d` rounded half-up, away from zero; `d` must be positive. Saturates at the i64 range.
fn div_round_half_up(n: i128, d: i128) -> i64 {
    let q = (n.abs() + d / 2) / d;
    let q = if n < 0 { -q } else { q };
    i64::try_from(q).unwrap_or(if q < 0 { i64::MIN } else { i64::MAX })
}
