//! Adversarial review (transport/ancillary batch): FAILING regression tests for
//! the #1086 P-bit handling of standalone `GenericNack` / `RangeNack`.
//!
//! The parser now strips RFC 3550 §6.4.1 padding, but the typed value cannot
//! carry it and `serialize_into` always writes `P = 0`: a padded standalone
//! NACK does not round-trip byte-identically (the workspace's hard invariant),
//! the very reason `RttEcho::parse` rejects a set P bit instead. A padding
//! count that is not a multiple of 4 also leaves orphan bytes after the last
//! whole FCI / range entry, which are silently dropped.

use broadcast_common::{Parse, Serialize};
use rist_runtime::{GenericNack, RangeNack};

const SSRC: u32 = 0xD561_5604;

fn padded_generic_nack(pad_count: u8) -> Vec<u8> {
    let mut v = vec![(2 << 6) | 0x20 | 1, 205, 0, 4];
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(&[0, 100, 0, 0]);
    v.extend_from_slice(&[0, 0, 0, pad_count]);
    v
}

fn padded_range_nack(pad_count: u8) -> Vec<u8> {
    let mut v = vec![(2 << 6) | 0x20, 204, 0, 4];
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(b"RIST");
    v.extend_from_slice(&[0, 100, 0, 0]);
    v.extend_from_slice(&[0, 0, 0, pad_count]);
    v
}

#[test]
fn padded_standalone_generic_nack_round_trips_byte_identically() {
    let wire = padded_generic_nack(4);
    let n = GenericNack::parse(&wire).expect("padded NACK parses");
    assert_eq!(
        n.try_to_bytes().unwrap(),
        wire,
        "padding + P bit lost on re-serialize"
    );
}

#[test]
fn padded_standalone_range_nack_round_trips_byte_identically() {
    let wire = padded_range_nack(4);
    let n = RangeNack::parse(&wire).expect("padded Range NACK parses");
    assert_eq!(
        n.try_to_bytes().unwrap(),
        wire,
        "padding + P bit lost on re-serialize"
    );
}

/// Padding count 2: the unpadded body is 18 bytes = 12-byte header + one
/// 4-byte FCI + 2 orphan bytes. Those 2 bytes are neither an FCI entry nor
/// padding; they are silently discarded instead of being rejected.
#[test]
fn padding_leaving_a_partial_fci_entry_is_rejected() {
    assert!(
        GenericNack::parse(&padded_generic_nack(2)).is_err(),
        "2 orphan FCI bytes silently dropped"
    );
    assert!(
        RangeNack::parse(&padded_range_nack(2)).is_err(),
        "2 orphan range-entry bytes silently dropped"
    );
}
