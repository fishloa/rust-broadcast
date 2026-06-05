//! T2-MI payload type 0x31: FEF part — I/Q data — §5.2.10.

use std::fmt;

use dvb_common::{Parse, Serialize};

use super::fef_null::S1Field;

/// FEF part: I/Q data payload (type 0x31) per ETSI TS 102 773 §5.2.10.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FefIqPayload<'a> {
    /// FEF index within super-frame.
    pub fef_idx: u8,
    /// S1 field per EN 302 755 §7.2.1.
    pub s1_field: S1Field,
    /// S2 field per EN 302 755 §7.2.1.
    pub s2_field: u8,
    /// Complex time-domain samples: 12-bit two's complement I, then 12-bit Q.
    pub fef_part_data: &'a [u8],
}

const FEF_IQ_HEADER_LEN: usize = 3;

impl<'a> Parse<'a> for FefIqPayload<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < FEF_IQ_HEADER_LEN {
            return Err(crate::Error::BufferTooShort {
                need: FEF_IQ_HEADER_LEN,
                have: bytes.len(),
                what: "FefIqPayload header",
            });
        }

        // Layout (Figure 13): fef_idx(8) | rfu(9) | s1_field(3) | s2_field(4) | data.
        // rfu spans all of byte 1 plus the top bit of byte 2.
        if bytes[1] != 0 || bytes[2] & 0x80 != 0 {
            return Err(crate::Error::ReservedBitsViolation {
                field: "9-bit rfu",
                reason: "Must be zero (ETSI TS 102 773 §5.2.10)",
            });
        }

        Ok(FefIqPayload {
            fef_idx: bytes[0],
            // byte 2: rfu(1) | s1_field(3) [6:4] | s2_field(4) [3:0]
            s1_field: S1Field::try_from((bytes[2] >> 4) & 0x07)?,
            s2_field: bytes[2] & 0x0F,
            fef_part_data: &bytes[FEF_IQ_HEADER_LEN..],
        })
    }
}

impl<'a> crate::traits::PayloadDef<'a> for FefIqPayload<'a> {
    const PACKET_TYPE: u8 = 0x31;
    const NAME: &'static str = "FEF_IQ";
}

impl Serialize for FefIqPayload<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        FEF_IQ_HEADER_LEN + self.fef_part_data.len()
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

        if !self.fef_part_data.is_empty() {
            buf[FEF_IQ_HEADER_LEN..FEF_IQ_HEADER_LEN + self.fef_part_data.len()]
                .copy_from_slice(self.fef_part_data);
        }

        Ok(self.serialized_len())
    }
}

impl fmt::Display for FefIqPayload<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FEF I/Q {{ fef_idx: {}, s1: {:?}, s2: {:04b}, data_len: {} }}",
            self.fef_idx,
            self.s1_field,
            self.s2_field,
            self.fef_part_data.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields_and_data() {
        let data = [0xDE, 0xAD];
        // fef_idx=5, rfu=0, byte2 = s1(2)<<4 | s2(0x0A) = 0x2A
        let mut buf = vec![0x05u8, 0x00, 0x2A];
        buf.extend_from_slice(&data);

        let result = FefIqPayload::parse(&buf).unwrap();
        assert_eq!(result.fef_idx, 5);
        assert_eq!(result.s1_field, S1Field::V2);
        assert_eq!(result.s2_field, 0x0A);
        assert_eq!(result.fef_part_data, &data[..]);
    }

    #[test]
    fn parse_rejects_nonzero_rfu() {
        let buf = [0x00u8, 0x1F, 0x00];
        assert!(FefIqPayload::parse(&buf).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let orig = FefIqPayload {
            fef_idx: 7,
            s1_field: S1Field::V3,
            s2_field: 0x0B,
            fef_part_data: &[0xCA, 0xFE],
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = FefIqPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }
}
