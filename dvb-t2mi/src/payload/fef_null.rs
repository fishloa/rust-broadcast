//! T2-MI payload type 0x30: FEF part — Null — §5.2.9.
//!
//! Null FEF part — modulator generates P1 preamble per S1/S2, zeros for remainder.

use std::fmt;

use num_enum::TryFromPrimitive;

use dvb_common::{Parse, Serialize};

/// S1 field (3 bits) per EN 302 755 §7.2.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
pub enum S1Field {
    /// S1 value V0 (000 = T2_SISO).
    V0 = 0,
    /// S1 value V1 (001 = T2_MISO).
    V1 = 1,
    /// S1 value V2 (010 = Non-T2).
    V2 = 2,
    /// S1 value V3 (011 = T2_LITE_SISO).
    V3 = 3,
    /// S1 value V4 (100 = T2_LITE_MISO).
    V4 = 4,
    /// S1 value V5 (101 = reserved).
    V5 = 5,
    /// S1 value V6 (110 = reserved).
    V6 = 6,
    /// S1 value V7 (111 = reserved).
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

impl S1Field {
    /// Per EN 302 755 §7.2.1 Table 18.
    #[must_use]
    pub fn meaning(self) -> &'static str {
        match self {
            Self::V0 => "T2_SISO",
            Self::V1 => "T2_MISO",
            Self::V2 => "Non-T2",
            Self::V3 => "T2_LITE_SISO",
            Self::V4 => "T2_LITE_MISO",
            Self::V5 => "reserved",
            Self::V6 => "reserved",
            Self::V7 => "reserved",
        }
    }
}

/// S2 field 1 (upper 3 bits of the 4-bit S2 field) per EN 302 755 §7.2.3 Tables 19-20.
///
/// Encodes the FFT size and guard-interval set for the T2 frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum S2Field1 {
    /// FFT size 1k / GI 1/128, 1/32, 1/16, 19/256, 1/8, 19/128, 1/4.
    Fft1k,
    /// FFT size 2k / GI 1/128, 1/32, 1/16, 19/256, 1/8, 19/128, 1/4.
    Fft2k,
    /// FFT size 4k / GI 1/128, 1/32, 1/16, 19/256, 1/8, 19/128, 1/4.
    Fft4k,
    /// FFT size 8k / GI 1/128, 1/32, 1/16, 19/256, 1/8, 19/128, 1/4.
    Fft8k,
    /// FFT size 16k / GI 1/128, 1/32, 1/16, 1/8, 19/128, 1/4.
    Fft16k,
    /// FFT size 32k / GI 1/128, 1/32, 1/16, 19/256.
    Fft32k,
    /// Reserved.
    Reserved1,
    /// Reserved.
    Reserved2,
}

impl S2Field1 {
    /// Decode from the 3-bit S2 field 1 (bits `[6:4]` of the S2 byte).
    /// Decode from the wire byte.  Every byte maps to a variant (lossless).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x07 {
            0 => Self::Fft1k,
            1 => Self::Fft2k,
            2 => Self::Fft4k,
            3 => Self::Fft8k,
            4 => Self::Fft16k,
            5 => Self::Fft32k,
            6 => Self::Reserved1,
            _ => Self::Reserved2,
        }
    }

    /// Encode to 3-bit value.
    /// Encode to the wire byte.  Inverse of `from_u8`.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Fft1k => 0,
            Self::Fft2k => 1,
            Self::Fft4k => 2,
            Self::Fft8k => 3,
            Self::Fft16k => 4,
            Self::Fft32k => 5,
            Self::Reserved1 => 6,
            Self::Reserved2 => 7,
        }
    }

    #[must_use]
    /// FFT size name (e.g. "1k", "2k").
    pub fn fft_size(self) -> &'static str {
        match self {
            Self::Fft1k => "1k",
            Self::Fft2k => "2k",
            Self::Fft4k => "4k",
            Self::Fft8k => "8k",
            Self::Fft16k => "16k",
            Self::Fft32k => "32k",
            Self::Reserved1 | Self::Reserved2 => "reserved",
        }
    }

    #[must_use]
    /// Guard-interval set description.
    pub fn guard_interval_set(self) -> &'static str {
        match self {
            Self::Fft1k => "1/128, 1/32, 1/16, 19/256, 1/8, 19/128, 1/4",
            Self::Fft2k => "1/128, 1/32, 1/16, 19/256, 1/8, 19/128, 1/4",
            Self::Fft4k => "1/128, 1/32, 1/16, 19/256, 1/8, 19/128, 1/4",
            Self::Fft8k => "1/128, 1/32, 1/16, 19/256, 1/8, 19/128, 1/4",
            Self::Fft16k => "1/128, 1/32, 1/16, 1/8, 19/128, 1/4",
            Self::Fft32k => "1/128, 1/32, 1/16, 19/256",
            Self::Reserved1 | Self::Reserved2 => "reserved",
        }
    }
}

/// FEF part: Null payload (type 0x30) per ETSI TS 102 773 §5.2.9.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FefNullPayload {
    /// FEF index within super-frame.
    pub fef_idx: u8,
    /// S1 field per EN 302 755 §7.2.1.
    pub s1_field: S1Field,
    /// S2 field per EN 302 755 §7.2.1.
    pub s2_field: u8,
}

impl FefNullPayload {
    /// Decode S2 field 1 (3 bits, upper nibble minus rfu).
    #[must_use]
    pub fn s2_field1(&self) -> S2Field1 {
        S2Field1::from_u8(self.s2_field >> 1)
    }

    /// S2 field 2: mixed flag (1 bit, bit 0).
    #[must_use]
    pub fn is_mixed(&self) -> bool {
        (self.s2_field & 0x01) != 0
    }
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
            "FEF Null {{ fef_idx: {}, s1: {:?}({}), s2: {:04b} }}",
            self.fef_idx,
            self.s1_field,
            self.s1_field.meaning(),
            self.s2_field
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

    #[test]
    fn s1_meaning_values() {
        assert_eq!(S1Field::V0.meaning(), "T2_SISO");
        assert_eq!(S1Field::V1.meaning(), "T2_MISO");
        assert_eq!(S1Field::V2.meaning(), "Non-T2");
        assert_eq!(S1Field::V3.meaning(), "T2_LITE_SISO");
        assert_eq!(S1Field::V4.meaning(), "T2_LITE_MISO");
        assert_eq!(S1Field::V5.meaning(), "reserved");
    }

    #[test]
    fn s2_field1_decode() {
        // s2_field = 0b0001 -> S2 field 1 = 000, mixed = 1
        let p = FefNullPayload {
            fef_idx: 0,
            s1_field: S1Field::V0,
            s2_field: 0x01,
        };
        assert_eq!(p.s2_field1(), S2Field1::Fft1k);
        assert!(p.is_mixed());

        // s2_field = 0b1100 -> S2 field 1 = 110 (reserved), mixed = 0
        let p = FefNullPayload {
            fef_idx: 0,
            s1_field: S1Field::V0,
            s2_field: 0x0C,
        };
        assert_eq!(p.s2_field1(), S2Field1::Reserved1);
        assert!(!p.is_mixed());
    }

    #[test]
    fn s2_field1_round_trip() {
        for v in 0u8..=7 {
            let s2 = S2Field1::from_u8(v);
            assert_eq!(s2.to_u8(), v, "S2Field1 round-trip failed for {v}");
        }
    }
}
