//! Live edge, presentation delay and join (ISO/IEC 23009-1 5.3.9.5.3, A.2
//! step 4; DASH-IF IOP v4.3 4.3.3.2.2 and 4.3.4).
//!
//! * `D` = `@suggestedPresentationDelay` when present, else
//!   `max(4 s, 3 x maxSegmentDuration)` (IOP 4.3.3.2.2: 2-4 segment
//!   durations and not under 4 s), clamped to `TSB - 2 x maxSegmentDuration`
//!   when a time-shift buffer exists; an explicit override replaces it.
//! * Join time (Period-relative) `T_join = utc_now - D - (AST + PeriodStart)`;
//!   the segment joined is the one containing it on the Representation's own
//!   timeline (audio and video boundaries differ), clamped to the segments
//!   that are complete and still inside the time-shift window.

use core::time::Duration;

use jiff::{SignedDuration, Timestamp};

use crate::address::plan::SegmentPlan;
use crate::timing::availability::complete_count_constant;
use broadcast_common::units::MICROS_PER_SEC;

/// IOP 4.3.3.2.2: the presentation delay is not under four seconds.
pub const MIN_PRESENTATION_DELAY: Duration = Duration::from_secs(4);
/// IOP 4.3.3.2.2: the delay is a small multiple of the segment duration.
const DELAY_SEGMENT_MULTIPLE: i128 = 3;
/// Segment durations kept free inside the time-shift buffer.
const TSB_GUARD_SEGMENTS: i128 = 2;

/// Standby bound used when the segment count is unknown.
const UNBOUNDED_COUNT: u64 = u64::MAX / 2;

/// The presentation delay `D`.
#[must_use]
pub fn presentation_delay(
    suggested: Option<Duration>,
    max_segment_duration: Duration,
    time_shift_buffer_depth: Option<Duration>,
    override_delay: Option<Duration>,
) -> Duration {
    if let Some(o) = override_delay {
        return o.max(Duration::ZERO);
    }
    let max_seg_us = duration_micros(max_segment_duration);
    let mut delay = match suggested {
        Some(s) => duration_micros(s),
        None => (MIN_PRESENTATION_DELAY.as_micros() as i128)
            .max(max_seg_us.saturating_mul(DELAY_SEGMENT_MULTIPLE)),
    };
    if let Some(tsb) = time_shift_buffer_depth {
        let tsb_us = duration_micros(tsb);
        let ceiling = tsb_us.saturating_sub(max_seg_us.saturating_mul(TSB_GUARD_SEGMENTS));
        delay = delay.min(ceiling);
    }
    micros_to_duration(delay.max(0))
}

/// `T_join`, Period-relative.
#[must_use]
pub fn join_time(
    utc_now: Timestamp,
    delay: Duration,
    ast: Timestamp,
    period_start: Duration,
) -> SignedDuration {
    let us = (utc_now.as_microsecond() as i128)
        .saturating_sub(duration_micros(delay))
        .saturating_sub(
            (ast.as_microsecond() as i128).saturating_add(duration_micros(period_start)),
        );
    let fallback = if us < 0 {
        SignedDuration::MIN
    } else {
        SignedDuration::MAX
    };
    SignedDuration::try_from_micros_i128(us).unwrap_or(fallback)
}

fn duration_micros(d: Duration) -> i128 {
    i128::from(d.as_secs()) * (MICROS_PER_SEC as i128) + i128::from(d.subsec_micros())
}

fn micros_to_duration(us: i128) -> Duration {
    Duration::from_micros(u64::try_from(us.max(0)).unwrap_or(u64::MAX))
}

/// The plan indices of the segments obtainable now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LiveWindow {
    /// Oldest segment still inside the time-shift window.
    pub oldest: u64,
    /// Newest complete segment.
    pub newest: u64,
}

/// Index of the newest complete segment at `utc_now`; `None` when none is
/// complete yet or the plan is static.
#[must_use]
pub fn newest_complete(plan: &SegmentPlan, utc_now: Timestamp) -> Option<u64> {
    let (ast, ato) = plan.avail_parts()?;
    let count = match (plan.len(), plan.constant_duration()) {
        (None, Some(d)) => complete_count_constant(
            utc_now.as_microsecond() as i128,
            ast,
            plan.period_start_micros(),
            ato,
            d,
            plan.timescale(),
        ),
        (len, _) => {
            let bound = len.unwrap_or(UNBOUNDED_COUNT);
            plan.leading_true(bound, |s| s.available_from.is_some_and(|a| a <= utc_now))
                .unwrap_or(0)
        }
    };
    count.checked_sub(1)
}

/// Index of the oldest segment whose availability has not ended at
/// `utc_now`; `None` when none is complete or all have expired.
#[must_use]
pub fn oldest_in_window(plan: &SegmentPlan, utc_now: Timestamp) -> Option<u64> {
    let newest = newest_complete(plan, utc_now)?;
    let expired = plan
        .leading_true(newest + 1, |s| {
            s.available_until.is_some_and(|u| u < utc_now)
        })
        .unwrap_or(0);
    (expired <= newest).then_some(expired)
}

/// The obtainable window at `utc_now`.
#[must_use]
pub fn window(plan: &SegmentPlan, utc_now: Timestamp) -> Option<LiveWindow> {
    Some(LiveWindow {
        oldest: oldest_in_window(plan, utc_now)?,
        newest: newest_complete(plan, utc_now)?,
    })
}

/// The segment to start with: the one containing `T_join`, clamped into the
/// window. `None` when nothing is obtainable.
#[must_use]
pub fn join_index(plan: &SegmentPlan, utc_now: Timestamp, delay: Duration) -> Option<u64> {
    let w = window(plan, utc_now)?;
    let (ast, _) = plan.avail_parts()?;
    let ast_ts = Timestamp::from_microsecond(i64::try_from(ast).ok()?).ok()?;
    let t = join_time(utc_now, delay, ast_ts, plan.period_start());
    let at = match plan.index_at_or_before(t) {
        Ok(Some(i)) => i,
        _ => w.oldest,
    };
    Some(at.clamp(w.oldest, w.newest))
}
