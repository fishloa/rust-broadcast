//! SCTE-35 emission points a transition implies (issue
//! [#748](https://github.com/fishloa/rust-broadcast/issues/748)):
//! `splice_insert()` — ANSI/SCTE 35 2023r1 §9.7.3, Table 10 — built and
//! serialized with `scte35-splice`'s own
//! [`Serialize`](broadcast_common::Serialize), not hand-assembled bytes.
//!
//! This module decides *what* cue to emit and *where* (after conditioning);
//! it does not decide *how the splice lands* against real segment/keyframe
//! boundaries — that is [`ssai_runtime::splice::condition_splice_point_wrapping`],
//! reused here rather than re-implemented, per the crate-root docs. Two
//! implementations of boundary conditioning could disagree about the same
//! boundary, which is exactly the bug class this workspace keeps finding.

use crate::error::Result;
use alloc::vec::Vec;
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::AnyCommand;
use scte35_splice::commands::SpliceInsert;
use scte35_splice::time::{BreakDuration, SpliceTime};
pub use ssai_runtime::splice::ConditionedSplicePoint;
use ssai_runtime::splice::PTS_MODULUS_33;

/// Which edge of an ad break a transition represents.
///
/// A [`crate::schedule::Schedule`] alone (Programme/Ad/Slate) does not
/// disambiguate "ad -> ad" within a multi-spot break from "ad -> programme"
/// return, so the caller supplies this explicitly rather than this crate
/// guessing it from entry kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BreakEdge {
    /// Entering a break — `out_of_network_indicator = true` (§9.7.3).
    Enter,
    /// Returning from a break — `out_of_network_indicator = false`.
    Return,
}

impl BreakEdge {
    /// Stable label.
    pub fn name(&self) -> &'static str {
        match self {
            BreakEdge::Enter => "enter",
            BreakEdge::Return => "return",
        }
    }
}
broadcast_common::impl_spec_display!(BreakEdge);

/// Build a `splice_insert()` command for a transition, snapping its target
/// instant onto the nearest of `candidates` (real segment or keyframe
/// boundaries) within `max_delta_ticks` of `requested_pts` — see
/// [`ssai_runtime::splice::condition_splice_point_wrapping`], which this delegates to
/// rather than duplicating. Errors (via [`crate::Error::SpliceConditioning`])
/// if no candidate is close enough, or none are supplied: this crate never
/// emits a cue for a splice point nothing is actually close to.
///
/// Distances are measured modulo 2^33 (the width of the SCTE-35 `pts_time`):
/// `delta_ticks`/`direction` are circular (a cue at `2^33 - 100` snapping to a
/// boundary at `50` is 150 ticks `After`). The returned
/// [`ConditionedSplicePoint`] reports `requested_pts` and `snapped_pts` in the
/// caller's own units (an unwrapped channel clock stays unwrapped).
///
/// `break_duration_ticks`, if given, sets `break_duration().duration` with
/// `auto_return = true` (the splicer returns to the network feed on its own
/// once the duration elapses — §9.7.3).
///
/// # Units (mandatory — PLAY-W1, #1126)
///
/// `requested_pts`, every entry of `candidates`, `max_delta_ticks` and
/// `break_duration_ticks` **must** already be counts of 90 kHz ticks:
/// `splice_time()`/`break_duration()` are fixed 90 kHz fields (ANSI/SCTE 35
/// 2023r1 §9.8.1/§9.8.2), and this function passes every one of these values
/// straight through to [`SpliceTime::with_pts`]/[`BreakDuration`] with no
/// unit conversion or validation. Passing a channel clock in any other unit
/// (27 MHz, milliseconds, nanoseconds, …) produces a structurally valid cue
/// that is silently wrong by that unit's fixed conversion factor. Convert
/// with `scte35_splice::time::duration_to_ticks` first if the channel clock
/// isn't already 90 kHz.
pub fn build_splice_insert(
    edge: BreakEdge,
    splice_event_id: u32,
    requested_pts: u64,
    candidates: &[u64],
    max_delta_ticks: u64,
    break_duration_ticks: Option<u64>,
) -> Result<(ConditionedSplicePoint, SpliceInsert)> {
    // `splice_time().pts_time` is a 33-bit field (ANSI/SCTE 35 §9.8.1;
    // `SpliceTime::with_pts` masks to it), so the instant a cue names — and the
    // boundary it snaps to — live on the circle of 2^33 ticks, wrapping about
    // every 26.5 h on a 24/7 channel: a cue just before the wrap whose nearest
    // real boundary is just after it is 150 ticks away, not ~8.6e9. Reduce
    // every input onto that circle and measure the circular distance
    // (ssai-runtime audit r14-SSAI-W2).
    let reduced: Vec<u64> = candidates.iter().map(|c| c % PTS_MODULUS_33).collect();
    let mut conditioned = ssai_runtime::splice::condition_splice_point_wrapping(
        requested_pts % PTS_MODULUS_33,
        &reduced,
        max_delta_ticks,
        PTS_MODULUS_33,
    )?;
    // Report in the caller's units: the instant they asked for and the
    // candidate they supplied (an unwrapped channel clock stays unwrapped),
    // with the circular delta/direction measured above.
    if let Some(original) = candidates
        .iter()
        .zip(&reduced)
        .find_map(|(orig, red)| (*red == conditioned.snapped_pts).then_some(*orig))
    {
        conditioned.snapped_pts = original;
    }
    conditioned.requested_pts = requested_pts;
    let insert = SpliceInsert {
        splice_event_id,
        splice_event_cancel_indicator: false,
        out_of_network_indicator: matches!(edge, BreakEdge::Enter),
        program_splice_flag: true,
        splice_immediate_flag: false,
        event_id_compliance_flag: true,
        splice_time: Some(SpliceTime::with_pts(conditioned.snapped_pts)),
        components: Vec::new(),
        break_duration: break_duration_ticks.map(|duration| BreakDuration {
            auto_return: true,
            duration,
        }),
        unique_program_id: 0,
        avail_num: 0,
        avails_expected: 0,
    };
    Ok((conditioned, insert))
}

/// Wrap a built [`SpliceInsert`] into a clear (unencrypted)
/// `splice_info_section()`, ready to serialize
/// (`broadcast_common::Serialize::to_bytes`) onto the SCTE-35 elementary
/// stream/PID.
#[must_use]
pub fn to_section<'a>(insert: SpliceInsert) -> SpliceInfoSection<'a> {
    SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(insert), &[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};
    use ssai_runtime::Error as SsaiError;

    #[test]
    fn builds_an_enter_break_cue_snapped_to_the_conditioned_point() {
        let (conditioned, insert) = build_splice_insert(
            BreakEdge::Enter,
            7,
            1_000,
            &[1_050, 2_000],
            100,
            Some(1_800_000),
        )
        .unwrap();
        assert_eq!(conditioned.snapped_pts, 1_050);
        assert_eq!(insert.splice_event_id, 7);
        assert!(insert.out_of_network_indicator);
        assert_eq!(insert.splice_time.unwrap().pts_time, Some(1_050));
        assert_eq!(insert.break_duration.unwrap().duration, 1_800_000);
        assert!(insert.break_duration.unwrap().auto_return);

        // The cue must reflect the *conditioned* point, not the raw request
        // — proves this isn't a passthrough of `requested_pts`.
        assert_ne!(insert.splice_time.unwrap().pts_time, Some(1_000));
    }

    #[test]
    fn builds_a_return_break_cue_with_no_duration() {
        let (_, insert) =
            build_splice_insert(BreakEdge::Return, 8, 1_000, &[1_000], 0, None).unwrap();
        assert!(!insert.out_of_network_indicator);
        assert!(insert.break_duration.is_none());
    }

    #[test]
    fn rejects_a_cue_with_no_boundary_within_tolerance() {
        let err = build_splice_insert(BreakEdge::Enter, 1, 1_000, &[5_000], 10, None).unwrap_err();
        match err {
            crate::Error::SpliceConditioning(SsaiError::NoAlignedBoundary {
                requested_pts,
                tolerance_ticks,
                nearest_delta_ticks,
            }) => {
                assert_eq!(requested_pts, 1_000);
                assert_eq!(tolerance_ticks, 10);
                assert_eq!(nearest_delta_ticks, 4_000);
            }
            other => panic!("expected SpliceConditioning(NoAlignedBoundary), got {other:?}"),
        }
    }

    #[test]
    fn section_round_trips_through_scte35_splices_own_serialize() {
        let (_, insert) =
            build_splice_insert(BreakEdge::Enter, 42, 1_000, &[1_000], 0, Some(900_000)).unwrap();
        let section = to_section(insert);
        let bytes = section.to_bytes();
        assert_eq!(bytes[0], scte35_splice::section::TABLE_ID);

        let parsed = SpliceInfoSection::parse(&bytes).unwrap();
        match parsed.clear.unwrap().command {
            AnyCommand::SpliceInsert(reparsed) => {
                assert_eq!(reparsed.splice_event_id, 42);
                assert_eq!(reparsed.splice_time.unwrap().pts_time, Some(1_000));
                assert_eq!(reparsed.break_duration.unwrap().duration, 900_000);
            }
            other => panic!("expected SpliceInsert, got {other:?}"),
        }
    }

    #[test]
    fn label_convention() {
        assert_eq!(BreakEdge::Enter.name(), "enter");
        assert_eq!(alloc::format!("{}", BreakEdge::Return), "return");
    }

    /// PLAY-W1 follow-up: the cue's `pts_time` is 33-bit, so a requested
    /// instant just before the wrap whose nearest real boundary is just after
    /// it is a 150-tick `After` snap, not an out-of-tolerance miss.
    #[test]
    fn a_cue_before_the_33_bit_wrap_snaps_to_a_boundary_after_it() {
        let requested = PTS_MODULUS_33 - 100;
        let (conditioned, insert) =
            build_splice_insert(BreakEdge::Enter, 9, requested, &[50], 1_000, None).unwrap();
        assert_eq!(conditioned.snapped_pts, 50);
        assert_eq!(conditioned.delta_ticks, 150);
        assert_eq!(conditioned.direction, ssai_runtime::SnapDirection::After);
        assert_eq!(insert.splice_time.unwrap().pts_time, Some(50));
        // Too tight a tolerance is still refused (circularly measured).
        assert!(matches!(
            build_splice_insert(BreakEdge::Enter, 9, requested, &[50], 149, None),
            Err(crate::Error::SpliceConditioning(
                SsaiError::NoAlignedBoundary {
                    nearest_delta_ticks: 150,
                    ..
                }
            ))
        ));
    }

    /// A channel clock already unrolled past 2^33 is reduced onto the
    /// SCTE-35 circle before conditioning: boundary just before the wrap,
    /// request just after it, in unwrapped terms.
    #[test]
    fn an_unwrapped_channel_clock_is_reduced_to_the_33_bit_circle() {
        let requested = PTS_MODULUS_33 + 10;
        let (conditioned, _) = build_splice_insert(
            BreakEdge::Return,
            1,
            requested,
            &[PTS_MODULUS_33 - 5],
            1_000,
            None,
        )
        .unwrap();
        // Reported in the caller's (unwrapped) units.
        assert_eq!(conditioned.requested_pts, requested);
        assert_eq!(conditioned.snapped_pts, PTS_MODULUS_33 - 5);
        assert_eq!(conditioned.delta_ticks, 15);
        assert_eq!(conditioned.direction, ssai_runtime::SnapDirection::Before);
    }
}
