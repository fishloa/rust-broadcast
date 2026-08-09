//! SCTE-35 emission points a transition implies (issue
//! [#748](https://github.com/fishloa/rust-broadcast/issues/748)):
//! `splice_insert()` — ANSI/SCTE 35 2023r1 §9.7.3, Table 10 — built and
//! serialized with `scte35-splice`'s own
//! [`Serialize`](broadcast_common::Serialize), not hand-assembled bytes.
//!
//! This module decides *what* cue to emit and *where* (after conditioning);
//! it does not decide *how the splice lands* against real segment/keyframe
//! boundaries — that is [`ssai_runtime::splice::condition_splice_point`],
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
/// [`ssai_runtime::splice::condition_splice_point`], which this delegates to
/// rather than duplicating. Errors (via [`crate::Error::SpliceConditioning`])
/// if no candidate is close enough, or none are supplied: this crate never
/// emits a cue for a splice point nothing is actually close to.
///
/// `break_duration_ticks`, if given, sets `break_duration().duration` with
/// `auto_return = true` (the splicer returns to the network feed on its own
/// once the duration elapses — §9.7.3).
pub fn build_splice_insert(
    edge: BreakEdge,
    splice_event_id: u32,
    requested_pts: u64,
    candidates: &[u64],
    max_delta_ticks: u64,
    break_duration_ticks: Option<u64>,
) -> Result<(ConditionedSplicePoint, SpliceInsert)> {
    let conditioned =
        ssai_runtime::splice::condition_splice_point(requested_pts, candidates, max_delta_ticks)?;
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
}
