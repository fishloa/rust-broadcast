//! PSI/SI section framing edges: `validate_crc` boundaries, the parsed CRC
//! accessor, `private_indicator` / `section_length >= 256` / `extension_id >=
//! 256` on the serializer, the short-form reserved bits, and the minimum
//! long-form length check.

use broadcast_common::Parse;
use mpeg_ts::section::{Section, validate_crc};

fn long_form(table_id: u8, private_indicator: bool, extension_id: u16, payload: &[u8]) -> Vec<u8> {
    let section_length = 5 + payload.len() + 4;
    let mut b = vec![0u8; 3];
    b[0] = table_id;
    b[1] = 0x80 | (u8::from(private_indicator) << 6) | 0x30 | ((section_length >> 8) & 0x0F) as u8;
    b[2] = (section_length & 0xFF) as u8;
    b.push((extension_id >> 8) as u8);
    b.push((extension_id & 0xFF) as u8);
    b.push(0xC1);
    b.push(0x00);
    b.push(0x00);
    b.extend_from_slice(payload);
    let crc = broadcast_common::crc32_mpeg2::compute(&b);
    b.extend_from_slice(&crc.to_be_bytes());
    b
}

fn short_form(table_id: u8, payload: &[u8]) -> Vec<u8> {
    let section_length = payload.len();
    let mut b = vec![table_id, 0x00, (section_length & 0xFF) as u8];
    b.extend_from_slice(payload);
    b
}

#[test]
fn validate_crc_accepts_a_three_byte_short_form_header() {
    assert!(validate_crc(&[0x00, 0x00, 0x00]).is_ok());
}

#[test]
fn validate_crc_accepts_an_eleven_byte_long_form_prefix() {
    let raw = [0x42u8, 0xB0, 0x08, 0, 0, 0, 0, 0, 0, 0, 0];
    assert!(validate_crc(&raw).is_ok());
}

#[test]
fn validate_crc_checks_a_twelve_byte_long_form_section() {
    let raw = long_form(0x42, false, 1, &[]);
    assert_eq!(raw.len(), 12);
    assert!(validate_crc(&raw).is_ok());

    let mut bad = raw.clone();
    bad[11] ^= 0xFF;
    assert!(matches!(
        validate_crc(&bad),
        Err(mpeg_ts::Error::CrcMismatch(_))
    ));
}

#[test]
fn validate_crc_does_not_check_a_short_form_section() {
    let mut raw = vec![0x70u8; 20];
    raw[0] = 0x70;
    raw[1] = 0x70;
    raw[2] = 0x11;
    assert!(validate_crc(&raw).is_ok());
}

#[test]
fn a_parsed_long_form_section_reports_its_wire_crc() {
    let raw = long_form(0x42, false, 0x0001, &[0xAA, 0xBB]);
    let expected = u32::from_be_bytes([
        raw[raw.len() - 4],
        raw[raw.len() - 3],
        raw[raw.len() - 2],
        raw[raw.len() - 1],
    ]);
    let section = Section::parse(&raw).expect("parse");
    assert_eq!(section.crc32(), Some(expected));

    let short = short_form(0x70, &[1, 2, 3]);
    assert_eq!(Section::parse(&short).expect("parse").crc32(), None);
}

#[test]
fn a_section_with_private_indicator_high_extension_and_high_length_round_trips() {
    let payload = vec![0x5Au8; 400];
    let raw = long_form(0x42, true, 0xABCD, &payload);
    assert_eq!(raw.len(), 3 + 5 + 400 + 4);
    assert_eq!(raw[1] & 0x40, 0x40);
    assert_eq!(raw[1] & 0x0F, 0x01);
    assert_eq!(raw[2], 0x99);
    assert_eq!(&raw[3..5], &[0xAB, 0xCD]);

    let section = Section::parse(&raw).expect("parse");
    assert!(section.private_indicator);
    assert_eq!(section.extension_id, 0xABCD);
    assert_eq!(section.section_length(), 409);
    assert_eq!(section.payload, &payload[..]);

    let out = section.serialize().expect("serialize");
    assert_eq!(out, raw);
}

#[test]
fn a_short_form_section_parses_with_the_reserved_byte5_bits() {
    let raw = short_form(0x70, &[0x11, 0x22, 0x33]);
    let section = Section::parse(&raw).expect("parse");
    assert!(!section.section_syntax_indicator);
    assert_eq!(section.byte5_reserved, 3);
    assert_eq!(section.payload, &[0x11, 0x22, 0x33]);
    assert_eq!(section.serialize().expect("serialize"), raw);
}

#[test]
fn a_long_form_section_shorter_than_the_minimum_is_rejected() {
    let raw = [0x42u8, 0xB0, 0x02, 0x00, 0x00];
    assert!(matches!(
        Section::parse(&raw),
        Err(mpeg_ts::Error::BufferTooShort(_))
    ));
}
