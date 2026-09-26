//! Real `libsrt` control-packet fixtures (issue #1060) — `tests/fixtures/README.md`
//! records how they were captured (a real `srt-live-transmit` 1.5.5 session,
//! sniffed on loopback).
//!
//! **Pre-fix failure** (before the `control::check_no_cif_or_libsrt_pad` fix):
//! `ControlPacket::parse` on either fixture returned
//! `Err(Error::UnexpectedTrailingBytes { what: "keep-alive CIF" | "ACKACK CIF",
//! extra: 4 })` — the old `check_no_cif` required an exactly-empty CIF, so a
//! real libsrt peer's KEEPALIVE/ACKACK packets (which always carry libsrt's
//! 4-byte zero pad, `srtcore/packet.cpp`'s `CPacket::pack`) were unparseable
//! and silently dropped by `io.rs`'s `let _ = self.ingress(...)`.

use srt_runtime::packet::{AckAckPacket, ControlPacket, KeepAlivePacket, SrtPacket};

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {path:?}: {e}"))
}

#[test]
fn real_libsrt_keepalive_parses_and_round_trips_byte_exact() {
    let bytes = fixture("libsrt_keepalive.bin");
    assert_eq!(
        bytes.len(),
        20,
        "fixture shape: 16-byte header + 4-byte pad"
    );

    let parsed = SrtPacket::parse(&bytes).expect("parse real libsrt KEEPALIVE");
    let SrtPacket::Control(ControlPacket::KeepAlive(ka)) = parsed else {
        panic!("expected a KeepAlive control packet, got {parsed:?}");
    };
    // Sanity: these came from the real capture (see fixtures/README.md), not
    // synthesized — timestamp/dest_socket_id are whatever the real session
    // happened to have at that moment, just confirm they decoded as the
    // right wire words.
    assert_eq!(
        ka,
        KeepAlivePacket {
            timestamp: u32::from_be_bytes([0x00, 0x0f, 0x4f, 0x79]),
            dest_socket_id: u32::from_be_bytes([0x1d, 0x50, 0x8c, 0x8a]),
        }
    );

    let mut out = vec![0u8; parsed.serialized_len()];
    let n = parsed.serialize_into(&mut out).expect("serialize");
    assert_eq!(n, bytes.len());
    assert_eq!(
        out, bytes,
        "must re-emit the exact captured libsrt bytes, pad included"
    );
}

#[test]
fn real_libsrt_ackack_parses_and_round_trips_byte_exact() {
    let bytes = fixture("libsrt_ackack.bin");
    assert_eq!(
        bytes.len(),
        20,
        "fixture shape: 16-byte header + 4-byte pad"
    );

    let parsed = SrtPacket::parse(&bytes).expect("parse real libsrt ACKACK");
    let SrtPacket::Control(ControlPacket::AckAck(ackack)) = parsed else {
        panic!("expected an AckAck control packet, got {parsed:?}");
    };
    assert_eq!(
        ackack,
        AckAckPacket {
            ack_number: 1,
            timestamp: u32::from_be_bytes([0x00, 0x0f, 0xc0, 0xd5]),
            dest_socket_id: u32::from_be_bytes([0x1d, 0x50, 0x8c, 0x8a]),
        }
    );

    let mut out = vec![0u8; parsed.serialized_len()];
    let n = parsed.serialize_into(&mut out).expect("serialize");
    assert_eq!(n, bytes.len());
    assert_eq!(
        out, bytes,
        "must re-emit the exact captured libsrt bytes, pad included"
    );
}
