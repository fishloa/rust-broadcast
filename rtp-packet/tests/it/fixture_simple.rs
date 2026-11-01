//! Real-fixture gate: `tests/fixtures/rtp_simple.bin` (RFC 3550 §5.1, the
//! `P=0 X=0 CC=0` simple case).
//!
//! ## Fixture provenance
//!
//! These are genuine wire bytes, not hand-typed: the 324-byte packet was
//! captured by running this workspace's own `transmux::RtpPacketiser`
//! (RFC 3550-compliant AAC-hbr / RFC 3640 packetiser) over the real broadcast
//! capture `fixtures/ts/h264_aac.ts` (already committed for transmux's own
//! RTP tests, `transmux/tests/rtp.rs`, issue #469), then saving one emitted
//! audio packet's exact wire bytes. The RTP fixed-header fields (marker,
//! payload type, sequence number, timestamp, SSRC) and the AAC-hbr payload
//! bytes are therefore real, spec-compliant wire content produced from a real
//! broadcast stream — not fabricated. `transmux` has no captured *external*
//! RTP/pcap stream in this repo (it only ever emits RTP, never ingests a
//! third-party capture), so this is the closest available "real capture" per
//! the project's real-fixture discipline (docs/CRATE-ACCEPTANCE.md).
//!
//! Padding / CSRC / header-extension fixtures are **not** exercised anywhere
//! in this workspace today (transmux never sets any of P/CC/X) and no public
//! RTP pcap is reachable from this sandboxed environment, so those cases use
//! spec-Table-derived vectors instead — see `tests/round_trip.rs`.

use broadcast_common::{Parse, Serialize};
use rtp_packet::RtpPacket;

fn fixture_bytes() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/rtp_simple.bin"
    ))
    .expect("rtp_simple.bin fixture must exist")
}

#[test]
fn parses_real_simple_case_header() {
    let bytes = fixture_bytes();
    let pkt = RtpPacket::parse(&bytes).expect("parse real fixture");

    assert!(pkt.marker, "AAC-hbr packetiser sets marker per AU");
    assert_eq!(pkt.payload_type, 97, "default audio payload type");
    assert_eq!(pkt.sequence_number, 5);
    assert_eq!(pkt.timestamp, 0x0000_1400);
    assert_eq!(pkt.ssrc, 0x1234_5678);
    assert_eq!(pkt.csrc_count(), 0, "simple case: CC=0");
    assert!(pkt.extension.is_none(), "simple case: X=0");
    assert!(pkt.padding.is_none(), "simple case: P=0");
    assert_eq!(
        pkt.payload.len(),
        bytes.len() - rtp_packet::FIXED_HEADER_LEN
    );
}

#[test]
fn byte_identical_round_trip_real_fixture() {
    let bytes = fixture_bytes();
    let pkt = RtpPacket::parse(&bytes).unwrap();

    let mut out = vec![0u8; pkt.serialized_len()];
    let n = pkt.serialize_into(&mut out).unwrap();
    assert_eq!(n, bytes.len());
    assert_eq!(
        out, bytes,
        "serialize(parse(bytes)) == bytes, byte-identical"
    );
}

#[test]
fn mutating_a_field_changes_the_wire_bytes() {
    // Proves serialize_into recomputes bytes from typed fields rather than
    // echoing a stored raw buffer.
    let bytes = fixture_bytes();
    let mut pkt = RtpPacket::parse(&bytes).unwrap();
    pkt.sequence_number = pkt.sequence_number.wrapping_add(1);

    let mut out = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut out).unwrap();

    assert_ne!(
        out, bytes,
        "mutated sequence number must change the wire bytes"
    );
    // The change is confined to exactly the 2-byte sequence-number field
    // (offsets 2..4); every other byte is untouched.
    assert_eq!(out[0], bytes[0]);
    assert_eq!(out[1], bytes[1]);
    assert_ne!(&out[2..4], &bytes[2..4]);
    assert_eq!(&out[4..], &bytes[4..]);
}
