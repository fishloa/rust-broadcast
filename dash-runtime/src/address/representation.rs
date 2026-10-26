//! One call from an MPD position to a ready [`SegmentPlan`]: resolve the
//! inherited Segment information, derive the Period's place on the timeline,
//! gather the BaseURL candidates and build the plan.

use thiserror::Error;
use transmux::SegmentIndexBox;

use super::plan::{DynamicContext, PlanInput, SegmentPlan};
use super::AddressError;
use crate::mpd::inherit::{InheritError, SegmentInfo, resolve_segment_info};
use crate::mpd::model::{Mpd, PresentationType};
use crate::mpd::urls::{BaseCandidate, representation_candidates};
use crate::selection::RepRef;
use crate::timing::availability::ato_seconds_to_us;
use crate::timing::period::{PeriodTiming, duration_us, period_timings};

/// Why a Representation could not be planned.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum PlanError {
    /// `RepRef` names nothing in this MPD.
    #[error("no such Representation")]
    NoSuchRepresentation,
    /// Segment information inheritance failed.
    #[error(transparent)]
    Inherit(#[from] InheritError),
    /// Addressing failed.
    #[error(transparent)]
    Address(#[from] AddressError),
    /// The Period has no PeriodStart yet (Early Available Period).
    #[error("the Period has no PeriodStart")]
    PeriodUnresolved,
    /// A dynamic MPD without `@availabilityStartTime`.
    #[error("dynamic MPD without availabilityStartTime")]
    MissingAvailabilityStartTime,
}

/// Knobs of [`plan_representation`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct PlanOptions<'a> {
    /// Apply `@availabilityTimeOffset` (early requests; LL-DASH).
    pub use_availability_time_offset: bool,
    /// `FetchTime + minimumUpdatePeriod` in UTC microseconds, for a trailing
    /// negative `@r` of a Period with no known end.
    pub horizon_utc_us: Option<i128>,
    /// The parsed `sidx` and its anchor, for a SegmentBase Representation.
    pub index: Option<(&'a SegmentIndexBox, u64)>,
}

/// A planned Representation.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlannedRepresentation {
    /// The segments.
    pub plan: SegmentPlan,
    /// The resolved Segment information.
    pub info: SegmentInfo,
    /// BaseURL candidates, best first.
    pub candidates: Vec<BaseCandidate>,
    /// The Period's timing.
    pub period: PeriodTiming,
}

fn info_tsb(info: &SegmentInfo) -> Option<std::time::Duration> {
    match info {
        SegmentInfo::Template(t) => t.multi.base.time_shift_buffer_depth,
        SegmentInfo::List(l) => l.multi.base.time_shift_buffer_depth,
        SegmentInfo::Base(b) => b.time_shift_buffer_depth,
    }
}

fn info_ato(info: &SegmentInfo) -> Option<f64> {
    match info {
        SegmentInfo::Template(t) => t.multi.base.availability_time_offset,
        SegmentInfo::List(l) => l.multi.base.availability_time_offset,
        SegmentInfo::Base(b) => b.availability_time_offset,
    }
}

/// Plan the Representation at `at`.
///
/// # Errors
/// [`PlanError`]; the caller drops the Representation (or, for
/// `AddressError::IndexRequired`, calls again with `opts.index`).
pub fn plan_representation(
    mpd: &Mpd,
    at: RepRef,
    opts: &PlanOptions<'_>,
) -> Result<PlannedRepresentation, PlanError> {
    let (period, set, rep) = at.resolve(mpd).ok_or(PlanError::NoSuchRepresentation)?;
    let info = resolve_segment_info(period, set, rep)?;
    let timing = *period_timings(mpd)
        .get(at.period)
        .ok_or(PlanError::NoSuchRepresentation)?;
    let start = timing.start_us.ok_or(PlanError::PeriodUnresolved)?;
    let candidates = representation_candidates(mpd, period, set, rep);
    let mut input = PlanInput::new(&rep.id, rep.bandwidth, &info);
    input.period_start_us = start;
    input.period_end_us = timing.end_us;
    input.index = opts.index;
    if mpd.kind == PresentationType::Dynamic {
        let ast = mpd
            .availability_start_time_us
            .ok_or(PlanError::MissingAvailabilityStartTime)?;
        let tsb = info_tsb(&info)
            .or_else(|| {
                rep.base_urls
                    .iter()
                    .find_map(|b| b.time_shift_buffer_depth)
            })
            .or(mpd.time_shift_buffer_depth)
            .map(duration_us);
        let mut ctx = DynamicContext::new(i128::from(ast), tsb);
        if opts.use_availability_time_offset {
            let base_ato = candidates.first().map_or(0.0, |c| c.availability_time_offset_s);
            ctx.availability_time_offset_us =
                ato_seconds_to_us(base_ato + info_ato(&info).unwrap_or(0.0));
        }
        ctx.horizon_utc_us = opts.horizon_utc_us;
        input.dynamic = Some(ctx);
    }
    let plan = SegmentPlan::build(&input)?;
    Ok(PlannedRepresentation {
        plan,
        info,
        candidates,
        period: timing,
    })
}
