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

/// The 33-bit modulus of an MPEG-2 PTS/DTS and of a SCTE-35 `pts_time`
/// (ISO/IEC 13818-1 §2.4.3.7 `PTS[32..0]`; ANSI/SCTE 35 2023r1 §9.8.1
/// `pts_time` is a 33-bit field), in 90 kHz ticks: 2^33 ≈ 26.5 hours.
pub const PTS_MODULUS_33: u64 = 1 << 33;

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
        // Equidistant candidates: prefer the one after the request (a splice
        // is cut at or after the cue, never before it, when there is a
        // choice) — so the result does not depend on slice order.
        if nearest.is_none_or(|(best_c, best)| {
            delta < best || (delta == best && c > requested_pts && best_c < requested_pts)
        }) {
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

/// [`condition_splice_point`] for a **wrapping** clock — the 33-bit PTS of a
/// transport stream or of a SCTE-35 `pts_time` (pass [`PTS_MODULUS_33`]) —
/// issue #1125 / audit r14-SSAI-W2.
///
/// Distances and the snap direction are measured on the circle of
/// `modulus` ticks: a cue at `2^33 - 100` whose nearest real boundary is at
/// `50` (just after the wrap, about every 26.5 h on a 24/7 channel) is
/// 150 ticks [`SnapDirection::After`], where the linear version measures
/// ~8.6 × 10⁹ ticks and drops the break. The shorter way round wins; on an
/// exact half-circle tie the forward ([`SnapDirection::After`]) way is
/// taken. [`ConditionedSplicePoint::delta_ticks`] is that circular distance.
///
/// Returns [`Error::PtsOutOfRange`] if `modulus` is zero or `requested_pts`
/// or any candidate is not `< modulus` (a value already wrapped is the
/// contract; silently reducing it would hide an unrolled timeline).
pub fn condition_splice_point_wrapping(
    requested_pts: u64,
    candidates: &[u64],
    max_delta_ticks: u64,
    modulus: u64,
) -> Result<ConditionedSplicePoint> {
    if modulus == 0 || requested_pts >= modulus {
        return Err(Error::PtsOutOfRange {
            pts: requested_pts,
            modulus,
        });
    }
    let m = u128::from(modulus);
    let r = u128::from(requested_pts);
    // (candidate, circular delta, forward?)
    let mut nearest: Option<(u64, u64, bool)> = None;
    for &c in candidates {
        if c >= modulus {
            return Err(Error::PtsOutOfRange { pts: c, modulus });
        }
        let cw = u128::from(c);
        let forward = (cw + m - r) % m;
        let backward = (r + m - cw) % m;
        let (delta, is_forward) = if forward <= backward {
            (forward, true)
        } else {
            (backward, false)
        };
        // `delta <= modulus / 2` always fits a u64; saturate rather than cast.
        let delta = u64::try_from(delta).unwrap_or(u64::MAX);
        // Equidistant candidates: prefer `After` (see
        // `condition_splice_point`), independent of slice order.
        if nearest.is_none_or(|(_, best, best_forward)| {
            delta < best || (delta == best && is_forward && !best_forward)
        }) {
            nearest = Some((c, delta, is_forward));
        }
    }
    let (snapped_pts, delta_ticks, is_forward) = nearest.ok_or(Error::NoCandidates)?;
    if delta_ticks > max_delta_ticks {
        return Err(Error::NoAlignedBoundary {
            requested_pts,
            tolerance_ticks: max_delta_ticks,
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

    /// Audit r14-SSAI-W2: cue just before the 33-bit wrap, boundary just
    /// after it. The linear function drops the break; the wrapping one
    /// reports the true 150-tick forward distance.
    #[test]
    fn wrapping_snaps_across_the_33_bit_boundary() {
        let cue = PTS_MODULUS_33 - 100;
        assert!(matches!(
            condition_splice_point(cue, &[50], 1_000),
            Err(Error::NoAlignedBoundary { .. })
        ));
        let r = condition_splice_point_wrapping(cue, &[50], 1_000, PTS_MODULUS_33).unwrap();
        assert_eq!(r.snapped_pts, 50);
        assert_eq!(r.delta_ticks, 150);
        assert_eq!(r.direction, SnapDirection::After);
    }

    #[test]
    fn wrapping_direction_before_across_the_wrap() {
        let r = condition_splice_point_wrapping(50, &[PTS_MODULUS_33 - 100], 1_000, PTS_MODULUS_33)
            .unwrap();
        assert_eq!(r.delta_ticks, 150);
        assert_eq!(r.direction, SnapDirection::Before);
    }

    #[test]
    fn wrapping_picks_nearest_among_both_sides_of_the_wrap() {
        let cue = PTS_MODULUS_33 - 100;
        let r = condition_splice_point_wrapping(
            cue,
            &[PTS_MODULUS_33 - 500, 90, 4_000],
            10_000,
            PTS_MODULUS_33,
        )
        .unwrap();
        assert_eq!(r.snapped_pts, 90);
        assert_eq!(r.delta_ticks, 190);
        let r = condition_splice_point_wrapping(
            cue,
            &[PTS_MODULUS_33 - 150, 90],
            10_000,
            PTS_MODULUS_33,
        )
        .unwrap();
        assert_eq!(r.snapped_pts, PTS_MODULUS_33 - 150);
        assert_eq!(r.delta_ticks, 50);
        assert_eq!(r.direction, SnapDirection::Before);
    }

    #[test]
    fn wrapping_exact_and_tolerance_and_tie() {
        let r = condition_splice_point_wrapping(7, &[7], 0, PTS_MODULUS_33).unwrap();
        assert_eq!(r.direction, SnapDirection::Exact);
        assert!(r.is_exact());
        let err = condition_splice_point_wrapping(PTS_MODULUS_33 - 100, &[50], 149, PTS_MODULUS_33)
            .unwrap_err();
        assert!(matches!(
            err,
            Error::NoAlignedBoundary {
                nearest_delta_ticks: 150,
                tolerance_ticks: 149,
                ..
            }
        ));
        let r = condition_splice_point_wrapping(0, &[5], 5, 10).unwrap();
        assert_eq!((r.delta_ticks, r.direction), (5, SnapDirection::After));
    }

    #[test]
    fn wrapping_rejects_out_of_range_inputs_and_empty_sets() {
        assert!(matches!(
            condition_splice_point_wrapping(PTS_MODULUS_33, &[1], 10, PTS_MODULUS_33),
            Err(Error::PtsOutOfRange { .. })
        ));
        assert!(matches!(
            condition_splice_point_wrapping(1, &[PTS_MODULUS_33], 10, PTS_MODULUS_33),
            Err(Error::PtsOutOfRange { .. })
        ));
        assert!(matches!(
            condition_splice_point_wrapping(0, &[0], 10, 0),
            Err(Error::PtsOutOfRange { .. })
        ));
        assert!(matches!(
            condition_splice_point_wrapping(1, &[], 10, PTS_MODULUS_33),
            Err(Error::NoCandidates)
        ));
        // Hostile: a modulus near u64::MAX must not overflow.
        let r = condition_splice_point_wrapping(u64::MAX - 2, &[1], u64::MAX, u64::MAX).unwrap();
        assert_eq!(r.delta_ticks, 3);
    }

    /// Two candidates equally far either side of the cue: `After` wins, in
    /// either slice order, for both the linear and the wrapping form.
    #[test]
    fn an_equidistant_tie_prefers_after_regardless_of_order() {
        for candidates in [[900u64, 1_100], [1_100, 900]] {
            let r = condition_splice_point(1_000, &candidates, 200).unwrap();
            assert_eq!((r.snapped_pts, r.direction), (1_100, SnapDirection::After));
        }
        for candidates in [[900u64, 1_100], [1_100, 900]] {
            let r =
                condition_splice_point_wrapping(1_000, &candidates, 200, PTS_MODULUS_33).unwrap();
            assert_eq!((r.snapped_pts, r.direction), (1_100, SnapDirection::After));
        }
        // Across the wrap: 100 ticks either side of 0.
        for candidates in [[PTS_MODULUS_33 - 100, 100], [100, PTS_MODULUS_33 - 100]] {
            let r = condition_splice_point_wrapping(0, &candidates, 200, PTS_MODULUS_33).unwrap();
            assert_eq!((r.snapped_pts, r.direction), (100, SnapDirection::After));
        }
    }

    #[test]
    fn direction_labels() {
        assert_eq!(SnapDirection::Exact.name(), "exact");
        assert_eq!(alloc::format!("{}", SnapDirection::After), "after");
    }
}
