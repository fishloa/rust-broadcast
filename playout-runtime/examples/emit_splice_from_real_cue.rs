//! End-to-end: plan a programme -> ad transition against the real,
//! non-IDR-aligned SCTE-35 cue in `fixtures/scte35-ssai/`, and emit the
//! `splice_insert()` that transition implies.
//!
//! Per `fixtures/scte35-ssai/PROVENANCE.md`, this DASH-IF `livesim2` capture
//! carries a genuine `splice_insert()` whose nominal target instant is 6000
//! ticks (67 ms) before the nearest real video keyframe — real encoder
//! behaviour, not a hand-built happy path. This example conditions against
//! that measured gap rather than assuming alignment away.
//!
//! ```sh
//! cargo run -p playout-runtime --example emit_splice_from_real_cue
//! ```

use broadcast_common::{Parse, Serialize};
use mp4_emsg::{EmsgBox, PresentationTime};
use playout_runtime::schedule::{CodecConfigId, EntryKind, Schedule, ScheduleEntry};
use playout_runtime::scte35::{BreakEdge, build_splice_insert, to_section};
use playout_runtime::transition::next_transition;
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::AnyCommand;
use std::fs;

/// The real nearest video keyframe's PTS, independently measured from this
/// same capture's `moof`/`traf`/`tfdt`/`trun` boxes — see
/// `fixtures/scte35-ssai/PROVENANCE.md`, "Cue-to-IDR alignment".
const NEAREST_KEYFRAME_PTS: u64 = 160_767_315_906_000;

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
    println!("real cue's nominal presentation_time = {requested_pts} (90kHz ticks)");

    // Plan the transition: the channel joins the ad at the real, conditioned
    // splice instant, not the cue's raw nominal instant.
    let mut schedule = Schedule::new();
    schedule
        .push(ScheduleEntry {
            id: "programme-1".into(),
            kind: EntryKind::Programme,
            planned_start: 0,
            source_start_pts: 0,
            codec_config: CodecConfigId(1),
        })
        .unwrap();
    schedule
        .push(ScheduleEntry {
            id: "ad-1".into(),
            kind: EntryKind::Ad,
            planned_start: NEAREST_KEYFRAME_PTS,
            source_start_pts: 0,
            codec_config: CodecConfigId(2),
        })
        .unwrap();

    let t = next_transition(&schedule, 0).expect("a transition is scheduled");
    println!(
        "planned transition: {} -> {} at pts={} discontinuity={}",
        t.from.id, t.to.id, t.plan.at_pts, t.plan.discontinuity
    );

    // Emit the SCTE-35 cue that transition implies, conditioned against the
    // real keyframe candidate.
    let (conditioned, insert) = build_splice_insert(
        BreakEdge::Enter,
        0x6A78_D416, // the real fixture's own splice_event_id.
        requested_pts,
        &[NEAREST_KEYFRAME_PTS],
        10_000,
        Some(1_800_000), // the real fixture's own 20s break_duration.
    )
    .expect("a real 67ms drift is within a 111ms (10_000-tick) tolerance");
    println!(
        "conditioned splice point: requested={} snapped={} delta={} ticks (~{:.1}ms) direction={}",
        conditioned.requested_pts,
        conditioned.snapped_pts,
        conditioned.delta_ticks,
        conditioned.delta_ticks as f64 / 90.0,
        conditioned.direction,
    );

    let section = to_section(insert);
    let bytes = section.to_bytes();
    println!("emitted splice_info_section: {} bytes", bytes.len());
    println!(
        "hex: {}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );

    let reparsed = SpliceInfoSection::parse(&bytes).expect("emitted section re-parses");
    match reparsed.clear.expect("clear section").command {
        AnyCommand::SpliceInsert(reparsed_insert) => {
            assert_eq!(reparsed_insert.splice_event_id, 0x6A78_D416);
            assert_eq!(
                reparsed_insert.splice_time.unwrap().pts_time,
                Some(NEAREST_KEYFRAME_PTS % (1u64 << 33))
            );
        }
        other => panic!("expected SpliceInsert, got {other:?}"),
    }
    println!("byte-exact round-trip: OK");
}
