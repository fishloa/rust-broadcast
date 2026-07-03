//! Real-fixture round-trip tests for sample-timing and segment-index boxes.
//!
//! These tests navigate the container hierarchy in real MP4 files and verify:
//!
//! - `stts` / `ctts` (and the trak's `elst`) extracted from `h264_aac_prog.mp4`
//!   via `moov → trak → [edts → elst] → mdia → minf → stbl`
//! - `sidx` from `h264_sidx.mp4` at top level via `box_iter`
//!
//! Each parsed box is serialised back and compared byte-for-byte with the
//! original box body (excluding the 8-byte box header).

use broadcast_common::{Parse, Serialize};
use transmux::{CompositionOffsetBox, EditListBox, SegmentIndexBox, TimeToSampleBox, box_iter};

/// Navigate a container hierarchy by following child boxes with the given
/// four-CC sequence. Returns the body of the final box in the chain.
fn enter_chain<'a>(data: &'a [u8], types: &[&[u8; 4]]) -> &'a [u8] {
    let mut current = data;
    for &ty in types {
        let mut offset = 0usize;
        loop {
            if offset + 8 > current.len() {
                panic!("box {:?} not found", std::str::from_utf8(ty).unwrap());
            }
            let size = u32::from_be_bytes([
                current[offset],
                current[offset + 1],
                current[offset + 2],
                current[offset + 3],
            ]) as usize;
            let box_ty = &current[offset + 4..offset + 8];
            if size == 0 || size < 8 {
                panic!(
                    "box {:?} not found (bad size)",
                    std::str::from_utf8(ty).unwrap()
                );
            }
            if box_ty == ty {
                current = &current[offset + 8..offset + size];
                break;
            }
            offset += size;
        }
    }
    current
}

/// Find a specific box by type inside a container's body.
/// Returns the full box bytes (including 8-byte header).
fn find_full_box<'a>(body: &'a [u8], four_cc: &[u8; 4]) -> &'a [u8] {
    let mut offset = 0usize;
    while offset + 8 <= body.len() {
        let size = u32::from_be_bytes([
            body[offset],
            body[offset + 1],
            body[offset + 2],
            body[offset + 3],
        ]) as usize;
        let ty = &body[offset + 4..offset + 8];
        if size == 0 || size < 8 {
            break;
        }
        if ty == four_cc {
            return &body[offset..offset + size];
        }
        offset += size;
    }
    panic!("box {:?} not found", std::str::from_utf8(four_cc).unwrap());
}

// ---------------------------------------------------------------------------
// Fixture: h264_aac_prog.mp4 — stts, ctts (v0, non-zero), elst
// ---------------------------------------------------------------------------

#[test]
fn stts_real_fixture_round_trip() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_prog.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");

    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let stts_box = find_full_box(stbl, b"stts");

    let parsed = TimeToSampleBox::parse(stts_box).expect("should parse stts from real fixture");

    // The fixture has 50 samples, all with delta=512
    assert_eq!(parsed.entries.len(), 1, "stts entry count");
    assert_eq!(parsed.entries[0].sample_count, 50, "stts sample_count");
    assert_eq!(parsed.entries[0].sample_delta, 512, "stts sample_delta");

    // Round-trip: serialized bytes must match original box
    let serialized = parsed.to_bytes();
    assert_eq!(
        serialized.len(),
        stts_box.len(),
        "stts serialized length matches original"
    );
    assert_eq!(
        &serialized[..],
        stts_box,
        "stts round-trip must be byte-identical"
    );
}

#[test]
fn ctts_real_fixture_round_trip() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_prog.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");

    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let ctts_box = find_full_box(stbl, b"ctts");

    let parsed =
        CompositionOffsetBox::parse(ctts_box).expect("should parse ctts from real fixture");

    // v0 with 34 entries and non-zero offsets (B-frames)
    assert_eq!(parsed.version, 0, "ctts version");
    assert_eq!(parsed.entries.len(), 34, "ctts entry count");

    // Verify a couple non-zero offsets
    assert_eq!(parsed.entries[0].sample_offset, 1024, "ctts[0] offset");
    assert_eq!(parsed.entries[1].sample_offset, 2048, "ctts[1] offset");
    assert_eq!(parsed.entries[2].sample_offset, 512, "ctts[2] offset");
    assert_eq!(parsed.entries[33].sample_offset, 1024, "ctts[33] offset");

    // Round-trip: serialized bytes must match original box
    let serialized = parsed.to_bytes();
    assert_eq!(
        serialized.len(),
        ctts_box.len(),
        "ctts serialized length matches original"
    );
    assert_eq!(
        &serialized[..],
        ctts_box,
        "ctts round-trip must be byte-identical"
    );
}

#[test]
fn elst_real_fixture_round_trip() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_prog.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");

    let trak = enter_chain(&data, &[b"moov", b"trak"]);
    // elst is inside edts, which is inside trak
    let edts = find_full_box(trak, b"edts");
    let elst_box = find_full_box(&edts[8..], b"elst"); // edts body (after its own header)

    let parsed = EditListBox::parse(elst_box).expect("should parse elst from real fixture");

    // v0 with 1 entry: segment_duration=2000, media_time=1024
    assert_eq!(parsed.version, 0, "elst version");
    assert_eq!(parsed.entries.len(), 1, "elst entry count");
    assert_eq!(
        parsed.entries[0].segment_duration, 2000,
        "elst segment_duration"
    );
    assert_eq!(parsed.entries[0].media_time, 1024, "elst media_time");
    assert_eq!(
        parsed.entries[0].media_rate_integer, 1,
        "elst media_rate_integer"
    );
    assert_eq!(
        parsed.entries[0].media_rate_fraction, 0,
        "elst media_rate_fraction"
    );

    // Round-trip: serialized bytes must match original box
    let serialized = parsed.to_bytes();
    assert_eq!(
        serialized.len(),
        elst_box.len(),
        "elst serialized length matches original"
    );
    assert_eq!(
        &serialized[..],
        elst_box,
        "elst round-trip must be byte-identical"
    );
}

#[test]
fn elst_mutation_changes_bytes() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_prog.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");
    let trak = enter_chain(&data, &[b"moov", b"trak"]);
    let edts = find_full_box(trak, b"edts");
    let elst_box = find_full_box(&edts[8..], b"elst");

    let mut parsed = EditListBox::parse(elst_box).expect("should parse elst");
    let original = parsed.to_bytes();
    parsed.entries[0].media_time = 512; // was 1024
    let mutated = parsed.to_bytes();
    assert_ne!(
        mutated, original,
        "mutating media_time must change serialized bytes"
    );
    // Verify the change is at the expected byte offset
    // elst v0: BoxHeader(8=0..7) + FullBox(4=8..11) + entry_count(4=12..15) + seg_dur(4=16..19) + media_time(4=20..23)
    let mt = u32::from_be_bytes(mutated[20..24].try_into().unwrap());
    assert_eq!(mt, 512, "media_time should be 512 at bytes 20..23");
}

// ---------------------------------------------------------------------------
// Fixture: h264_sidx.mp4 — sidx at top level via box_iter
// ---------------------------------------------------------------------------

#[test]
fn sidx_real_fixture_round_trip() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_sidx.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");

    // Find sidx via box_iter
    let mut found = false;
    for box_ref in box_iter(&data) {
        let (br, _consumed) = box_ref.expect("valid box");
        if br.header.box_type.is(b"sidx") {
            // Reconstruct full box bytes: header + body
            let hdr_len = br.header.serialized_len();
            let mut full_box = vec![0u8; hdr_len + br.body.len()];
            br.header.serialize_into(&mut full_box[..hdr_len]).unwrap();
            full_box[hdr_len..].copy_from_slice(br.body);

            let parsed =
                SegmentIndexBox::parse(&full_box).expect("should parse sidx from real fixture");

            // Verify known values
            assert_eq!(parsed.version, 0, "sidx version");
            assert_eq!(parsed.reference_id, 1, "sidx reference_id");
            assert_eq!(parsed.timescale, 90000, "sidx timescale");
            assert_eq!(parsed.earliest_presentation_time, 0, "sidx EPT");
            assert_eq!(parsed.first_offset, 68, "sidx first_offset");
            assert_eq!(parsed.references.len(), 2, "sidx reference_count");

            assert_eq!(parsed.references[0].reference_type, 0);
            assert_eq!(parsed.references[0].referenced_size, 1000);
            assert_eq!(parsed.references[0].subsegment_duration, 180000);
            assert_eq!(
                parsed.references[0].starts_with_sap, 1,
                "sidx[0].starts_with_sap"
            );
            assert_eq!(parsed.references[0].sap_type, 1, "sidx[0].sap_type");

            assert_eq!(parsed.references[1].referenced_size, 1200);

            // Round-trip
            let serialized = parsed.to_bytes();
            assert_eq!(
                serialized.len(),
                full_box.len(),
                "sidx serialized length matches original"
            );
            assert_eq!(
                &serialized[..],
                &full_box[..],
                "sidx round-trip must be byte-identical"
            );
            found = true;
            break;
        }
    }
    assert!(found, "sidx box not found in fixture");
}

#[test]
fn sidx_mutation_changes_bytes() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_sidx.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");

    let mut parsed = None;
    for box_ref in box_iter(&data) {
        let (br, _consumed) = box_ref.expect("valid box");
        if br.header.box_type.is(b"sidx") {
            let hdr_len = br.header.serialized_len();
            let mut full_box = vec![0u8; hdr_len + br.body.len()];
            br.header.serialize_into(&mut full_box[..hdr_len]).unwrap();
            full_box[hdr_len..].copy_from_slice(br.body);
            parsed = Some(SegmentIndexBox::parse(&full_box).expect("should parse"));
            break;
        }
    }
    let mut sidx = parsed.expect("sidx must exist");
    let original = sidx.to_bytes();

    // Mutate referenced_size of first reference
    sidx.references[0].referenced_size = 999;
    let mutated = sidx.to_bytes();
    assert_ne!(
        mutated, original,
        "mutating referenced_size must change serialized bytes"
    );
}
