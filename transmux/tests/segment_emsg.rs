//! Gate tests for `emsg` (Event Message Box) emission in media segments.
//!
//! Covers [`transmux::build_media_segment_with_events`]:
//!
//! 1. **emsg present + round-trips (real SCTE-35 payload)** — builds a segment
//!    with one `emsg` v0 box carrying the committed SCTE 35 splice-info fixture,
//!    locates the box in the output, asserts placement (after `styp`, before
//!    `moof`), and round-trips all fields through `mp4_emsg::EmsgBox::parse`.
//! 2. **Placement/order** — two `emsg` boxes appear in the output, in order,
//!    before `moof`.
//! 3. **No-emsg unchanged** — `build_media_segment_with_events(.., &[])` is
//!    byte-identical to `build_media_segment(..)` on the same tracks.
//!
//! Spec citations:
//! - ISO/IEC 14496-12 §8.8 (movie fragments / `emsg` box).
//! - DASH-IF IOP Part 10 §6.1 (Events and Timed Metadata in MPEG-DASH / CMAF):
//!   `emsg` boxes follow `styp` and precede `moof` in each media segment.
//! - ANSI/SCTE 214-3 / DASH-IF Part 10 §7.3: SCTE 35 binary scheme
//!   `urn:scte:scte35:2013:bin`.

use mp4_emsg::{EMSG_BOX_TYPE, EmsgBox, EmsgVersion, PresentationTime};
use transmux::{FragmentTrackData, Sample, build_media_segment, build_media_segment_with_events};

/// The SCTE 35 scheme URI for binary `splice_info_section` delivery
/// (ANSI/SCTE 214-3 / DASH-IF IOP Part 10 §7.3).
const SCTE35_SCHEME: &str = "urn:scte:scte35:2013:bin";

/// Load the committed SCTE 35 `emsg` v0 fixture and return the raw `emsg` box
/// bytes plus the `message_data` slice within it (the `splice_info_section`).
fn scte35_fixture_emsg_bytes() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/shared/scte35_emsg_v0.bin"
    );
    std::fs::read(path).expect("fixtures/shared/scte35_emsg_v0.bin must be committed")
}

/// Build a minimal one-track segment (one zero-byte sync sample) for use as a
/// stable test vehicle — we care about the box structure, not the payload.
fn minimal_tracks() -> Vec<Sample> {
    vec![Sample::new(vec![0u8; 4], 3000, true, 0)]
}

fn minimal_frag(samples: &[Sample]) -> Vec<FragmentTrackData<'_>> {
    vec![FragmentTrackData {
        track_id: 1,
        base_media_decode_time: 0,
        samples,
    }]
}

/// Walk the box list in `data` and return a vec of `(start, end, fourcc)` for
/// each top-level box found.  `start` is inclusive, `end` exclusive.
fn scan_boxes(data: &[u8]) -> Vec<(usize, usize, [u8; 4])> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 {
            break;
        }
        let mut cc = [0u8; 4];
        cc.copy_from_slice(&data[off + 4..off + 8]);
        let end = off + size;
        if end > data.len() {
            break;
        }
        out.push((off, end, cc));
        off = end;
    }
    out
}

// ---------------------------------------------------------------------------
// Test 1: single emsg with real SCTE-35 payload — presence, placement, round-trip
// ---------------------------------------------------------------------------

#[test]
fn emsg_present_and_round_trips_real_scte35_payload() {
    // Parse the committed fixture to extract the typed fields + message_data.
    let fixture_bytes = scte35_fixture_emsg_bytes();
    let fixture_emsg = EmsgBox::parse(&fixture_bytes).expect("fixture must parse");

    assert_eq!(fixture_emsg.scheme_id_uri, SCTE35_SCHEME);
    assert_eq!(fixture_emsg.version(), EmsgVersion::SegmentRelative);
    // The fixture carries a real splice_info_section (FC 30 … CRC).
    assert!(fixture_emsg.is_scte35());
    assert!(
        fixture_emsg.message_data.len() >= 14,
        "minimum splice_info_section size"
    );
    assert_eq!(
        fixture_emsg.message_data[0], 0xFC,
        "splice_info_section table_id"
    );

    // Build a segment with this emsg.
    let samples = minimal_tracks();
    let tracks = minimal_frag(&samples);
    let seg = build_media_segment_with_events(1, &tracks, std::slice::from_ref(&fixture_emsg))
        .expect("build must succeed");

    // Scan the top-level box list.
    let boxes = scan_boxes(&seg);
    let box_fourccs: Vec<[u8; 4]> = boxes.iter().map(|b| b.2).collect();

    // Must contain styp, emsg, moof, mdat in that order.
    let styp_pos = box_fourccs
        .iter()
        .position(|c| c == b"styp")
        .expect("styp missing");
    let emsg_pos = box_fourccs
        .iter()
        .position(|c| c == &EMSG_BOX_TYPE)
        .expect("emsg missing");
    let moof_pos = box_fourccs
        .iter()
        .position(|c| c == b"moof")
        .expect("moof missing");
    let mdat_pos = box_fourccs
        .iter()
        .position(|c| c == b"mdat")
        .expect("mdat missing");

    assert!(
        styp_pos < emsg_pos,
        "emsg ({emsg_pos}) must come after styp ({styp_pos})"
    );
    assert!(
        emsg_pos < moof_pos,
        "emsg ({emsg_pos}) must come before moof ({moof_pos})"
    );
    assert!(
        moof_pos < mdat_pos,
        "moof ({moof_pos}) must come before mdat ({mdat_pos})"
    );

    // Byte-offset placement assertion: styp_end <= emsg_start < moof_start.
    let styp_end = boxes[styp_pos].1;
    let emsg_start = boxes[emsg_pos].0;
    let moof_start = boxes[moof_pos].0;

    assert!(
        styp_end <= emsg_start,
        "emsg byte start ({emsg_start}) must be >= styp end ({styp_end})"
    );
    assert!(
        emsg_start < moof_start,
        "emsg byte start ({emsg_start}) must be < moof start ({moof_start})"
    );

    // Round-trip: parse the emsg box from the output and compare all fields.
    let emsg_slice = &seg[emsg_start..boxes[emsg_pos].1];
    let reparsed = EmsgBox::parse(emsg_slice).expect("emsg in segment must re-parse");

    assert_eq!(reparsed.scheme_id_uri, fixture_emsg.scheme_id_uri);
    assert_eq!(reparsed.value, fixture_emsg.value);
    assert_eq!(reparsed.timescale, fixture_emsg.timescale);
    assert_eq!(reparsed.presentation_time, fixture_emsg.presentation_time);
    assert_eq!(reparsed.event_duration, fixture_emsg.event_duration);
    assert_eq!(reparsed.id, fixture_emsg.id);
    assert_eq!(
        reparsed.message_data, fixture_emsg.message_data,
        "message_data (splice_info_section) must survive the round-trip"
    );
    assert!(reparsed.is_scte35());
    assert_eq!(
        reparsed.message_data[0], 0xFC,
        "splice_info_section table_id survives"
    );
}

// ---------------------------------------------------------------------------
// Test 2: two emsg boxes appear in order, both before moof
// ---------------------------------------------------------------------------

#[test]
fn two_emsgs_appear_in_order_before_moof() {
    // Two distinct emsg v0 boxes with different ids.
    let msg_a = [0xFCu8, 0x30, 0x01];
    let msg_b = [0xFCu8, 0x30, 0x02];

    let emsg_a = EmsgBox {
        scheme_id_uri: SCTE35_SCHEME,
        value: "",
        timescale: 90_000,
        presentation_time: PresentationTime::Delta(0),
        event_duration: 0xFFFF_FFFF,
        id: 1,
        message_data: &msg_a,
    };
    let emsg_b = EmsgBox {
        scheme_id_uri: SCTE35_SCHEME,
        value: "",
        timescale: 90_000,
        presentation_time: PresentationTime::Delta(9000),
        event_duration: 0xFFFF_FFFF,
        id: 2,
        message_data: &msg_b,
    };

    let samples = minimal_tracks();
    let tracks = minimal_frag(&samples);
    let seg = build_media_segment_with_events(2, &tracks, &[emsg_a.clone(), emsg_b.clone()])
        .expect("build must succeed");

    let boxes = scan_boxes(&seg);
    let box_fourccs: Vec<[u8; 4]> = boxes.iter().map(|b| b.2).collect();

    // Collect all emsg positions.
    let emsg_positions: Vec<usize> = box_fourccs
        .iter()
        .enumerate()
        .filter_map(|(i, c)| if c == &EMSG_BOX_TYPE { Some(i) } else { None })
        .collect();

    assert_eq!(
        emsg_positions.len(),
        2,
        "exactly two emsg boxes must be present"
    );

    let moof_pos = box_fourccs
        .iter()
        .position(|c| c == b"moof")
        .expect("moof missing");

    // Both emsg boxes precede moof.
    for &ep in &emsg_positions {
        assert!(
            ep < moof_pos,
            "emsg at box-index {ep} must precede moof at {moof_pos}"
        );
    }

    // First emsg has id=1, second has id=2 (order is preserved).
    let first_emsg_bytes = &seg[boxes[emsg_positions[0]].0..boxes[emsg_positions[0]].1];
    let second_emsg_bytes = &seg[boxes[emsg_positions[1]].0..boxes[emsg_positions[1]].1];

    let first = EmsgBox::parse(first_emsg_bytes).expect("first emsg must parse");
    let second = EmsgBox::parse(second_emsg_bytes).expect("second emsg must parse");

    assert_eq!(first.id, 1, "first emsg id must be 1");
    assert_eq!(second.id, 2, "second emsg id must be 2");
    assert_eq!(first.message_data, msg_a.as_slice());
    assert_eq!(second.message_data, msg_b.as_slice());

    // Both precede moof by byte offset too.
    let moof_start = boxes[moof_pos].0;
    assert!(boxes[emsg_positions[0]].1 <= moof_start);
    assert!(boxes[emsg_positions[1]].1 <= moof_start);
}

// ---------------------------------------------------------------------------
// Test 3: empty emsg slice → byte-identical to build_media_segment
// ---------------------------------------------------------------------------

#[test]
fn no_emsg_output_byte_identical_to_base_fn() {
    let samples = minimal_tracks();
    let tracks_a = minimal_frag(&samples);
    let tracks_b = minimal_frag(&samples);

    let base = build_media_segment(3, &tracks_a).expect("base build must succeed");
    let with_events =
        build_media_segment_with_events(3, &tracks_b, &[]).expect("with_events build must succeed");

    assert_eq!(
        base, with_events,
        "build_media_segment_with_events with empty emsgs must be byte-identical to build_media_segment"
    );
}
