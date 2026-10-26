//! PeriodStart derivation and Period kinds (ISO/IEC 23009-1 5.3.2.1).
//!
//! The five rules are a plain list in the standard, evaluated here per Period
//! in document order:
//! 1. `@start` present: PeriodStart = `@start`.
//! 2. `@start` absent, previous Period has `@duration`: previous start plus
//!    previous duration.
//! 3. `@start` absent, first Period of a static MPD: 0.
//! 4. `@start` absent, first Period or previous without `@duration`, dynamic
//!    MPD: Early Available Period (no PeriodStart yet).
//! 5. `@duration` present and (the next Period has `@start` or
//!    `MPD@minimumUpdatePeriod` exists): Early Terminated Period.

use std::time::Duration;

use broadcast_common::impl_spec_display;

use crate::mpd::model::{Mpd, PresentationType};

const MICROS_PER_SECOND: i128 = 1_000_000;
const NANOS_PER_MICRO: u32 = 1_000;

/// Duration to microseconds.
#[must_use]
pub fn duration_us(d: Duration) -> i128 {
    i128::from(d.as_secs()) * MICROS_PER_SECOND + i128::from(d.subsec_nanos() / NANOS_PER_MICRO)
}

/// 5.3.2.1 Period kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PeriodKind {
    /// Ends where the next Period starts (or the presentation ends).
    Regular,
    /// Ends after its own `@duration`.
    EarlyTerminated,
    /// Announced before it has a PeriodStart; not part of the presentation.
    EarlyAvailable,
}

impl PeriodKind {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Regular => "regular",
            Self::EarlyTerminated => "early_terminated",
            Self::EarlyAvailable => "early_available",
        }
    }
}
impl_spec_display!(PeriodKind);

/// A Period's place on the presentation timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PeriodTiming {
    /// PeriodStart in microseconds from the presentation start; `None` until
    /// one can be derived.
    pub start_us: Option<i128>,
    /// The Period end in the same domain, when known.
    pub end_us: Option<i128>,
    /// The kind.
    pub kind: PeriodKind,
}

/// Derive the timing of every Period of `mpd`, in document order.
#[must_use]
pub fn period_timings(mpd: &Mpd) -> Vec<PeriodTiming> {
    let mut out: Vec<PeriodTiming> = Vec::with_capacity(mpd.periods.len());
    let starts: Vec<Option<i128>> = {
        let mut starts: Vec<Option<i128>> = Vec::with_capacity(mpd.periods.len());
        for (i, p) in mpd.periods.iter().enumerate() {
            let start = if let Some(s) = p.start {
                Some(duration_us(s))
            } else if i == 0 {
                (mpd.kind == PresentationType::Static).then_some(0)
            } else {
                let prev = &mpd.periods[i - 1];
                match (prev.duration, starts[i - 1]) {
                    (Some(d), Some(ps)) => Some(ps.saturating_add(duration_us(d))),
                    _ => None,
                }
            };
            starts.push(start);
        }
        starts
    };
    for (i, p) in mpd.periods.iter().enumerate() {
        let start = starts[i];
        let next_has_start = mpd.periods.get(i + 1).is_some_and(|n| n.start.is_some());
        let early_terminated =
            p.duration.is_some() && (next_has_start || mpd.minimum_update_period.is_some());
        let kind = match start {
            None if mpd.kind == PresentationType::Dynamic => PeriodKind::EarlyAvailable,
            _ if early_terminated => PeriodKind::EarlyTerminated,
            _ => PeriodKind::Regular,
        };
        let next_start = starts.get(i + 1).copied().flatten();
        let end = match (start, p.duration) {
            (Some(s), Some(d)) => Some(s.saturating_add(duration_us(d))),
            (Some(_), None) => next_start.or_else(|| {
                (i + 1 == mpd.periods.len())
                    .then(|| mpd.media_presentation_duration.map(duration_us))
                    .flatten()
            }),
            (None, _) => None,
        };
        out.push(PeriodTiming {
            start_us: start,
            end_us: end,
            kind,
        });
    }
    out
}
