//! Round-trip tests for the `rfc8285` feature (RFC 8285 one-byte/two-byte
//! header-extension element multiplexing), gated behind
//! `#![cfg(feature = "rfc8285")]` so this file is skipped entirely when the
//! feature is off (`--no-default-features` / default builds).
//!
//! ## Provenance
//!
//! RFC 8285 §4.2 and §4.3 each give a worked example diagram of three
//! extension elements + padding, but the diagrams give concrete hex only for
//! the fixed profile id (`0xBEDE` / `0x100_`) and the `length` word count —
//! the element IDs and data payloads are left abstract ("ID", "data" in the
//! diagram, not literal bytes). The vectors below instantiate those exact
//! worked-example *structures* (element count and `L` values) with concrete
//! ID/data values chosen for this crate, matching
//! `rtp-packet/docs/rfc8285_header_ext.md`'s transcription — spec-structure-
//! derived, not verbatim RFC bytes, and independent of this crate's own
//! serializer (so a parse/serialize bug that merely agreed with itself would
//! still be caught against the hand-built vector).
//!
//! The RFC's own diagrams place their padding *between* elements (legal per
//! §4.1.2: padding "MAY be placed between extension elements ... or after
//! the last"), but this crate's `Serialize` canonicalizes padding to a
//! single trailing run on output (padding carries no semantic content, so
//! its exact wire position is not preserved — see
//! `docs/rfc8285_header_ext.md`'s "Judgment calls" section). So each worked
//! example below has two vectors: the RFC's own inter-element-padding
//! layout (`*_SPEC_LAYOUT`, used to check *decoding* matches the spec
//! exactly) and this crate's canonical trailing-padding form (`*_CANONICAL`,
//! used for genuine byte-identical `Serialize` round trips).

#![cfg(feature = "rfc8285")]

use broadcast_common::{Parse, Serialize};
use rtp_packet::rfc8285::{
    ExtensionElements, OneByteElement, OneByteElements, OneByteId, TwoByteElement, TwoByteElements,
    TwoByteId, parse_extensions,
};
use rtp_packet::{Error, HeaderExtension, RtpPacket};

fn one_byte_worked_example_elements() -> OneByteElements<'static> {
    OneByteElements(vec![
        OneByteElement {
            id: OneByteId::new(1).unwrap(),
            data: &[0x11],
        },
        OneByteElement {
            id: OneByteId::new(2).unwrap(),
            data: &[0x22, 0x33],
        },
        OneByteElement {
            id: OneByteId::new(3).unwrap(),
            data: &[0x44, 0x55, 0x66, 0x77],
        },
    ])
}

fn two_byte_worked_example_elements() -> TwoByteElements<'static> {
    TwoByteElements(vec![
        TwoByteElement {
            id: TwoByteId::new(10).unwrap(),
            data: &[],
        },
        TwoByteElement {
            id: TwoByteId::new(20).unwrap(),
            data: &[0x99],
        },
        TwoByteElement {
            id: TwoByteId::new(30).unwrap(),
            data: &[0x01, 0x02, 0x03, 0x04],
        },
    ])
}

// ── §4.2 one-byte-form worked example ───────────────────────────────────────
//
// elem(id=1, L=0 -> 1 data byte), elem(id=2, L=1 -> 2 data bytes),
// elem(id=3, L=3 -> 4 data bytes). 12 bytes = 3 words, matching the RFC
// diagram's `length=3`.

/// The RFC diagram's own layout: 2 padding bytes between elements 2 and 3.
#[rustfmt::skip]
const ONE_BYTE_BODY_SPEC_LAYOUT: [u8; 12] = [
    1 << 4, 0x11,
    (2 << 4) | 1, 0x22, 0x33,
    0x00, 0x00, // pad, per the RFC's own diagram placement
    (3 << 4) | 3, 0x44, 0x55, 0x66, 0x77,
];

/// This crate's canonical form: the same elements, padding at the tail.
#[rustfmt::skip]
const ONE_BYTE_BODY_CANONICAL: [u8; 12] = [
    1 << 4, 0x11,
    (2 << 4) | 1, 0x22, 0x33,
    (3 << 4) | 3, 0x44, 0x55, 0x66, 0x77,
    0x00, 0x00, // trailing pad
];

#[test]
fn one_byte_worked_example_spec_layout_decodes_correctly() {
    let elements =
        OneByteElements::parse(&ONE_BYTE_BODY_SPEC_LAYOUT).expect("parse §4.2 worked example");
    assert_eq!(elements, one_byte_worked_example_elements());
}

#[test]
fn one_byte_canonical_form_is_a_byte_identical_round_trip() {
    let elements = one_byte_worked_example_elements();
    let mut out = vec![0u8; elements.serialized_len()];
    elements.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, ONE_BYTE_BODY_CANONICAL,
        "byte-identical to the spec-derived canonical vector"
    );
    assert_eq!(OneByteElements::parse(&out).unwrap(), elements);
}

// ── §4.3 two-byte-form worked example ───────────────────────────────────────
//
// elem(id=10, L=0 -> 0 data bytes), elem(id=20, L=1 -> 1 data byte),
// elem(id=30, L=4 -> 4 data bytes). 12 bytes = 3 words, matching the RFC
// diagram's `length=3`.

/// The RFC diagram's own layout: 1 padding byte between elements 2 and 3.
#[rustfmt::skip]
const TWO_BYTE_BODY_SPEC_LAYOUT: [u8; 12] = [
    10, 0,
    20, 1, 0x99,
    0x00, // pad, per the RFC's own diagram placement
    30, 4, 0x01, 0x02, 0x03, 0x04,
];

/// This crate's canonical form: the same elements, padding at the tail.
#[rustfmt::skip]
const TWO_BYTE_BODY_CANONICAL: [u8; 12] = [
    10, 0,
    20, 1, 0x99,
    30, 4, 0x01, 0x02, 0x03, 0x04,
    0x00, // trailing pad
];

#[test]
fn two_byte_worked_example_spec_layout_decodes_correctly() {
    let elements =
        TwoByteElements::parse(&TWO_BYTE_BODY_SPEC_LAYOUT).expect("parse §4.3 worked example");
    assert_eq!(elements, two_byte_worked_example_elements());
}

#[test]
fn two_byte_canonical_form_is_a_byte_identical_round_trip() {
    let elements = two_byte_worked_example_elements();
    let mut out = vec![0u8; elements.serialized_len()];
    elements.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, TWO_BYTE_BODY_CANONICAL,
        "byte-identical to the spec-derived canonical vector"
    );
    assert_eq!(TwoByteElements::parse(&out).unwrap(), elements);
}

// ── Full-stack composition: RtpPacket -> HeaderExtension -> rfc8285 ─────────

#[test]
fn full_rtp_packet_with_one_byte_extension_round_trips_through_rfc8285() {
    let pkt = RtpPacket {
        marker: true,
        payload_type: 96,
        sequence_number: 7,
        timestamp: 0x1000,
        ssrc: 0xDEAD_BEEF,
        csrc: vec![],
        extension: Some(HeaderExtension {
            profile_id: rtp_packet::rfc8285::ONE_BYTE_PROFILE_ID,
            data: &ONE_BYTE_BODY_CANONICAL,
        }),
        padding: None,
        payload: b"payload",
    };

    let mut wire = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut wire).unwrap();
    let reparsed = RtpPacket::parse(&wire).unwrap();
    assert_eq!(reparsed, pkt);

    let ext = reparsed.extension.expect("X=1");
    let elements = parse_extensions(&ext).expect("recognized one-byte profile");
    let ExtensionElements::OneByte(one_byte) = elements else {
        panic!("expected one-byte form");
    };
    assert_eq!(one_byte, one_byte_worked_example_elements());

    // Re-serialize the decoded elements and confirm they reproduce the
    // exact original extension `data` slot, including the zero-pad tail.
    let mut redata = vec![0u8; one_byte.serialized_len()];
    one_byte.serialize_into(&mut redata).unwrap();
    assert_eq!(redata, ONE_BYTE_BODY_CANONICAL);
}

#[test]
fn full_rtp_packet_with_two_byte_extension_round_trips_through_rfc8285() {
    let pkt = RtpPacket {
        marker: false,
        payload_type: 100,
        sequence_number: 8,
        timestamp: 0x2000,
        ssrc: 0xCAFE_BABE,
        csrc: vec![0x1111_1111],
        extension: Some(HeaderExtension {
            profile_id: 0x1007, // 0x1000 | appbits(7)
            data: &TWO_BYTE_BODY_CANONICAL,
        }),
        padding: None,
        payload: b"x",
    };

    let mut wire = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut wire).unwrap();
    let reparsed = RtpPacket::parse(&wire).unwrap();
    assert_eq!(reparsed, pkt);

    let ext = reparsed.extension.expect("X=1");
    let elements = parse_extensions(&ext).expect("recognized two-byte profile");
    let ExtensionElements::TwoByte(two_byte) = elements else {
        panic!("expected two-byte form");
    };
    assert_eq!(two_byte, two_byte_worked_example_elements());

    let mut redata = vec![0u8; two_byte.serialized_len()];
    two_byte.serialize_into(&mut redata).unwrap();
    assert_eq!(redata, TWO_BYTE_BODY_CANONICAL);
}

#[test]
fn non_rfc8285_profile_id_is_a_distinct_error_not_a_malformed_packet() {
    // A HeaderExtension with some other profile_id is perfectly valid RFC
    // 3550 wire data — RFC 8285 interpretation is opt-in/profile-scoped, so
    // this must be a distinct "not this profile" result, not conflated with
    // a parse failure of the packet itself.
    let data = [0u8; 4];
    let ext = HeaderExtension {
        profile_id: 0x0001,
        data: &data,
    };
    assert!(matches!(
        parse_extensions(&ext),
        Err(Error::NotRfc8285Extension { profile_id: 0x0001 })
    ));
}

#[test]
fn one_byte_stop_marker_mid_stream_keeps_only_prior_elements() {
    // elem(id=5, 1 byte), then ID=15 (reserved stop marker) with a nonzero
    // length nibble that MUST be ignored -- processing terminates, and the
    // trailing byte after it is never interpreted as another element.
    #[rustfmt::skip]
    let body: [u8; 4] = [
        5 << 4, 0xAB,
        (15 << 4) | 7, 0xFF,
    ];
    let ext = HeaderExtension {
        profile_id: rtp_packet::rfc8285::ONE_BYTE_PROFILE_ID,
        data: &body,
    };
    let elements = parse_extensions(&ext).unwrap();
    let ExtensionElements::OneByte(one_byte) = elements else {
        panic!("expected one-byte form");
    };
    assert_eq!(one_byte.elements().len(), 1);
    assert_eq!(one_byte.elements()[0].id.get(), 5);
    assert_eq!(one_byte.elements()[0].data, &[0xAB]);
}
