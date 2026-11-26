//! T2-MI payload type 0x12: P2 bias balancing cells — §5.2.6.

use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

use super::{BYTE_MASK_U16, RFU_TOP_BIT_SHIFT};

/// P2 bias balancing payload (type 0x12), ETSI TS 102 773 §5.2.6 Figure 9.
///
/// Layout: `frame_idx(8) | rfu(17) | num_active_bias_cells_per_p2(15)` = 40 bits
/// = 5 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct P2BiasPayload {
    /// FRAME_IDX of the T2 frame carrying the bias balancing cells.
    pub frame_idx: u8,
    /// Number of active bias balancing cells per P2 symbol (15-bit).
    pub num_active_bias_cells_per_p2: u16,
}

const P2_BIAS_HEADER_LEN: usize = 5;

/// Offsets of the reserved bytes within the payload.
const RFU_FIRST_OFFSET: usize = 1;
const RFU_SECOND_OFFSET: usize = 2;
/// Offset of the byte carrying the reserved top bit and the cell count's high
/// bits.
const COUNT_HIGH_OFFSET: usize = 3;
/// Offset of the low byte of the cell count.
const COUNT_LOW_OFFSET: usize = 4;

/// Mask of the reserved top bit of the count's high byte.
const RFU_TOP_BIT_MASK: u8 = 0x80;
/// Shifts assembling the 17-bit reserved value for the error report.
const RFU_HIGH_SHIFT: u32 = 9;
const RFU_MID_SHIFT: u32 = 1;

/// Mask of the seven cell-count bits carried in its high byte.
const COUNT_HIGH_BYTE_MASK: u16 = 0x7F;
/// Shift placing the cell count's high bits.
const COUNT_HIGH_SHIFT: u32 = 8;
/// Max value of the 15-bit cell count.
const COUNT_MAX: u16 = 0x7FFF;
impl<'a> Parse<'a> for P2BiasPayload {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        need_in(bytes, P2_BIAS_HEADER_LEN, "P2BiasPayload header")?;

        // rfu is 17 bits: all of bytes[1], bytes[2], and the top bit of bytes[3].
        if bytes[RFU_FIRST_OFFSET] != 0
            || bytes[RFU_SECOND_OFFSET] != 0
            || (bytes[COUNT_HIGH_OFFSET] & RFU_TOP_BIT_MASK) != 0
        {
            return Err(crate::Error::ReservedBits(
                broadcast_core::error::ReservedBits {
                    field: "17-bit RFU",
                    expected: 0,
                    found: (u64::from(bytes[RFU_FIRST_OFFSET]) << RFU_HIGH_SHIFT)
                        | (u64::from(bytes[RFU_SECOND_OFFSET]) << RFU_MID_SHIFT)
                        | u64::from(bytes[COUNT_HIGH_OFFSET] >> RFU_TOP_BIT_SHIFT),
                },
            ));
        }

        // num_active_bias_cells_per_p2 is 15 bits: bottom 7 of bytes[3] + bytes[4].
        let num_active = ((bytes[COUNT_HIGH_OFFSET] as u16 & COUNT_HIGH_BYTE_MASK)
            << COUNT_HIGH_SHIFT)
            | (bytes[COUNT_LOW_OFFSET] as u16);

        Ok(P2BiasPayload {
            frame_idx: bytes[0],
            num_active_bias_cells_per_p2: num_active,
        })
    }
}

impl crate::traits::PayloadDef<'_> for P2BiasPayload {
    const PACKET_TYPE: u8 = 0x12;
    const NAME: &'static str = "P2_BIAS";
}

impl Serialize for P2BiasPayload {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        P2_BIAS_HEADER_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        need_out(buf, self.serialized_len())?;

        if self.num_active_bias_cells_per_p2 > COUNT_MAX {
            return Err(crate::Error::FieldOverflow(
                broadcast_core::len::FieldOverflow {
                    field: "num_active_bias_cells_per_p2",
                    value: u64::from(self.num_active_bias_cells_per_p2),
                    max: u64::from(COUNT_MAX),
                },
            ));
        }

        buf[0] = self.frame_idx;
        buf[RFU_FIRST_OFFSET] = 0; // rfu
        buf[RFU_SECOND_OFFSET] = 0; // rfu
        buf[COUNT_HIGH_OFFSET] = ((self.num_active_bias_cells_per_p2 >> COUNT_HIGH_SHIFT) as u8)
            & COUNT_HIGH_BYTE_MASK as u8;
        buf[COUNT_LOW_OFFSET] = (self.num_active_bias_cells_per_p2 & BYTE_MASK_U16) as u8;

        Ok(self.serialized_len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields() {
        // frame_idx=0x42, rfu=0, num_active=0x0F
        let buf = [0x42u8, 0x00, 0x00, 0x00, 0x0F];
        let result = P2BiasPayload::parse(&buf).unwrap();
        assert_eq!(result.frame_idx, 0x42);
        assert_eq!(result.num_active_bias_cells_per_p2, 0x0F);
    }

    #[test]
    fn parse_extracts_15bit_max() {
        // num_active = 0x7FFF (all 15 bits): bytes[3]=0x7F, bytes[4]=0xFF
        let buf = [0x00u8, 0x00, 0x00, 0x7F, 0xFF];
        let result = P2BiasPayload::parse(&buf).unwrap();
        assert_eq!(result.num_active_bias_cells_per_p2, 0x7FFF);
    }

    #[test]
    fn parse_rejects_nonzero_rfu() {
        // top bit of bytes[3] is part of the RFU
        let buf = [0x00u8, 0x00, 0x00, 0x80, 0x00];
        assert_eq!(
            P2BiasPayload::parse(&buf).err(),
            Some(crate::Error::ReservedBits(
                broadcast_core::error::ReservedBits {
                    field: "17-bit RFU",
                    expected: 0,
                    found: 1
                }
            ))
        );
        // a byte in the middle of the RFU span
        assert_eq!(
            P2BiasPayload::parse(&[0x00u8, 0x01, 0x00, 0x00, 0x00]).err(),
            Some(crate::Error::ReservedBits(
                broadcast_core::error::ReservedBits {
                    field: "17-bit RFU",
                    expected: 0,
                    found: 512
                }
            ))
        );
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            &(P2BiasPayload::parse(&[0x00, 0x00, 0x00, 0x00])),
            Err(crate::error::Error::BufferTooShort(..))
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let orig = P2BiasPayload {
            frame_idx: 5,
            num_active_bias_cells_per_p2: 3000,
        };
        let mut buf = [0u8; 5];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = P2BiasPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let payload = P2BiasPayload {
            frame_idx: 0,
            num_active_bias_cells_per_p2: 0,
        };
        let mut buf = [0u8; 4];
        assert_eq!(
            payload.serialize_into(&mut buf).err(),
            Some(crate::Error::OutputBufferTooSmall(
                broadcast_core::error::OutputBufferTooSmall { need: 5, have: 4 }
            ))
        );
    }
}
