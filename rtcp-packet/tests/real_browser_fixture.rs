//! Real-capture regression for r14-RTCP-C1 (#1071): `CompoundPacket::parse`
//! used to reject a **whole** compound datagram outright the moment it
//! contained a packet type outside the RFC 3550 §6 core set (200-204),
//! discarding a leading, perfectly valid SR/RR along with it — losing a
//! browser's own sender/receiver stats, not just the unsupported feedback.
//!
//! `tests/spec_vectors.rs` notes this crate had no real captured RTCP traffic
//! to draw on; this file closes that gap with one captured from a real
//! Chromium `RTCPeerConnection` (see `tests/fixtures/README.md` for exact
//! provenance) rather than another hand-built or spec-derived vector — this
//! specific bug (rejecting on PT 205/206/207) can only be demonstrated by
//! bytes a real peer actually sends, since a spec-derived vector risks
//! reproducing the same misunderstanding of the spec that caused the bug.

use broadcast_common::{Parse, Serialize};
use rtcp_packet::{CompoundPacket, RtcpPacket};

const FIXTURE: &[u8] = include_bytes!("fixtures/chrome_rr_pli_remb.bin");

#[test]
fn real_chrome_compound_parses_all_three_packets() {
    assert_eq!(FIXTURE.len(), 68);

    let compound = CompoundPacket::parse(FIXTURE)
        .expect("a real Chromium RR+PLI+REMB compound packet must parse");
    assert_eq!(compound.packets.len(), 3);

    // 1) RR (RFC 3550 §6.4.2, PT 201): the packet this crate must not lose.
    match &compound.packets[0] {
        RtcpPacket::ReceiverReport(rr) => {
            assert_eq!(rr.ssrc, 1);
            assert_eq!(rr.report_blocks.len(), 1);
            let rb = &rr.report_blocks[0];
            assert_eq!(rb.ssrc, 0x407E_522F);
            assert_eq!(rb.fraction_lost, 0);
            assert_eq!(rb.cumulative_lost, 0);
            assert_eq!(rb.ext_highest_seq, 5146);
            assert_eq!(rb.jitter, 482);
            assert_eq!(rb.lsr, 0);
            assert_eq!(rb.dlsr, 0);
        }
        other => panic!("expected the RR to survive, got {other:?}"),
    }

    // 2) PSFB PLI (RFC 4585 §6.3.1, PT 206 FMT 1) — unrecognized PT, opaque.
    match &compound.packets[1] {
        RtcpPacket::Unknown {
            packet_type,
            count,
            payload,
            ..
        } => {
            assert_eq!(*packet_type, 206);
            assert_eq!(*count, 1);
            // sender SSRC=1, media source SSRC=0x407E522F, no FCI.
            assert_eq!(payload, &[0x00, 0x00, 0x00, 0x01, 0x40, 0x7E, 0x52, 0x2F]);
        }
        other => panic!("expected the PLI as RtcpPacket::Unknown, got {other:?}"),
    }

    // 3) PSFB REMB (draft-alvestrand-rmcat-remb-03, PT 206 FMT 15) — likewise
    // opaque; carries the ASCII "REMB" marker inside its payload.
    match &compound.packets[2] {
        RtcpPacket::Unknown {
            packet_type,
            count,
            payload,
            ..
        } => {
            assert_eq!(*packet_type, 206);
            assert_eq!(*count, 15);
            assert_eq!(&payload[8..12], b"REMB");
        }
        other => panic!("expected the REMB as RtcpPacket::Unknown, got {other:?}"),
    }
}

#[test]
fn real_chrome_compound_round_trips_byte_identical() {
    let compound = CompoundPacket::parse(FIXTURE).unwrap();
    let mut out = vec![0u8; compound.serialized_len()];
    compound.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, FIXTURE,
        "byte-identical to the real captured Chromium datagram"
    );
}
