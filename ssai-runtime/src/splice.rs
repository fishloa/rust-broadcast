//! Splice-point conditioning: aligning a SCTE-35 cue's target presentation
//! time (`splice_time().pts_time`, ANSI/SCTE 35 2023r1 §9.8.1, or a DASH
//! `emsg`'s `presentation_time`) against the primary content's actual
//! segment/keyframe boundaries.
//!
//! Real cues are frequently **not** IDR-aligned. `fixtures/scte35/PROVENANCE.md`
//! documents a genuine DASH-IF `livesim2` capture whose nearest video
//! keyframe lands 6000 ticks (67 ms) *after* the cue's nominal presentation
//! time at the shared 90 kHz clock — see `ssai-runtime/examples/condition_real_cue.rs`,
//! which reproduces that exact measurement through this module rather than
//! asserting it. [`condition_splice_point`] measures that offset instead of
//! assuming it away, so a caller (the playlist renderer, or a transmux-side
//! splicer) can make an informed choice: snap to the actual boundary and
//! accept the drift, or reject the cue as un-splice-able when nothing is
//! close enough.
//!
//! Every instant here is a [`Clock33`] — a validated 90 kHz PTS/`pts_time`
//! sample on the 33-bit circle (ISO/IEC 13818-1 §2.4.3.7 `PTS[32..0]`,
//! ANSI/SCTE 35 2023r1 §9.8.1), which wraps about every 26.5 h — and the
//! arithmetic goes through `broadcast_common::clock33`, so there is exactly
//! one conditioning algorithm and it measures across the wrap. A linear twin
//! beside it was tried and dropped (audit 19, issue #2726): a caller picking
//! it measured a cue just before the wrap against a boundary just after it
//! as ~8.6 × 10⁹ ticks away and dropped the break. This module does not
//! decode SCTE-35 or emsg (that's `scte35-splice` / `isobmff`'s job).

use broadcast_common::clock33::{Clock33, WRAP_33BIT};
use broadcast_core::timescale::{self, Rounding, Timescale};
use core::time::Duration;

use crate::error::{Error, Result};

/// The 90 kHz clock PTS/DTS and a SCTE-35 `pts_time` are sampled into
/// (ISO/IEC 13818-1 §2.4.3.7; ANSI/SCTE 35 2023r1 §9.8.1): the unit of every
/// [`Clock33`] in this module's API and of a tolerance [`Duration`].
const PTS_TIMESCALE: Timescale = broadcast_common::clock33::PTS_TIMESCALE;

/// Where the chosen boundary landed relative to the requested instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapDirection {
    /// The candidate matches the requested instant exactly.
    Exact,
    /// The candidate is before the requested instant.
    Before,
    /// The candidate is after the requested instant — the common case for a
    /// real, non-IDR-aligned cue (see the module docs).
    After,
}

impl SnapDirection {
    /// Stable label.
    pub fn name(&self) -> &'static str {
        match self {
            SnapDirection::Exact => "exact",
            SnapDirection::Before => "before",
            SnapDirection::After => "after",
        }
    }
}
broadcast_core::impl_spec_display!(SnapDirection);

/// The result of conditioning one splice point against a set of candidate
/// boundaries (segment starts, or sync-sample/IDR timestamps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConditionedSplicePoint {
    /// The cue's nominal target.
    pub requested_pts: Clock33,
    /// The nearest candidate boundary actually chosen.
    pub snapped_pts: Clock33,
    /// The circular distance from `requested_pts` to `snapped_pts`, in 90 kHz
    /// ticks — at most `2^32` (half the circle).
    pub delta_ticks: u64,
    /// Whether the snap landed before, after, or exactly on the request.
    pub direction: SnapDirection,
}

impl ConditionedSplicePoint {
    /// Whether the snap was exact (no drift at all).
    pub fn is_exact(&self) -> bool {
        self.delta_ticks == 0
    }
}

/// The tolerance `max_delta` in 90 kHz ticks, rounded up: a tolerance the
/// caller expressed `Floor`-ed down could reject a boundary that is exactly
/// at their stated drift.
fn tolerance_ticks(max_delta: Duration) -> u64 {
    let nanos = i128::try_from(max_delta.as_nanos()).unwrap_or(i128::MAX);
    let ticks = timescale::rescale(nanos, Timescale::NANOS, PTS_TIMESCALE, Rounding::Ceil);
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// Find the candidate boundary nearest `requested_pts`, and error out if the
/// nearest one is farther than `max_delta` away — the caller decides how
/// much drift is tolerable (a live low-latency splice may need a tight
/// bound; a VOD ad break can tolerate a full GOP).
///
/// Every instant is a 90 kHz 33-bit PTS/`pts_time`, and distances and the
/// snap direction are measured on that circle (issue #1125 / audit
/// r14-SSAI-W2): a cue at `2^33 - 100` whose nearest real boundary is at
/// `50` (just after the wrap, about every 26.5 h on a 24/7 channel) is 150
/// ticks [`SnapDirection::After`], where a linear measure finds ~8.6 × 10⁹
/// ticks and drops the break. The shorter way round wins; on an exact
/// half-circle tie the forward ([`SnapDirection::After`]) way is taken.
/// [`ConditionedSplicePoint::delta_ticks`] is that circular distance.
///
/// `candidates` need not be sorted; every entry is checked, and the result
/// does not depend on slice order. The candidate sets this crate is used
/// with (segment or sync-sample boundaries within one GOP of the cue) are
/// small enough that a linear scan is the right tool: no allocation, no
/// requirement that the caller maintain sorted order.
///
/// Returns [`Error::NoCandidates`] if `candidates` is empty, and
/// [`Error::NoAlignedBoundary`] if the nearest candidate exceeds
/// `max_delta` — conditioning never silently accepts a splice point nothing
/// is actually close to.
pub fn condition_splice_point(
    requested_pts: Clock33,
    candidates: &[Clock33],
    max_delta: Duration,
) -> Result<ConditionedSplicePoint> {
    let r = u128::from(requested_pts.ticks());
    let modulus = u128::from(WRAP_33BIT);
    let tolerance_ticks = tolerance_ticks(max_delta);
    // (candidate, circular delta, forward?)
    let mut nearest: Option<(Clock33, u64, bool)> = None;
    for &candidate in candidates {
        let c = u128::from(candidate.ticks());
        let forward = (c + modulus - r) % modulus;
        let backward = (r + modulus - c) % modulus;
        let (delta, is_forward) = if forward <= backward {
            (forward, true)
        } else {
            (backward, false)
        };
        // Both are below the modulus, so the shorter leg fits a u64.
        let delta = u64::try_from(delta).unwrap_or(u64::MAX);
        // Equidistant candidates: prefer the one after the request (a splice
        // is cut at or after the cue, never before it, when there is a
        // choice) — so the result does not depend on slice order.
        if nearest.is_none_or(|(_, best, best_forward)| {
            delta < best || (delta == best && is_forward && !best_forward)
        }) {
            nearest = Some((candidate, delta, is_forward));
        }
    }
    let (snapped_pts, delta_ticks, is_forward) = nearest.ok_or(Error::NoCandidates)?;
    if delta_ticks > tolerance_ticks {
        return Err(Error::NoAlignedBoundary {
            requested_pts: requested_pts.ticks(),
            tolerance_ticks,
            nearest_delta_ticks: delta_ticks,
        });
    }
    let direction = if delta_ticks == 0 {
        SnapDirection::Exact
    } else if is_forward {
        SnapDirection::After
    } else {
        SnapDirection::Before
    };
    Ok(ConditionedSplicePoint {
        requested_pts,
        snapped_pts,
        delta_ticks,
        direction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn pts(ticks: u64) -> Clock33 {
        match Clock33::try_new(ticks) {
            Ok(c) => c,
            Err(_) => Clock33::ZERO,
        }
    }

    /// A tolerance of `ticks` 90 kHz ticks. Exact: `ticks * 10^9` is divisible
    /// by 90 000 for every `ticks` the tests use.
    fn ticks(n: u64) -> Duration {
        Duration::from_nanos(n * 1_000_000_000 / 90_000)
    }

    /// The 33-bit modulus, for expressing "just before the wrap".
    const WRAP: u64 = broadcast_common::clock33::WRAP_33BIT;

    #[test]
    fn snaps_to_nearest_candidate() {
        let result =
            condition_splice_point(pts(1_000), &[pts(500), pts(1_050), pts(2_000)], ticks(100))
                .unwrap();
        assert_eq!(result.snapped_pts, pts(1_050));
        assert_eq!(result.delta_ticks, 50);
        assert_eq!(result.direction, SnapDirection::After);
        assert!(!result.is_exact());
    }

    #[test]
    fn exact_match_reports_zero_delta() {
        let result = condition_splice_point(pts(1_000), &[pts(1_000)], Duration::ZERO).unwrap();
        assert_eq!(result.delta_ticks, 0);
        assert_eq!(result.direction, SnapDirection::Exact);
        assert!(result.is_exact());
    }

    #[test]
    fn before_direction_when_candidate_precedes_request() {
        let result = condition_splice_point(pts(1_000), &[pts(900)], ticks(200)).unwrap();
        assert_eq!(result.snapped_pts, pts(900));
        assert_eq!(result.direction, SnapDirection::Before);
    }

    #[test]
    fn rejects_a_candidate_outside_tolerance() {
        let err = condition_splice_point(pts(1_000), &[pts(2_000)], ticks(500)).unwrap_err();
        match err {
            Error::NoAlignedBoundary {
                requested_pts,
                tolerance_ticks,
                nearest_delta_ticks,
            } => {
                assert_eq!(requested_pts, 1_000);
                assert_eq!(tolerance_ticks, 500);
                assert_eq!(nearest_delta_ticks, 1_000);
            }
            other => panic!("expected NoAlignedBoundary, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_candidates() {
        let err = condition_splice_point(pts(1_000), &[], ticks(500)).unwrap_err();
        assert!(matches!(err, Error::NoCandidates));
    }

    /// Audit r14-SSAI-W2: cue just before the 33-bit wrap, boundary just after
    /// it. There is one conditioning algorithm and it reports the true 150-tick
    /// forward distance rather than dropping the break (audit 19, issue #2726:
    /// the linear twin that used to sit beside it failed exactly here).
    #[test]
    fn snaps_across_the_33_bit_boundary() {
        let cue = pts(WRAP - 100);
        let r = condition_splice_point(cue, &[pts(50)], ticks(1_000)).unwrap();
        assert_eq!(r.snapped_pts, pts(50));
        assert_eq!(r.delta_ticks, 150);
        assert_eq!(r.direction, SnapDirection::After);
    }

    #[test]
    fn direction_before_across_the_wrap() {
        let r = condition_splice_point(pts(50), &[pts(WRAP - 100)], ticks(1_000)).unwrap();
        assert_eq!(r.delta_ticks, 150);
        assert_eq!(r.direction, SnapDirection::Before);
    }

    #[test]
    fn picks_nearest_among_both_sides_of_the_wrap() {
        let cue = pts(WRAP - 100);
        let r = condition_splice_point(cue, &[pts(WRAP - 500), pts(90), pts(4_000)], ticks(10_000))
            .unwrap();
        assert_eq!(r.snapped_pts, pts(90));
        assert_eq!(r.delta_ticks, 190);
        let r = condition_splice_point(cue, &[pts(WRAP - 150), pts(90)], ticks(10_000)).unwrap();
        assert_eq!(r.snapped_pts, pts(WRAP - 150));
        assert_eq!(r.delta_ticks, 50);
        assert_eq!(r.direction, SnapDirection::Before);
    }

    #[test]
    fn exact_and_tolerance_and_tie() {
        let r = condition_splice_point(pts(7), &[pts(7)], Duration::ZERO).unwrap();
        assert_eq!(r.direction, SnapDirection::Exact);
        assert!(r.is_exact());
        let err = condition_splice_point(pts(WRAP - 100), &[pts(50)], ticks(149)).unwrap_err();
        assert!(matches!(
            err,
            Error::NoAlignedBoundary {
                nearest_delta_ticks: 150,
                tolerance_ticks: 149,
                ..
            }
        ));
        let r = condition_splice_point(Clock33::ZERO, &[pts(5)], ticks(5)).unwrap();
        assert_eq!((r.delta_ticks, r.direction), (5, SnapDirection::After));
    }

    /// A tolerance is a `Duration` on the 90 kHz clock, rounded up: a fraction of
    /// a tick is still a tick of tolerance, and a value too large for the clock
    /// simply accepts any candidate.
    #[test]
    fn a_duration_tolerance_is_converted_to_ticks() {
        // One nanosecond is ~10^-5 of a 90 kHz tick: rounded up, it is one tick
        // of tolerance, and rounded down it would be zero (and reject a 1-tick
        // drift).
        let r = condition_splice_point(pts(1_000), &[pts(1_001)], Duration::from_nanos(1)).unwrap();
        assert_eq!(r.delta_ticks, 1);
        // The shorter way round the circle from 1000 to just before the wrap
        // is backwards, 1100 ticks.
        let r = condition_splice_point(pts(1_000), &[pts(WRAP - 100)], Duration::MAX).unwrap();
        assert_eq!(r.delta_ticks, 1_100);
        assert_eq!(r.direction, SnapDirection::Before);
    }

    /// Two candidates equally far either side of the cue: `After` wins, in
    /// either slice order.
    #[test]
    fn an_equidistant_tie_prefers_after_regardless_of_order() {
        for candidates in [[pts(900), pts(1_100)], [pts(1_100), pts(900)]] {
            let r = condition_splice_point(pts(1_000), &candidates, ticks(200)).unwrap();
            assert_eq!(
                (r.snapped_pts, r.direction),
                (pts(1_100), SnapDirection::After)
            );
        }
        // Across the wrap: 100 ticks either side of 0.
        for candidates in [[pts(WRAP - 100), pts(100)], [pts(100), pts(WRAP - 100)]] {
            let r = condition_splice_point(Clock33::ZERO, &candidates, ticks(200)).unwrap();
            assert_eq!(
                (r.snapped_pts, r.direction),
                (pts(100), SnapDirection::After)
            );
        }
    }

    #[test]
    fn direction_labels() {
        assert_eq!(SnapDirection::Exact.name(), "exact");
        assert_eq!(alloc::format!("{}", SnapDirection::After), "after");
    }
}
