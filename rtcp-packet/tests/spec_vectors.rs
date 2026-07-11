//! Byte-identical round-trip tests over **spec-derived** wire vectors (RFC
//! 3550 §6), one per RTCP packet type plus a compound packet.
//!
//! ## Provenance
//!
//! `rtcp-packet` has no real captured RTCP traffic to draw a fixture from:
//! unlike `rtp-packet` (whose `tests/fixtures/rtp_simple.bin` was captured by
//! running this workspace's own `transmux::RtpPacketizer`), transmux's RTCP
//! module (`transmux::rtcp`, pre-extraction) was never wired to a hub
//! `Package`/`Unpackage` spoke — it's a standalone codec with no producer in
//! this repo, and no live network/pcap access is reachable from this
//! sandboxed environment. Per the project's documented fallback
//! (`docs/CRATE-ACCEPTANCE.md` §3 — "no real capture exists, gate is the
//! biting round-trip"), every byte vector below is **computed directly from
//! the RFC 3550 §6 bit diagrams** (`rtcp-packet/docs/rtcp.md`) with a
//! standalone script, independently of this crate's own serializer — so a
//! bug that made `serialize`/`parse` agree with each other but disagree with
//! the spec would still be caught. (Additional non-fixture round-trip and
//! mutation-bites coverage lives in the in-module `#[cfg(test)]` blocks of
//! `src/packet.rs`, carried over unchanged from the pre-extraction
//! `transmux::rtcp` implementation.)

use broadcast_common::{Parse, Serialize};
use rtcp_packet::{
    App, Bye, CompoundPacket, Error, PT_APP, PT_BYE, PT_RECEIVER_REPORT, PT_SENDER_REPORT,
    PT_SOURCE_DESCRIPTION, ReceiverReport, RtcpPacket, RtcpPacketType, SdesItemType, SenderReport,
    SourceDescription,
};

// ── SR — one report block, cumulative_lost = -5 (§6.4.1) ───────────────────
//
// header: V=2 P=0 RC=1, PT=200, length=12 (13 words - 1)
// SSRC=0x11223344, NTP MSW=0xAABBCCDD, NTP LSW=0x10203040, RTP ts=0x00050000,
// packet_count=100, octet_count=5000
// report block: SSRC_n=0xCAFEBABE, fraction_lost=10, cumulative_lost=-5
// (0xFFFFFB two's complement 24-bit), ext_seq=0x00010005, jitter=200,
// lsr=0xB7052000, dlsr=0x00005400
#[rustfmt::skip]
const SR_VECTOR: [u8; 52] = [
    0x81, 0xC8, 0x00, 0x0C,
    0x11, 0x22, 0x33, 0x44,
    0xAA, 0xBB, 0xCC, 0xDD,
    0x10, 0x20, 0x30, 0x40,
    0x00, 0x05, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x64,
    0x00, 0x00, 0x13, 0x88,
    0xCA, 0xFE, 0xBA, 0xBE,
    0x0A, 0xFF, 0xFF, 0xFB,
    0x00, 0x01, 0x00, 0x05,
    0x00, 0x00, 0x00, 0xC8,
    0xB7, 0x05, 0x20, 0x00,
    0x00, 0x00, 0x54, 0x00,
];

#[test]
fn sr_vector_parses_and_round_trips() {
    let sr = SenderReport::parse(&SR_VECTOR).expect("parse spec-derived SR vector");
    assert_eq!(sr.ssrc, 0x1122_3344);
    assert_eq!(sr.ntp_msw, 0xAABB_CCDD);
    assert_eq!(sr.ntp_lsw, 0x1020_3040);
    assert_eq!(sr.rtp_timestamp, 0x0005_0000);
    assert_eq!(sr.packet_count, 100);
    assert_eq!(sr.octet_count, 5000);
    assert_eq!(sr.report_blocks.len(), 1);
    let rb = &sr.report_blocks[0];
    assert_eq!(rb.ssrc, 0xCAFE_BABE);
    assert_eq!(rb.fraction_lost, 10);
    assert_eq!(rb.cumulative_lost, -5, "24-bit signed sign-extension");
    assert_eq!(rb.ext_highest_seq, 0x0001_0005);
    assert_eq!(rb.jitter, 200);
    assert_eq!(rb.lsr, 0xB705_2000);
    assert_eq!(rb.dlsr, 0x0000_5400);

    let mut out = vec![0u8; sr.serialized_len()];
    sr.serialize_into(&mut out).unwrap();
    assert_eq!(out, SR_VECTOR, "byte-identical to the spec-derived vector");
}

// ── RR — one report block, fraction_lost=255, cumulative_lost=-1 ──────────
#[rustfmt::skip]
const RR_VECTOR: [u8; 32] = [
    0x81, 0xC9, 0x00, 0x07,
    0x0A, 0x0B, 0x0C, 0x0D,
    0xDD, 0xCC, 0xBB, 0xAA,
    0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00,
];

#[test]
fn rr_vector_parses_and_round_trips() {
    let rr = ReceiverReport::parse(&RR_VECTOR).expect("parse spec-derived RR vector");
    assert_eq!(rr.ssrc, 0x0A0B_0C0D);
    assert_eq!(rr.report_blocks.len(), 1);
    let rb = &rr.report_blocks[0];
    assert_eq!(rb.ssrc, 0xDDCC_BBAA);
    assert_eq!(rb.fraction_lost, 255);
    assert_eq!(rb.cumulative_lost, -1);
    assert_eq!(rb.ext_highest_seq, 0xFFFF_0001);
    assert_eq!(rb.jitter, 0);
    assert_eq!(rb.lsr, 0);
    assert_eq!(rb.dlsr, 0);

    let mut out = vec![0u8; rr.serialized_len()];
    rr.serialize_into(&mut out).unwrap();
    assert_eq!(out, RR_VECTOR, "byte-identical to the spec-derived vector");
}

// ── SDES — one chunk, CNAME "ab" + TOOL "cde", padded to 32 bits (§6.5) ────
#[rustfmt::skip]
const SDES_VECTOR: [u8; 20] = [
    0x81, 0xCA, 0x00, 0x04,
    0x12, 0x34, 0x56, 0x78,
    0x01, 0x02, 0x61, 0x62, // CNAME len=2 "ab"
    0x06, 0x03, 0x63, 0x64, // TOOL len=3 "cd"
    0x65, 0x00, 0x00, 0x00, // "e" + null terminator + 2 pad bytes
];

#[test]
fn sdes_vector_parses_and_round_trips() {
    let sdes = SourceDescription::parse(&SDES_VECTOR).expect("parse spec-derived SDES vector");
    assert_eq!(sdes.chunks.len(), 1);
    let chunk = &sdes.chunks[0];
    assert_eq!(chunk.source, 0x1234_5678);
    assert_eq!(chunk.items.len(), 2);
    assert_eq!(chunk.items[0].item_type, SdesItemType::CName);
    assert_eq!(chunk.items[0].text, "ab");
    assert_eq!(chunk.items[1].item_type, SdesItemType::Tool);
    assert_eq!(chunk.items[1].text, "cde");

    let mut out = vec![0u8; sdes.serialized_len()];
    sdes.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, SDES_VECTOR,
        "byte-identical to the spec-derived vector"
    );
}

// ── BYE — SC=2, reason "hi" padded to 32 bits (§6.6) ────────────────────────
#[rustfmt::skip]
const BYE_VECTOR: [u8; 16] = [
    0x82, 0xCB, 0x00, 0x03,
    0x11, 0x11, 0x22, 0x22,
    0x33, 0x33, 0x44, 0x44,
    0x02, 0x68, 0x69, 0x00, // length=2 "hi" + 1 pad byte
];

#[test]
fn bye_vector_parses_and_round_trips() {
    let bye = Bye::parse(&BYE_VECTOR).expect("parse spec-derived BYE vector");
    assert_eq!(bye.sources, vec![0x1111_2222, 0x3333_4444]);
    assert_eq!(bye.reason.as_deref(), Some("hi"));

    let mut out = vec![0u8; bye.serialized_len()];
    bye.serialize_into(&mut out).unwrap();
    assert_eq!(out, BYE_VECTOR, "byte-identical to the spec-derived vector");
}

// ── APP — subtype=5, name "TEST", 8 bytes of data (§6.7) ────────────────────
#[rustfmt::skip]
const APP_VECTOR: [u8; 20] = [
    0x85, 0xCC, 0x00, 0x04,
    0xFE, 0xED, 0xFA, 0xCE,
    0x54, 0x45, 0x53, 0x54, // "TEST"
    0x01, 0x02, 0x03, 0x04,
    0x05, 0x06, 0x07, 0x08,
];

#[test]
fn app_vector_parses_and_round_trips() {
    let app = App::parse(&APP_VECTOR).expect("parse spec-derived APP vector");
    assert_eq!(app.subtype, 5);
    assert_eq!(app.ssrc, 0xFEED_FACE);
    assert_eq!(&app.name, b"TEST");
    assert_eq!(app.data, vec![1, 2, 3, 4, 5, 6, 7, 8]);

    let mut out = vec![0u8; app.serialized_len()];
    app.serialize_into(&mut out).unwrap();
    assert_eq!(out, APP_VECTOR, "byte-identical to the spec-derived vector");
}

// ── PT constants sanity (§6.1) ──────────────────────────────────────────────

#[test]
fn pt_constants() {
    assert_eq!(PT_SENDER_REPORT, 200);
    assert_eq!(PT_RECEIVER_REPORT, 201);
    assert_eq!(PT_SOURCE_DESCRIPTION, 202);
    assert_eq!(PT_BYE, 203);
    assert_eq!(PT_APP, 204);
}

// ── Compound packet — SR + SDES concatenation is a valid compound (§6.1) ───

#[test]
fn compound_of_spec_vectors_sr_then_sdes() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&SR_VECTOR);
    bytes.extend_from_slice(&SDES_VECTOR);

    let cp = CompoundPacket::parse(&bytes).expect("SR+SDES is a valid compound packet");
    assert_eq!(cp.packets.len(), 2);
    assert_eq!(cp.packets[0].packet_type(), RtcpPacketType::SenderReport);
    assert_eq!(
        cp.packets[1].packet_type(),
        RtcpPacketType::SourceDescription
    );

    let mut out = vec![0u8; cp.serialized_len()];
    cp.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, bytes,
        "byte-identical compound round trip over spec-derived sub-packets"
    );
}

#[test]
fn compound_sdes_first_is_rejected() {
    // SDES (PT 202) is not SR/RR, so it must never lead a compound packet.
    let err = CompoundPacket::parse(&SDES_VECTOR);
    assert!(matches!(
        err,
        Err(Error::InvalidValue {
            field: "rtcp_compound",
            ..
        })
    ));
}

// ── Error paths ──────────────────────────────────────────────────────────

#[test]
fn rejects_short_buffer() {
    assert!(matches!(
        SenderReport::parse(&SR_VECTOR[..3]),
        Err(Error::BufferTooShort { .. })
    ));
}

#[test]
fn rejects_bad_version() {
    let mut bytes = SR_VECTOR;
    bytes[0] = 0x40; // V=1
    assert!(matches!(
        RtcpPacket::parse(&bytes),
        Err(Error::InvalidValue { .. })
    ));
}

#[test]
fn rejects_wrong_packet_type_for_typed_parse() {
    // RR_VECTOR is PT 201; parsing it directly as a SenderReport must fail.
    assert!(matches!(
        SenderReport::parse(&RR_VECTOR),
        Err(Error::InvalidValue {
            field: "rtcp_pt",
            ..
        })
    ));
}
