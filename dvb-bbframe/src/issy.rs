//! ISSY (Input Stream Synchronization) field decoding per EN 302 755 Annex C.
//!
//! ISSY carries timing information (PCR LSB, TTO, BUFS) for jitter-free
//! transport reconstruction at the receiver.

/// Decoded ISSY value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Issy {
    /// Reserved code (0xE0xxxx-0xFFxxxx) — not transmitted in DVB-T2.
    Reserved,
    /// Short format: PCR lower 16 bits.
    PcrLsb(u16),
    /// Short format: Time To Output (TTO) — 16 bits.
    Tto(u16),
    /// Short format: Buffer Status (BUFS) — 16 bits.
    Bufs(u16),
    /// Long format: full PCR (32 bits) + TTO (8 bits).
    Long {
        /// Full PCR value (32 bits).
        pcr: u32,
        /// Time To Output (8 bits).
        tto: u8,
    },
}

/// Decode a 2-byte short ISSY field per Annex C Table C.1.
///
/// The first nibble (bits [15:12]) determines the type:
/// - `0x0`-`0xD`: PCR LSB (16 bits)
/// - `0xE`: Reserved (not transmitted in DVB-T2)
/// - `0xF`: Not valid for short format
pub fn decode_issy_short(bytes: [u8; 2]) -> Issy {
    let val = u16::from_be_bytes(bytes);
    let tag = (val >> 12) & 0x0F;
    let value = val & 0x0FFF;

    match tag {
        0x00..=0x0D => Issy::PcrLsb(value),
        0x0E => Issy::Reserved,
        0x0F => Issy::Tto(value),
        _ => Issy::PcrLsb(value),
    }
}

/// Decode a 3-byte long ISSY field per Annex C Table C.1.
///
/// The first nibble (bits [23:20]) determines the type:
/// - `0xE`: Long format — BUFS (20 bits) in lower bytes
/// - `0xF`: Long format — TTO (24 bits) in lower bytes
/// - Other: reserved
pub fn decode_issy_long(bytes: [u8; 3]) -> Issy {
    // bytes[0] high nibble = tag (4 bits)
    // remaining 20 bits = value
    let tag = (bytes[0] >> 4) & 0x0F;
    let value = ((bytes[0] & 0x0F) as u32) << 16 | (bytes[1] as u32) << 8 | (bytes[2] as u32);

    match tag {
        0x00..=0x0D => Issy::Reserved, // Only valid in short format
        0x0E => {
            // BUFS: 20-bit buffer status value
            let bufs = value as u16;
            Issy::Bufs(bufs)
        }
        0x0F => {
            // Full PCR (20 bits upper 12) + TTO (bits lower 8)
            let pcr = value >> 8;
            let tto = (value & 0xFF) as u8;
            Issy::Long { pcr, tto }
        }
        _ => Issy::Reserved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcr_lsb_decodes_correctly() {
        // PCR LSB value with tag in 0x0-0xD range
        let bytes = [0x0A, 0xBC];
        match decode_issy_short(bytes) {
            Issy::PcrLsb(v) => assert_eq!(v, 0xABC),
            other => panic!("Expected PcrLsb, got {other:?}"),
        }
    }

    #[test]
    fn reserved_code_e_is_rejected() {
        let bytes = [0xE0, 0x00];
        assert_eq!(decode_issy_short(bytes), Issy::Reserved);
    }

    #[test]
    fn bufs_decodes_from_long_format() {
        // Tag 0xE in high nibble of first byte
        let bytes = [0xE0, 0x12, 0x34];
        match decode_issy_long(bytes) {
            Issy::Bufs(v) => assert_eq!(v, 0x1234),
            other => panic!("Expected Bufs, got {other:?}"),
        }
    }

    #[test]
    fn long_format_decodes_pcr_and_tto() {
        // Tag 0xF in high nibble
        // Format: 0xF0 [pcr_high:pcr_low] [tto]
        let bytes = [0xF1, 0x23, 0x45];
        match decode_issy_long(bytes) {
            Issy::Long { pcr, tto } => {
                assert_eq!(pcr, 0x123);
                assert_eq!(tto, 0x45);
            }
            other => panic!("Expected Long, got {other:?}"),
        }
    }
}
