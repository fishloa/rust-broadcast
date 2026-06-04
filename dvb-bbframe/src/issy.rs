//! ISSY (Input Stream SYnchronizer) field decoding per EN 302 755 §5.1.7 / Annex C.
//!
//! ISSY carries the Input Stream Clock Reference (ISCR) and, in its long form,
//! buffer-status / time-to-output signalling, used for jitter-free transport
//! reconstruction at the receiver. The first bit selects the form:
//!
//! ```text
//!   bit7 = 0          -> ISCR short: 15-bit ISCR    (2-byte ISSY)
//!   bit7 = 1, bit6 = 0 -> ISCR long: 22-bit ISCR    (3-byte ISSY)
//!   bit7 = 1, bit6 = 1 -> BUFS / TTO signalling      (3-byte ISSY)
//! ```

/// Decoded ISSY value (EN 302 755 §5.1.7, Annex C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Issy {
    /// ISCR short form — 15-bit Input Stream Clock Reference (2-byte ISSY).
    IscrShort(u16),
    /// ISCR long form — 22-bit Input Stream Clock Reference (3-byte ISSY).
    IscrLong(u32),
    /// Long-form BUFS / TTO signalling (3-byte ISSY, `11` prefix). The 22-bit
    /// payload is exposed raw; see EN 302 755 Annex C for the BUFS/TTO sub-coding.
    Signalling(u32),
}

/// Decode a 2-byte (short) ISSY field.
///
/// Returns `Some(Issy::IscrShort)` when the short-form bit (bit 7 of byte 0) is
/// `0`; `None` otherwise (a `1` prefix means a long-form field, which is 3 bytes
/// and must be decoded with [`decode_issy_long`]).
#[must_use]
pub fn decode_issy_short(bytes: [u8; 2]) -> Option<Issy> {
    if bytes[0] & 0x80 != 0 {
        return None;
    }
    let iscr = ((bytes[0] as u16 & 0x7F) << 8) | bytes[1] as u16;
    Some(Issy::IscrShort(iscr))
}

/// Decode a 3-byte (long) ISSY field.
///
/// Byte 0 bit 7 must be `1` (long form). Byte 0 bit 6 then selects: `0` → 22-bit
/// ISCR long; `1` → BUFS/TTO signalling. Returns `None` if bit 7 is `0` (that is
/// a short-form field — use [`decode_issy_short`]).
#[must_use]
pub fn decode_issy_long(bytes: [u8; 3]) -> Option<Issy> {
    if bytes[0] & 0x80 == 0 {
        return None;
    }
    let payload = ((bytes[0] as u32 & 0x3F) << 16) | (bytes[1] as u32) << 8 | bytes[2] as u32;
    if bytes[0] & 0x40 == 0 {
        Some(Issy::IscrLong(payload)) // '10' prefix
    } else {
        Some(Issy::Signalling(payload)) // '11' prefix (BUFS / TTO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iscr_short_decodes_15_bits() {
        // bit7=0 → ISCR short. 0x7ABC → iscr = 0x7ABC & 0x7FFF.
        assert_eq!(
            decode_issy_short([0x7A, 0xBC]),
            Some(Issy::IscrShort(0x7ABC))
        );
        assert_eq!(decode_issy_short([0x00, 0x01]), Some(Issy::IscrShort(1)));
    }

    #[test]
    fn short_rejects_long_prefix() {
        // bit7=1 is a long-form field, not short.
        assert_eq!(decode_issy_short([0x80, 0x00]), None);
    }

    #[test]
    fn iscr_long_decodes_22_bits() {
        // '10' prefix: byte0 = 0b10_xxxxxx. 0x80|0x3F = 0xBF top.
        assert_eq!(
            decode_issy_long([0xBF, 0xFF, 0xFF]),
            Some(Issy::IscrLong(0x3FFFFF))
        );
        assert_eq!(
            decode_issy_long([0x80, 0x12, 0x34]),
            Some(Issy::IscrLong(0x1234))
        );
    }

    #[test]
    fn signalling_decodes_with_11_prefix() {
        // '11' prefix: byte0 bit7=1, bit6=1.
        assert_eq!(
            decode_issy_long([0xC0, 0x12, 0x34]),
            Some(Issy::Signalling(0x1234))
        );
    }

    #[test]
    fn long_rejects_short_prefix() {
        assert_eq!(decode_issy_long([0x00, 0x00, 0x00]), None);
    }
}
