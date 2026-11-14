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
//! at its edge with [`instant_micros`] (or [`window_micros`] for a configured
//! window) and back with [`micros_as_duration`].
//!
//! # Rounding
//!
//! One placeholder. A *reading* (a clock instant) or a *difference* of two is
//! truncated to the microsecond it is in ([`instant_micros`]): rounding a
//! reading up makes a logged "now" replay a microsecond in the future, and
//! makes `ceil(a) - ceil(b)` disagree with `ceil(a - b)` by
//! [`MAX_DIFFERENCE_ERROR`]. A *configured* window that must not become zero
//! (a jitter window, a hold, a bucket) rounds up ([`window_micros`]), so a
//! sub-microsecond setting is one microsecond rather than an empty window.
//! This is the workspace's rule: floor a reading, raise only what a zero would
//! make meaningless.

use core::time::Duration;

pub use broadcast_common::clock33::Clock27;

/// Nanoseconds in a microsecond: a span is carried in whole microseconds.
const NANOS_PER_MICRO: u128 = 1_000;
const NANOS_PER_MICRO_U64: u64 = 1_000;

/// The microsecond an instant names, truncated: the whole microsecond it falls
/// in. A span too long for the microsecond range clamps to [`u64::MAX`].
#[must_use]
pub fn instant_micros(span: Duration) -> u64 {
    let micros = span.as_nanos() / NANOS_PER_MICRO;
    u64::try_from(micros).unwrap_or(u64::MAX)
}

/// The microseconds a configured window spans, rounded **up**.
///
/// This is the only rounding-up conversion: a sub-microsecond window is one
/// microsecond, never zero, so a caller that set a 500 ns window cannot end up
/// with a zero-length one. A span too long for the microsecond range clamps to
/// [`u64::MAX`].
#[must_use]
pub fn window_micros(span: Duration) -> u64 {
    let micros = span.as_nanos().div_ceil(NANOS_PER_MICRO);
    u64::try_from(micros).unwrap_or(u64::MAX)
}

/// Whole microseconds as a [`Duration`].
#[must_use]
pub fn micros_as_duration(micros: u64) -> Duration {
    Duration::from_micros(micros)
}

/// The most a difference of two truncated readings can understate the real one:
/// each reading is floored by up to one microsecond less than it, and the two
/// roundings cannot exceed this together.
pub const MAX_DIFFERENCE_ERROR: Duration = Duration::from_nanos(NANOS_PER_MICRO_U64 - 1);

/// `a + b` in whole microseconds, saturating at the end of the microsecond
/// range rather than panicking on an overflow.
#[must_use]
pub fn duration_add(a: Duration, b: Duration) -> Duration {
    micros_as_duration(instant_micros(a).saturating_add(instant_micros(b)))
}

/// The span from `earlier` to `later`, the whole microsecond it is in, zero
/// when `later` is not after it (a span never goes backwards).
#[must_use]
pub fn duration_between(earlier: Duration, later: Duration) -> Duration {
    micros_as_duration(instant_micros(later).saturating_sub(instant_micros(earlier)))
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_DIFFERENCE_ERROR, duration_between, instant_micros, micros_as_duration, window_micros,
    };
    use core::time::Duration;

    #[test]
    fn a_reading_is_the_microsecond_it_is_in() {
        assert_eq!(instant_micros(Duration::ZERO), 0);
        assert_eq!(instant_micros(Duration::from_nanos(1)), 0);
        assert_eq!(instant_micros(Duration::from_nanos(999)), 0);
        assert_eq!(instant_micros(Duration::from_nanos(1_000)), 1);
        assert_eq!(instant_micros(Duration::from_nanos(1_999)), 1);
        assert_eq!(instant_micros(Duration::from_secs(2)), 2_000_000);
        assert_eq!(instant_micros(Duration::MAX), u64::MAX);
        assert_eq!(micros_as_duration(1_800_000), Duration::from_millis(1_800));
    }

    #[test]
    fn a_configured_window_rounds_up_to_whole_microseconds() {
        assert_eq!(window_micros(Duration::ZERO), 0);
        assert_eq!(window_micros(Duration::from_nanos(1)), 1);
        assert_eq!(window_micros(Duration::from_nanos(999)), 1);
        assert_eq!(window_micros(Duration::from_nanos(1_000)), 1);
        assert_eq!(window_micros(Duration::from_secs(2)), 2_000_000);
        assert_eq!(window_micros(Duration::MAX), u64::MAX);
    }

    #[test]
    fn a_difference_is_the_whole_microsecond_between_two_readings() {
        assert_eq!(
            duration_between(Duration::from_nanos(1_500), Duration::from_nanos(3_500)),
            Duration::from_micros(2),
            "1 us to 3 us, not 2 us to 4 us"
        );
        assert!(
            duration_between(Duration::from_nanos(1_500), Duration::from_nanos(3_500))
                >= Duration::from_nanos(3_500) - Duration::from_nanos(1_500) - MAX_DIFFERENCE_ERROR
        );
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
