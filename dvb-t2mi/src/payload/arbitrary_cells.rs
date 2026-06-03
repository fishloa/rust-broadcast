//! T2-MI payload type 0x02: Arbitrary cell insertion — §5.2.3.

use dvb_common::{Parse, Serialize};

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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArbitraryCellsPayload<'a> {
    /// FRAME_IDX of T2 frame.
    pub frame_idx: u8,
    /// Transmitter identifier (0x0000 = broadcast to all).
    pub tx_identifier: u16,
    /// Cell address per EN 302 755 §8.3.6.2 (22-bit field).
    pub start_cell_address: u32,
    /// Raw I/Q sample data (12-bit two's complement I + 12-bit Q pairs).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub arbitrary_cell_data: &'a [u8],
}

const ARB_CELLS_HEADER_LEN: usize = 8;
const CELL_ADDR_MASK: u32 = 0x003F_FFFF; // 22 bits

impl<'a> Parse<'a> for ArbitraryCellsPayload<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < ARB_CELLS_HEADER_LEN {
            return Err(crate::Error::BufferTooShort {
                need: ARB_CELLS_HEADER_LEN,
                have: bytes.len(),
                what: "ArbitraryCellsPayload header",
            });
        }

        let frame_idx = bytes[0];
        let tx_identifier = u16::from_be_bytes([bytes[1], bytes[2]]);

        // RFU: bytes 3-4 (16 bits) + byte 5 top 2 bits = 18 bits total, must be 0
        if bytes[3] != 0 || bytes[4] != 0 || (bytes[5] & 0xC0 != 0) {
            return Err(crate::Error::ReservedBitsViolation {
                field: "18-bit RFU",
                reason: "Must be zero (ETSI TS 102 773 §5.2.3)",
            });
        }

        // start_cell_address: byte 5 bottom 6 bits + bytes 6-7 = 22 bits
        let start_cell_address =
            ((bytes[5] as u32 & 0x3F) << 16) | ((bytes[6] as u32) << 8) | (bytes[7] as u32);

        Ok(ArbitraryCellsPayload {
            frame_idx,
            tx_identifier,
            start_cell_address,
            arbitrary_cell_data: &bytes[ARB_CELLS_HEADER_LEN..],
        })
    }
}

impl Serialize for ArbitraryCellsPayload<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        ARB_CELLS_HEADER_LEN + self.arbitrary_cell_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        if buf.len() < self.serialized_len() {
            return Err(crate::Error::OutputBufferTooSmall {
                need: self.serialized_len(),
                have: buf.len(),
            });
        }

        if self.start_cell_address > CELL_ADDR_MASK {
            return Err(crate::Error::ReservedBitsViolation {
                field: "start_cell_address",
                reason: "Must fit in 22 bits",
            });
        }

        buf[0] = self.frame_idx;
        let tx_id = self.tx_identifier.to_be_bytes();
        buf[1] = tx_id[0];
        buf[2] = tx_id[1];
        buf[3] = 0; // RFU byte 1
        buf[4] = 0; // RFU byte 2
        buf[5] = ((self.start_cell_address >> 16) & 0x3F) as u8; // RFU top 2 bits = 0, cell addr top 6
        buf[6] = ((self.start_cell_address >> 8) & 0xFF) as u8;
        buf[7] = (self.start_cell_address & 0xFF) as u8;

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
        assert!(ArbitraryCellsPayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_rejects_nonzero_rfu_byte_3() {
        let buf = [0x00u8, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
        assert!(ArbitraryCellsPayload::parse(&buf).is_err());
    }

    #[test]
    fn parse_rejects_nonzero_rfu_byte_5_high_bits() {
        let mut buf = [0x00u8; 8];
        buf[5] = 0xC0; // top 2 bits set = RFU violation
        assert!(ArbitraryCellsPayload::parse(&buf).is_err());
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
        assert!(payload.serialize_into(&mut buf).is_err());
    }
}
