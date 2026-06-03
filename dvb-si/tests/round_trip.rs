//! Round-trip tests: parse → serialize → parse must produce identical bytes
//! and an identical struct for every parser type in the crate.

use dvb_si::section::Section;
use dvb_common::{Parse, Serialize};

#[test]
fn long_form_section_round_trip_is_identity() {
    let payload: [u8; 8] = [0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04];
    // Construct a full long-form section with a real CRC.
    let section_length: u16 = (5 + payload.len() as u16) + 4;
    let mut raw: Vec<u8> = Vec::with_capacity(3 + section_length as usize);
    raw.push(0x42); // table_id (SDT actual — arbitrary)
    raw.push(0xF0 | ((section_length >> 8) as u8 & 0x0F)); // SSI=1, PI=1, reserved=11, length hi
    raw.push((section_length & 0xFF) as u8);
    raw.extend_from_slice(&0x1234u16.to_be_bytes()); // extension_id
    raw.push(0xC0 | (5 << 1) | 1); // version=5, current_next=1
    raw.push(0); // section_number
    raw.push(0); // last_section_number
    raw.extend_from_slice(&payload);
    let crc = dvb_common::crc32_mpeg2::compute(&raw);
    raw.extend_from_slice(&crc.to_be_bytes());

    let parsed = Section::parse(&raw).expect("parse");
    let mut out = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut out).expect("serialize");
    assert_eq!(out, raw, "round-trip bytes differ");

    let reparsed = Section::parse(&out).expect("reparse");
    assert_eq!(parsed.table_id, reparsed.table_id);
    assert_eq!(parsed.section_syntax_indicator, reparsed.section_syntax_indicator);
    assert_eq!(parsed.private_indicator, reparsed.private_indicator);
    assert_eq!(parsed.section_length, reparsed.section_length);
    assert_eq!(parsed.extension_id, reparsed.extension_id);
    assert_eq!(parsed.version_number, reparsed.version_number);
    assert_eq!(parsed.current_next_indicator, reparsed.current_next_indicator);
    assert_eq!(parsed.section_number, reparsed.section_number);
    assert_eq!(parsed.last_section_number, reparsed.last_section_number);
    assert_eq!(parsed.payload, reparsed.payload);
    assert_eq!(parsed.crc32, reparsed.crc32);
}

#[test]
fn short_form_section_round_trip_is_identity() {
    // SSI=0: 3-byte header + payload, no CRC.
    let payload: [u8; 5] = [0x01, 0x02, 0x03, 0x04, 0x05];
    let section_length = payload.len() as u16;
    let mut raw: Vec<u8> = Vec::with_capacity(3 + section_length as usize);
    raw.push(0x70); // table_id (TDT — real short-form table)
    raw.push(0x70 | ((section_length >> 8) as u8 & 0x0F)); // SSI=0, reserved_future_use=1, reserved=11
    raw.push((section_length & 0xFF) as u8);
    raw.extend_from_slice(&payload);

    let parsed = Section::parse(&raw).expect("parse");
    let mut out = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut out).expect("serialize");
    assert_eq!(out, raw);
    assert!(parsed.crc32.is_none());
}

#[test]
fn serialize_rejects_too_small_buffer() {
    let payload: [u8; 3] = [0xAA, 0xBB, 0xCC];
    let section_length = payload.len() as u16;
    let mut raw: Vec<u8> = Vec::new();
    raw.push(0x70);
    raw.push(0x30 | ((section_length >> 8) as u8 & 0x0F));
    raw.push((section_length & 0xFF) as u8);
    raw.extend_from_slice(&payload);

    let parsed = Section::parse(&raw).unwrap();
    let mut too_small = vec![0u8; parsed.serialized_len() - 1];
    let err = parsed.serialize_into(&mut too_small).unwrap_err();
    assert!(
        matches!(err, dvb_si::error::Error::OutputBufferTooSmall { .. }),
        "expected OutputBufferTooSmall, got {err:?}"
    );
}

#[test]
fn st_short_form_round_trip_is_identity() {
    use dvb_si::tables::st::{St, TABLE_ID};

    // Build a short-form ST section with 3 stuffing bytes.
    let payload: [u8; 3] = [0x00, 0x00, 0x00];
    let section_length = payload.len() as u16;
    let mut raw: Vec<u8> = Vec::with_capacity(3 + section_length as usize);
    raw.push(TABLE_ID); // table_id = 0x72
    raw.push(0x70 | ((section_length >> 8) as u8 & 0x0F)); // SSI=0, reserved_future_use=1, reserved=11
    raw.push((section_length & 0xFF) as u8);
    raw.extend_from_slice(&payload);

    let parsed = St::parse(&raw).expect("parse ST");
    assert_eq!(parsed.len(), 3);

    let mut out = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut out).expect("serialize");
    assert_eq!(out, raw);

    let reparsed = St::parse(&out).expect("reparse");
    assert_eq!(parsed, reparsed);
}

#[test]
fn st_empty_round_trip() {
    use dvb_si::tables::st::{St, TABLE_ID};

    let raw: Vec<u8> = vec![TABLE_ID, 0x70, 0x00];
    let parsed = St::parse(&raw).expect("parse empty ST");
    assert!(parsed.is_empty());

    let mut out = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut out).expect("serialize");
    assert_eq!(out, raw);

    let reparsed = St::parse(&out).expect("reparse");
    assert_eq!(parsed, reparsed);
}

#[test]
fn st_serialize_rejects_too_small_buffer() {
    use dvb_si::tables::st::{St, TABLE_ID};

    let payload: [u8; 5] = [0x00, 0x00, 0x00, 0x00, 0x00];
    let section_length = payload.len() as u16;
    let mut raw: Vec<u8> = Vec::with_capacity(3 + section_length as usize);
    raw.push(TABLE_ID);
    raw.push(0x30 | ((section_length >> 8) as u8 & 0x0F));
    raw.push((section_length & 0xFF) as u8);
    raw.extend_from_slice(&payload);

    let parsed = St::parse(&raw).unwrap();
    let mut too_small = vec![0u8; parsed.serialized_len() - 1];
    let err = parsed.serialize_into(&mut too_small).unwrap_err();
    assert!(
        matches!(err, dvb_si::error::Error::OutputBufferTooSmall { .. }),
        "expected OutputBufferTooSmall, got {err:?}"
    );
}
