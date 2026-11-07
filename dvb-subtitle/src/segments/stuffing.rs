//! Stuffing segment — ETSI EN 300 743 Table 7 (segment_type 0xFF).
//!
//! Stuffing bytes have no semantic meaning; their segment_type is 0xFF.
//! The body is opaque raw bytes.

use crate::error::{Error, Result};
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// The stuffing segment_type.
pub const SEGMENT_TYPE: u8 = 0xFF;
/// Header: sync_byte(1) + segment_type(1) + page_id(2) + segment_length(2) = 6 bytes.
pub const HEADER_LEN: usize = 6;

/// A stuffing segment — opaque data bytes following the header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StuffingSegment<'a> {
    /// The page_id from the segment header.
    pub page_id: u16,
    /// Opaque stuffing data bytes.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub data: &'a [u8],
}

impl<'a> Parse<'a> for StuffingSegment<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, HEADER_LEN, "stuffing_segment")?;
        if bytes[0] != crate::pes_data_field::SYNC_BYTE {
            return Err(Error::InvalidSync(broadcast_common::error::InvalidSync {
                what: "sync_byte",
                expected: u32::from(crate::pes_data_field::SYNC_BYTE),
                found: u32::from(bytes[0]),
            }));
        }
        if bytes[1] != SEGMENT_TYPE {
            return Err(Error::UnknownSegmentType(bytes[1]));
        }
        let page_id = u16::from_be_bytes([bytes[2], bytes[3]]);
        let segment_length = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        let total = HEADER_LEN + segment_length;
        need_in(bytes, total, "stuffing_segment data")?;
        Ok(StuffingSegment {
            page_id,
            data: &bytes[HEADER_LEN..total],
        })
    }
}

impl Serialize for StuffingSegment<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        buf[0] = 0x0F;
        buf[1] = SEGMENT_TYPE;
        buf[2..4].copy_from_slice(&self.page_id.to_be_bytes());
        let seg_len = crate::segments::check_segment_length(self.data.len())?;
        buf[4..6].copy_from_slice(&seg_len.to_be_bytes());
        buf[6..len].copy_from_slice(self.data);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    #[test]
    fn round_trip() {
        let bytes = [0x0F, 0xFF, 0x00, 0x00, 0x00, 0x03, 0xAA, 0xBB, 0xCC];
        let seg = StuffingSegment::parse(&bytes).unwrap();
        assert_eq!(seg.data, &[0xAA, 0xBB, 0xCC]);
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);

        // Biting test: page_id changes output
        let mut seg2 = seg.clone();
        seg2.page_id = 9;
        let out2 = seg2.try_to_bytes().unwrap();
        assert_ne!(out2, bytes);
        let reparse = StuffingSegment::parse(&out2).unwrap();
        assert_eq!(reparse.page_id, 9);
    }

    /// DS-W2 (#1108): sync_byte wasn't checked by any typed segment parser.
    #[test]
    fn rejects_bad_sync_byte() {
        let bytes = [0x00, 0xFF, 0x00, 0x00, 0x00, 0x03, 0xAA, 0xBB, 0xCC];
        assert!(matches!(
            StuffingSegment::parse(&bytes),
            Err(Error::InvalidSync(broadcast_common::error::InvalidSync {
                found: 0x00,
                ..
            }))
        ));
    }
}
