//! Known-vector cross-checks for CRC-32 MPEG-2.

use dvb_common::crc32_mpeg2::{compute, POLY};

#[test]
fn poly_constant_is_expected() {
    assert_eq!(POLY, 0x04C1_1DB7);
}

#[test]
fn empty_input_returns_init_value() {
    assert_eq!(compute(&[]), 0xFFFF_FFFF);
}

#[test]
fn canonical_check_string_matches_rfc_value() {
    assert_eq!(compute(b"123456789"), 0x0376_E6E7);
}

#[test]
fn zero_byte_produces_expected_crc() {
    assert_eq!(compute(&[0x00]), 0x4E08_BFB4);
}
