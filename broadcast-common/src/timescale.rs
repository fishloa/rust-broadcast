//! Rescaling a tick count between two timescales (ticks per second).
//!
//! One home for `ticks * to / from` so the rounding mode is explicit at every
//! call site and the edge cases are decided once:
//!
//! - Timescales are any unsigned integer type up to `u64`. A timescale of `0`
//!   (a parsed `mdhd`/`TimeScale` field can carry it) is treated as `1` on
//!   either side; the functions never divide by zero.
//! - The multiply saturates instead of wrapping, so an absurd tick count
//!   clamps rather than producing a small wrong value.
//! - The `_i64` / `_u64` forms saturate to the target type; `_u64` clamps a
//!   negative result to `0`.

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

/// `ticks` rescaled from `from` to `to` ticks per second.
#[must_use]
pub fn rescale(ticks: i128, from: impl Into<u64>, to: impl Into<u64>, rounding: Rounding) -> i128 {
    let den = i128::from(from.into().max(1));
    let num = ticks.saturating_mul(i128::from(to.into().max(1)));
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

/// A media timescale (ticks per second), guaranteed non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timescale(pub core::num::NonZeroU32);

impl Timescale {
    /// Construct from a raw value, returning `None` for zero.
    #[must_use]
    pub const fn new(value: u32) -> Option<Self> {
        match core::num::NonZeroU32::new(value) {
            Some(nz) => Some(Self(nz)),
            None => None,
        }
    }

    /// Fallible constructor; zero is an error.
    pub fn try_new(value: u32) -> Result<Self, crate::error::InvalidValue> {
        core::num::NonZeroU32::new(value)
            .map(Self)
            .ok_or(crate::error::InvalidValue {
                field: "timescale",
                value: value as u64,
                reason: "must be non-zero",
            })
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
    fn from(t: Timescale) -> u32 {
        t.0.get()
    }
}

impl From<Timescale> for u64 {
    fn from(t: Timescale) -> u64 {
        t.0.get() as u64
    }
}

impl From<u32> for Timescale {
    fn from(v: u32) -> Self {
        Self(core::num::NonZeroU32::new(v).unwrap_or(core::num::NonZeroU32::new(1).unwrap()))
    }
}

impl From<Timescale> for usize {
    fn from(t: Timescale) -> usize {
        t.0.get() as usize
    }
}

/// Ticks per second of the microsecond timescale.
pub const MICROS_PER_SECOND: u32 = 1_000_000;

/// `ticks` at `timescale` as microseconds.
#[must_use]
pub fn to_micros(ticks: i128, timescale: impl Into<u64>, rounding: Rounding) -> i128 {
    rescale(ticks, timescale, MICROS_PER_SECOND, rounding)
}

/// `us` microseconds as ticks at `timescale`.
#[must_use]
pub fn from_micros(us: i128, timescale: impl Into<u64>, rounding: Rounding) -> i128 {
    rescale(us, MICROS_PER_SECOND, timescale, rounding)
}

/// Exact comparison of `a / a_ts` with `b / b_ts` seconds, without rounding.
/// A timescale of `0` counts as `1`, as in [`rescale`].
#[must_use]
pub fn cmp_ticks(
    a: i64,
    a_ts: impl Into<u64>,
    b: i64,
    b_ts: impl Into<u64>,
) -> core::cmp::Ordering {
    let a_ts = i128::from(a_ts.into().max(1));
    let b_ts = i128::from(b_ts.into().max(1));
    (i128::from(a) * b_ts).cmp(&(i128::from(b) * a_ts))
}

/// [`rescale`] on `i64` ticks, saturating to the `i64` range.
#[must_use]
pub fn rescale_i64(
    ticks: i64,
    from: impl Into<u64>,
    to: impl Into<u64>,
    rounding: Rounding,
) -> i64 {
    let scaled = rescale(i128::from(ticks), from, to, rounding);
    i64::try_from(scaled).unwrap_or(if scaled < 0 { i64::MIN } else { i64::MAX })
}

/// [`rescale`] on `u64` ticks, saturating to the `u64` range.
#[must_use]
pub fn rescale_u64(
    ticks: u64,
    from: impl Into<u64>,
    to: impl Into<u64>,
    rounding: Rounding,
) -> u64 {
    let scaled = rescale(i128::from(ticks), from, to, rounding);
    u64::try_from(scaled).unwrap_or(if scaled < 0 { 0 } else { u64::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn rounding_modes_differ_on_a_half_and_a_negative() {
        assert_eq!(rescale(3, 2u32, 1u32, Rounding::Floor), 1);
        assert_eq!(rescale(3, 2u32, 1u32, Rounding::Nearest), 2);
        assert_eq!(rescale(3, 2u32, 1u32, Rounding::TowardZero), 1);
        assert_eq!(rescale(-3, 2u32, 1u32, Rounding::Floor), -2);
        assert_eq!(rescale(-3, 2u32, 1u32, Rounding::Nearest), -2);
        assert_eq!(rescale(-3, 2u32, 1u32, Rounding::TowardZero), -1);
    }

    #[test]
    fn micros_helpers_are_rescale_against_one_million() {
        assert_eq!(to_micros(90_000, 90_000u32, Rounding::Floor), 1_000_000);
        assert_eq!(to_micros(1, 3u32, Rounding::TowardZero), 333_333);
        assert_eq!(to_micros(-1, 3u32, Rounding::Floor), -333_334);
        assert_eq!(from_micros(1_000_001, 90_000u32, Rounding::Ceil), 90_001);
        assert_eq!(from_micros(1_000_001, 90_000u32, Rounding::Floor), 90_000);
        assert_eq!(to_micros(5, 0u32, Rounding::Floor), 5_000_000);
    }

    #[test]
    fn identity_and_known_conversion() {
        assert_eq!(
            rescale(90_000, 90_000u32, 90_000u32, Rounding::Nearest),
            90_000
        );
        assert_eq!(
            rescale(48_000, 48_000u32, 90_000u32, Rounding::Floor),
            90_000
        );
        assert_eq!(
            rescale_u64(1024, 48_000u32, 90_000u32, Rounding::Nearest),
            1920
        );
    }

    #[test]
    fn zero_timescale_is_one_never_a_divide_by_zero() {
        assert_eq!(rescale(7, 0u32, 1u32, Rounding::Floor), 7);
        assert_eq!(rescale(7, 1u32, 0u32, Rounding::Floor), 7);
        assert_eq!(rescale_i64(-7, 0u32, 0u32, Rounding::TowardZero), -7);
    }

    #[test]
    fn saturates_instead_of_wrapping() {
        assert_eq!(
            rescale_i64(i64::MAX, 1u32, u32::MAX, Rounding::Floor),
            i64::MAX
        );
        assert_eq!(
            rescale_i64(i64::MIN, 1u32, u32::MAX, Rounding::Floor),
            i64::MIN
        );
        assert_eq!(
            rescale_u64(u64::MAX, 1u32, u32::MAX, Rounding::Nearest),
            u64::MAX
        );
        assert_eq!(rescale_u64(0, 1u32, 1u32, Rounding::Nearest), 0);
        assert_eq!(rescale(i128::MAX, 1u32, 2u32, Rounding::Floor), i128::MAX);
    }

    #[test]
    fn ceil_rounds_toward_positive_infinity() {
        assert_eq!(rescale(3, 2u32, 1u32, Rounding::Ceil), 2);
        assert_eq!(rescale(4, 2u32, 1u32, Rounding::Ceil), 2);
        assert_eq!(rescale(-3, 2u32, 1u32, Rounding::Ceil), -1);
        assert_eq!(rescale(-4, 2u32, 1u32, Rounding::Ceil), -2);
        assert_eq!(rescale(5, 0u32, 1u32, Rounding::Ceil), 5);
        assert_eq!(
            rescale_i64(i64::MAX, 1u32, u32::MAX, Rounding::Ceil),
            i64::MAX
        );
    }

    #[test]
    fn cmp_ticks_is_exact_across_timescales() {
        use core::cmp::Ordering::{Equal, Greater, Less};
        assert_eq!(cmp_ticks(90_000, 90_000u32, 48_000, 48_000u32), Equal);
        assert_eq!(cmp_ticks(1, 3u32, 1, 2u32), Less);
        assert_eq!(cmp_ticks(i64::MAX, 1u32, i64::MAX, 2u32), Greater);
        assert_eq!(cmp_ticks(5, 0u32, 5, 1u32), Equal);
    }

    #[test]
    fn labels() {
        assert_eq!(Rounding::Ceil.to_string(), "ceil");
        assert_eq!(Rounding::Nearest.name(), "nearest");
        assert_eq!(Rounding::TowardZero.to_string(), "toward zero");
    }
}
