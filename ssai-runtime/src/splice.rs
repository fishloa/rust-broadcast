//! Splice-point conditioning: aligning a SCTE-35 cue's target presentation
//! time (`splice_time().pts_time`, ANSI/SCTE 35 2023r1 §9.8.1, or a DASH
//! `emsg`'s `presentation_time`) against the primary content's actual
//! segment/keyframe boundaries.
//!
//! Real cues are frequently **not** IDR-aligned. `fixtures/scte35-ssai/PROVENANCE.md`
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
//! This module works in whatever clock unit the caller's candidates use — it
//! does no 90 kHz-specific math and does not itself decode SCTE-35 or emsg
//! (that's `scte35-splice` / `mp4-emsg`'s job; this crate's core takes plain
//! tick counts so it never needs those crates as a runtime dependency).

use crate::error::{Error, Result};

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
broadcast_common::impl_spec_display!(SnapDirection);

/// The result of conditioning one splice point against a set of candidate
/// boundaries (segment starts, or sync-sample/IDR timestamps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConditionedSplicePoint {
    /// The cue's nominal target, in whatever clock unit the caller's
    /// candidates share (this module does no unit conversion).
    pub requested_pts: u64,
    /// The nearest candidate boundary actually chosen.
    pub snapped_pts: u64,
    /// `|snapped_pts - requested_pts|`.
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

/// Find the candidate boundary nearest `requested_pts`, and error out if the
/// nearest one is farther than `max_delta_ticks` away — the caller decides
/// how much drift is tolerable (a live low-latency splice may need a tight
/// bound; a VOD ad break can tolerate a full GOP).
///
/// `candidates` need not be sorted; every entry is checked. The candidate
/// sets this crate is used with (segment or sync-sample boundaries within
/// one GOP of the cue) are small enough that a linear scan is the right
/// tool: no allocation, no requirement that the caller maintain sorted
/// order.
///
/// Returns [`Error::NoCandidates`] if `candidates` is empty, and
/// [`Error::NoAlignedBoundary`] if the nearest candidate exceeds
/// `max_delta_ticks` — conditioning never silently accepts a splice point
/// nothing is actually close to.
pub fn condition_splice_point(
    requested_pts: u64,
    candidates: &[u64],
    max_delta_ticks: u64,
) -> Result<ConditionedSplicePoint> {
    let mut nearest: Option<(u64, u64)> = None; // (candidate, delta)
    for &c in candidates {
        let delta = c.abs_diff(requested_pts);
        if nearest.is_none_or(|(_, best)| delta < best) {
            nearest = Some((c, delta));
        }
    }
    let (snapped_pts, delta_ticks) = nearest.ok_or(Error::NoCandidates)?;
    if delta_ticks > max_delta_ticks {
        return Err(Error::NoAlignedBoundary {
            requested_pts,
            tolerance_ticks: max_delta_ticks,
            nearest_delta_ticks: delta_ticks,
        });
    }
    let direction = if snapped_pts == requested_pts {
        SnapDirection::Exact
    } else if snapped_pts < requested_pts {
        SnapDirection::Before
    } else {
        SnapDirection::After
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

    #[test]
    fn snaps_to_nearest_candidate() {
        let result = condition_splice_point(1_000, &[500, 1_050, 2_000], 100).unwrap();
        assert_eq!(result.snapped_pts, 1_050);
        assert_eq!(result.delta_ticks, 50);
        assert_eq!(result.direction, SnapDirection::After);
        assert!(!result.is_exact());
    }

    #[test]
    fn exact_match_reports_zero_delta() {
        let result = condition_splice_point(1_000, &[1_000], 0).unwrap();
        assert_eq!(result.delta_ticks, 0);
        assert_eq!(result.direction, SnapDirection::Exact);
        assert!(result.is_exact());
    }

    #[test]
    fn before_direction_when_candidate_precedes_request() {
        let result = condition_splice_point(1_000, &[900], 200).unwrap();
        assert_eq!(result.snapped_pts, 900);
        assert_eq!(result.direction, SnapDirection::Before);
    }

    #[test]
    fn rejects_a_candidate_outside_tolerance() {
        let err = condition_splice_point(1_000, &[2_000], 500).unwrap_err();
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
        let err = condition_splice_point(1_000, &[], 500).unwrap_err();
        assert!(matches!(err, Error::NoCandidates));
    }

    #[test]
    fn direction_labels() {
        assert_eq!(SnapDirection::Exact.name(), "exact");
        assert_eq!(alloc::format!("{}", SnapDirection::After), "after");
    }
}
