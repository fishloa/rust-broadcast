//! End of Display Set Segment — ETSI EN 300 743 §7.2.6, Table 28 (segment_type 0x80).

use crate::error::{Error, Result};
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// The end_of_display_set_segment segment_type.
pub const SEGMENT_TYPE: u8 = 0x80;
/// Header: sync_byte(1) + segment_type(1) + page_id(2) + segment_length(2) = 6 bytes. segment_length must be 0.
pub const HEADER_LEN: usize = 6;

/// End of Display Set Segment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EndOfDisplaySetSegment {
    /// The page_id (from generic segment header).
    pub page_id: u16,
}

impl<'a> Parse<'a> for EndOfDisplaySetSegment {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, HEADER_LEN, "end_of_display_set_segment")?;
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
        let segment_length = u16::from_be_bytes([bytes[4], bytes[5]]);
        if segment_length != 0 {
            return Err(Error::SegmentTooLarge);
        }
        Ok(EndOfDisplaySetSegment { page_id })
    }
}

impl Serialize for EndOfDisplaySetSegment {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        buf[0] = 0x0F;
        buf[1] = SEGMENT_TYPE;
        buf[2..4].copy_from_slice(&self.page_id.to_be_bytes());
        buf[4..6].copy_from_slice(&0u16.to_be_bytes()); // segment_length = 0
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    #[test]
    fn serialize_into_short_output_is_output_buffer_too_small() {
        let bytes = [0x0F, 0x80, 0x00, 0x01, 0x00, 0x00];
        let seg = EndOfDisplaySetSegment::parse(&bytes).unwrap();
        let mut out = [0u8; 5];
        assert_eq!(
            seg.serialize_into(&mut out),
            Err(crate::Error::OutputBufferTooSmall(
                broadcast_common::error::OutputBufferTooSmall { need: 6, have: 5 }
            ))
        );
    }

    #[test]
    fn round_trip() {
        let bytes = [0x0F, 0x80, 0x00, 0x01, 0x00, 0x00];
        let seg = EndOfDisplaySetSegment::parse(&bytes).unwrap();
        assert_eq!(seg.page_id, 1);
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);

        // Biting test: mutation changes output
        let mut seg2 = seg.clone();
        seg2.page_id = 5;
        let out2 = seg2.try_to_bytes().unwrap();
        assert_ne!(out2, bytes);
        let reparse = EndOfDisplaySetSegment::parse(&out2).unwrap();
        assert_eq!(reparse.page_id, 5);
    }

    /// DS-W2 (#1108): sync_byte wasn't checked by any typed segment parser
    /// (only by `PesDataField`, whose loop only ever calls a typed parser
    /// with `bytes[0] == 0x0F` anyway), so a direct caller of `Self::parse`
    /// could parse a non-segment, and re-serializing it would silently
    /// change byte 0 back to `0x0F`.
    #[test]
    fn rejects_bad_sync_byte() {
        let bytes = [0x00, 0x80, 0x00, 0x01, 0x00, 0x00];
        assert!(matches!(
            EndOfDisplaySetSegment::parse(&bytes),
            Err(Error::InvalidSync(broadcast_common::error::InvalidSync {
                found: 0x00,
                ..
            }))
        ));
    }
}
