//! T2-MI payload type 0x30: FEF part — Null — §5.2.9.
//!
//! Null FEF part — modulator generates P1 preamble per S1/S2, zeros for remainder.

use std::fmt;

use num_enum::TryFromPrimitive;

use dvb_common::{Parse, Serialize};

/// S1 field (3 bits) per EN 302 755 §7.2.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum S1Field {
    /// S1 value V0.
    V0 = 0,
    /// S1 value V1.
    V1 = 1,
    /// S1 value V2.
    V2 = 2,
    /// S1 value V3.
    V3 = 3,
    /// S1 value V4.
    V4 = 4,
    /// S1 value V5.
    V5 = 5,
    /// S1 value V6.
    V6 = 6,
    /// S1 value V7.
    V7 = 7,
}

impl From<S1Field> for u8 {
    fn from(s: S1Field) -> Self {
        s as u8
    }
}

impl From<num_enum::TryFromPrimitiveError<S1Field>> for crate::error::Error {
    fn from(_: num_enum::TryFromPrimitiveError<S1Field>) -> Self {
        crate::error::Error::ReservedBitsViolation {
            field: "s1_field",
            reason: "Must be 0..=7",
        }
    }
}

/// FEF part: Null payload (type 0x30) per ETSI TS 102 773 §5.2.9.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FefNullPayload {
    /// FEF index within super-frame.
    pub fef_idx: u8,
    /// S1 field per EN 302 755 §7.2.1.
    pub s1_field: S1Field,
    /// S2 field per EN 302 755 §7.2.1.
    pub s2_field: u8,
}

const FEF_NULL_LEN: usize = 3;

impl<'a> Parse<'a> for FefNullPayload {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < FEF_NULL_LEN {
            return Err(crate::Error::BufferTooShort {
                need: FEF_NULL_LEN,
                have: bytes.len(),
                what: "FefNullPayload",
            });
        }
        // Layout (Figure 12): fef_idx(8) | rfu(9) | s1_field(3) | s2_field(4).
        // rfu spans all of byte 1 plus the top bit of byte 2.
        if bytes[1] != 0 || bytes[2] & 0x80 != 0 {
            return Err(crate::Error::ReservedBitsViolation {
                field: "9-bit rfu",
                reason: "Must be zero (ETSI TS 102 773 §5.2.9)",
            });
        }
        Ok(FefNullPayload {
            fef_idx: bytes[0],
            // byte 2: rfu(1) | s1_field(3) [6:4] | s2_field(4) [3:0]
            s1_field: S1Field::try_from((bytes[2] >> 4) & 0x07)?,
            s2_field: bytes[2] & 0x0F,
        })
    }
}

impl<'a> crate::traits::PayloadDef<'a> for FefNullPayload {
    const PACKET_TYPE: u8 = 0x30;
    const NAME: &'static str = "FEF_NULL";
}

impl Serialize for FefNullPayload {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        FEF_NULL_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        if buf.len() < self.serialized_len() {
            return Err(crate::Error::OutputBufferTooSmall {
                need: self.serialized_len(),
                have: buf.len(),
            });
        }
        if self.s2_field > 0x0F {
            return Err(crate::Error::ReservedBitsViolation {
                field: "s2_field",
                reason: "Must fit in 4 bits",
            });
        }
        buf[0] = self.fef_idx;
        buf[1] = 0; // rfu (high 8 of the 9 reserved bits)
        buf[2] = ((u8::from(self.s1_field) & 0x07) << 4) | (self.s2_field & 0x0F);
        Ok(self.serialized_len())
    }
}

impl fmt::Display for FefNullPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FEF Null {{ fef_idx: {}, s1: {:?}, s2: {:04b} }}",
            self.fef_idx, self.s1_field, self.s2_field
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields() {
        // fef_idx=5, rfu=0, byte2 = s1(1)<<4 | s2(0x0A) = 0x1A
        let buf = [0x05u8, 0x00, 0x1A];
        let result = FefNullPayload::parse(&buf).unwrap();
        assert_eq!(result.fef_idx, 5);
        assert_eq!(result.s1_field, S1Field::V1);
        assert_eq!(result.s2_field, 0x0A);
    }

    #[test]
    fn parse_rejects_nonzero_rfu() {
        let buf = [0x00u8, 0x1F, 0x00];
        assert!(FefNullPayload::parse(&buf).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let orig = FefNullPayload {
            fef_idx: 3,
            s1_field: S1Field::V4,
            s2_field: 0x0C,
        };
        let mut buf = [0u8; FEF_NULL_LEN];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = FefNullPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn display_output() {
        let p = FefNullPayload {
            fef_idx: 0,
            s1_field: S1Field::V0,
            s2_field: 0,
        };
        assert!(p.to_string().contains("FEF Null"));
    }

    #[test]
    fn exhaustive_byte_sweep() {
        let mut matched = 0u16;
        for byte in 0u8..=0xFF {
            if let Ok(v) = S1Field::try_from(byte) {
                assert_eq!(v as u8, byte, "round-trip failed for {byte:#04x}");
                matched += 1;
            }
        }
        assert_eq!(matched, 8, "expected 8 matched variants");
    }
}
