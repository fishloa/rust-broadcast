//! Segment availability arithmetic for dynamic MPDs (ISO/IEC 23009-1
//! 5.3.9.5.3; DASH-IF IOP v4.3 4.3.3.2). Integer microseconds in `i128`, so
//! no input can wrap.
//!
//! * `SegAvailStart = AST + PeriodStart + MPDstart + MPDduration - ATO`
//! * `SegAvailEnd   = AST + PeriodStart + MPDstart + 2 * MPDduration + TSB`
//!   (`SegAvailStart` without the ATO, plus one more duration and the
//!   time-shift buffer; Annex A.3.4 prints the same quantity)
//! * Initialization Segment: available from `AST + PeriodStart` (5.3.9.5.2).

const MICROS_PER_SECOND: i128 = 1_000_000;
const MICROS_PER_SECOND_F64: f64 = 1_000_000.0;
const ATO_CLAMP_US: i128 = 1 << 80;

/// Convert `@availabilityTimeOffset` seconds (possibly `INF` or negative) to
/// microseconds, clamped so later sums stay in range. A NaN is zero.
#[must_use]
pub fn ato_seconds_to_us(seconds: f64) -> i128 {
    let us = seconds * MICROS_PER_SECOND_F64;
    (us as i128).clamp(-ATO_CLAMP_US, ATO_CLAMP_US)
}

/// Ticks to microseconds, rounded toward negative infinity.
#[must_use]
pub fn ticks_to_us(ticks: i128, timescale: u64) -> i128 {
    ticks
        .saturating_mul(MICROS_PER_SECOND)
        .div_euclid(i128::from(timescale.max(1)))
}

/// [`segment_available_from`] for a segment given in ticks: the end of the
/// segment (`MPDstart + MPDduration`) is converted once, so rounding never
/// accumulates.
#[must_use]
pub fn segment_available_from_ticks(
    ast_us: i128,
    period_start_us: i128,
    start_ticks: i128,
    duration_ticks: i128,
    timescale: u64,
    ato_us: i128,
) -> i128 {
    segment_available_from(
        ast_us,
        period_start_us,
        ticks_to_us(start_ticks.saturating_add(duration_ticks), timescale),
        0,
        ato_us,
    )
}

/// [`segment_available_until`] for a segment given in ticks.
#[must_use]
pub fn segment_available_until_ticks(
    ast_us: i128,
    period_start_us: i128,
    start_ticks: i128,
    duration_ticks: i128,
    timescale: u64,
    tsb_us: Option<i128>,
) -> Option<i128> {
    let end2 = start_ticks.saturating_add(duration_ticks.saturating_mul(2));
    segment_available_until(
        ast_us,
        period_start_us,
        ticks_to_us(end2, timescale),
        0,
        tsb_us,
    )
}

/// Availability start of a Media Segment.
#[must_use]
pub fn segment_available_from(
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
pub fn segment_available_until(
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
            .saturating_add(mpd_duration_us.saturating_mul(2))
            .saturating_add(tsb)
    })
}

/// Availability start of the Initialization Segment.
#[must_use]
pub fn init_available_from(ast_us: i128, period_start_us: i128) -> i128 {
    ast_us.saturating_add(period_start_us)
}

/// Number of complete segments of constant duration `duration_ticks` at
/// `utc_us`: the largest `n` with `AST + PS + n * d - ATO <= utc`, computed
/// exactly against per-segment microsecond rounding
/// (`floor(n * d * 1e6 / timescale)`).
#[must_use]
pub fn complete_count_constant(
    utc_us: i128,
    ast_us: i128,
    period_start_us: i128,
    ato_us: i128,
    duration_ticks: u64,
    timescale: u64,
) -> u64 {
    if duration_ticks == 0 || timescale == 0 {
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
    let numerator = x1.saturating_mul(i128::from(timescale)).saturating_sub(1);
    let denominator = i128::from(duration_ticks).saturating_mul(MICROS_PER_SECOND);
    let n = numerator / denominator;
    u64::try_from(n.max(0)).unwrap_or(u64::MAX)
}
