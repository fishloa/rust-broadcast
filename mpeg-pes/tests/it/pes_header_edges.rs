//! PES header edges: the 2-bit scrambling field, the `PES_packet_length`
//! zero-length rule, the 255-byte optional-field limit, the `InvalidSync`
//! payload, and truncated optional-field regions.

use mpeg_pes::{
    Error, PACKET_START_CODE_PREFIX, PesHeader, PesPacket, ScramblingControl, StreamId,
};

const OPTIONAL_FIELDS_MAX: usize = 255;

fn raw_pes(stream_id: u8, f2: u8, opt: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&PACKET_START_CODE_PREFIX);
    b.push(stream_id);
    let body = 3 + opt.len() + payload.len();
    b.extend_from_slice(&(body as u16).to_be_bytes());
    b.push(0x80);
    b.push(f2);
    b.push(opt.len() as u8);
    b.extend_from_slice(opt);
    b.extend_from_slice(payload);
    b
}

#[test]
fn a_zero_length_payload_is_refused_for_an_audio_stream_but_allowed_for_video() {
    let audio = PesPacket::new(StreamId::new(0xC0), &[]);
    assert!(matches!(
        audio.serialize_into(&mut [0u8; 32]),
        Err(Error::FieldOverflow(_))
    ));

    let video = PesPacket::new(StreamId::new(0xE0), &[]);
    let mut buf = [0u8; 6];
    assert_eq!(video.serialize_into(&mut buf).expect("unbounded"), 6);
    assert_eq!(&buf[4..6], &[0, 0]);
}

#[test]
fn the_optional_field_limit_is_255_bytes() {
    let mut header = PesHeader::new();
    let stuffing = vec![0xFFu8; OPTIONAL_FIELDS_MAX];
    header.header_stuffing = &stuffing;
    let mut packet = PesPacket::new(StreamId::new(0xC0), &[]);
    packet.header = Some(header);

    let mut buf = vec![0u8; packet.serialized_len()];
    packet.serialize_into(&mut buf).expect("255 bytes fit");
    assert_eq!(buf[8], 255);
    let parsed = PesPacket::parse(&buf).expect("parse");
    assert_eq!(parsed, packet);

    let mut header = PesHeader::new();
    let stuffing = vec![0xFFu8; OPTIONAL_FIELDS_MAX + 1];
    header.header_stuffing = &stuffing;
    let mut packet = PesPacket::new(StreamId::new(0xC0), &[]);
    packet.header = Some(header);
    let mut buf = vec![0u8; packet.serialized_len()];
    assert!(matches!(
        packet.serialize_into(&mut buf),
        Err(Error::OptionalFieldsTooLarge(256))
    ));
}

#[test]
fn serialize_optional_fields_reports_the_stuffing_it_wrote() {
    let mut header = PesHeader::new();
    let stuffing = vec![0xFFu8; 3];
    header.header_stuffing = &stuffing;
    let mut buf = [0u8; 3];
    let written = header
        .serialize_optional_fields(&mut buf)
        .expect("three stuffing bytes");
    assert_eq!(written, 3);
    assert_eq!(buf, [0xFF, 0xFF, 0xFF]);
}

#[test]
fn invalid_sync_reports_the_three_bytes_it_found() {
    let cases = [
        ([0x00u8, 0x00, 0x02, 0xE0, 0x00, 0x00], 0x000002u32),
        ([0x01, 0x00, 0x02, 0xE0, 0x00, 0x00], 0x010002),
        ([0x00, 0x05, 0x02, 0xE0, 0x00, 0x00], 0x000502),
    ];
    for (bytes, found) in cases {
        let err = PesPacket::parse(&bytes).expect_err("bad prefix");
        match err {
            Error::InvalidSync(sync) => {
                assert_eq!(sync.found, found);
                assert_eq!(sync.expected, 0x000001);
            }
            other => panic!("expected InvalidSync, got {other:?}"),
        }
    }
}

#[test]
fn a_packet_too_short_for_the_fixed_header_is_rejected() {
    assert!(matches!(
        PesPacket::parse(&[0x00, 0x00, 0x01, 0xE0, 0x00, 0x00]),
        Err(Error::BufferTooShort(_))
    ));
}

#[test]
fn a_timestamp_flag_with_an_empty_optional_region_is_rejected() {
    assert!(matches!(
        PesPacket::parse(&raw_pes(0xE0, 0x80, &[], &[])),
        Err(Error::BufferTooShort(_))
    ));
    assert!(matches!(
        PesPacket::parse(&raw_pes(0xE0, 0xC0, &[], &[])),
        Err(Error::BufferTooShort(_))
    ));
}

#[test]
fn a_trick_mode_flag_with_an_empty_optional_region_is_rejected() {
    assert!(matches!(
        PesPacket::parse(&raw_pes(0xE0, 0x08, &[], &[])),
        Err(Error::BufferTooShort(_))
    ));
}

#[test]
fn an_additional_copy_info_flag_with_an_empty_optional_region_is_rejected() {
    assert!(matches!(
        PesPacket::parse(&raw_pes(0xE0, 0x04, &[], &[])),
        Err(Error::BufferTooShort(_))
    ));
}

#[test]
fn the_scrambling_control_label_and_display_agree() {
    assert_eq!(ScramblingControl::NotScrambled.name(), "not_scrambled");
    assert_eq!(ScramblingControl::NotScrambled.to_string(), "not_scrambled");
    assert_eq!(ScramblingControl::Reserved(2).name(), "reserved");
    assert_eq!(ScramblingControl::Reserved(2).to_string(), "reserved(0x02)");
}
