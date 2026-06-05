//! T2-MI payload type 0x32: FEF part — composite — §5.2.11.
//!
//! Carries composition information for FEF parts. The actual sub-parts
//! are delivered via type 0x33 packets.

use std::fmt;

use dvb_common::{Parse, Serialize};

use super::fef_null::S1Field;

/// FEF part: composite payload (type 0x32) per ETSI TS 102 773 §5.2.11.
///
/// Layout (Figure 15):
/// - byte 0: fef_idx (8 bits)
/// - byte 1 \[7\]: rfu1 (1 bit) — must be 0
/// - byte 1 \[6:4\]: s1_field (3 bits)
/// - byte 1 \[3:0\]: s2_field (4 bits)
/// - bytes 2-5: rfu2 (32 bits) — must be 0
/// - bytes 6-7: num_subparts (16 bits)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FefCompositePayload {
    /// FEF index within super-frame.
    pub fef_idx: u8,
    /// S1 field per EN 302 755 §7.2.1.
    pub s1_field: S1Field,
    /// S2 field per EN 302 755 §7.2.1.
    pub s2_field: u8,
    /// Total number of sub-parts P.
    pub num_subparts: u16,
}

const FEF_COMPOSITE_LEN: usize = 8;

impl<'a> Parse<'a> for FefCompositePayload {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < FEF_COMPOSITE_LEN {
            return Err(crate::Error::BufferTooShort {
                need: FEF_COMPOSITE_LEN,
                have: bytes.len(),
                what: "FefCompositePayload header",
            });
        }

        // byte 1 [7]: rfu1 — must be 0
        if bytes[1] & 0x80 != 0 {
            return Err(crate::Error::ReservedBitsViolation {
                field: "byte 1 [7] rfu1",
                reason: "Must be zero (ETSI TS 102 773 §5.2.11)",
            });
        }
        // bytes 2-5: 32-bit rfu2 — must be 0
        if bytes[2] != 0 || bytes[3] != 0 || bytes[4] != 0 || bytes[5] != 0 {
            return Err(crate::Error::ReservedBitsViolation {
                field: "32-bit RFU (rfu2)",
                reason: "Must be zero (ETSI TS 102 773 §5.2.11)",
            });
        }

        Ok(FefCompositePayload {
            fef_idx: bytes[0],
            s1_field: S1Field::try_from((bytes[1] >> 4) & 0x07)?,
            s2_field: bytes[1] & 0x0F,
            num_subparts: u16::from_be_bytes([bytes[6], bytes[7]]),
        })
    }
}

impl<'a> crate::traits::PayloadDef<'a> for FefCompositePayload {
    const PACKET_TYPE: u8 = 0x32;
    const NAME: &'static str = "FEF_COMPOSITE";
}

impl Serialize for FefCompositePayload {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        FEF_COMPOSITE_LEN
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
        buf[1] = ((u8::from(self.s1_field) << 4) & 0x70) | (self.s2_field & 0x0F);
        // rfu2 = 0
        buf[2] = 0;
        buf[3] = 0;
        buf[4] = 0;
        buf[5] = 0;
        let ns = self.num_subparts.to_be_bytes();
        buf[6] = ns[0];
        buf[7] = ns[1];

        Ok(self.serialized_len())
    }
}

impl fmt::Display for FefCompositePayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FEF Composite {{ fef_idx: {}, s1: {:?}, s2: {:04b}, subparts: {} }}",
            self.fef_idx, self.s1_field, self.s2_field, self.num_subparts
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields() {
        let buf = [0x03u8, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05];
        let result = FefCompositePayload::parse(&buf).unwrap();
        assert_eq!(result.fef_idx, 3);
        assert_eq!(result.s1_field, S1Field::V1);
        assert_eq!(result.s2_field, 0);
        assert_eq!(result.num_subparts, 5);
    }

    #[test]
    fn parse_rejects_nonzero_rfu1() {
        let buf = [0x00u8, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert!(FefCompositePayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_rejects_nonzero_32bit_rfu2() {
        let buf = [0x00u8, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert!(FefCompositePayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let buf = [0x00u8; 7];
        assert!(FefCompositePayload::parse(&buf).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let orig = FefCompositePayload {
            fef_idx: 10,
            s1_field: S1Field::V7,
            s2_field: 0x0F,
            num_subparts: 100,
        };
        let mut buf = [0u8; FEF_COMPOSITE_LEN];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = FefCompositePayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let payload = FefCompositePayload {
            fef_idx: 0,
            s1_field: S1Field::V0,
            s2_field: 0,
            num_subparts: 0,
        };
        let mut buf = [0u8; 7];
        assert!(payload.serialize_into(&mut buf).is_err());
    }

    #[test]
    fn serialize_zeros_rfu_bits() {
        let payload = FefCompositePayload {
            fef_idx: 1,
            s1_field: S1Field::V0,
            s2_field: 0,
            num_subparts: 1,
        };
        let mut buf = [0xFFu8; FEF_COMPOSITE_LEN];
        payload.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[1] & 0x80, 0); // rfu1
        assert_eq!(&buf[2..6], &[0u8; 4]); // rfu2
    }
}
