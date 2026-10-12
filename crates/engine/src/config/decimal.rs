//! Exact decimals in config (R29, ADR-004). `0.55` becomes `Bps(5500)` with integer arithmetic;
//! a value with more decimal places than the unit holds is refused, never rounded.
use crate::money::{Bps, Money};
use serde::de::{self, Deserialize, Deserializer, Visitor};
use std::fmt;

/// A ratio written as a decimal fraction (`0.55` = 55%).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DecBps(pub Bps);

/// An amount written in major units (`40` = HK$40.00).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DecMoney(pub Money);

impl<'de> Deserialize<'de> for DecBps {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = d.deserialize_any(Scaled { places: 4 })?;
        u32::try_from(v)
            .map(|v| DecBps(Bps(v)))
            .map_err(|_| de::Error::custom(format!("ratio out of range: {v} bps")))
    }
}

impl<'de> Deserialize<'de> for DecMoney {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(Scaled { places: 2 })
            .map(|v| DecMoney(Money(v)))
    }
}

struct Scaled {
    places: u32,
}

impl Scaled {
    fn int<E: de::Error>(&self, v: i64) -> Result<i64, E> {
        10_i64
            .checked_pow(self.places)
            .and_then(|scale| v.checked_mul(scale))
            .ok_or_else(|| E::custom(format!("{v} is out of range")))
    }
}

impl Visitor<'_> for Scaled {
    type Value = i64;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "a decimal with at most {} decimal places", self.places)
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<i64, E> {
        self.int(v)
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<i64, E> {
        let v = i64::try_from(v).map_err(|_| E::custom(format!("{v} is out of range")))?;
        self.int(v)
    }

    /// The parser hands over a binary float; its shortest round-trip text is the literal as
    /// written, which is then parsed exactly.
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<i64, E> {
        parse_scaled(&format!("{v}"), self.places).map_err(E::custom)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<i64, E> {
        parse_scaled(v, self.places).map_err(E::custom)
    }
}

/// Parses a plain decimal ("12", "0.55", "-3.5") into an integer scaled by 10^places.
pub(crate) fn parse_scaled(text: &str, places: u32) -> Result<i64, String> {
    let t = text.trim();
    let (negative, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (int_part, frac_part) = digits.split_once('.').unwrap_or((digits, ""));
    let frac = frac_part.trim_end_matches('0');
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if (int_part.is_empty() && frac_part.is_empty()) || !all_digits(int_part) || !all_digits(frac) {
        return Err(format!("{t:?} is not a plain decimal"));
    }
    let places_usize = usize::try_from(places).unwrap_or(usize::MAX);
    if frac.len() > places_usize {
        return Err(format!(
            "{t} has more than {places} decimal places (R29: config decimals are exact)"
        ));
    }
    let overflow = || format!("{t} is out of range");
    let mut v: i64 = 0;
    let frac_digits = frac
        .bytes()
        .chain(std::iter::repeat(b'0'))
        .take(places_usize);
    for b in int_part.bytes().chain(frac_digits) {
        v = v
            .checked_mul(10)
            .and_then(|v| v.checked_add(i64::from(b - b'0')))
            .ok_or_else(overflow)?;
    }
    Ok(if negative { -v } else { v })
}

#[cfg(test)]
mod tests {
    use super::parse_scaled;

    #[test]
    fn decimals_parse_exactly() {
        assert_eq!(parse_scaled("0.55", 4), Ok(5_500));
        assert_eq!(parse_scaled("40", 2), Ok(4_000));
        assert_eq!(parse_scaled("12.5", 2), Ok(1_250));
        assert_eq!(parse_scaled("0.1200", 2), Ok(12));
        assert_eq!(parse_scaled("-3.5", 1), Ok(-35));
        assert_eq!(parse_scaled(".5", 4), Ok(5_000));
    }

    #[test]
    fn too_many_places_or_junk_is_refused() {
        assert!(parse_scaled("0.12345", 4).is_err());
        assert!(parse_scaled("1e3", 2).is_err());
        assert!(parse_scaled("", 2).is_err());
        assert!(parse_scaled("1.2.3", 2).is_err());
    }
}
