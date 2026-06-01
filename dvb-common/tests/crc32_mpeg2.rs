//! Known-vector cross-checks for CRC-32 MPEG-2.

use dvb_common::crc32_mpeg2::{compute, POLY, TABLE};

#[test]
fn poly_constant_is_expected() {
    assert_eq!(POLY, 0x04C1_1DB7);
}

#[test]
fn table_entry_zero_is_zero() {
    assert_eq!(TABLE[0], 0x0000_0000);
}

#[test]
fn empty_input_returns_init_value() {
    // No bytes consumed → shift register never shifted, stays at 0xFFFFFFFF.
    assert_eq!(compute(&[]), 0xFFFF_FFFF);
}

#[test]
fn canonical_check_string_matches_rfc_value() {
    // "123456789" — the canonical CRC check string. For CRC-32 MPEG-2
    // (poly 0x04C11DB7, init 0xFFFFFFFF, MSB-first, no reflection,
    // no final XOR) the documented check value is 0x0376E6E7.
    assert_eq!(compute(b"123456789"), 0x0376_E6E7);
}

#[test]
fn single_zero_byte_matches_table_entry() {
    // compute(&[0x00]) starts with crc = 0xFFFF_FFFF, top byte XOR 0x00 = 0xFF.
    // Result is (crc << 8) ^ TABLE[0xFF] = 0xFFFF_FF00 ^ TABLE[0xFF].
    let expected = 0xFFFF_FF00u32 ^ TABLE[0xFF];
    assert_eq!(compute(&[0x00]), expected);
}
