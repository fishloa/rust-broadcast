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
    /// Toward zero (plain integer division).
    TowardZero,
}

impl Rounding {
    /// Label of the rounding mode.
    pub fn name(&self) -> &'static str {
        match self {
            Rounding::Floor => "floor",
            Rounding::Nearest => "nearest",
            Rounding::TowardZero => "toward zero",
        }
    }
}

crate::impl_spec_display!(Rounding);

/// `ticks` rescaled from `from` to `to` ticks per second.
#[must_use]
pub fn rescale(
    ticks: i128,
    from: impl Into<u64>,
    to: impl Into<u64>,
    rounding: Rounding,
) -> i128 {
    let den = i128::from(from.into().max(1));
    let num = ticks.saturating_mul(i128::from(to.into().max(1)));
    match rounding {
        Rounding::Floor => num.div_euclid(den),
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
    fn identity_and_known_conversion() {
        assert_eq!(rescale(90_000, 90_000u32, 90_000u32, Rounding::Nearest), 90_000);
        assert_eq!(rescale(48_000, 48_000u32, 90_000u32, Rounding::Floor), 90_000);
        assert_eq!(rescale_u64(1024, 48_000u32, 90_000u32, Rounding::Nearest), 1920);
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
    fn labels() {
        assert_eq!(Rounding::Nearest.name(), "nearest");
        assert_eq!(Rounding::TowardZero.to_string(), "toward zero");
    }
}
