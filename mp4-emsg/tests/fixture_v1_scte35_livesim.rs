//! Real-world fixture: a **version 1** `emsg` box carrying an SCTE 35
//! `splice_info_section`, extracted from a live DASH segment produced by the
//! DASH-IF reference live simulator
//! (`livesim2.dashif.org/livesim2/scte35_2/testpic_2s`, `InbandEventStream`
//! `urn:scte:scte35:2013:bin`).
//!
//! This complements the constructed v0 fixture (`scte35_emsg_v0.bin`): it is a
//! genuine packager-emitted v1 box (representation-relative timing,
//! `presentation_time` as a u64), so it carries the real field values and the
//! exact wire layout a DASH client encounters — the kind of coverage a
//! hand-built vector can miss.
//!
//! Asserts the decoded fields, SCTE 35 recognition, and a byte-exact round-trip.

use std::fs;

use mp4_emsg::{EmsgBox, EmsgVersion, PresentationTime};

fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/shared/emsg_v1_scte35_livesim.bin"
    );
    fs::read(path).expect("fixture emsg_v1_scte35_livesim.bin must be committed")
}

#[test]
fn parses_real_v1_scte35_fields() {
    let data = fixture();
    let b = EmsgBox::parse(&data).expect("real v1 emsg must parse");

    assert_eq!(b.version(), EmsgVersion::RepresentationRelative);
    assert_eq!(b.scheme_id_uri, "urn:scte:scte35:2013:bin");
    assert_eq!(b.value, "");
    assert_eq!(b.timescale, 90_000);
    assert_eq!(
        b.presentation_time,
        PresentationTime::Absolute(160_391_675_700_000)
    );
    assert_eq!(b.event_duration, 900_000);
    assert_eq!(b.id, 1_782_129_730);

    // SCTE 35 scheme recognised; message_data is the splice_info_section.
    assert!(b.is_scte35());
    assert_eq!(b.message_data[0], 0xFC, "SCTE 35 table_id");
}

#[test]
fn real_v1_byte_exact_round_trip() {
    let data = fixture();
    let b = EmsgBox::parse(&data).unwrap();

    // size recomputes to the on-wire box length.
    assert_eq!(b.serialized_len(), data.len());
    let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    assert_eq!(size as usize, b.serialized_len());

    // serialize is byte-identical to the captured box.
    let out = b.to_vec().unwrap();
    assert_eq!(
        out, data,
        "serialize must reproduce the real v1 box byte-for-byte"
    );

    // serialize → parse → equal.
    assert_eq!(EmsgBox::parse(&out).unwrap(), b);
}
