//! Real-fixture bite test (issue #929 prep;
//! `fixtures/scte35-ssai/PROVENANCE.md`).
//!
//! Reads the committed, genuine DASH-IF `livesim2` SCTE-35 `emsg` capture,
//! decodes the real cue's `presentation_time`, and conditions it against the
//! independently-measured real nearest video keyframe from the same capture
//! (160_767_315_906_000, per `PROVENANCE.md`'s "Cue-to-IDR alignment"
//! section — extracted straight from the fragment's `moof`/`traf`/`tfdt`/
//! `trun` boxes, bypassing the `stsd`/`avcC` path issue #952 tracks as
//! broken for this exact fixture).
//!
//! The two numbers are NOT equal — this capture's cue is genuinely 6000
//! ticks (67 ms) off the nearest keyframe. A correct `condition_splice_point`
//! must report exactly that gap and must reject it under a tolerance
//! tighter than 6000 ticks. A broken implementation that always reports an
//! exact match (or silently accepts any drift) fails this test — the
//! anti-cheat property "tests must bite" requires.
use broadcast_common::Parse;
use mp4_emsg::{EmsgBox, PresentationTime};
use scte35_splice::SpliceInfoSection;
use ssai_runtime::error::Error;
use ssai_runtime::splice::{SnapDirection, condition_splice_point};

/// The real nearest video keyframe's pts, in the same absolute 90 kHz
/// representation clock as the emsg's `presentation_time` — measured
/// independently from the fragment's own boxes, per `PROVENANCE.md`.
const NEAREST_KEYFRAME_PTS: u64 = 160_767_315_906_000;

/// The measured real gap: `NEAREST_KEYFRAME_PTS - presentation_time`.
const MEASURED_GAP_TICKS: u64 = 6_000;

fn real_cue_presentation_time() -> u64 {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/scte35-ssai/emsg_splice_insert.bin"
    );
    let bytes = std::fs::read(path).expect("read committed real emsg fixture");
    let emsg = EmsgBox::parse(&bytes).expect("parse real emsg box");
    assert!(emsg.is_scte35(), "fixture must carry a SCTE-35 emsg");

    // Decode the splice_info_section too, proving `message_data` really is
    // one (not just claimed by scheme_id_uri) — a genuine, wire-accurate
    // splice_insert(), not a hand-typed vector.
    let section = SpliceInfoSection::parse(emsg.message_data).expect("parse splice_info_section");
    assert!(
        section.clear.is_some(),
        "this fixture's cue is a clear (unencrypted) command"
    );

    match emsg.presentation_time {
        PresentationTime::Absolute(t) => t,
        other => panic!("fixture is expected to be a version-1 (Absolute) emsg, got {other:?}"),
    }
}

#[test]
fn real_cue_is_genuinely_not_idr_aligned() {
    let requested_pts = real_cue_presentation_time();
    assert_ne!(
        requested_pts, NEAREST_KEYFRAME_PTS,
        "the whole point of this fixture is that the cue is NOT IDR-aligned"
    );
}

#[test]
fn conditioning_reproduces_the_measured_real_gap() {
    let requested_pts = real_cue_presentation_time();

    let conditioned = condition_splice_point(requested_pts, &[NEAREST_KEYFRAME_PTS], 10_000)
        .expect("the real 67ms drift is within a 111ms tolerance");

    assert_eq!(conditioned.requested_pts, requested_pts);
    assert_eq!(conditioned.snapped_pts, NEAREST_KEYFRAME_PTS);
    assert_eq!(
        conditioned.delta_ticks, MEASURED_GAP_TICKS,
        "must reproduce PROVENANCE.md's independently measured 67ms/6000-tick gap"
    );
    assert_eq!(
        conditioned.direction,
        SnapDirection::After,
        "the real keyframe lands AFTER the cue's nominal instant"
    );
    assert!(!conditioned.is_exact());
}

#[test]
fn a_tolerance_tighter_than_the_real_gap_is_rejected() {
    let requested_pts = real_cue_presentation_time();

    // One video frame at 30fps/90kHz is 3000 ticks (33ms) — tighter than the
    // real 67ms gap, so a live low-latency caller with that bound must be
    // told "no", not silently handed a splice point 67ms off.
    let err = condition_splice_point(requested_pts, &[NEAREST_KEYFRAME_PTS], 3_000).unwrap_err();
    match err {
        Error::NoAlignedBoundary {
            requested_pts: rp,
            tolerance_ticks,
            nearest_delta_ticks,
        } => {
            assert_eq!(rp, requested_pts);
            assert_eq!(tolerance_ticks, 3_000);
            assert_eq!(nearest_delta_ticks, MEASURED_GAP_TICKS);
        }
        other => panic!("expected NoAlignedBoundary, got {other:?}"),
    }
}
