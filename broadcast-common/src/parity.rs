//! Odd-parity protected bytes.
//!
//! Several broadcast byte formats protect each 7-bit data value with an 8th
//! (most significant) parity bit chosen so the byte has an **odd** number of
//! set bits: CEA-608 line-21 characters / control codes (CTA-608-E §4.2) and
//! EBU Teletext `txt_data_block` bytes (ETSI EN 300 706 §8.1). Odd parity
//! *detects* a single-bit error but cannot correct it (unlike Hamming 8/4).
//! This module is the one owner of the three operations every consumer needs.

/// `true` if `byte` has an odd number of set bits over all 8 bits (the parity
/// bit included): the byte is a valid odd-parity-protected value.
#[must_use]
pub const fn has_odd_parity(byte: u8) -> bool {
    byte.count_ones() & 1 == 1
}

/// Protect a 7-bit data value with odd parity: only the low 7 bits of `data7`
/// are used, and bit 8 (`0x80`) is set exactly when needed to make the total
/// number of set bits odd.
#[must_use]
pub const fn set_odd_parity(data7: u8) -> u8 {
    let d = data7 & 0x7F;
    if d.count_ones() & 1 == 0 { d | 0x80 } else { d }
}

/// Check and strip the parity bit: the 7-bit data value if `byte` has odd
/// parity, `None` on a parity error (an even number of set bits). Detects but
/// cannot correct a single-bit error.
#[must_use]
pub const fn strip_parity(byte: u8) -> Option<u8> {
    if has_odd_parity(byte) {
        Some(byte & 0x7F)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_data_value_round_trips_with_odd_parity() {
        for d in 0u8..=0x7F {
            let b = set_odd_parity(d);
            assert!(has_odd_parity(b), "{d:#04x} -> {b:#04x}");
            assert_eq!(b & 0x7F, d, "data bits untouched");
            assert_eq!(strip_parity(b), Some(d));
        }
    }

    #[test]
    fn the_parity_bit_is_set_only_when_needed() {
        assert_eq!(set_odd_parity(0x00), 0x80); // zero ones -> needs the bit
        assert_eq!(set_odd_parity(0x01), 0x01); // already odd
        assert_eq!(set_odd_parity(0x03), 0x83); // two ones -> needs the bit
        assert_eq!(set_odd_parity(0x7F), 0x7F); // seven ones: odd
    }

    #[test]
    fn only_the_low_seven_bits_of_the_input_are_used() {
        assert_eq!(set_odd_parity(0xFF), set_odd_parity(0x7F));
        assert_eq!(set_odd_parity(0x80), set_odd_parity(0x00));
    }

    #[test]
    fn a_single_flipped_bit_is_detected_never_corrected() {
        for d in 0u8..=0x7F {
            let good = set_odd_parity(d);
            for bit in 0..8 {
                assert_eq!(strip_parity(good ^ (1 << bit)), None, "{d:#04x} bit {bit}");
            }
        }
    }

    #[test]
    fn matches_the_spec_corner_values() {
        // CEA-608 RCL control code 0x14 has two set bits -> parity bit set
        // (the wire value 0x94 quoted in caption-convert's CEA-608 fixture).
        assert_eq!(set_odd_parity(0x14), 0x94);
        // ASCII 'H' (0x48, two set bits) -> 0xC8 (EN 300 706 §8.1 example form).
        assert_eq!(set_odd_parity(b'H'), 0xC8);
        assert_eq!(strip_parity(0xC8), Some(b'H'));
    }
}
