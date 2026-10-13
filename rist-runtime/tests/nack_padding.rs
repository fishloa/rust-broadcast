//! RIST-W5 (#1086): the P bit (RFC 3550 §6.4.1) on a standalone Generic NACK,
//! Range NACK or RTT Echo must be honoured or rejected, never ignored (an
//! ignored P bit made the padding words parse as FCI entries / RTT-echo
//! padding and re-serialized with P cleared).

use broadcast_common::{Parse, Serialize};
use rist_runtime::{Error, GenericNack, RangeNack, RistReceiverCompound, RttEcho};

const SSRC: u32 = 0xD561_5604;
const PT_RTPFB: u8 = 205;
const PT_APP: u8 = 204;

/// Generic NACK (FMT 1) with one FCI entry (PID 100, BLP 0) and one padding
/// word `00 00 00 04`, P bit set. 5 words, length field 4.
fn padded_generic_nack() -> Vec<u8> {
    let mut v = vec![(2 << 6) | 0x20 | 1, PT_RTPFB, 0, 4];
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(&[0, 100, 0, 0]);
    v.extend_from_slice(&[0, 0, 0, 4]);
    v
}

/// Range NACK (APP, subtype 0, name "RIST") with one range entry
/// (start 100, extra 0) and one padding word. 5 words, length field 4.
fn padded_range_nack() -> Vec<u8> {
    let mut v = vec![(2 << 6) | 0x20, PT_APP, 0, 4];
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(b"RIST");
    v.extend_from_slice(&[0, 100, 0, 0]);
    v.extend_from_slice(&[0, 0, 0, 4]);
    v
}

#[test]
fn standalone_generic_nack_honours_the_p_bit() {
    let n = GenericNack::parse(&padded_generic_nack()).unwrap();
    assert_eq!(n.nacks.len(), 1, "padding parsed as an FCI entry");
    assert_eq!((n.nacks[0].pid, n.nacks[0].blp), (100, 0));
}

#[test]
fn standalone_range_nack_honours_the_p_bit() {
    let n = RangeNack::parse(&padded_range_nack()).unwrap();
    assert_eq!(n.ranges.len(), 1, "padding parsed as a range entry");
    assert_eq!(n.ranges[0].start, 100);
}

#[test]
fn p_bit_with_bad_count_is_rejected() {
    for count in [0u8, 17] {
        let mut b = padded_generic_nack();
        *b.last_mut().unwrap() = count;
        assert!(
            matches!(GenericNack::parse(&b), Err(Error::InvalidPaddingCount { .. })),
            "count {count}"
        );
        let mut b = padded_range_nack();
        *b.last_mut().unwrap() = count;
        assert!(
            matches!(RangeNack::parse(&b), Err(Error::InvalidPaddingCount { .. })),
            "count {count}"
        );
    }
}

/// TR-06-1 §5.2.6 fixes P = 0 on the RTT Echo: a set bit is rejected, not
/// silently cleared on re-serialize.
#[test]
fn rtt_echo_with_p_bit_is_rejected() {
    let mut v = vec![(2 << 6) | 0x20 | 2, PT_APP, 0, 5];
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(b"RIST");
    v.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    v.extend_from_slice(&[0, 0, 0, 0]);
    assert!(matches!(RttEcho::parse(&v), Err(Error::PaddingBitSet)));
}

/// Inside a compound a padded NACK sub-packet must round-trip.
#[test]
fn padded_nack_in_compound_round_trips() {
    // RR (2 words) + SDES(CNAME "x") + padded NACK
    let mut c = vec![2 << 6, 201, 0, 1];
    c.extend_from_slice(&SSRC.to_be_bytes());
    c.extend_from_slice(&[(2 << 6) | 1, 202, 0, 2]);
    c.extend_from_slice(&SSRC.to_be_bytes());
    c.extend_from_slice(&[1, 1, b'x', 0]);
    c.extend_from_slice(&padded_generic_nack());
    let parsed = RistReceiverCompound::parse(&c).expect("padded NACK in a compound parses");
    // A padded modelled sub-packet is preserved verbatim as an
    // `UnknownPacket` (the typed value cannot carry the padding), keeping its
    // wire position; it must not be dropped or misparsed.
    assert_eq!(parsed.nacks.len() + parsed.unknown.len(), 1);
    assert_eq!(parsed.try_to_bytes().unwrap(), c);
}
