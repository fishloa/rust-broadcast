//! Fixture test: a version 0 `emsg` box carrying a real SCTE 35
//! `splice_info_section` in `message_data`.
//!
//! The embedded SCTE 35 message is the canonical `splice_insert()` example
//! reproduced across SCTE 35 references (Cablelabs/Comcast tutorials, AWS
//! Elemental docs, the `threefive` corpus): `splice_event_id` 0x4800008F, with
//! a valid `splice_info_section` CRC_32. It is wrapped in an `emsg` v0 box with
//! `scheme_id_uri = "urn:scte:scte35:2013:bin"`, an empty `value`, and
//! `timescale = 90000`.
//!
//! Asserts the decoded fields, a byte-exact round-trip, and that `size`
//! recomputes to the fixture length.

use std::fs;

use mp4_emsg::{EmsgBox, EmsgVersion, PresentationTime};

fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/shared/scte35_emsg_v0.bin"
    );
    fs::read(path).expect("fixture scte35_emsg_v0.bin must be committed")
}

#[test]
fn parses_scte35_emsg_fields() {
    let data = fixture();
    let b = EmsgBox::parse(&data).expect("emsg must parse");

    assert_eq!(b.version(), EmsgVersion::SegmentRelative);
    assert_eq!(b.scheme_id_uri, "urn:scte:scte35:2013:bin");
    assert_eq!(b.value, "");
    assert_eq!(b.timescale, 90_000);
    assert_eq!(b.presentation_time, PresentationTime::Delta(0));
    assert_eq!(b.event_duration, 0xFFFF_FFFF);
    assert_eq!(b.id, 1);

    // SCTE 35 scheme is recognised; message_data is the splice_info_section.
    assert!(b.is_scte35());
    assert_eq!(b.message_data.len(), 50);
    assert_eq!(b.message_data[0], 0xFC, "SCTE 35 table_id");
}

#[test]
fn scte35_emsg_byte_exact_round_trip_and_size_recompute() {
    let data = fixture();
    let b = EmsgBox::parse(&data).unwrap();

    // size recomputes to the actual box length.
    assert_eq!(b.serialized_len(), data.len());
    // The on-wire size field equals serialized_len().
    let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    assert_eq!(size as usize, b.serialized_len());

    let out = b.to_vec().unwrap();
    assert_eq!(out, data, "serialize must be byte-identical to the fixture");

    // serialize → parse → equal.
    assert_eq!(EmsgBox::parse(&out).unwrap(), b);
}

#[test]
fn corrupting_message_data_changes_wire_bytes() {
    let data = fixture();
    let b = EmsgBox::parse(&data).unwrap();

    // Mutate one message_data byte through typed reconstruction.
    let mut md = b.message_data.to_vec();
    md[0] ^= 0xFF;
    let mutated = EmsgBox {
        message_data: &md,
        ..b.clone()
    };
    let out = mutated.to_vec().unwrap();
    assert_ne!(
        out, data,
        "different message_data must change the wire bytes"
    );
    assert_eq!(out.len(), data.len(), "but the box length is unchanged");
}
