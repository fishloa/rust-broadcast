//! T2-MI payload type 0x02: Arbitrary cell insertion — §5.2.3.

use broadcast_common::len::need_out;
use broadcast_common::{Parse, Serialize};

/// Arbitrary cell insertion payload (type 0x02) per ETSI TS 102 773 §5.2.3.
///
/// Layout (byte offsets relative to payload start):
/// - byte 0: frame_idx (8 bits)
/// - byte 1-2: tx_identifier (16 bits) — 0x0000 = broadcast
/// - byte 3-4: rfu (16 bits) — must be 0
/// - byte 5 \[7:6\]: rfu (2 bits) — must be 0
/// - byte 5 \[5:0\] + byte 6-7: start_cell_address (22 bits)
/// - bytes 8..: arbitrary_cell_data (variable I/Q pairs)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ArbitraryCellsPayload<'a> {
    /// FRAME_IDX of T2 frame.
    pub frame_idx: u8,
    /// Transmitter identifier (0x0000 = broadcast to all).
    pub tx_identifier: u16,
    /// Cell address per EN 302 755 §8.3.6.2 (22-bit field).
    pub start_cell_address: u32,
    /// Raw I/Q sample data (12-bit two's complement I + 12-bit Q pairs).
    pub arbitrary_cell_data: &'a [u8],
}

const ARB_CELLS_HEADER_LEN: usize = 8;
const CELL_ADDR_MASK: u32 = 0x003F_FFFF; // 22 bits

/// Offset of `tx_identifier` within the payload header.
const TX_IDENTIFIER_OFFSET: usize = 1;
/// Offsets of the two reserved header bytes.
const RFU_FIRST_OFFSET: usize = 3;
const RFU_SECOND_OFFSET: usize = 4;
/// Offset of the byte carrying the reserved prefix and the cell address's
/// high bits.
const START_CELL_HIGH_OFFSET: usize = 5;
/// Offset of the middle cell-address byte.
const START_CELL_MID_OFFSET: usize = 6;
/// Offset of the low cell-address byte.
const START_CELL_LOW_OFFSET: usize = 7;

/// Mask of the two reserved bits at the top of the high cell-address byte.
const RFU_PREFIX_MASK: u8 = 0xC0;
/// Shift placing the reserved prefix in the high cell-address byte.
const RFU_PREFIX_SHIFT: u32 = 6;
/// Shifts assembling the 18-bit reserved value for the error report.
const RFU_HIGH_SHIFT: u32 = 10;
const RFU_MID_SHIFT: u32 = 2;

/// Shift placing the cell address's high byte.
const CELL_ADDR_HIGH_SHIFT: u32 = 16;
/// Shift placing the cell address's middle byte.
const CELL_ADDR_MID_SHIFT: u32 = 8;
/// Mask of the six cell-address bits carried in its high byte.
const CELL_ADDR_HIGH_BYTE_MASK: u32 = 0x3F;
/// Mask of one byte.
const BYTE_MASK: u32 = 0xFF;

impl<'a> Parse<'a> for ArbitraryCellsPayload<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        let (hdr, rest) = bytes.split_first_chunk::<ARB_CELLS_HEADER_LEN>().ok_or(
            crate::Error::BufferTooShort(broadcast_common::error::BufferTooShort {
                need: ARB_CELLS_HEADER_LEN,
                have: bytes.len(),
                what: "ArbitraryCellsPayload header",
            }),
        )?;

        let frame_idx = hdr[0];
        let tx_identifier =
            u16::from_be_bytes([hdr[TX_IDENTIFIER_OFFSET], hdr[TX_IDENTIFIER_OFFSET + 1]]);

        // RFU: bytes 3-4 (16 bits) + byte 5 top 2 bits = 18 bits total, must be 0
        if hdr[RFU_FIRST_OFFSET] != 0
            || hdr[RFU_SECOND_OFFSET] != 0
            || (hdr[START_CELL_HIGH_OFFSET] & RFU_PREFIX_MASK != 0)
        {
            return Err(crate::Error::ReservedBits(
                broadcast_common::error::ReservedBits {
                    field: "18-bit RFU",
                    expected: 0,
                    found: (u64::from(hdr[RFU_FIRST_OFFSET]) << RFU_HIGH_SHIFT)
                        | (u64::from(hdr[RFU_SECOND_OFFSET]) << RFU_MID_SHIFT)
                        | u64::from(hdr[START_CELL_HIGH_OFFSET] >> RFU_PREFIX_SHIFT),
                },
            ));
        }

        // start_cell_address: byte 5 bottom 6 bits + bytes 6-7 = 22 bits
        let start_cell_address = ((hdr[START_CELL_HIGH_OFFSET] as u32 & CELL_ADDR_HIGH_BYTE_MASK)
            << CELL_ADDR_HIGH_SHIFT)
            | ((hdr[START_CELL_MID_OFFSET] as u32) << CELL_ADDR_MID_SHIFT)
            | (hdr[START_CELL_LOW_OFFSET] as u32);

        Ok(ArbitraryCellsPayload {
            frame_idx,
            tx_identifier,
            start_cell_address,
            arbitrary_cell_data: rest,
        })
    }
}

impl<'a> crate::traits::PayloadDef<'a> for ArbitraryCellsPayload<'a> {
    const PACKET_TYPE: u8 = 0x02;
    const NAME: &'static str = "ARBITRARY_CELLS";
}

impl Serialize for ArbitraryCellsPayload<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        ARB_CELLS_HEADER_LEN + self.arbitrary_cell_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        need_out(buf, self.serialized_len())?;

        if self.start_cell_address > CELL_ADDR_MASK {
            return Err(crate::Error::FieldOverflow(
                broadcast_common::len::FieldOverflow {
                    field: "start_cell_address",
                    value: u64::from(self.start_cell_address),
                    max: u64::from(CELL_ADDR_MASK),
                },
            ));
        }

        buf[0] = self.frame_idx;
        let tx_id = self.tx_identifier.to_be_bytes();
        buf[TX_IDENTIFIER_OFFSET] = tx_id[0];
        buf[TX_IDENTIFIER_OFFSET + 1] = tx_id[1];
        buf[RFU_FIRST_OFFSET] = 0; // RFU byte 1
        buf[RFU_SECOND_OFFSET] = 0; // RFU byte 2
        buf[START_CELL_HIGH_OFFSET] =
            ((self.start_cell_address >> CELL_ADDR_HIGH_SHIFT) & CELL_ADDR_HIGH_BYTE_MASK) as u8;
        buf[START_CELL_MID_OFFSET] =
            ((self.start_cell_address >> CELL_ADDR_MID_SHIFT) & BYTE_MASK) as u8;
        buf[START_CELL_LOW_OFFSET] = (self.start_cell_address & BYTE_MASK) as u8;

        if !self.arbitrary_cell_data.is_empty() {
            buf[ARB_CELLS_HEADER_LEN..ARB_CELLS_HEADER_LEN + self.arbitrary_cell_data.len()]
                .copy_from_slice(self.arbitrary_cell_data);
        }

        Ok(self.serialized_len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parse_extracts_fields() {
        let data = [0x10, 0xCA, 0xFE];
        let mut buf = vec![0x42u8, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00];
        buf.extend_from_slice(&data);

        let result = ArbitraryCellsPayload::parse(&buf).unwrap();
        assert_eq!(result.frame_idx, 0x42);
        assert_eq!(result.tx_identifier, 0x0001);
        assert_eq!(result.start_cell_address, 0);
        assert_eq!(result.arbitrary_cell_data, &data[..]);
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let buf = [0x00u8; 7];
        assert!(matches!(
            &(ArbitraryCellsPayload::parse(&buf)),
            Err(crate::error::Error::BufferTooShort(..))
        ));
    }

    #[test]
    fn parse_rejects_nonzero_rfu_byte_3() {
        let buf = [0x00u8, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(
            ArbitraryCellsPayload::parse(&buf).err(),
            Some(crate::Error::ReservedBits(
                broadcast_common::error::ReservedBits {
                    field: "18-bit RFU",
                    expected: 0,
                    found: 1024
                }
            ))
        );
    }

    #[test]
    fn parse_rejects_nonzero_rfu_byte_5_high_bits() {
        let mut buf = [0x00u8; 8];
        buf[5] = 0xC0; // top 2 bits set = RFU violation
        assert_eq!(
            ArbitraryCellsPayload::parse(&buf).err(),
            Some(crate::Error::ReservedBits(
                broadcast_common::error::ReservedBits {
                    field: "18-bit RFU",
                    expected: 0,
                    found: 3
                }
            ))
        );
    }

    #[test]
    fn serialize_round_trip() {
        let orig = ArbitraryCellsPayload {
            frame_idx: 0xAB,
            tx_identifier: 0x0005,
            start_cell_address: 0x123456,
            arbitrary_cell_data: &[0xDE, 0xAD],
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = ArbitraryCellsPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn tx_id_broadcast_accepted() {
        let buf = [0x00u8; 8];
        let result = ArbitraryCellsPayload::parse(&buf).unwrap();
        assert_eq!(result.tx_identifier, 0x0000);
    }

    #[test]
    fn cell_address_max_22bit() {
        let max_addr: u32 = 0x3F_FFFF;
        let buf = [0x00u8, 0x00, 0x00, 0x00, 0x00, 0x3F, 0xFF, 0xFF];
        let result = ArbitraryCellsPayload::parse(&buf).unwrap();
        assert_eq!(result.start_cell_address, max_addr);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let payload = ArbitraryCellsPayload {
            frame_idx: 0,
            tx_identifier: 0,
            start_cell_address: 0,
            arbitrary_cell_data: &[],
        };
        let mut buf = [0u8; 7];
        assert!(matches!(
            &(payload.serialize_into(&mut buf)),
            Err(crate::error::Error::OutputBufferTooSmall(..))
        ));
    }
}
