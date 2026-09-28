//! RIST-W3/W5/W6 coverage: the `ReportPart` accessor, RFC 3550 P-bit padding
//! preservation through a compound, and verbatim round-trip of sub-packets
//! whose packet type this crate does not model (TR-06-1:2020 §5.2, RFC 3550
//! §6.4.1).

use broadcast_common::{Parse, Serialize};
use rist_runtime::{Error, ReportPart, RistReceiverCompound, RistSenderCompound, UnknownPacket};
use rtcp_packet::{ReceiverReport, SenderReport};

/// Common-header length in bytes (RFC 3550 §6.4.1).
const RTCP_HEADER_LEN: usize = 4;
/// One 32-bit word, in bytes.
const WORD_LEN: usize = 4;
/// RTCP PT for Receiver Report (RFC 3550 §6.4.2).
const PT_RECEIVER_REPORT: u8 = 201;
/// RTCP PT for Source Description (RFC 3550 §6.5).
const PT_SOURCE_DESCRIPTION: u8 = 202;
/// RTCP PT for a type outside the RFC 3550 §6 core set used here (XR,
/// RFC 3611) — deliberately unmodelled by this crate.
const PT_UNMODELLED: u8 = 207;
/// SSRC used throughout (an arbitrary fixture value).
const SSRC: u32 = 0xD561_5604;
/// CNAME used throughout.
const CNAME: &str = "rist-test";

/// Wire layout of a minimal RR sub-packet: `V=2, P, RC` in byte 0.
fn rr_bytes(padded: bool) -> Vec<u8> {
    // Unpadded RR: 2 words (header + SSRC); the length field is words - 1.
    // Padded RR: 3 words (body 8 + 3 zero pad bytes + count byte 4); RFC
    // 3550 §6.1 says the count covers the padding plus itself, and the
    // count byte lands in the final word.
    let mut v = Vec::new();
    let p_bit = if padded { 0x20 } else { 0 };
    let words = if padded { 3 } else { 2 };
    v.extend_from_slice(&[(2 << 6) | p_bit, PT_RECEIVER_REPORT]);
    v.extend_from_slice(&((words - 1) as u16).to_be_bytes());
    v.extend_from_slice(&SSRC.to_be_bytes());
    if padded {
        v.extend_from_slice(&[0, 0, 0, 4]);
    }
    v
}

/// `ReportPart::name()`/`ssrc()` project the variant identity (RIST-W3).
#[test]
fn report_part_names_and_ssrc() {
    let sr = SenderReport {
        ssrc: SSRC,
        ntp_msw: 0,
        ntp_lsw: 0,
        rtp_timestamp: 0,
        packet_count: 0,
        octet_count: 0,
        report_blocks: vec![],
    };
    let rr = ReceiverReport {
        ssrc: SSRC | 1,
        report_blocks: vec![],
    };
    let part_sr = ReportPart::Sr(sr);
    let part_rr = ReportPart::Rr(rr);
    assert_eq!(part_sr.name(), "SR");
    assert_eq!(part_rr.name(), "RR");
    assert_eq!(part_sr.ssrc(), SSRC);
    assert_eq!(part_rr.ssrc(), SSRC | 1);
    // `Display` delegates to `name()` (#204 convention).
    assert_eq!(part_sr.to_string(), "SR");
    assert_eq!(part_rr.to_string(), "RR");
}

/// A padded RR report sub-packet round-trips byte-identically, and the
/// preserved region is exposed on the compound (RIST-W5).
#[test]
fn padded_report_sub_packet_round_trip() {
    let mut compound_bytes = rr_bytes(true);
    // SDES(CNAME) trailer so the compound is structurally valid.
    compound_bytes.extend_from_slice(&sdes_bytes());
    let parsed = RistReceiverCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.report.name(), "RR");
    // 4-byte P-bit region: three preserved zero bytes + count byte.
    assert_eq!(parsed.report_padding, vec![0, 0, 0]);
    assert_eq!(parsed.cname, CNAME);
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);
}

/// A set P bit whose count byte is 0 is rejected (RIST-W5).
#[test]
fn zero_padding_count_is_error() {
    let mut compound_bytes = rr_bytes(false);
    // Set the P bit and make the count byte (last byte of the RR sub-packet,
    // which here is the last SSRC byte) claim zero padding.
    compound_bytes[0] |= 0x20;
    // The sub-packet's last byte is at index 7: set it to a value the count
    // byte will read as 0.
    compound_bytes[7] = 0;
    let err = RistReceiverCompound::parse(&compound_bytes).unwrap_err();
    assert!(
        matches!(err, Error::InvalidPaddingCount { count: 0, .. }),
        "unexpected error: {err:?}"
    );
}

/// An SR/RR appearing *after* the first sub-packet is preserved verbatim as
/// an `UnknownPacket` and round-trips (RIST-W6: unknown sub-packets are not
/// dropped or reordered).
#[test]
fn unknown_sub_packet_round_trip() {
    let mut compound_bytes = rr_bytes(false);
    compound_bytes.extend_from_slice(&sdes_bytes());
    // Unmodelled PT 207 (XR) sub-packet: header + one zero word.
    let xr: Vec<u8> = vec![(2 << 6), PT_UNMODELLED, 0, 1, 0, 0, 0, 0];
    compound_bytes.extend_from_slice(&xr);
    let parsed = RistReceiverCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.unknown.len(), 1);
    let unk: &UnknownPacket = &parsed.unknown[0];
    assert_eq!(unk.packet_type, PT_UNMODELLED);
    assert_eq!(unk.payload, vec![0, 0, 0, 0]);
    assert_eq!(unk.name(), "reserved");
    // The unknown keeps its wire position (after the SDES).
    assert_eq!(unk.position, Some(1));
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);
}

/// An unknown sub-packet carrying P-bit padding also round-trips, with the
/// padding region preserved (RIST-W6 + RIST-W5 combined).
#[test]
fn unknown_sub_packet_with_padding_round_trip() {
    let mut compound_bytes = rr_bytes(false);
    compound_bytes.extend_from_slice(&sdes_bytes());
    // PT 207: 4-byte header + 4-byte body + 3-byte padding region + count
    // byte 4 = 12 bytes total, so the length field is 2 (words - 1).
    let mut xr: Vec<u8> = vec![(2 << 6) | 0x20, PT_UNMODELLED, 0, 2];
    xr.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
    xr.extend_from_slice(&[0, 0, 0, 4]);
    compound_bytes.extend_from_slice(&xr);
    let parsed = RistReceiverCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.unknown.len(), 1);
    let unk = &parsed.unknown[0];
    assert_eq!(unk.payload, vec![0xAA, 0xBB, 0xCC, 0xDD]);
    assert_eq!(unk.padding, vec![0, 0, 0]);
    assert_eq!(parsed.unknown[0].position, Some(1));
    assert_eq!(parsed.report.name(), "RR");
    assert_eq!(parsed.cname, CNAME);
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);
}

/// The `UnknownPacket` builder API keeps hand-built positions valid: a
/// fresh unknown (`position: None`) is appended after parsed ones.
#[test]
fn hand_built_unknown_appends_last() {
    let compound = RistSenderCompound {
        report: ReportPart::Rr(ReceiverReport {
            ssrc: SSRC,
            report_blocks: vec![],
        }),
        report_padding: vec![],
        cname: String::from(CNAME),
        rtt_echo: None,
        unknown: vec![UnknownPacket {
            packet_type: PT_UNMODELLED,
            count: 0,
            payload: vec![0; WORD_LEN],
            padding: vec![],
            position: None,
        }],
    };
    let bytes = compound.try_to_bytes().unwrap();
    let parsed = RistSenderCompound::parse(&bytes).unwrap();
    // Parse re-records the position; re-serialize is stable.
    assert_eq!(parsed.unknown[0].position, Some(1));
    let again = parsed.try_to_bytes().unwrap();
    assert_eq!(again, bytes);
    // And the hand-built original still serializes identically.
    assert_eq!(compound.try_to_bytes().unwrap(), bytes);
    let _ = RTCP_HEADER_LEN;
}

/// SDES(CNAME) for a single source; matches `build_sdes` in the crate.
fn sdes_bytes() -> Vec<u8> {
    let text = CNAME.as_bytes();
    // chunk: 4 ssrc + 2 header + text + 1 terminator, padded to a word.
    let chunk_unpadded = WORD_LEN + 2 + text.len() + 1;
    let chunk_padded = chunk_unpadded.div_ceil(WORD_LEN) * WORD_LEN;
    let total = RTCP_HEADER_LEN + chunk_padded;
    let mut v = Vec::new();
    let length_field = (total / WORD_LEN - 1) as u16;
    v.extend_from_slice(&[(2 << 6) | 1, PT_SOURCE_DESCRIPTION]);
    v.extend_from_slice(&length_field.to_be_bytes());
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.push(1); // CNAME
    v.push(text.len() as u8);
    v.extend_from_slice(text);
    v.push(0); // terminator
    while v.len() % WORD_LEN != 0 {
        v.push(0);
    }
    v
}
