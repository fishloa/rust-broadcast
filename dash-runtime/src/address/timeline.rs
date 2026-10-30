//! `SegmentTimeline` as a lazy run table (ISO/IEC 23009-1 5.3.9.6).
//!
//! A timeline is never expanded into one entry per segment: each `S` becomes
//! one run `(first time, first number, duration, count)` and a segment is
//! found by binary search. A `@r` of four billion therefore costs one run;
//! the total is still bounded by
//! [`crate::limits::MAX_TIMELINE_SEGMENTS`] so index arithmetic cannot
//! overflow.
//!
//! Rules (5.3.9.6.1, restated in `docs/specs/dash/23009-1`):
//! `@t` is absolute and never smaller than the previous run's end; absent, a
//! run starts where the previous ended (the first at 0); `@n` restarts the
//! numbering; `@r >= 0` gives `r + 1` segments; a negative `@r` repeats up to
//! the next `S@t`, or for the last `S` to the Period end, as
//! `ceil((end - time) / d)` (the spec prints no rounding rule; this is the
//! `transmux::SegmentTimeline::resolve` reading, which the differential test
//! holds this to).

use crate::limits::MAX_TIMELINE_SEGMENTS;
use crate::mpd::model::SegmentTimeline;

use super::AddressError;

/// One segment of a timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TimelineSegment {
    /// `$Number$`.
    pub number: u64,
    /// Start on the media timeline in `@timescale` ticks (`$Time$`).
    pub time: u64,
    /// Duration in ticks.
    pub duration: u64,
}

#[derive(Debug, Clone, Copy)]
struct Run {
    first_index: u64,
    time: u64,
    number: u64,
    d: u64,
    count: u64,
}

/// The lazily expanded timeline.
#[derive(Debug, Clone, Default)]
pub struct TimelineIndex {
    runs: Vec<Run>,
    total: u64,
}

/// Tolerance, in `@timescale` ticks, applied to a declared `@t` that falls
/// slightly below the computed end of the previous run: DASH 5.3.9.6.1
/// requires `@t` to be at least that end, but nominal durations on the last
/// Segment of a run can overshoot the real boundary by one tick, so a
/// one-tick shortfall is trusted rather than rejected.
const TIMELINE_T_TOLERANCE_TICKS: u64 = 1;

impl TimelineIndex {
    /// Build the run table.
    ///
    /// `period_end_ticks` is on the media timeline (`PTO + (PEwc - PSwc)`),
    /// used by a trailing negative `@r`; `None` makes such a run a single
    /// segment.
    ///
    /// # Errors
    /// [`AddressError`] for a non-increasing `@t`/`@n`, an oversized or
    /// overflowing timeline, or an unsupported feature.
    pub fn build(
        timeline: &SegmentTimeline,
        start_number: u64,
        period_end_ticks: Option<u64>,
    ) -> Result<Self, AddressError> {
        if let Some(u) = timeline.unsupported {
            return Err(AddressError::Unsupported(u));
        }
        let entries = &timeline.entries;
        let mut runs = Vec::with_capacity(entries.len());
        let mut time: u64 = 0;
        let mut number = start_number;
        let mut total: u64 = 0;
        for (i, s) in entries.iter().enumerate() {
            if let Some(t) = s.t {
                if time.saturating_sub(t) > TIMELINE_T_TOLERANCE_TICKS {
                    return Err(AddressError::TimelineNotIncreasing { t, floor: time });
                }
                time = t;
            }
            if let Some(n) = s.n {
                if n < number {
                    return Err(AddressError::NumberNotIncreasing {
                        n,
                        expected: number,
                    });
                }
                number = n;
            }
            let count = if s.r >= 0 {
                s.r.unsigned_abs().saturating_add(1)
            } else {
                let next_t = entries.get(i + 1).and_then(|n| n.t);
                if let Some(next_t) = next_t
                    && time.saturating_sub(next_t) > TIMELINE_T_TOLERANCE_TICKS
                {
                    return Err(AddressError::TimelineNotIncreasing {
                        t: next_t,
                        floor: time,
                    });
                }
                match next_t.or(period_end_ticks) {
                    Some(end) => end.saturating_sub(time).div_ceil(s.d),
                    None => 1,
                }
            };
            if count > MAX_TIMELINE_SEGMENTS || total.saturating_add(count) > MAX_TIMELINE_SEGMENTS
            {
                return Err(AddressError::TimelineTooLong {
                    count_hint: total.saturating_add(count),
                });
            }
            if count > 0 {
                runs.push(Run {
                    first_index: total,
                    time,
                    number,
                    d: s.d,
                    count,
                });
            }
            let span = s.d.checked_mul(count).ok_or(AddressError::Overflow)?;
            let cut_at_next = s.r < 0 && entries.get(i + 1).is_some_and(|n| n.t.is_some());
            time = if cut_at_next {
                entries[i + 1].t.unwrap_or(time)
            } else {
                time.checked_add(span).ok_or(AddressError::Overflow)?
            };
            number = number.saturating_add(count);
            total += count;
        }
        Ok(Self { runs, total })
    }

    /// Number of segments.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.total
    }

    /// Whether the timeline has no segment.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// Segment at zero-based `index`.
    #[must_use]
    pub fn segment(&self, index: u64) -> Option<TimelineSegment> {
        if index >= self.total {
            return None;
        }
        let at = self.runs.partition_point(|r| r.first_index <= index) - 1;
        let run = self.runs[at];
        let k = index - run.first_index;
        Some(TimelineSegment {
            number: run.number.saturating_add(k),
            time: run.time.saturating_add(k.saturating_mul(run.d)),
            duration: run.d,
        })
    }

    /// Index of the last segment whose start is at or before `time` (media
    /// timeline ticks); `None` when `time` precedes the first segment.
    #[must_use]
    pub fn index_at_or_before(&self, time: u64) -> Option<u64> {
        let at = self.runs.partition_point(|r| r.time <= time);
        if at == 0 {
            return None;
        }
        let run = self.runs[at - 1];
        let k = ((time - run.time) / run.d).min(run.count - 1);
        Some(run.first_index + k)
    }

    /// How many segments have a number at or below `max_number`
    /// (`@endNumber`: a segment numbered above it is invalid, 5.3.9.5.3).
    #[must_use]
    pub fn count_up_to_number(&self, max_number: u64) -> u64 {
        let mut count = 0u64;
        for run in &self.runs {
            if run.number > max_number {
                break;
            }
            let fit = (max_number - run.number).saturating_add(1);
            count += fit.min(run.count);
            if fit < run.count {
                break;
            }
        }
        count
    }

    /// The segments in order (lazy).
    pub fn iter(&self) -> impl Iterator<Item = TimelineSegment> + '_ {
        (0..self.total).filter_map(|i| self.segment(i))
    }
}
