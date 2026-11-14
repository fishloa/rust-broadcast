//! Time on the public surface: a span is a [`Duration`], a position is a
//! [`Clock27`].
//!
//! The controller keeps **two clocks**: the host's monotonic clock (arrival
//! instants, ticks, timeouts) and the playback (PTS) domain (27 MHz tick
//! positions, `PTS90k x 300`). The workspace has one type for each
//! (`docs/API-CONVENTIONS.md`): a *span* of host time is
//! [`core::time::Duration`], and a *position* in the playback domain is
//! [`broadcast_common::clock33::Clock27`], the validated 27 MHz clock value
//! that cannot hold a PCR extension or a wrap outside the clock. The
//! controller itself computes host spans in whole microseconds, so it converts
//! at its edge with [`duration_micros`] and back with [`micros_as_duration`].

use core::time::Duration;

pub use broadcast_common::clock33::Clock27;

/// Nanoseconds in a microsecond: a span is carried in whole microseconds.
const NANOS_PER_MICRO: u128 = 1_000;

/// A span in whole microseconds, rounded **up**.
///
/// The controller computes host spans in whole microseconds, so this is its
/// only rounding. A sub-microsecond span is one microsecond, never zero, so a
/// caller that set a 500 ns window cannot end up with a zero-length one; a
/// span too long for the microsecond range clamps to [`u64::MAX`].
#[must_use]
pub fn duration_micros(span: Duration) -> u64 {
    let micros = span.as_nanos().div_ceil(NANOS_PER_MICRO);
    u64::try_from(micros).unwrap_or(u64::MAX)
}

/// Whole microseconds as a [`Duration`].
#[must_use]
pub fn micros_as_duration(micros: u64) -> Duration {
    Duration::from_micros(micros)
}

/// `a + b` in whole microseconds, saturating at the end of the microsecond
/// range rather than panicking on an overflow.
#[must_use]
pub fn duration_add(a: Duration, b: Duration) -> Duration {
    micros_as_duration(duration_micros(a).saturating_add(duration_micros(b)))
}

/// `a - b`, zero when `b` is not below `a` (a span never goes backwards).
#[must_use]
pub fn duration_sub(a: Duration, b: Duration) -> Duration {
    micros_as_duration(duration_micros(a).saturating_sub(duration_micros(b)))
}

/// The span from `earlier` to `later`, zero when `later` is not after it.
#[must_use]
pub fn duration_between(earlier: Duration, later: Duration) -> Duration {
    duration_sub(later, earlier)
}

#[cfg(test)]
mod tests {
    use super::{duration_between, duration_micros, micros_as_duration};
    use core::time::Duration;

    #[test]
    fn a_span_rounds_up_to_whole_microseconds() {
        assert_eq!(duration_micros(Duration::ZERO), 0);
        assert_eq!(duration_micros(Duration::from_nanos(1)), 1);
        assert_eq!(duration_micros(Duration::from_nanos(999)), 1);
        assert_eq!(duration_micros(Duration::from_nanos(1_000)), 1);
        assert_eq!(duration_micros(Duration::from_secs(2)), 2_000_000);
        assert_eq!(duration_micros(Duration::MAX), u64::MAX);
        assert_eq!(micros_as_duration(1_800_000), Duration::from_millis(1_800));
    }

    #[test]
    fn a_backwards_span_is_zero_not_a_negative_duration() {
        assert_eq!(
            duration_between(Duration::from_millis(5), Duration::from_millis(2)),
            Duration::ZERO
        );
    }

    #[test]
    fn a_position_is_a_checked_27mhz_clock_value() {
        let p = super::Clock27::from_27mhz(27_000_000).expect("one second");
        assert_eq!(p.ticks(), 27_000_000);
    }
}
