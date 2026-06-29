//! Binary-coded decimal (BCD) codec for DVB wire fields.
//!
//! DVB packs many numeric fields as BCD — each 4-bit nibble holds one decimal
//! digit (0–9). Examples: the HHMMSS of a UTC time, the frequency / symbol-rate
//! of the delivery-system descriptors, the HHMM of a local-time offset. These
//! helpers convert between the packed BCD representation and plain integers in
//! both directions so callers never hand-decode nibbles.
//!
//! Every decode has a symmetric encode; both reject out-of-range input by
//! returning `None` rather than producing garbage.

/// Largest number of BCD nibbles representable in the [`bcd_to_decimal`] /
/// [`decimal_to_bcd`] `u64` carrier.
pub const MAX_NIBBLES: u8 = 16;

/// Decode a packed-BCD byte (two nibbles) to `0..=99`.
///
/// Returns `None` if either nibble is greater than 9.
#[must_use]
pub fn from_bcd_byte(byte: u8) -> Option<u8> {
    bcd_to_decimal(u64::from(byte), 2).map(|v| v as u8)
}

/// Encode `0..=99` to a packed-BCD byte.
///
/// Returns `None` if `value > 99`.
#[must_use]
pub fn to_bcd_byte(value: u8) -> Option<u8> {
    decimal_to_bcd(u64::from(value), 2).map(|v| v as u8)
}

/// Decode the low `nibbles` BCD digits of `raw` to a decimal value.
///
/// Each nibble (most-significant first) contributes one decimal digit. Returns
/// `None` if any of those nibbles is greater than 9, or if `nibbles` exceeds
/// [`MAX_NIBBLES`].
#[must_use]
pub fn bcd_to_decimal(raw: u64, nibbles: u8) -> Option<u64> {
    if nibbles > MAX_NIBBLES {
        return None;
    }
    let mut acc = 0u64;
    for i in (0..nibbles).rev() {
        let digit = (raw >> (i * 4)) & 0x0F;
        if digit > 9 {
            return None;
        }
        acc = acc * 10 + digit;
    }
    Some(acc)
}

/// Encode `value` as `nibbles` packed-BCD digits in the low bits of a `u64`.
///
/// Returns `None` if `value` needs more than `nibbles` decimal digits, or if
/// `nibbles` exceeds [`MAX_NIBBLES`].
#[must_use]
pub fn decimal_to_bcd(value: u64, nibbles: u8) -> Option<u64> {
    if nibbles > MAX_NIBBLES {
        return None;
    }
    let mut packed = 0u64;
    let mut remaining = value;
    for i in 0..nibbles {
        let digit = remaining % 10;
        packed |= digit << (i * 4);
        remaining /= 10;
    }
    if remaining != 0 {
        // value had more digits than `nibbles` could hold.
        return None;
    }
    Some(packed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_round_trips_across_full_range() {
        for v in 0..=99u8 {
            let bcd = to_bcd_byte(v).expect("0..=99 encodes");
            assert_eq!(from_bcd_byte(bcd), Some(v), "round-trip {v}");
        }
    }

    #[test]
    fn from_bcd_byte_rejects_non_decimal_nibbles() {
        assert_eq!(from_bcd_byte(0x1A), None);
        assert_eq!(from_bcd_byte(0xA1), None);
        assert_eq!(from_bcd_byte(0x99), Some(99));
    }

    #[test]
    fn to_bcd_byte_rejects_over_99() {
        assert_eq!(to_bcd_byte(100), None);
        assert_eq!(to_bcd_byte(99), Some(0x99));
    }

    #[test]
    fn multi_nibble_round_trips() {
        // 8 BCD nibbles in a u32-shaped value (satellite frequency: 11725000).
        let raw = 0x1172_5000u64;
        assert_eq!(bcd_to_decimal(raw, 8), Some(11_725_000));
        assert_eq!(decimal_to_bcd(11_725_000, 8), Some(raw));
    }

    #[test]
    fn bcd_to_decimal_rejects_bad_nibble() {
        assert_eq!(bcd_to_decimal(0x000A_0000, 8), None);
    }

    #[test]
    fn decimal_to_bcd_rejects_overflow() {
        // 9 digits won't fit in 8 nibbles.
        assert_eq!(decimal_to_bcd(100_000_000, 8), None);
        assert_eq!(decimal_to_bcd(99_999_999, 8), Some(0x9999_9999));
    }
}
