//! Rescaling a tick count between two timescales (ticks per second).
//!
//! One home for `ticks * to / from` so the rounding mode is explicit at every
//! call site and the edge cases are decided once:
//!
//! - A timescale is a [`Timescale`], which is non-zero by construction, so a
//!   rescale never divides by zero and there is no "treat `0` as `1`" path to
//!   inherit: a zero timescale arriving from the wire (`mdhd`/`TimeScale`, an
//!   MPD `@timescale`, a manifest) is an error at the boundary that parses it
//!   ([`Timescale::try_new`]), never a silently wrong value here.
//! - The multiply saturates instead of wrapping, so an absurd tick count
//!   clamps rather than producing a small wrong value.
//! - The `_i64` / `_u64` forms saturate to the target type; `_u64` clamps a
//!   negative result to `0`.
//!
//! [`rescale`]: self::rescale

use crate::error::InvalidValue;

/// How a fractional result is brought to a whole tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Rounding {
    /// Toward negative infinity.
    Floor,
    /// To the nearest tick, halves away from zero.
    Nearest,
    /// Toward positive infinity.
    Ceil,
    /// Toward zero (plain integer division).
    TowardZero,
}

impl Rounding {
    /// Label of the rounding mode.
    pub fn name(&self) -> &'static str {
        match self {
            Rounding::Floor => "floor",
            Rounding::Ceil => "ceil",
            Rounding::Nearest => "nearest",
            Rounding::TowardZero => "toward zero",
        }
    }
}

crate::impl_spec_display!(Rounding);

/// A media timescale (ticks per second), guaranteed non-zero.
///
/// The wire can carry a zero timescale (a parsed `mdhd`/`TimeScale` field, an
/// MPD `@timescale`, a Smooth `StreamIndex@TimeScale`); such a field is an
/// error at the parse boundary ([`Self::try_new`]), not a value to flatten to
/// `1`. Every rescale below takes a `Timescale`, so a zero can neither reach
/// the arithmetic nor become a silently wrong result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timescale(pub core::num::NonZeroU32);

impl Timescale {
    /// Ticks per second of the microsecond timescale.
    pub const MICROS: Self = Self(match core::num::NonZeroU32::new(MICROS_PER_SECOND) {
        Some(nz) => nz,
        None => unreachable!(),
    });

    /// Construct from a raw value, returning `None` for zero.
    #[must_use]
    pub const fn new(value: u32) -> Option<Self> {
        match core::num::NonZeroU32::new(value) {
            Some(nz) => Some(Self(nz)),
            None => None,
        }
    }

    /// Fallible constructor; zero is an error.
    ///
    /// # Errors
    /// [`InvalidValue`] (`field: "timescale"`) when `value` is zero.
    pub fn try_new(value: u32) -> Result<Self, InvalidValue> {
        core::num::NonZeroU32::new(value)
            .map(Self)
            .ok_or(InvalidValue {
                field: "timescale",
                value: u64::from(value),
                reason: "must be non-zero",
            })
    }

    /// The same timescale as a `u64` (the wire/form fields that carry it
    /// wider than 32 bits, e.g. an MPD `@timescale`).
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0.get() as u64
    }

    /// The underlying ticks-per-second value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl From<core::num::NonZeroU32> for Timescale {
    fn from(v: core::num::NonZeroU32) -> Self {
        Self(v)
    }
}

impl From<Timescale> for u32 {
    fn from(t: Timescale) -> Self {
        t.0.get()
    }
}

impl From<Timescale> for u64 {
    fn from(t: Timescale) -> Self {
        t.0.get() as u64
    }
}

impl From<Timescale> for core::num::NonZeroU32 {
    fn from(t: Timescale) -> Self {
        t.0
    }
}

impl From<u32> for Timescale {
    fn from(v: u32) -> Self {
        Self(core::num::NonZeroU32::new(v).unwrap_or(core::num::NonZeroU32::MIN))
    }
}

impl From<Timescale> for i128 {
    fn from(t: Timescale) -> Self {
        i128::from(t.0.get())
    }
}

impl From<Timescale> for usize {
    fn from(t: Timescale) -> Self {
        t.0.get() as usize
    }
}

/// Ticks per second of the microsecond timescale ([`Timescale::MICROS`]).
pub const MICROS_PER_SECOND: u32 = 1_000_000;

/// `ticks` rescaled from `from` to `to` ticks per second.
#[must_use]
pub fn rescale(ticks: i128, from: Timescale, to: Timescale, rounding: Rounding) -> i128 {
    let den = i128::from(from.get());
    let num = ticks.saturating_mul(i128::from(to.get()));
    match rounding {
        Rounding::Floor => num.div_euclid(den),
        Rounding::Ceil => {
            let floor = num.div_euclid(den);
            if num.rem_euclid(den) == 0 {
                floor
            } else {
                floor.saturating_add(1)
            }
        }
        Rounding::TowardZero => num / den,
        Rounding::Nearest => {
            let half = den / 2;
            if num >= 0 {
                num.saturating_add(half) / den
            } else {
                num.saturating_sub(half) / den
            }
        }
    }
}

/// `ticks` at `timescale` as microseconds.
#[must_use]
pub fn to_micros(ticks: i128, timescale: Timescale, rounding: Rounding) -> i128 {
    rescale(ticks, timescale, Timescale::MICROS, rounding)
}

/// `us` microseconds as ticks at `timescale`.
#[must_use]
pub fn from_micros(us: i128, timescale: Timescale, rounding: Rounding) -> i128 {
    rescale(us, Timescale::MICROS, timescale, rounding)
}

/// Exact comparison of `a / a_ts` with `b / b_ts` seconds, without rounding.
#[must_use]
pub fn cmp_ticks(a: i64, a_ts: Timescale, b: i64, b_ts: Timescale) -> core::cmp::Ordering {
    let a_ts = i128::from(a_ts.get());
    let b_ts = i128::from(b_ts.get());
    (i128::from(a) * b_ts).cmp(&(i128::from(b) * a_ts))
}

/// [`rescale`] on `i64` ticks, saturating to the `i64` range.
#[must_use]
pub fn rescale_i64(ticks: i64, from: Timescale, to: Timescale, rounding: Rounding) -> i64 {
    let scaled = rescale(i128::from(ticks), from, to, rounding);
    i64::try_from(scaled).unwrap_or(if scaled < 0 { i64::MIN } else { i64::MAX })
}

/// [`rescale`] on `u64` ticks, saturating to the `u64` range.
#[must_use]
pub fn rescale_u64(ticks: u64, from: Timescale, to: Timescale, rounding: Rounding) -> u64 {
    let scaled = rescale(i128::from(ticks), from, to, rounding);
    u64::try_from(scaled).unwrap_or(if scaled < 0 { 0 } else { u64::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    const T1: Timescale = match Timescale::new(1) {
        Some(t) => t,
        None => unreachable!(),
    };
    const T48K: Timescale = match Timescale::new(48_000) {
        Some(t) => t,
        None => unreachable!(),
    };
    const T90K: Timescale = match Timescale::new(90_000) {
        Some(t) => t,
        None => unreachable!(),
    };

    #[test]
    fn rounding_modes_differ_on_a_half_and_a_negative() {
        let two = Timescale::from(2u32);
        assert_eq!(rescale(3, two, T1, Rounding::Floor), 1);
        assert_eq!(rescale(3, two, T1, Rounding::Nearest), 2);
        assert_eq!(rescale(3, two, T1, Rounding::TowardZero), 1);
        assert_eq!(rescale(-3, two, T1, Rounding::Floor), -2);
        assert_eq!(rescale(-3, two, T1, Rounding::Nearest), -2);
        assert_eq!(rescale(-3, two, T1, Rounding::TowardZero), -1);
    }

    #[test]
    fn micros_helpers_are_rescale_against_one_million() {
        let three = Timescale::from(3u32);
        assert_eq!(to_micros(90_000, T90K, Rounding::Floor), 1_000_000);
        assert_eq!(to_micros(1, three, Rounding::TowardZero), 333_333);
        assert_eq!(to_micros(-1, three, Rounding::Floor), -333_334);
        assert_eq!(from_micros(1_000_001, T90K, Rounding::Ceil), 90_001);
        assert_eq!(from_micros(1_000_001, T90K, Rounding::Floor), 90_000);
        assert_eq!(to_micros(5, T1, Rounding::Floor), 5_000_000);
    }

    #[test]
    fn identity_and_known_conversion() {
        assert_eq!(rescale(90_000, T90K, T90K, Rounding::Nearest), 90_000);
        assert_eq!(rescale(48_000, T48K, T90K, Rounding::Floor), 90_000);
        assert_eq!(rescale_u64(1024, T48K, T90K, Rounding::Nearest), 1920);
    }

    /// A zero timescale cannot reach the arithmetic: the type has no zero
    /// value, and the boundary that reads a wire value refuses it with a
    /// typed error naming the field instead of flattening it to `1`.
    #[test]
    fn zero_timescale_is_rejected_not_flattened_to_one() {
        assert_eq!(Timescale::new(0), None);
        let err = Timescale::try_new(0).expect_err("a zero timescale is an error");
        assert_eq!(err.field, "timescale");
        assert_eq!(err.value, 0);
        assert_eq!(err.reason, "must be non-zero");
        assert_eq!(Timescale::try_new(1).map(Timescale::get), Ok(1));
        assert_eq!(Timescale::try_new(90_000).map(Timescale::get), Ok(90_000));
        // 1 Hz is the identity timescale, never a stand-in for "unknown".
        assert_eq!(to_micros(7, T1, Rounding::Floor), 7_000_000);
    }

    #[test]
    fn saturates_instead_of_wrapping() {
        let max = Timescale::from(u32::MAX);
        assert_eq!(rescale_i64(i64::MAX, T1, max, Rounding::Floor), i64::MAX);
        assert_eq!(rescale_i64(i64::MIN, T1, max, Rounding::Floor), i64::MIN);
        assert_eq!(rescale_u64(u64::MAX, T1, max, Rounding::Nearest), u64::MAX);
        assert_eq!(rescale_u64(0, T1, T1, Rounding::Nearest), 0);
        assert_eq!(
            rescale(i128::MAX, T1, Timescale::from(2u32), Rounding::Floor),
            i128::MAX
        );
    }

    #[test]
    fn ceil_rounds_toward_positive_infinity() {
        let two = Timescale::from(2u32);
        assert_eq!(rescale(3, two, T1, Rounding::Ceil), 2);
        assert_eq!(rescale(4, two, T1, Rounding::Ceil), 2);
        assert_eq!(rescale(-3, two, T1, Rounding::Ceil), -1);
        assert_eq!(rescale(-4, two, T1, Rounding::Ceil), -2);
        assert_eq!(rescale(5, T1, T1, Rounding::Ceil), 5);
        assert_eq!(
            rescale_i64(i64::MAX, T1, Timescale::from(u32::MAX), Rounding::Ceil),
            i64::MAX
        );
    }

    #[test]
    fn cmp_ticks_is_exact_across_timescales() {
        use core::cmp::Ordering::{Equal, Greater, Less};
        assert_eq!(cmp_ticks(90_000, T90K, 48_000, T48K), Equal);
        assert_eq!(
            cmp_ticks(1, Timescale::from(3u32), 1, Timescale::from(2u32)),
            Less
        );
        assert_eq!(
            cmp_ticks(i64::MAX, T1, i64::MAX, Timescale::from(2u32)),
            Greater
        );
        assert_eq!(cmp_ticks(5, T1, 5, T1), Equal);
    }

    #[test]
    fn labels() {
        assert_eq!(Rounding::Ceil.to_string(), "ceil");
        assert_eq!(Rounding::Nearest.name(), "nearest");
        assert_eq!(Rounding::TowardZero.to_string(), "toward zero");
    }
}
