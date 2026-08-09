//! Condition the real SCTE-35 cue in
//! `fixtures/scte35-ssai/emsg_splice_insert.bin` against the same capture's
//! real, independently-measured nearest video keyframe.
//!
//! Per `fixtures/scte35-ssai/PROVENANCE.md`, this DASH-IF `livesim2` capture's
//! cue is genuinely NOT IDR-aligned: the nearest video keyframe lands 6000
//! ticks (67 ms) after the cue's nominal presentation time on the shared
//! 90 kHz clock. This example re-derives that gap through
//! `ssai_runtime::splice::condition_splice_point` rather than asserting it —
//! showing how a caller should treat a non-IDR-aligned real-world cue: snap
//! to the nearest boundary within a tolerance, and get an error instead of a
//! silent lie when nothing is close enough.
//!
//! ```sh
//! cargo run -p ssai-runtime --example condition_real_cue
//! ```
use broadcast_common::Parse;
use mp4_emsg::{EmsgBox, PresentationTime};
use scte35_splice::SpliceInfoSection;
use ssai_runtime::splice::condition_splice_point;
use std::fs;

fn main() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/scte35-ssai/emsg_splice_insert.bin"
    );
    let emsg_bytes = fs::read(path).expect("read real emsg fixture (see PROVENANCE.md)");
    let emsg = EmsgBox::parse(&emsg_bytes).expect("parse real emsg box");
    assert!(emsg.is_scte35(), "fixture must carry a SCTE-35 emsg");

    let requested_pts = match emsg.presentation_time {
        PresentationTime::Absolute(t) => t,
        other => panic!("fixture is expected to be a v1 (Absolute) emsg, got {other:?}"),
    };
    println!(
        "cue presentation_time = {requested_pts} (90kHz ticks, absolute representation clock)"
    );

    // Decode the splice_info_section for completeness — its
    // splice_time().pts_time is a 33-bit, wrap-modulo value keyed to a
    // *different* instant of the same clock than the emsg's absolute
    // presentation_time; unrolling that wrap is `timed-metadata::Timeline`'s
    // job, out of scope for this example.
    let section = SpliceInfoSection::parse(emsg.message_data).expect("parse splice_info_section");
    println!("decoded splice_info_section: {section:?}");

    // The real nearest video keyframe, measured independently straight from
    // this fixture's own moof/traf/tfdt/trun boxes (PROVENANCE.md's
    // "Cue-to-IDR alignment" section): pts=160767315906000, i.e. 6000 ticks
    // (67ms) AFTER the cue. That is real-world, non-IDR-aligned behaviour,
    // not a fixture bug — hard-coded here as the independently-verified
    // ground truth rather than re-deriving it, since that derivation needs a
    // working `Fmp4Demux` track and this exact fixture currently defeats
    // that (issue #952).
    let nearest_keyframe_pts = 160_767_315_906_000u64;
    let candidates = [nearest_keyframe_pts];

    let tight = condition_splice_point(requested_pts, &candidates, 3_000);
    println!("tolerance=3000 ticks (~33ms, one 30fps frame) -> {tight:?}");
    assert!(
        tight.is_err(),
        "a real 67ms drift must exceed a 33ms tolerance"
    );

    let loose = condition_splice_point(requested_pts, &candidates, 10_000)
        .expect("a real 67ms drift is within a 111ms tolerance");
    println!("tolerance=10000 ticks (~111ms) -> {loose:?}");
    assert_eq!(
        loose.delta_ticks, 6_000,
        "must reproduce the measured 67ms/6000-tick gap"
    );
    println!(
        "conditioning correctly measured the real, non-IDR-aligned gap: {} ticks (~{:.1}ms)",
        loose.delta_ticks,
        loose.delta_ticks as f64 / 90.0
    );
}
