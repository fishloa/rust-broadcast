//! Real-fixture test: plan a programme -> ad transition and emit the
//! `splice_insert()` it implies, driven by the genuine, non-IDR-aligned cue
//! in `fixtures/scte35-ssai/` (DASH-IF `livesim2` capture, Apache-2.0; see
//! `fixtures/scte35-ssai/PROVENANCE.md`).
//!
//! This is the same real cue `ssai-runtime/examples/condition_real_cue.rs`
//! conditions: the nearest video keyframe lands 6000 ticks (67 ms) *after*
//! the cue's nominal presentation time on the shared 90 kHz clock — real
//! encoder behaviour, not a fixture bug. A transition planned against this
//! cue has to cope with that drift rather than assume alignment; this test
//! asserts the measured gap rather than a hand-picked convenient number, so
//! it fails loudly if `build_splice_insert` stops actually conditioning
//! (e.g. a regression that passes `requested_pts` straight through).

use broadcast_common::{Parse, Serialize};
use mp4_emsg::{EmsgBox, PresentationTime};
use playout_runtime::schedule::{CodecConfigId, EntryKind, Schedule, ScheduleEntry};
use playout_runtime::scte35::{BreakEdge, build_splice_insert, to_section};
use playout_runtime::transition::TransitionPlan;
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::AnyCommand;
use std::fs;

/// The real nearest video keyframe's PTS, independently measured straight
/// from this same capture's `moof`/`traf`/`tfdt`/`trun` boxes
/// (`fixtures/scte35-ssai/PROVENANCE.md`, "Cue-to-IDR alignment"). Hard-coded
/// as the independently-verified ground truth, exactly as
/// `ssai-runtime/examples/condition_real_cue.rs` does — re-deriving it needs
/// a working `Fmp4Demux` track and this exact fixture currently defeats that
/// (issue #952, documented in the same PROVENANCE.md).
const NEAREST_KEYFRAME_PTS: u64 = 160_767_315_906_000;

/// The real, measured gap between the cue's nominal instant and that
/// keyframe: 6000 ticks at 90 kHz = 67 ms.
const MEASURED_GAP_TICKS: u64 = 6_000;

fn real_cue_requested_pts() -> u64 {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/scte35-ssai/emsg_splice_insert.bin"
    );
    let emsg_bytes = fs::read(path).expect("read real emsg fixture (see PROVENANCE.md)");
    let emsg = EmsgBox::parse(&emsg_bytes).expect("parse real emsg box");
    assert!(emsg.is_scte35(), "fixture must carry a SCTE-35 emsg");
    match emsg.presentation_time {
        PresentationTime::Absolute(t) => t,
        other => panic!("fixture is expected to be a v1 (Absolute) emsg, got {other:?}"),
    }
}

#[test]
fn transition_into_a_real_non_idr_aligned_cue_plans_a_continuous_timeline() {
    let requested_pts = real_cue_requested_pts();

    // The channel is playing a programme; it transitions into an ad whose
    // own asset timeline starts at a nonzero PTS (e.g. a clip trimmed from a
    // longer source file — the realistic case, and the one that actually
    // exercises the rebase subtraction rather than a no-op offset), joining
    // the channel at the real, *conditioned* splice instant — not the cue's
    // raw nominal instant.
    const AD_SOURCE_START_PTS: u64 = 12_345;
    let programme = ScheduleEntry {
        id: "programme-1".into(),
        kind: EntryKind::Programme,
        planned_start: 0,
        source_start_pts: 0,
        codec_config: CodecConfigId(1),
    };
    let ad = ScheduleEntry {
        id: "ad-1".into(),
        kind: EntryKind::Ad,
        planned_start: NEAREST_KEYFRAME_PTS,
        source_start_pts: AD_SOURCE_START_PTS,
        codec_config: CodecConfigId(2), // a different codec config: real SSAI splices commonly do.
    };

    let mut schedule = Schedule::new();
    schedule.push(programme.clone()).unwrap();
    schedule.push(ad.clone()).unwrap();

    let plan = TransitionPlan::plan(&programme, &ad);
    assert_eq!(plan.at_pts, NEAREST_KEYFRAME_PTS);
    assert!(
        plan.discontinuity,
        "a differing codec config across the join must be flagged, never silently absorbed"
    );

    // Timeline continuity: the ad's own first sample (at its declared
    // source_start_pts) must land exactly on the real keyframe instant once
    // rebased, and stay continuous (no drift introduced by the rebase
    // itself) for samples after it.
    assert_eq!(plan.rebase(AD_SOURCE_START_PTS), Some(NEAREST_KEYFRAME_PTS));
    assert_eq!(
        plan.rebase(AD_SOURCE_START_PTS + 90_000),
        Some(NEAREST_KEYFRAME_PTS + 90_000)
    );

    // The SCTE-35 emission point this transition implies: conditioned
    // against the real keyframe, not the cue's raw nominal instant.
    let (conditioned, insert) = build_splice_insert(
        BreakEdge::Enter,
        0x6A78_D416, // the real fixture's own splice_event_id (PROVENANCE.md).
        requested_pts,
        &[NEAREST_KEYFRAME_PTS],
        10_000,
        Some(1_800_000), // the real fixture's own 20s break_duration.
    )
    .unwrap();

    // This is the measured reality, not an assumption: the cue is genuinely
    // 67 ms off the nearest keyframe. A regression that hard-codes zero
    // drift (or silently accepts the raw request) fails this assertion.
    assert_eq!(conditioned.delta_ticks, MEASURED_GAP_TICKS);
    assert_eq!(conditioned.snapped_pts, NEAREST_KEYFRAME_PTS);
    assert_eq!(
        insert.splice_time.unwrap().pts_time,
        Some(NEAREST_KEYFRAME_PTS % (1u64 << 33)),
        "the emitted cue must carry the conditioned (snapped) instant, not the raw request"
    );
    assert_ne!(
        insert.splice_time.unwrap().pts_time,
        Some(requested_pts % (1u64 << 33)),
        "the emitted cue must not be a passthrough of the raw, non-aligned request"
    );

    // A tighter tolerance than the real 67 ms drift must be rejected, not
    // silently satisfied — the same real-world check
    // `ssai-runtime/examples/condition_real_cue.rs` performs.
    let too_tight = build_splice_insert(
        BreakEdge::Enter,
        1,
        requested_pts,
        &[NEAREST_KEYFRAME_PTS],
        3_000,
        None,
    );
    assert!(
        too_tight.is_err(),
        "a real 67ms drift must exceed a 33ms (3000-tick) tolerance"
    );

    // The built command round-trips byte-for-byte through scte35-splice's
    // own Serialize/Parse — proving this crate emits a real, wire-valid
    // splice_info_section, not just an in-memory struct.
    let section = to_section(insert);
    let bytes = section.to_bytes();
    let reparsed = SpliceInfoSection::parse(&bytes).expect("reparse emitted section");
    match reparsed.clear.unwrap().command {
        AnyCommand::SpliceInsert(reparsed_insert) => {
            assert_eq!(reparsed_insert.splice_event_id, 0x6A78_D416);
            assert!(reparsed_insert.out_of_network_indicator);
            assert_eq!(
                reparsed_insert.splice_time.unwrap().pts_time,
                Some(NEAREST_KEYFRAME_PTS % (1u64 << 33))
            );
            assert_eq!(reparsed_insert.break_duration.unwrap().duration, 1_800_000);
        }
        other => panic!("expected SpliceInsert, got {other:?}"),
    }
}
