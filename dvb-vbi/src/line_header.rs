//! Shared first-byte bit-packing for the Teletext / VPS / WSS / CC data units —
//! ETSI EN 301 775 §4.5.1, §4.6.1, §4.7.1, §4.8.1 (Tables 4, 6, 8, 10).
//!
//! Each of these data fields begins with the same first byte (MSB→LSB):
//! `[7:6]` reserved_future_use = `11`, `[5]` field_parity, `[4:0]` line_offset.
//! (The monochrome data unit, §4.9.1, packs its first byte differently — two
//! segment flags occupy `[7:6]` instead — and so does *not* use this header.)

use crate::error::{Error, Result};

/// Size in bytes of the shared first-byte line header.
pub const LINE_HEADER_LEN: usize = 1;

/// The fixed 2-bit `reserved_future_use` prefix (`11`) occupying bits `[7:6]`
/// of the header byte.
pub const RESERVED_PREFIX: u8 = 0b11;

/// The shared first byte of the Teletext / VPS / WSS / CC data fields:
/// a fixed `reserved_future_use` = `11` prefix, then `field_parity` and a 5-bit
/// `line_offset` (ETSI EN 301 775 §4.5.1 et al.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LineHeader {
    /// `field_parity` (1 bit): `1` = first field of a frame, `0` = second field.
    pub field_parity: bool,
    /// `line_offset` (5 bits): the VBI line, coded per the unit's line_offset
    /// table. Range `0..=31`.
    pub line_offset: u8,
}

impl LineHeader {
    /// Construct a header from its fields.
    pub fn new(field_parity: bool, line_offset: u8) -> Self {
        LineHeader {
            field_parity,
            line_offset,
        }
    }

    /// Decode the header from a single byte. The `reserved_future_use` prefix
    /// bits `[7:6]` are not validated (decoders ignore RFU); only the typed
    /// fields are extracted.
    pub fn from_byte(byte: u8) -> Self {
        LineHeader {
            field_parity: (byte & 0b0010_0000) != 0,
            line_offset: byte & 0b0001_1111,
        }
    }

    /// Encode the header to its single wire byte: `11` | field_parity |
    /// line_offset. Errors if `line_offset` does not fit in 5 bits.
    pub fn to_byte(self) -> Result<u8> {
        if self.line_offset > 0b0001_1111 {
            return Err(Error::FieldTooWide {
                what: "line_offset",
                value: self.line_offset as u32,
                bits: 5,
            });
        }
        let mut b = RESERVED_PREFIX << 6;
        if self.field_parity {
            b |= 0b0010_0000;
        }
        b |= self.line_offset;
        Ok(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_bytes() {
        // The header byte decodes/encodes losslessly for any input whose RFU
        // prefix is the canonical `11` (the only value the encoder emits).
        for raw in 0u16..=0xFF {
            let raw = raw as u8;
            let h = LineHeader::from_byte(raw);
            let re = h.to_byte().unwrap();
            // Re-encode forces the canonical RFU prefix `11`; compare the
            // non-RFU bits only.
            assert_eq!(re & 0b0011_1111, raw & 0b0011_1111, "raw={raw:#04X}");
            assert_eq!(re >> 6, RESERVED_PREFIX);
        }
    }

    #[test]
    fn field_split() {
        // 0xD0 = 11 0 10000 -> parity=0, line_offset=16 (a VPS header).
        let h = LineHeader::from_byte(0xD0);
        assert!(!h.field_parity);
        assert_eq!(h.line_offset, 16);
        assert_eq!(h.to_byte().unwrap(), 0xD0);

        // parity=1, line_offset=21 (a CC first-field header) -> 11 1 10101 = 0xF5.
        let h = LineHeader::new(true, 21);
        assert_eq!(h.to_byte().unwrap(), 0xF5);
    }

    #[test]
    fn rejects_overwide_line_offset() {
        assert!(matches!(
            LineHeader::new(true, 32).to_byte(),
            Err(Error::FieldTooWide { .. })
        ));
    }
}
