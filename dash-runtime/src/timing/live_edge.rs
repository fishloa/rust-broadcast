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

use std::time::Duration;

use crate::address::plan::SegmentPlan;
use crate::timing::availability::complete_count_constant;
use crate::timing::period::duration_us;

const MICROS_PER_SECOND: i128 = 1_000_000;
/// IOP 4.3.3.2.2: the presentation delay is not under four seconds.
pub const MIN_PRESENTATION_DELAY_US: i128 = 4 * MICROS_PER_SECOND;
/// IOP 4.3.3.2.2: the delay is a small multiple of the segment duration.
pub const DELAY_SEGMENT_MULTIPLE: i128 = 3;
/// Segment durations kept free inside the time-shift buffer.
pub const TSB_GUARD_SEGMENTS: i128 = 2;

/// The presentation delay `D`, in microseconds.
#[must_use]
pub fn presentation_delay_us(
    suggested: Option<Duration>,
    max_segment_duration_us: i128,
    time_shift_buffer_depth_us: Option<i128>,
    override_us: Option<i128>,
) -> i128 {
    if let Some(o) = override_us {
        return o.max(0);
    }
    let mut delay = suggested.map_or_else(
        || {
            MIN_PRESENTATION_DELAY_US
                .max(max_segment_duration_us.saturating_mul(DELAY_SEGMENT_MULTIPLE))
        },
        duration_us,
    );
    if let Some(tsb) = time_shift_buffer_depth_us {
        let ceiling = tsb.saturating_sub(max_segment_duration_us.saturating_mul(TSB_GUARD_SEGMENTS));
        delay = delay.min(ceiling);
    }
    delay.max(0)
}

/// `T_join`, Period-relative microseconds.
#[must_use]
pub fn join_time_us(utc_now_us: i128, delay_us: i128, ast_us: i128, period_start_us: i128) -> i128 {
    utc_now_us
        .saturating_sub(delay_us)
        .saturating_sub(ast_us.saturating_add(period_start_us))
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

/// Index of the newest complete segment at `utc_now_us`; `None` when none is
/// complete yet or the plan is static.
#[must_use]
pub fn newest_complete(plan: &SegmentPlan, utc_now_us: i128) -> Option<u64> {
    let (ast, ato) = plan.avail_parts()?;
    let count = match (plan.len(), plan.constant_duration()) {
        (None, Some(d)) => complete_count_constant(
            utc_now_us,
            ast,
            plan.period_start_us(),
            ato,
            d,
            plan.timescale(),
        ),
        (len, _) => {
            let bound = len.unwrap_or(u64::MAX / 2);
            plan.leading_true(bound, |s| {
                s.available_from_us.is_some_and(|a| a <= utc_now_us)
            })
        }
    };
    count.checked_sub(1)
}

/// Index of the oldest segment whose availability has not ended at
/// `utc_now_us`; `None` when none is complete or all have expired.
#[must_use]
pub fn oldest_in_window(plan: &SegmentPlan, utc_now_us: i128) -> Option<u64> {
    let newest = newest_complete(plan, utc_now_us)?;
    let expired = plan.leading_true(newest + 1, |s| {
        s.available_until_us.is_some_and(|u| u < utc_now_us)
    });
    (expired <= newest).then_some(expired)
}

/// The obtainable window at `utc_now_us`.
#[must_use]
pub fn window(plan: &SegmentPlan, utc_now_us: i128) -> Option<LiveWindow> {
    Some(LiveWindow {
        oldest: oldest_in_window(plan, utc_now_us)?,
        newest: newest_complete(plan, utc_now_us)?,
    })
}

/// The segment to start with: the one containing `T_join`, clamped into the
/// window. `None` when nothing is obtainable.
#[must_use]
pub fn join_index(plan: &SegmentPlan, utc_now_us: i128, delay_us: i128) -> Option<u64> {
    let w = window(plan, utc_now_us)?;
    let (ast, _) = plan.avail_parts()?;
    let t = join_time_us(utc_now_us, delay_us, ast, plan.period_start_us());
    let at = plan.index_at_or_before(t).unwrap_or(w.oldest);
    Some(at.clamp(w.oldest, w.newest))
}
