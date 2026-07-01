//! Real-fixture round-trip and navigation tests for the typed init-segment (moov) box tree.
//!
//! EXIT CRITERION 1: Parse the ENTIRE `moov` box from `h264_aac_prog.mp4`, serialise it back,
//!   and assert byte-identical to the original moov box bytes.
//! EXIT CRITERION 2: After parsing, reach the video trak's avc1 sample entry + audio trak's
//!   mp4a sample entry (assert both found).
//! EXIT CRITERION 3: A leaf-box unit test (mvhd v0) round-trips (in `init_segment` module).

use broadcast_common::{Parse, Serialize};
use transmux::{MovieBox, SampleDescriptionBox, SampleEntryVariant};

/// Find a top-level box by type and return its full bytes (header + body).
fn find_top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
    let mut offset = 0usize;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        if size < 8 {
            break;
        }
        let ty = &data[offset + 4..offset + 8];
        if ty == fourcc {
            return &data[offset..offset + size];
        }
        offset += size;
    }
    panic!("box {:?} not found", std::str::from_utf8(fourcc).unwrap());
}

// ---------------------------------------------------------------------------
// Debug: ensure stsd parses correctly from the fixture
// ---------------------------------------------------------------------------

#[test]
fn stsd_parses_from_fixture() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_prog.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");

    // Navigate manually to stsd bytes
    fn find_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
        let mut off = 0usize;
        while off + 8 <= data.len() {
            let size = u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
                as usize;
            if size < 8 {
                break;
            }
            if &data[off + 4..off + 8] == fourcc {
                return &data[off..off + size];
            }
            off += size;
        }
        panic!("box {:?} not found", std::str::from_utf8(fourcc).unwrap());
    }

    let moov = find_box(&data, b"moov");
    // moov is full box bytes; children start at offset 8
    let moov_body = &moov[8..];
    let trak1 = find_box(moov_body, b"trak");
    let trak1_body = &trak1[8..];
    let mdia = find_box(trak1_body, b"mdia");
    let mdia_body = &mdia[8..];
    let minf = find_box(mdia_body, b"minf");
    let minf_body = &minf[8..];
    let stbl = find_box(minf_body, b"stbl");
    let stbl_body = &stbl[8..];
    let stsd_box = find_box(stbl_body, b"stsd");

    let stsd = SampleDescriptionBox::parse(stsd_box).expect("should parse stsd from real fixture");

    assert_eq!(stsd.entries.len(), 1, "stsd should have 1 entry");
}

// ---------------------------------------------------------------------------
// EXIT CRITERION 1: moov round-trip (byte-identical)
// ---------------------------------------------------------------------------

#[test]
fn moov_real_fixture_round_trip() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_prog.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");

    // Find the moov box at top level
    let moov_bytes = find_top_box(&data, b"moov");

    // Parse the entire moov tree
    let moov = MovieBox::parse(moov_bytes).expect("should parse moov from real fixture");

    // Verify key mvhd fields
    assert_eq!(moov.mvhd.version, 0, "mvhd version");
    assert_eq!(moov.mvhd.timescale, 1000, "mvhd timescale");
    assert_eq!(moov.mvhd.duration, 2000, "mvhd duration");
    assert_eq!(moov.mvhd.next_track_id, 3, "mvhd next_track_id");

    // Verify we have two tracks
    assert_eq!(moov.tracks.len(), 2, "moov track count");

    // Re-serialize and compare byte-for-byte
    let serialized = moov.to_bytes();
    assert_eq!(
        serialized.len(),
        moov_bytes.len(),
        "moov serialized length matches original"
    );
    assert_eq!(
        &serialized[..],
        moov_bytes,
        "moov round-trip must be byte-identical"
    );
}

// ---------------------------------------------------------------------------
// Fragmented-init moov round-trip: exercises mvex/trex (+ udta) byte-identity
// ---------------------------------------------------------------------------

#[test]
fn moov_frag_fixture_round_trip() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_frag.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");
    let moov_bytes = find_top_box(&data, b"moov");

    let moov = MovieBox::parse(moov_bytes).expect("should parse fragmented-init moov");

    // mvex must be typed (not swept into opaque) and carry one trex per track.
    let mvex = moov.mvex.as_ref().expect("fragmented moov must have mvex");
    assert_eq!(mvex.trex.len(), 2, "two trex (video + audio)");
    assert_eq!(mvex.trex[0].track_id, 1, "trex[0] track_id");
    assert_eq!(mvex.trex[1].track_id, 2, "trex[1] track_id");
    assert_eq!(
        mvex.trex[0].default_sample_description_index, 1,
        "trex default_sample_description_index"
    );
    // udta and any other non-mvex children stay preserved (round-trip proves it).
    assert!(!moov.opaque.is_empty(), "udta preserved in opaque");

    let serialized = moov.to_bytes();
    assert_eq!(
        &serialized[..],
        moov_bytes,
        "fragmented moov round-trip must be byte-identical"
    );
}

// ---------------------------------------------------------------------------
// EXIT CRITERION 2: Navigation — reach avc1 and mp4a sample entries
// ---------------------------------------------------------------------------

#[test]
fn moov_navigation_reaches_avc1_and_mp4a() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_prog.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");
    let moov_bytes = find_top_box(&data, b"moov");
    let moov = MovieBox::parse(moov_bytes).expect("should parse moov");

    // First trak should be video (avc1)
    let vid_trak = &moov.tracks[0];
    assert!(vid_trak.mdia.is_some(), "video trak should have mdia");
    let vid_minf = vid_trak
        .mdia
        .as_ref()
        .unwrap()
        .minf
        .as_ref()
        .expect("video mdia should have minf");
    let vid_stbl = vid_minf.stbl.as_ref().expect("video minf should have stbl");

    // Find stsd in stbl children
    let vid_stsd = vid_stbl
        .children
        .iter()
        .find_map(|c| {
            if let transmux::StblChild::Stsd(ref s) = c {
                Some(s)
            } else {
                None
            }
        })
        .expect("stbl should contain stsd");

    // Check first entry is avc1
    let vid_entry = vid_stsd
        .entries
        .first()
        .expect("stsd should have at least one entry");
    match vid_entry {
        SampleEntryVariant::Avc1(avc1) => {
            assert_eq!(
                &avc1.codec_type, b"avc1",
                "video sample entry should be avc1"
            );
            assert!(!avc1.config.config.sps.is_empty(), "avc1 should have SPS");
        }
        other => panic!("expected Avc1, got {:?}", std::mem::discriminant(other)),
    }

    // Second trak should be audio (mp4a)
    let aud_trak = &moov.tracks[1];
    let aud_mdia = aud_trak.mdia.as_ref().expect("audio trak should have mdia");
    let aud_minf = aud_mdia.minf.as_ref().expect("audio mdia should have minf");
    let aud_stbl = aud_minf.stbl.as_ref().expect("audio minf should have stbl");

    let aud_stsd = aud_stbl
        .children
        .iter()
        .find_map(|c| {
            if let transmux::StblChild::Stsd(ref s) = c {
                Some(s)
            } else {
                None
            }
        })
        .expect("audio stbl should contain stsd");

    let aud_entry = aud_stsd
        .entries
        .first()
        .expect("audio stsd should have at least one entry");
    match aud_entry {
        SampleEntryVariant::Mp4a(mp4a) => {
            assert!(
                !mp4a.config_boxes.is_empty(),
                "mp4a should have config boxes"
            );
            // Verify esds is present
            let has_esds = mp4a.config_boxes.iter().any(|c| &c.box_type == b"esds");
            assert!(has_esds, "mp4a should contain esds box");
        }
        other => panic!("expected Mp4a, got {:?}", std::mem::discriminant(other)),
    }
}

// ---------------------------------------------------------------------------
// Mutate a tkhd field → bytes change
// ---------------------------------------------------------------------------

#[test]
fn tkhd_mutation_changes_bytes() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_prog.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");
    let moov_bytes = find_top_box(&data, b"moov");
    let moov = MovieBox::parse(moov_bytes).expect("should parse moov");

    let original = moov.tracks[0].tkhd.to_bytes();

    // Clone and mutate
    let mut moov2 = moov.clone();
    moov2.tracks[0].tkhd.track_id = 99;
    let mutated = moov2.tracks[0].tkhd.to_bytes();

    assert_ne!(
        original, mutated,
        "mutating track_id must change serialized bytes"
    );
}
