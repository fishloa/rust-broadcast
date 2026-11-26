//! RIST-W3/W5/W6 coverage: the `ReportPart` accessor, RFC 3550 P-bit padding
//! preservation through a compound, and verbatim round-trip of sub-packets
//! whose packet type this crate does not model (TR-06-1:2020 §5.2, RFC 3550
//! §6.4.1).

use broadcast_core::error::BufferTooShort;
use broadcast_core::{Parse, Serialize};
use rist_runtime::{
    Error, GenericNack, NackFci, ReportPart, RistReceiverCompound, RistSenderCompound, RttEcho,
    RttEchoKind, UnknownPacket,
};
use rtp_packet::Ssrc;
use rtp_packet::rtcp::{ReceiverReport, SenderReport};

/// Common-header length in bytes (RFC 3550 §6.4.1).
const RTCP_HEADER_LEN: usize = 4;
/// One 32-bit word, in bytes.
const WORD_LEN: usize = 4;
/// RTCP PT for Sender Report (RFC 3550 §6.4.1).
const PT_SENDER_REPORT: u8 = 200;
/// RTCP PT for Receiver Report (RFC 3550 §6.4.2).
const PT_RECEIVER_REPORT: u8 = 201;
/// RTCP PT for Source Description (RFC 3550 §6.5).
const PT_SOURCE_DESCRIPTION: u8 = 202;
/// RTCP PT for a type outside the RFC 3550 §6 core set used here (XR,
/// RFC 3611) — deliberately unmodelled by this crate.
const PT_UNMODELLED: u8 = 207;
/// RTCP PT for Transport-Layer Feedback (RFC 4585 §6.1).
const PT_RTPFB: u8 = 205;
/// RTCP PT for APP (RFC 3550 §6.7).
const PT_APP: u8 = 204;
/// Generic NACK FMT (RFC 4585 §6.2.1).
const FMT_GENERIC_NACK: u8 = 1;
/// SSRC used throughout (an arbitrary fixture value).
const SSRC: u32 = 0xD561_5604;
/// CNAME used throughout.
const CNAME: &str = "rist-test";

/// Wire layout of an SR sub-packet: 28 bytes of sender info, optionally
/// followed by `fill` padding bytes and their count byte.
fn sr_bytes(fill: &[u8]) -> Vec<u8> {
    let total = 28 + fill.len() + usize::from(!fill.is_empty());
    let p_bit = if fill.is_empty() { 0 } else { 0x20 };
    let mut v = Vec::new();
    v.extend_from_slice(&[(2 << 6) | p_bit, PT_SENDER_REPORT]);
    v.extend_from_slice(&((total / WORD_LEN - 1) as u16).to_be_bytes());
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(&0xE0E1_E2E3u32.to_be_bytes());
    v.extend_from_slice(&0x1020_3040u32.to_be_bytes());
    v.extend_from_slice(&0x0009_0000u32.to_be_bytes());
    v.extend_from_slice(&100u32.to_be_bytes());
    v.extend_from_slice(&50_000u32.to_be_bytes());
    v.extend_from_slice(fill);
    if !fill.is_empty() {
        v.push((fill.len() + 1) as u8);
    }
    v
}

/// Wire layout of an RR sub-packet (8-byte body) with an optional `fill`
/// padding region followed by its count byte. RFC 3550 §6.1 says the count
/// covers the padding plus itself, and the count byte lands in the final
/// word, so a padded RR is 8 + fill + 1 bytes rounded into whole words.
fn rr_bytes_with_fill(fill: &[u8]) -> Vec<u8> {
    let total = 8 + fill.len() + usize::from(!fill.is_empty());
    let p_bit = if fill.is_empty() { 0 } else { 0x20 };
    let mut v = Vec::new();
    v.extend_from_slice(&[(2 << 6) | p_bit, PT_RECEIVER_REPORT]);
    v.extend_from_slice(&((total / WORD_LEN - 1) as u16).to_be_bytes());
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(fill);
    if !fill.is_empty() {
        v.push((fill.len() + 1) as u8);
    }
    v
}

/// Wire layout of a minimal RR sub-packet: `V=2, P, RC` in byte 0.
fn rr_bytes(padded: bool) -> Vec<u8> {
    // Unpadded RR: 2 words (header + SSRC); the length field is words - 1.
    // Padded RR: 3 words (body 8 + 3 zero pad bytes + count byte 4).
    if padded {
        rr_bytes_with_fill(&[0, 0, 0])
    } else {
        rr_bytes_with_fill(&[])
    }
}

/// `ReportPart::name()`/`ssrc()` project the variant identity (RIST-W3).
#[test]
fn report_part_names_and_ssrc() {
    let sr = SenderReport::new(Ssrc::new(SSRC), 0, 0, 0, 0, 0, vec![]);
    let rr = ReceiverReport::new(Ssrc::new(SSRC | 1), vec![]);
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
        report: ReportPart::Rr(ReceiverReport::new(Ssrc::new(SSRC), vec![])),
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

/// SDES(CNAME) with a P-bit `fill` padding region and its count byte.
fn padded_sdes_bytes(fill: &[u8]) -> Vec<u8> {
    let mut v = sdes_bytes();
    v[0] |= 0x20;
    v.extend_from_slice(fill);
    v.push((fill.len() + 1) as u8);
    let total = v.len();
    v[2..4].copy_from_slice(&((total / WORD_LEN - 1) as u16).to_be_bytes());
    v
}

/// A padded Sender Report sub-packet round-trips byte-identically and
/// exposes its preserved region (RIST-W5).
#[test]
fn padded_sender_report_round_trips() {
    let mut compound_bytes = sr_bytes(&[0, 0, 0]);
    compound_bytes.extend_from_slice(&sdes_bytes());
    let parsed = RistSenderCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.report.name(), "SR");
    assert_eq!(parsed.report_padding, vec![0, 0, 0]);
    assert_eq!(parsed.cname, CNAME);
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);
}

/// RFC 3550 §6.4.1 fixes the report padding fill to zero, so a non-zero fill
/// is rejected with the count byte and the unpadded body length.
#[test]
fn a_non_zero_report_padding_fill_is_rejected() {
    // SR: 28-byte body + three fill bytes + count byte -> body 24.
    let mut compound_bytes = sr_bytes(&[0, 0, 1]);
    compound_bytes.extend_from_slice(&sdes_bytes());
    assert_eq!(
        RistSenderCompound::parse(&compound_bytes).unwrap_err(),
        Error::InvalidPaddingCount { count: 4, body: 24 }
    );

    // RR: 8-byte body + three fill bytes + count byte -> body 4.
    let mut compound_bytes = rr_bytes_with_fill(&[0, 0, 1]);
    compound_bytes.extend_from_slice(&sdes_bytes());
    assert_eq!(
        RistReceiverCompound::parse(&compound_bytes).unwrap_err(),
        Error::InvalidPaddingCount { count: 4, body: 4 }
    );
}

/// A longer all-zero fill is valid and preserved on both reports.
#[test]
fn a_longer_zero_report_padding_fill_is_accepted() {
    let mut compound_bytes = rr_bytes_with_fill(&[0; 7]);
    compound_bytes.extend_from_slice(&sdes_bytes());
    let parsed = RistReceiverCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.report_padding, vec![0; 7]);
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);

    let mut compound_bytes = sr_bytes(&[0; 7]);
    compound_bytes.extend_from_slice(&sdes_bytes());
    let parsed = RistSenderCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.report_padding, vec![0; 7]);
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);
}

/// An unknown sub-packet's padding is not required to be zero, so it must be
/// copied into a caller's dirty output buffer, not merely left alone.
#[test]
fn unknown_sub_packet_padding_survives_a_dirty_buffer() {
    let mut compound_bytes = rr_bytes(false);
    compound_bytes.extend_from_slice(&sdes_bytes());
    // PT 207: header + one body word + three fill bytes + count byte = 12.
    let mut xr: Vec<u8> = vec![(2 << 6) | 0x20, PT_UNMODELLED, 0, 2];
    xr.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
    xr.extend_from_slice(&[0xDE, 0xAD, 0xBE, 4]);
    compound_bytes.extend_from_slice(&xr);

    let parsed = RistReceiverCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.unknown[0].padding, vec![0xDE, 0xAD, 0xBE]);

    let mut out = vec![0xFFu8; compound_bytes.len()];
    assert_eq!(
        parsed.serialize_into(&mut out).unwrap(),
        compound_bytes.len()
    );
    assert_eq!(out, compound_bytes);
}

/// A hand-built padded NACK slot writes its preserved padding region into a
/// dirty output buffer.
#[test]
fn hand_built_padded_nack_padding_survives_a_dirty_buffer() {
    let compound = RistReceiverCompound {
        report: ReportPart::Rr(ReceiverReport::new(Ssrc::new(SSRC), vec![])),
        report_padding: vec![],
        cname: String::from(CNAME),
        nacks: vec![GenericNack {
            ssrc_sender: SSRC,
            ssrc_media: SSRC,
            nacks: vec![NackFci { pid: 100, blp: 0 }],
            padding: vec![1, 2, 3],
        }],
        range_nacks: vec![],
        rtt_echo: None,
        unknown: vec![],
    };

    let mut expected = rr_bytes(false);
    expected.extend_from_slice(&sdes_bytes());
    // Generic NACK (FMT 1): header + two SSRCs + one FCI entry + three fill
    // bytes + count byte = 20, so the length field is 4.
    expected.extend_from_slice(&[(2 << 6) | 0x20 | FMT_GENERIC_NACK, PT_RTPFB, 0, 4]);
    expected.extend_from_slice(&SSRC.to_be_bytes());
    expected.extend_from_slice(&SSRC.to_be_bytes());
    expected.extend_from_slice(&[0, 100, 0, 0]);
    expected.extend_from_slice(&[1, 2, 3, 4]);

    assert_eq!(compound.serialized_len(), expected.len());
    let mut out = vec![0xFFu8; compound.serialized_len()];
    assert_eq!(compound.serialize_into(&mut out).unwrap(), expected.len());
    assert_eq!(out, expected);
}

/// An unknown sub-packet keeps its wire position among the modelled ones.
#[test]
fn an_unknown_packet_before_the_sdes_keeps_its_wire_position() {
    let mut compound_bytes = rr_bytes(false);
    compound_bytes.extend_from_slice(&[(2 << 6), PT_UNMODELLED, 0, 1]);
    compound_bytes.extend_from_slice(&[0; WORD_LEN]);
    compound_bytes.extend_from_slice(&sdes_bytes());

    let parsed = RistReceiverCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.unknown.len(), 1);
    assert_eq!(parsed.unknown[0].position, Some(0));
    assert_eq!(parsed.cname, CNAME);
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);
}

/// Every slot keeps its wire position in a compound with an unknown packet
/// before the SDES and another after the RTT Echo.
#[test]
fn unknowns_keep_their_places_among_every_modelled_slot() {
    let mut compound_bytes = sr_bytes(&[]);
    compound_bytes.extend_from_slice(&[(2 << 6), PT_UNMODELLED, 0, 1]);
    compound_bytes.extend_from_slice(&[0; WORD_LEN]);
    compound_bytes.extend_from_slice(&sdes_bytes());
    compound_bytes.extend_from_slice(
        &RttEcho {
            kind: RttEchoKind::Request,
            ssrc_media: SSRC,
            timestamp: 0x0102_0304_0506_0708,
            processing_delay_us: 0,
            padding: vec![],
        }
        .try_to_bytes()
        .unwrap(),
    );
    compound_bytes.extend_from_slice(&[(2 << 6), PT_UNMODELLED, 0, 1]);
    compound_bytes.extend_from_slice(&[0; WORD_LEN]);

    let parsed = RistSenderCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.unknown.len(), 2);
    assert_eq!(parsed.unknown[0].position, Some(0));
    assert_eq!(parsed.unknown[1].position, Some(3));
    assert!(parsed.rtt_echo.is_some());
    assert_eq!(parsed.cname, CNAME);
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);
}

/// A receiver compound must preserve an RTPFB FMT and an APP subtype it does
/// not model verbatim (RFC 5506 forwarding), not route them into the typed
/// RIST parsers.
#[test]
fn a_receiver_keeps_unmodelled_fmt_and_subtype_verbatim() {
    let mut compound_bytes = rr_bytes(false);
    compound_bytes.extend_from_slice(&sdes_bytes());
    // PT 205 with FMT 15 (transport-cc, RFC 8888) — not a Generic NACK.
    compound_bytes.extend_from_slice(&[(2 << 6) | 15, PT_RTPFB, 0, 1]);
    compound_bytes.extend_from_slice(&[0; WORD_LEN]);
    // PT 204 with subtype 7 and an APP name that is not "RIST".
    compound_bytes.extend_from_slice(&[(2 << 6) | 7, PT_APP, 0, 2]);
    compound_bytes.extend_from_slice(b"XXXX");
    compound_bytes.extend_from_slice(&[0; WORD_LEN]);

    let parsed = RistReceiverCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.unknown.len(), 2);
    assert_eq!(parsed.unknown[0].packet_type, PT_RTPFB);
    assert_eq!(parsed.unknown[0].count, 15);
    assert_eq!(parsed.unknown[0].position, Some(1));
    assert_eq!(parsed.unknown[1].packet_type, PT_APP);
    assert_eq!(parsed.unknown[1].count, 7);
    assert_eq!(parsed.unknown[1].payload, b"XXXX\0\0\0\0");
    assert_eq!(parsed.unknown[1].position, Some(2));
    assert!(parsed.nacks.is_empty());
    assert!(parsed.range_nacks.is_empty());
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);
}

/// The same per-direction rule for a sender: a NACK is the receiver's to
/// send, so one arriving in a sender compound is preserved, malformed or not.
#[test]
fn a_sender_keeps_unmodelled_nacks_verbatim() {
    let mut compound_bytes = sr_bytes(&[]);
    compound_bytes.extend_from_slice(&sdes_bytes());
    let nack = GenericNack {
        ssrc_sender: SSRC,
        ssrc_media: SSRC,
        nacks: vec![NackFci {
            pid: 100,
            blp: 0xABCD,
        }],
        padding: vec![],
    }
    .try_to_bytes()
    .unwrap();
    compound_bytes.extend_from_slice(&nack);

    let parsed = RistSenderCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.unknown.len(), 1);
    assert_eq!(parsed.unknown[0].packet_type, PT_RTPFB);
    assert_eq!(parsed.unknown[0].position, Some(1));
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);

    // A Range NACK beyond TR-06-1 §5.3.2.2's 16-entry limit is malformed as a
    // typed value, but as an unmodelled sub-packet it is preserved.
    let mut compound_bytes = sr_bytes(&[]);
    compound_bytes.extend_from_slice(&sdes_bytes());
    compound_bytes.extend_from_slice(&[(2 << 6), PT_APP, 0, 19]);
    compound_bytes.extend_from_slice(&SSRC.to_be_bytes());
    compound_bytes.extend_from_slice(b"RIST");
    for i in 0..17u16 {
        compound_bytes.extend_from_slice(&i.to_be_bytes());
        compound_bytes.extend_from_slice(&0u16.to_be_bytes());
    }
    let parsed = RistSenderCompound::parse(&compound_bytes).unwrap();
    assert!(parsed.unknown.len() == 1);
    assert_eq!(parsed.unknown[0].packet_type, PT_APP);
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);
}

/// A compound whose only SDES is padded keeps the padding rather than taking
/// it as the CNAME, which leaves no CNAME at all.
#[test]
fn a_compound_whose_only_sdes_is_padded_is_missing_the_cname() {
    let compounded = |sdes: Vec<u8>| {
        let mut v = rr_bytes(false);
        v.extend_from_slice(&sdes);
        v
    };
    let padded = padded_sdes_bytes(&[0, 0, 0]);
    assert_eq!(
        RistReceiverCompound::parse(&compounded(padded.clone())).unwrap_err(),
        Error::MissingCname
    );

    let mut sender_bytes = sr_bytes(&[]);
    sender_bytes.extend_from_slice(&padded);
    assert_eq!(
        RistSenderCompound::parse(&sender_bytes).unwrap_err(),
        Error::MissingCname
    );
}

/// A padded second SDES is preserved verbatim (with its padding), so it
/// round-trips rather than losing the region.
#[test]
fn a_padded_second_sdes_is_preserved_verbatim() {
    let mut compound_bytes = rr_bytes(false);
    compound_bytes.extend_from_slice(&sdes_bytes());
    compound_bytes.extend_from_slice(&padded_sdes_bytes(&[0, 0, 0]));

    let parsed = RistReceiverCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.cname, CNAME);
    assert_eq!(parsed.unknown.len(), 1);
    assert_eq!(parsed.unknown[0].packet_type, PT_SOURCE_DESCRIPTION);
    assert_eq!(parsed.unknown[0].padding, vec![0, 0, 0]);
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);

    let mut compound_bytes = sr_bytes(&[]);
    compound_bytes.extend_from_slice(&sdes_bytes());
    compound_bytes.extend_from_slice(&padded_sdes_bytes(&[0, 0, 0]));

    let parsed = RistSenderCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.cname, CNAME);
    assert_eq!(parsed.unknown.len(), 1);
    assert_eq!(parsed.unknown[0].padding, vec![0, 0, 0]);
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);
}

/// A 4-byte sub-packet (length field 0, so the common header is the whole
/// packet) at the end of a compound is valid.
#[test]
fn a_header_only_unknown_sub_packet_round_trips() {
    let mut compound_bytes = rr_bytes(false);
    compound_bytes.extend_from_slice(&sdes_bytes());
    compound_bytes.extend_from_slice(&[(2 << 6) | 1, PT_UNMODELLED, 0, 0]);

    let parsed = RistReceiverCompound::parse(&compound_bytes).unwrap();
    assert_eq!(parsed.unknown.len(), 1);
    assert_eq!(parsed.unknown[0].packet_type, PT_UNMODELLED);
    assert_eq!(parsed.unknown[0].count, 1);
    assert!(parsed.unknown[0].payload.is_empty());
    assert_eq!(parsed.try_to_bytes().unwrap(), compound_bytes);
}

/// Less than a common header of trailing bytes is a `BufferTooShort`, never
/// a read past the end.
#[test]
fn trailing_bytes_below_a_common_header_are_a_buffer_too_short() {
    for n in 1..RTCP_HEADER_LEN {
        let mut receiver_bytes = rr_bytes(false);
        receiver_bytes.extend_from_slice(&sdes_bytes());
        receiver_bytes.extend_from_slice(&vec![0; n]);
        assert_eq!(
            RistReceiverCompound::parse(&receiver_bytes).unwrap_err(),
            Error::BufferTooShort(BufferTooShort {
                need: RTCP_HEADER_LEN,
                have: n,
                what: "RTCP common header",
            })
        );

        let mut sender_bytes = sr_bytes(&[]);
        sender_bytes.extend_from_slice(&sdes_bytes());
        sender_bytes.extend_from_slice(&vec![0; n]);
        assert_eq!(
            RistSenderCompound::parse(&sender_bytes).unwrap_err(),
            Error::BufferTooShort(BufferTooShort {
                need: RTCP_HEADER_LEN,
                have: n,
                what: "RTCP common header",
            })
        );
    }
}
