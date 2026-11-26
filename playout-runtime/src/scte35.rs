//! SCTE-35 emission points a transition implies (issue
//! [#748](https://github.com/fishloa/rust-broadcast/issues/748)):
//! `splice_insert()` — ANSI/SCTE 35 2023r1 §9.7.3, Table 10 — built and
//! serialized with `scte35-splice`'s own
//! [`Serialize`](broadcast_core::Serialize), not hand-assembled bytes.
//!
//! This module decides *what* cue to emit and *where* (after conditioning);
//! it does not decide *how the splice lands* against real segment/keyframe
//! boundaries — that is [`ssai_runtime::splice::condition_splice_point`],
//! reused here rather than re-implemented, per the crate-root docs. Two
//! implementations of boundary conditioning could disagree about the same
//! boundary, which is exactly the bug class this workspace keeps finding.

use crate::error::Result;
use broadcast_common::clock33::{self, Clock33};
use core::time::Duration;
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::AnyCommand;
use scte35_splice::commands::{ProgramSplice, SpliceInsert, SpliceMode};
use scte35_splice::descriptor_loop::SpliceDescriptorLoop;
use scte35_splice::time::{BreakDuration, SpliceTime, duration_to_ticks};
pub use ssai_runtime::splice::ConditionedSplicePoint;

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
broadcast_core::impl_spec_display!(BreakEdge);

/// Build a `splice_insert()` command for a transition, snapping its target
/// instant onto the nearest of `candidates` (real segment or keyframe
/// boundaries) within `max_delta` of `requested_pts` — see
/// [`ssai_runtime::splice::condition_splice_point`], which this delegates to
/// rather than duplicating. Errors (via [`crate::Error::SpliceConditioning`])
/// if no candidate is close enough, or none are supplied: this crate never
/// emits a cue for a splice point nothing is actually close to.
///
/// `requested_pts` is reduced onto the 33-bit PTS circle `splice_time()`'s
/// `pts_time` lives on (ANSI/SCTE 35 §9.8.1) before conditioning, and the
/// returned [`ConditionedSplicePoint`] reports the reduced instant and
/// boundary alongside the `SpliceInsert`'s `splice_time`, so the cue and its
/// reported point always agree (an unwrapped channel clock still gets a
/// correctly cue-located one).
///
/// `break_duration`, if given and below 2^33 ticks (the width of the
/// `break_duration()` field, else [`crate::Error::BreakDurationTooLong`]),
/// sets `break_duration().duration` with `auto_return = true` (the splicer
/// returns to the network feed on its own once the duration elapses —
/// §9.7.3).
pub fn build_splice_insert(
    edge: BreakEdge,
    splice_event_id: u32,
    requested_pts: Clock33,
    candidates: &[Clock33],
    max_delta: Duration,
    break_duration: Option<Duration>,
) -> Result<(ConditionedSplicePoint, SpliceInsert)> {
    let break_duration_ticks = break_duration
        .map(|duration| {
            duration_to_ticks(duration, clock33::WRAP_33BIT - 1).ok_or_else(|| {
                crate::Error::BreakDurationTooLong {
                    nanos: u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX),
                }
            })
        })
        .transpose()?;
    let conditioned =
        ssai_runtime::splice::condition_splice_point(requested_pts, candidates, max_delta)?;
    let insert = SpliceInsert {
        splice_event_id,
        cancel: None,
        out_of_network_indicator: matches!(edge, BreakEdge::Enter),
        event_id_compliance_flag: true,
        // `Clock33::ticks` is always below 2^33, so re-validating the reduced
        // instant cannot fail.
        mode: SpliceMode::Program(ProgramSplice::At(SpliceTime::at(
            Clock33::try_new(conditioned.snapped_pts.ticks()).unwrap_or(conditioned.snapped_pts),
        ))),
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
/// (`broadcast_core::Serialize::to_bytes`) onto the SCTE-35 elementary
/// stream/PID.
#[must_use]
pub fn to_section<'a>(insert: SpliceInsert) -> SpliceInfoSection<'a> {
    SpliceInfoSection::new_clear(
        AnyCommand::SpliceInsert(insert),
        SpliceDescriptorLoop::new(&[]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_core::{Parse, Serialize};
    use ssai_runtime::Error as SsaiError;

    const WRAP: u64 = clock33::WRAP_33BIT;

    fn pts(ticks: u64) -> Clock33 {
        Clock33::try_new(ticks).unwrap()
    }

    /// A tolerance/duration of `n` 90 kHz ticks, exactly (every value used is
    /// a whole number of ticks).
    fn ticks(n: u64) -> Duration {
        Duration::from_nanos(n * 1_000_000_000 / 90_000)
    }

    /// The one program-mode `splice_time()` these cues carry (they are always
    /// scheduled program splices; the mode cannot disagree with the flags).
    fn program_splice_time(insert: &SpliceInsert) -> SpliceTime {
        match &insert.mode {
            SpliceMode::Program(ProgramSplice::At(splice_time)) => *splice_time,
            other => panic!("expected a scheduled program splice, got {other:?}"),
        }
    }

    /// The cue and the reported point agree on one instant: a cue just before
    /// the wrap snapping to a boundary just after it names the *reduced*
    /// boundary in `pts_time`, so a caller rendering the returned point and a
    /// decoder reading the emitted cue see the same boundary.
    #[test]
    fn the_cue_and_the_reported_point_name_the_same_boundary() {
        let (conditioned, insert) = build_splice_insert(
            BreakEdge::Return,
            1,
            pts(WRAP - 100),
            &[pts(50)],
            ticks(1_000),
            None,
        )
        .unwrap();
        assert_eq!(conditioned.snapped_pts, pts(50));
        assert_eq!(
            program_splice_time(&insert).pts_time.map(|c| c.ticks()),
            Some(50)
        );
    }

    #[test]
    fn builds_an_enter_break_cue_snapped_to_the_conditioned_point() {
        let (conditioned, insert) = build_splice_insert(
            BreakEdge::Enter,
            7,
            pts(1_000),
            &[pts(1_050), pts(2_000)],
            ticks(100),
            Some(ticks(1_800_000)),
        )
        .unwrap();
        assert_eq!(conditioned.snapped_pts, pts(1_050));
        assert_eq!(insert.splice_event_id, 7);
        assert!(insert.out_of_network_indicator);
        assert_eq!(
            program_splice_time(&insert).pts_time.map(|c| c.ticks()),
            Some(1_050)
        );
        assert_eq!(insert.break_duration.unwrap().duration, 1_800_000);
        assert!(insert.break_duration.unwrap().auto_return);

        // The cue must reflect the *conditioned* point, not the raw request
        // — proves this isn't a passthrough of `requested_pts`.
        assert_ne!(
            program_splice_time(&insert).pts_time.map(|c| c.ticks()),
            Some(1_000)
        );
    }

    /// The `break_duration()` field is 33 bits, so a duration under 2^33
    /// ticks fits and 2^33 ticks or more does not.
    #[test]
    fn a_break_duration_past_33_bits_is_rejected_at_construction() {
        let max = ticks(90_000);
        let (_, insert) = build_splice_insert(
            BreakEdge::Enter,
            1,
            pts(1_000),
            &[pts(1_000)],
            ticks(0),
            Some(max),
        )
        .unwrap();
        assert_eq!(insert.break_duration.unwrap().duration, 90_000);
        let err = build_splice_insert(
            BreakEdge::Enter,
            1,
            pts(1_000),
            &[pts(1_000)],
            ticks(0),
            Some(ticks(WRAP) + ticks(90_000)),
        )
        .unwrap_err();
        assert!(matches!(err, crate::Error::BreakDurationTooLong { .. }));
    }

    /// The `break_duration()` field is 33 bits: the largest expressible
    /// duration is exactly 2^33 - 1 ticks, and 2^33 ticks is refused.
    #[test]
    fn the_break_duration_cap_is_the_largest_33_bit_tick_count() {
        let largest = Duration::from_nanos(95_443_717_677_778);
        let (_, insert) = build_splice_insert(
            BreakEdge::Enter,
            1,
            pts(1_000),
            &[pts(1_000)],
            ticks(0),
            Some(largest),
        )
        .unwrap();
        assert_eq!(insert.break_duration.unwrap().duration, WRAP - 1);

        let section = to_section(insert);
        let bytes = section.try_to_bytes().unwrap();
        let parsed = SpliceInfoSection::parse(&bytes).unwrap();
        match parsed.clear.unwrap().command {
            AnyCommand::SpliceInsert(reparsed) => {
                assert_eq!(reparsed.break_duration.unwrap().duration, WRAP - 1);
            }
            other => panic!("expected SpliceInsert, got {other:?}"),
        }

        let too_long = Duration::from_nanos(95_443_717_688_889);
        let err = build_splice_insert(
            BreakEdge::Enter,
            1,
            pts(1_000),
            &[pts(1_000)],
            ticks(0),
            Some(too_long),
        )
        .unwrap_err();
        assert!(matches!(err, crate::Error::BreakDurationTooLong { .. }));
    }

    #[test]
    fn builds_a_return_break_cue_with_no_duration() {
        let (_, insert) = build_splice_insert(
            BreakEdge::Return,
            8,
            pts(1_000),
            &[pts(1_000)],
            ticks(0),
            None,
        )
        .unwrap();
        assert!(!insert.out_of_network_indicator);
        assert!(insert.break_duration.is_none());
    }

    #[test]
    fn rejects_a_cue_with_no_boundary_within_tolerance() {
        let err = build_splice_insert(
            BreakEdge::Enter,
            1,
            pts(1_000),
            &[pts(5_000)],
            ticks(10),
            None,
        )
        .unwrap_err();
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
        let (_, insert) = build_splice_insert(
            BreakEdge::Enter,
            42,
            pts(1_000),
            &[pts(1_000)],
            ticks(0),
            Some(ticks(900_000)),
        )
        .unwrap();
        let section = to_section(insert);
        let bytes = section.try_to_bytes().unwrap();
        assert_eq!(bytes[0], scte35_splice::section::TABLE_ID);

        let parsed = SpliceInfoSection::parse(&bytes).unwrap();
        match parsed.clear.unwrap().command {
            AnyCommand::SpliceInsert(reparsed) => {
                assert_eq!(reparsed.splice_event_id, 42);
                assert_eq!(
                    program_splice_time(&reparsed).pts_time.map(|c| c.ticks()),
                    Some(1_000)
                );
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
        let requested = pts(WRAP - 100);
        let (conditioned, insert) = build_splice_insert(
            BreakEdge::Enter,
            9,
            requested,
            &[pts(50)],
            ticks(1_000),
            None,
        )
        .unwrap();
        assert_eq!(conditioned.snapped_pts, pts(50));
        assert_eq!(conditioned.delta_ticks, 150);
        assert_eq!(conditioned.direction, ssai_runtime::SnapDirection::After);
        assert_eq!(
            program_splice_time(&insert).pts_time.map(|c| c.ticks()),
            Some(50)
        );
        // Too tight a tolerance is still refused (circularly measured).
        assert!(matches!(
            build_splice_insert(BreakEdge::Enter, 9, requested, &[pts(50)], ticks(149), None),
            Err(crate::Error::SpliceConditioning(
                SsaiError::NoAlignedBoundary {
                    nearest_delta_ticks: 150,
                    ..
                }
            ))
        ));
    }
}
