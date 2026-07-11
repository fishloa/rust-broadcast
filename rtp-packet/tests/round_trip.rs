//! Byte-identical round-trip tests for the padding / CSRC-list / header-
//! extension cases (RFC 3550 §5.1 / §5.3.1), none of which any real stream in
//! this workspace exercises today (transmux's own RTP code never sets P, CC,
//! or X — see `transmux/src/rtp.rs`).
//!
//! ## Provenance
//!
//! No real capture with padding, a non-empty CSRC list, or a header extension
//! was reachable from this sandboxed environment (no live network access to
//! e.g. Wireshark's public sample-captures site). Per the project's
//! documented fallback (docs/CRATE-ACCEPTANCE.md — "no real capture exists,
//! gate is the biting round-trip", the same discipline already used for
//! `smpte2038`'s `anc.bin` / `scte104`'s hand-built vectors), every byte
//! vector below is **hand-constructed directly from the RFC 3550 §5.1/§5.3.1
//! bit diagrams** (`rtp-packet/docs/rtp-header.md`), independently of this
//! crate's own serializer — so a bug that made `serialize`/`parse` agree with
//! each other but disagree with the spec would still be caught.

use broadcast_common::{Parse, Serialize};
use rtp_packet::{Error, RtpPacket};

// ── Padding (P=1) — RFC 3550 §5.1 ───────────────────────────────────────────
//
// byte0 = 0b1010_0000 (V=2, P=1, X=0, CC=0)
// byte1 = 0b0110_0000 (M=0, PT=96)
// seq=1, ts=0, ssrc=1, payload=[DE AD BE EF], padding=[00 00 00 04] (3 filler
// octets + the trailing count byte "including itself" = 4).
#[rustfmt::skip]
const PADDING_VECTOR: [u8; 20] = [
    0xA0, 0x60, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01,
    0xDE, 0xAD, 0xBE, 0xEF,
    0x00, 0x00, 0x00, 0x04,
];

#[test]
fn padding_vector_parses_and_round_trips() {
    let pkt = RtpPacket::parse(&PADDING_VECTOR).expect("parse spec-derived padding vector");
    assert!(!pkt.marker);
    assert_eq!(pkt.payload_type, 96);
    assert_eq!(pkt.sequence_number, 1);
    assert_eq!(pkt.timestamp, 0);
    assert_eq!(pkt.ssrc, 1);
    assert_eq!(pkt.csrc_count(), 0);
    assert!(pkt.extension.is_none());
    assert_eq!(pkt.payload, &[0xDE, 0xAD, 0xBE, 0xEF]);
    let pad = pkt.padding.expect("P=1");
    assert_eq!(pad, &[0x00, 0x00, 0x00, 0x04]);
    assert_eq!(*pad.last().unwrap(), pad.len() as u8);

    let mut out = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, PADDING_VECTOR,
        "byte-identical to the spec-derived vector"
    );
}

// ── Non-empty CSRC list (CC=2) — RFC 3550 §5.1 ──────────────────────────────
//
// byte0 = 0b1000_0010 (V=2, P=0, X=0, CC=2)
// byte1 = 0b0000_0011 (M=0, PT=3)
// seq=2, ts=0x10, ssrc=0xAABBCCDD, csrc=[0x11111111, 0x22222222],
// payload=[01 02 03].
#[rustfmt::skip]
const CSRC_VECTOR: [u8; 23] = [
    0x82, 0x03, 0x00, 0x02,
    0x00, 0x00, 0x00, 0x10,
    0xAA, 0xBB, 0xCC, 0xDD,
    0x11, 0x11, 0x11, 0x11,
    0x22, 0x22, 0x22, 0x22,
    0x01, 0x02, 0x03,
];

#[test]
fn csrc_vector_parses_and_round_trips() {
    let pkt = RtpPacket::parse(&CSRC_VECTOR).expect("parse spec-derived CSRC vector");
    assert_eq!(pkt.payload_type, 3);
    assert_eq!(pkt.sequence_number, 2);
    assert_eq!(pkt.timestamp, 0x10);
    assert_eq!(pkt.ssrc, 0xAABB_CCDD);
    assert_eq!(pkt.csrc, vec![0x1111_1111, 0x2222_2222]);
    assert_eq!(pkt.payload, &[0x01, 0x02, 0x03]);
    assert!(pkt.padding.is_none());
    assert!(pkt.extension.is_none());

    let mut out = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, CSRC_VECTOR,
        "byte-identical to the spec-derived vector"
    );
}

#[test]
fn csrc_count_is_always_derived_from_the_list() {
    // Building a packet with a 15-entry CSRC list (the field's max, §5.1) must
    // serialize CC=15, never a stray/independent count.
    let pkt = RtpPacket {
        marker: false,
        payload_type: 0,
        sequence_number: 0,
        timestamp: 0,
        ssrc: 0,
        csrc: vec![0; 15],
        extension: None,
        padding: None,
        payload: &[],
    };
    let mut out = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut out).unwrap();
    assert_eq!(out[0] & 0x0F, 15);
}

// ── Header extension (X=1) — RFC 3550 §5.3.1 ────────────────────────────────
//
// byte0 = 0b1001_0000 (V=2, P=0, X=1, CC=0)
// byte1 = 0b1110_0100 (M=1, PT=100)
// seq=3, ts=0x20, ssrc=0x99999999, extension profile_id=0xBEDE, length=2
// words (8 bytes) of opaque data, payload=[FF].
#[rustfmt::skip]
const EXTENSION_VECTOR: [u8; 25] = [
    0x90, 0xE4, 0x00, 0x03,
    0x00, 0x00, 0x00, 0x20,
    0x99, 0x99, 0x99, 0x99,
    0xBE, 0xDE, 0x00, 0x02,
    0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80,
    0xFF,
];

#[test]
fn extension_vector_parses_and_round_trips() {
    let pkt = RtpPacket::parse(&EXTENSION_VECTOR).expect("parse spec-derived extension vector");
    assert!(pkt.marker);
    assert_eq!(pkt.payload_type, 100);
    assert_eq!(pkt.sequence_number, 3);
    assert_eq!(pkt.timestamp, 0x20);
    assert_eq!(pkt.ssrc, 0x9999_9999);
    let ext = pkt.extension.expect("X=1");
    assert_eq!(ext.profile_id, 0xBEDE);
    assert_eq!(ext.length_words(), 2);
    assert_eq!(ext.data, &[0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80]);
    assert_eq!(pkt.payload, &[0xFF]);

    let mut out = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, EXTENSION_VECTOR,
        "byte-identical to the spec-derived vector"
    );
}

#[test]
fn extension_with_zero_length_word_count_round_trips() {
    // §5.3.1: "the header extension contains a 16-bit length field that
    // counts the number of 32-bit words in the extension ... therefore zero
    // is a valid length".
    #[rustfmt::skip]
    let bytes: [u8; 16] = [
        0x90, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0xAB, 0xCD, 0x00, 0x00, // profile_id=0xABCD, length=0
    ];
    let pkt = RtpPacket::parse(&bytes).unwrap();
    let ext = pkt.extension.unwrap();
    assert_eq!(ext.profile_id, 0xABCD);
    assert_eq!(ext.length_words(), 0);
    assert!(ext.data.is_empty());
    assert!(pkt.payload.is_empty());

    let mut out = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut out).unwrap();
    assert_eq!(out, bytes);
}

// ── Error paths ──────────────────────────────────────────────────────────

#[test]
fn rejects_short_buffer() {
    assert!(matches!(
        RtpPacket::parse(&[0x80, 0x60, 0x00]),
        Err(Error::BufferTooShort { .. })
    ));
}

#[test]
fn rejects_truncated_csrc_list() {
    // CC=1 declared but no CSRC bytes follow the fixed header.
    let mut bytes = PADDING_VECTOR[..12].to_vec();
    bytes[0] = 0x80 | 0x01; // V=2 P=0 X=0 CC=1
    assert!(matches!(
        RtpPacket::parse(&bytes),
        Err(Error::BufferTooShort {
            what: "CSRC list",
            ..
        })
    ));
}

#[test]
fn rejects_truncated_extension_data() {
    // X=1, length says 2 words (8 bytes) but only 4 are present.
    #[rustfmt::skip]
    let bytes: [u8; 20] = [
        0x90, 0x60, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x01, 0x00, 0x02,
        0x01, 0x02, 0x03, 0x04,
    ];
    assert!(matches!(
        RtpPacket::parse(&bytes),
        Err(Error::BufferTooShort {
            what: "header extension data",
            ..
        })
    ));
}
