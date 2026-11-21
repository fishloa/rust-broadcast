//! Truncated and under-minimum inputs to the standalone RTCP parsers: a
//! hostile or cut-short datagram must return `BufferTooShort` instead of
//! reading past the buffer, and a packet longer than its own length field
//! (trailing bytes after it, e.g. the next sub-packet) parses fine.

use broadcast_common::Parse;
use broadcast_common::Serialize;
use broadcast_common::error::BufferTooShort;
use rist_runtime::{Error, GenericNack, NackFci, PacketRange, RangeNack, RttEcho};

const PT_RTPFB: u8 = 205;
const PT_APP: u8 = 204;
const SSRC: u32 = 0xD561_5604;

/// A valid Generic NACK (FMT 1) with one FCI entry: 16 bytes.
fn generic_nack_bytes() -> Vec<u8> {
    let mut v = vec![(2 << 6) | 1, PT_RTPFB, 0, 3];
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(&[0, 100, 0, 0]);
    v
}

/// A valid Range NACK (APP subtype 0, name "RIST") with one entry: 16 bytes.
fn range_nack_bytes() -> Vec<u8> {
    let mut v = vec![2 << 6, PT_APP, 0, 3];
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(b"RIST");
    v.extend_from_slice(&[0, 100, 0, 0]);
    v
}

/// A valid RTT Echo Request with no padding: 24 bytes.
fn rtt_echo_bytes() -> Vec<u8> {
    let mut v = vec![(2 << 6) | 2, PT_APP, 0, 5];
    v.extend_from_slice(&SSRC.to_be_bytes());
    v.extend_from_slice(b"RIST");
    v.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    v.extend_from_slice(&[0, 0, 0, 0]);
    v
}

fn common_header(need: usize, have: usize) -> Error {
    Error::BufferTooShort(BufferTooShort {
        need,
        have,
        what: "RTCP common header",
    })
}

fn buffer_too_short(need: usize, have: usize, what: &'static str) -> Error {
    Error::BufferTooShort(BufferTooShort { need, have, what })
}

#[test]
fn every_truncation_of_a_generic_nack_is_a_buffer_too_short() {
    let bytes = generic_nack_bytes();
    for n in 0..bytes.len() {
        let parsed = GenericNack::parse(&bytes[..n]);
        assert!(
            matches!(parsed, Err(Error::BufferTooShort(_))),
            "n={n}: {parsed:?}"
        );
    }
}

#[test]
fn every_truncation_of_a_range_nack_is_a_buffer_too_short() {
    let bytes = range_nack_bytes();
    for n in 0..bytes.len() {
        let parsed = RangeNack::parse(&bytes[..n]);
        assert!(
            matches!(parsed, Err(Error::BufferTooShort(_))),
            "n={n}: {parsed:?}"
        );
    }
}

#[test]
fn every_truncation_of_an_rtt_echo_is_a_buffer_too_short() {
    let bytes = rtt_echo_bytes();
    for n in 0..bytes.len() {
        let parsed = RttEcho::parse(&bytes[..n]);
        assert!(
            matches!(parsed, Err(Error::BufferTooShort(_))),
            "n={n}: {parsed:?}"
        );
    }
}

/// A header-only prefix names the total length its `length` field claims.
#[test]
fn a_header_only_prefix_names_the_claimed_length() {
    assert_eq!(
        GenericNack::parse(&generic_nack_bytes()[..4]).unwrap_err(),
        buffer_too_short(16, 4, "Generic NACK")
    );
    assert_eq!(
        RangeNack::parse(&range_nack_bytes()[..4]).unwrap_err(),
        buffer_too_short(16, 4, "Range NACK")
    );
    assert_eq!(
        RttEcho::parse(&rtt_echo_bytes()[..4]).unwrap_err(),
        buffer_too_short(24, 4, "RTT Echo")
    );
}

/// A length field of 1 claims 8 bytes total, which is shorter than every
/// message's own minimum.
#[test]
fn a_length_field_of_one_is_below_every_minimum() {
    let mut nack = vec![(2 << 6) | 1, PT_RTPFB, 0, 1];
    nack.extend_from_slice(&[0; 4]);
    assert_eq!(
        GenericNack::parse(&nack).unwrap_err(),
        buffer_too_short(12, 8, "Generic NACK")
    );

    let mut range_nack = vec![2 << 6, PT_APP, 0, 1];
    range_nack.extend_from_slice(&[0; 4]);
    assert_eq!(
        RangeNack::parse(&range_nack).unwrap_err(),
        buffer_too_short(12, 8, "Range NACK")
    );

    let mut echo = vec![(2 << 6) | 2, PT_APP, 0, 1];
    echo.extend_from_slice(&[0; 4]);
    assert_eq!(
        RttEcho::parse(&echo).unwrap_err(),
        buffer_too_short(24, 8, "RTT Echo")
    );
}

/// A length field of 0 is the 4-byte common header on its own, still below
/// every minimum (and not a `BufferTooShort` for the header itself).
#[test]
fn a_length_field_of_zero_is_below_every_minimum() {
    assert_eq!(
        GenericNack::parse(&[(2 << 6) | 1, PT_RTPFB, 0, 0]).unwrap_err(),
        buffer_too_short(12, 4, "Generic NACK")
    );
    assert_eq!(
        RangeNack::parse(&[2 << 6, PT_APP, 0, 0]).unwrap_err(),
        buffer_too_short(12, 4, "Range NACK")
    );
    assert_eq!(
        RttEcho::parse(&[(2 << 6) | 2, PT_APP, 0, 0]).unwrap_err(),
        buffer_too_short(24, 4, "RTT Echo")
    );
}

/// Two- and three-byte prefixes are missing the common header itself.
#[test]
fn a_prefix_below_the_common_header_names_four_bytes() {
    for n in 0..4usize {
        assert_eq!(
            GenericNack::parse(&generic_nack_bytes()[..n]).unwrap_err(),
            common_header(4, n)
        );
        assert_eq!(
            RangeNack::parse(&range_nack_bytes()[..n]).unwrap_err(),
            common_header(4, n)
        );
        assert_eq!(
            RttEcho::parse(&rtt_echo_bytes()[..n]).unwrap_err(),
            common_header(4, n)
        );
    }
}

/// A padded NACK whose count byte eats into the SSRC fields leaves an
/// unpadded body below the message minimum.
#[test]
fn a_padded_nack_whose_count_eats_the_ssrcs_is_buffer_too_short() {
    // 4 words (16 bytes), count byte 8: the unpadded end is 8, below 12.
    let mut nack = vec![(2 << 6) | 0x20 | 1, PT_RTPFB, 0, 3];
    nack.extend_from_slice(&[0; 8]);
    nack.extend_from_slice(&[0, 0, 0, 8]);
    assert_eq!(nack.len(), 16);
    assert_eq!(
        GenericNack::parse(&nack).unwrap_err(),
        buffer_too_short(12, 8, "Generic NACK")
    );

    let mut range_nack = vec![(2 << 6) | 0x20, PT_APP, 0, 3];
    range_nack.extend_from_slice(&SSRC.to_be_bytes());
    range_nack.extend_from_slice(b"RIST");
    range_nack.extend_from_slice(&[0, 0, 0, 8]);
    assert_eq!(range_nack.len(), 16);
    assert_eq!(
        RangeNack::parse(&range_nack).unwrap_err(),
        buffer_too_short(12, 8, "Range NACK")
    );
}

/// Bytes after the sub-packet's own length field belong to the next packet
/// in the compound, so the parser must stop at its own length.
#[test]
fn trailing_bytes_beyond_the_length_field_are_ignored() {
    for extra in [&[0u8; 4][..], &[0xFF; 8][..]] {
        let mut bytes = generic_nack_bytes();
        bytes.extend_from_slice(extra);
        let parsed = GenericNack::parse(&bytes).unwrap();
        assert_eq!(parsed.nacks, vec![NackFci { pid: 100, blp: 0 }]);
        assert_eq!(parsed.try_to_bytes().unwrap(), generic_nack_bytes());

        let mut bytes = range_nack_bytes();
        bytes.extend_from_slice(extra);
        let parsed = RangeNack::parse(&bytes).unwrap();
        assert_eq!(
            parsed.ranges,
            vec![PacketRange {
                start: 100,
                additional: 0
            }]
        );
        assert_eq!(parsed.try_to_bytes().unwrap(), range_nack_bytes());

        let mut bytes = rtt_echo_bytes();
        bytes.extend_from_slice(extra);
        let parsed = RttEcho::parse(&bytes).unwrap();
        assert_eq!(parsed.padding, Vec::<u8>::new());
        assert_eq!(parsed.try_to_bytes().unwrap(), rtt_echo_bytes());
    }
}
