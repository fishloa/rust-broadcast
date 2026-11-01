//! Adversarial review (dvb-si tables/core batch): PES header bits that
//! `PesPacket::parse` ACCEPTS but `serialize_into` silently rewrites.
//!
//! The Unreleased CHANGELOG fixes two lossy spots (stuffing bytes, PES
//! extension reserved bits) and the `ES_rate` markers, but the same
//! parse-tolerant / serialize-normalise pattern remains on other Table 2-21
//! fields: the `additional_copy_info` marker bit, the DSM trick-mode
//! `reserved` bits, and the ESCR `reserved(2)`. Each vector below parses
//! successfully and must therefore re-serialize byte-identically (or parse
//! must reject it, as it does for a cleared PTS marker bit).
use mpeg_pes::PesPacket;

fn round_trip(b: &[u8]) -> Vec<u8> {
    let p = PesPacket::parse(b).expect("vector parses");
    let mut out = vec![0u8; p.serialized_len()];
    let n = p.serialize_into(&mut out).expect("serialize");
    out.truncate(n);
    out
}

/// `additional_copy_info_flag = 1`; the byte is `marker_bit(1) |
/// additional_copy_info(7)` with the marker cleared (`0x0A`). Re-emitted as
/// `0x8A`.
#[test]
fn additional_copy_info_marker_bit_round_trips() {
    let b = [0x00, 0x00, 0x01, 0xE0, 0x00, 0x00, 0x80, 0x04, 0x01, 0x0A];
    assert_eq!(round_trip(&b), b);
}

/// `DSM_trick_mode_flag = 1`, `trick_mode_control = 010` (freeze_frame):
/// `field_id(2) = 10 | reserved(3) = 010` (`0x52`). The reserved bits are
/// re-emitted as `000` (`0x50`).
#[test]
fn dsm_trick_mode_reserved_bits_round_trip() {
    let b = [0x00, 0x00, 0x01, 0xE0, 0x00, 0x00, 0x80, 0x08, 0x01, 0x52];
    assert_eq!(round_trip(&b), b);
}

/// `ESCR_flag = 1` with the leading `reserved(2)` of the ESCR field `00`
/// instead of `11` (all marker bits set, base = extension = 0). Re-emitted
/// with `11`.
#[test]
fn escr_reserved_bits_round_trip() {
    let b = [
        0x00, 0x00, 0x01, 0xE0, 0x00, 0x00, 0x80, 0x20, 0x06, 0x04, 0x00, 0x04, 0x00, 0x04, 0x01,
    ];
    assert_eq!(round_trip(&b), b);
}
