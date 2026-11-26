//! Segment availability arithmetic for dynamic MPDs (ISO/IEC 23009-1
//! 5.3.9.5.3; DASH-IF IOP v4.3 4.3.3.2). Integer microseconds in `i128`, so
//! no input can wrap.
//!
//! * `SegAvailStart = AST + PeriodStart + MPDstart + MPDduration - ATO`
//! * `SegAvailEnd   = AST + PeriodStart + MPDstart + 2 * MPDduration + TSB`
//!   (`SegAvailStart` without the ATO, plus one more duration and the
//!   time-shift buffer; Annex A.3.4 prints the same quantity)
//! * Initialization Segment: available from `AST + PeriodStart` (5.3.9.5.2).

use broadcast_common::units::MICROS_PER_SEC;
use broadcast_core::timescale::{Rounding, Timescale, to_micros};
use jiff::SignedDuration;

/// Factor extending a duration into its availability window.
const AVAILABILITY_WINDOW_FACTOR: i128 = 2;

/// Convert `@availabilityTimeOffset` seconds (possibly `INF` or negative) to a
/// signed duration, clamped so later sums stay in range. A NaN is zero.
#[must_use]
pub fn ato_seconds(seconds: f64) -> SignedDuration {
    if seconds.is_nan() {
        return SignedDuration::ZERO;
    }
    if seconds == f64::INFINITY {
        return SignedDuration::MAX;
    }
    if seconds == f64::NEG_INFINITY {
        return SignedDuration::MIN;
    }
    SignedDuration::try_from_secs_f64(seconds).unwrap_or_else(|_| {
        if seconds.is_sign_positive() {
            SignedDuration::MAX
        } else {
            SignedDuration::MIN
        }
    })
}

/// Ticks to a signed media duration, rounded toward negative infinity.
#[must_use]
pub(crate) fn ticks_to_micros(ticks: i128, timescale: Timescale) -> i128 {
    to_micros(ticks, timescale, Rounding::Floor)
}

/// [`segment_available_from`] for a segment given in ticks: the end of the
/// segment (`MPDstart + MPDduration`) is converted once, so rounding never
/// accumulates.
#[must_use]
pub(crate) fn segment_available_from_ticks(
    ast_us: i128,
    period_start_us: i128,
    start_ticks: i128,
    duration_ticks: i128,
    timescale: Timescale,
    ato_us: i128,
) -> i128 {
    segment_available_from(
        ast_us,
        period_start_us,
        ticks_to_micros(start_ticks.saturating_add(duration_ticks), timescale),
        0,
        ato_us,
    )
}

/// [`segment_available_until`] for a segment given in ticks.
#[must_use]
pub(crate) fn segment_available_until_ticks(
    ast_us: i128,
    period_start_us: i128,
    start_ticks: i128,
    duration_ticks: i128,
    timescale: Timescale,
    tsb_us: Option<i128>,
) -> Option<i128> {
    let end2 =
        start_ticks.saturating_add(duration_ticks.saturating_mul(AVAILABILITY_WINDOW_FACTOR));
    segment_available_until(
        ast_us,
        period_start_us,
        ticks_to_micros(end2, timescale),
        0,
        tsb_us,
    )
}

/// Availability start of a Media Segment.
#[must_use]
pub(crate) fn segment_available_from(
    ast_us: i128,
    period_start_us: i128,
    mpd_start_us: i128,
    mpd_duration_us: i128,
    ato_us: i128,
) -> i128 {
    ast_us
        .saturating_add(period_start_us)
        .saturating_add(mpd_start_us)
        .saturating_add(mpd_duration_us)
        .saturating_sub(ato_us)
}

/// Availability end of a Media Segment; `None` without a time-shift buffer
/// depth (the segment never leaves the window).
#[must_use]
pub(crate) fn segment_available_until(
    ast_us: i128,
    period_start_us: i128,
    mpd_start_us: i128,
    mpd_duration_us: i128,
    tsb_us: Option<i128>,
) -> Option<i128> {
    tsb_us.map(|tsb| {
        ast_us
            .saturating_add(period_start_us)
            .saturating_add(mpd_start_us)
            .saturating_add(mpd_duration_us.saturating_mul(AVAILABILITY_WINDOW_FACTOR))
            .saturating_add(tsb)
    })
}

/// Availability start of the Initialization Segment.
#[must_use]
pub(crate) fn init_available_from(ast_us: i128, period_start_us: i128) -> i128 {
    ast_us.saturating_add(period_start_us)
}

/// Number of complete segments of constant duration `duration_ticks` at
/// `utc_us`: the largest `n` with `AST + PS + n * d - ATO <= utc`, computed
/// exactly against per-segment microsecond rounding
/// (`floor(n * d * 1e6 / timescale)`).
#[must_use]
pub(crate) fn complete_count_constant(
    utc_us: i128,
    ast_us: i128,
    period_start_us: i128,
    ato_us: i128,
    duration_ticks: u64,
    timescale: Timescale,
) -> u64 {
    if duration_ticks == 0 {
        return 0;
    }
    let x = utc_us
        .saturating_sub(ast_us)
        .saturating_sub(period_start_us)
        .saturating_add(ato_us);
    let x1 = x.saturating_add(1);
    if x1 <= 0 {
        return 0;
    }
    let numerator = x1
        .saturating_mul(i128::from(timescale.get()))
        .saturating_sub(1);
    let denominator = i128::from(duration_ticks).saturating_mul(MICROS_PER_SEC as i128);
    let n = numerator / denominator;
    u64::try_from(n.max(0)).unwrap_or(u64::MAX)
}
