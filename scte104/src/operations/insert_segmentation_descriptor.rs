//! insert_segmentation_descriptor_request_data() — ANSI/SCTE 104 2023 §9.8.7, Table 9-29 (opID 0x010B).
//!
//! Supplemental usage. Creates a segmentation descriptor in the resulting
//! SCTE 35 section.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for insert_segmentation_descriptor_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x010B;

/// insert_segmentation_descriptor_request_data() — §9.8.7, Table 9-29.
///
/// The wire format carries a variable-length `segmentation_upid` field whose
/// length is determined by `segmentation_upid_length`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InsertSegmentationDescriptor<'a> {
    /// `segmentation_event_id` — 4 bytes.
    pub segmentation_event_id: u32,
    /// `segmentation_event_cancel_indicator` — 1 byte.
    pub segmentation_event_cancel_indicator: u8,
    /// `duration` — 2 bytes, whole seconds.
    pub duration: u16,
    /// `segmentation_upid_type` — 1 byte.
    pub segmentation_upid_type: u8,
    /// `segmentation_upid` data — variable length.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub segmentation_upid: &'a [u8],
    /// `segmentation_type_id` — 1 byte.
    pub segmentation_type_id: u8,
    /// `segment_num` — 1 byte.
    pub segment_num: u8,
    /// `segments_expected` — 1 byte.
    pub segments_expected: u8,
    /// `duration_extension_frames` — 1 byte.
    pub duration_extension_frames: u8,
    /// `delivery_not_restricted_flag` — 1 byte.
    pub delivery_not_restricted_flag: u8,
    /// `web_delivery_allowed_flag` — 1 byte.
    pub web_delivery_allowed_flag: u8,
    /// `no_regional_blackout_flag` — 1 byte.
    pub no_regional_blackout_flag: u8,
    /// `archive_allowed_flag` — 1 byte.
    pub archive_allowed_flag: u8,
    /// `device_restrictions` — 1 byte.
    pub device_restrictions: u8,
    /// `insert_sub_segment_info` — 1 byte.
    pub insert_sub_segment_info: u8,
    /// `sub_segment_num` — 1 byte.
    pub sub_segment_num: u8,
    /// `sub_segments_expected` — 1 byte.
    pub sub_segments_expected: u8,
}

/// The minimum fixed-size portion before the variable `segmentation_upid`.
pub const FIXED_LEN: usize = 7;
/// The fixed-size portion after `segmentation_upid`.
pub const TAIL_LEN: usize = 13;

impl<'a> Parse<'a> for InsertSegmentationDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FIXED_LEN + TAIL_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_LEN + TAIL_LEN,
                have: bytes.len(),
                what: "insert_segmentation_descriptor (minimum)",
            });
        }
        let segmentation_event_id = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let segmentation_event_cancel_indicator = bytes[4];
        let duration = u16::from_be_bytes([bytes[5], bytes[6]]);
        let segmentation_upid_type = bytes[7];
        let upid_len = bytes[8] as usize;
        let need = FIXED_LEN + 2 + upid_len + TAIL_LEN;
        if bytes.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: bytes.len(),
                what: "insert_segmentation_descriptor upid",
            });
        }
        let upid = &bytes[9..9 + upid_len];
        let tail_off = 9 + upid_len;
        Ok(Self {
            segmentation_event_id,
            segmentation_event_cancel_indicator,
            duration,
            segmentation_upid_type,
            segmentation_upid: upid,
            segmentation_type_id: bytes[tail_off],
            segment_num: bytes[tail_off + 1],
            segments_expected: bytes[tail_off + 2],
            duration_extension_frames: bytes[tail_off + 3],
            delivery_not_restricted_flag: bytes[tail_off + 4],
            web_delivery_allowed_flag: bytes[tail_off + 5],
            no_regional_blackout_flag: bytes[tail_off + 6],
            archive_allowed_flag: bytes[tail_off + 7],
            device_restrictions: bytes[tail_off + 8],
            insert_sub_segment_info: bytes[tail_off + 9],
            sub_segment_num: bytes[tail_off + 10],
            sub_segments_expected: bytes[tail_off + 11],
        })
    }
}

impl Serialize for InsertSegmentationDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        FIXED_LEN + 2 + self.segmentation_upid.len() + TAIL_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.segmentation_event_id.to_be_bytes());
        buf[4] = self.segmentation_event_cancel_indicator;
        buf[5..7].copy_from_slice(&self.duration.to_be_bytes());
        buf[7] = self.segmentation_upid_type;
        buf[8] = self.segmentation_upid.len() as u8;
        let upid_len = self.segmentation_upid.len();
        buf[9..9 + upid_len].copy_from_slice(self.segmentation_upid);
        let tail_off = 9 + upid_len;
        buf[tail_off] = self.segmentation_type_id;
        buf[tail_off + 1] = self.segment_num;
        buf[tail_off + 2] = self.segments_expected;
        buf[tail_off + 3] = self.duration_extension_frames;
        buf[tail_off + 4] = self.delivery_not_restricted_flag;
        buf[tail_off + 5] = self.web_delivery_allowed_flag;
        buf[tail_off + 6] = self.no_regional_blackout_flag;
        buf[tail_off + 7] = self.archive_allowed_flag;
        buf[tail_off + 8] = self.device_restrictions;
        buf[tail_off + 9] = self.insert_sub_segment_info;
        buf[tail_off + 10] = self.sub_segment_num;
        buf[tail_off + 11] = self.sub_segments_expected;
        Ok(need)
    }
}

impl<'a> OperationDef<'a> for InsertSegmentationDescriptor<'a> {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "INSERT_SEGMENTATION_DESCRIPTOR";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = InsertSegmentationDescriptor {
            segmentation_event_id: 0x42,
            segmentation_event_cancel_indicator: 0,
            duration: 3600,
            segmentation_upid_type: 1,
            segmentation_upid: &[0x01, 0x02, 0x03],
            segmentation_type_id: 0x30,
            segment_num: 1,
            segments_expected: 5,
            duration_extension_frames: 0,
            delivery_not_restricted_flag: 1,
            web_delivery_allowed_flag: 0,
            no_regional_blackout_flag: 0,
            archive_allowed_flag: 1,
            device_restrictions: 0,
            insert_sub_segment_info: 0,
            sub_segment_num: 0,
            sub_segments_expected: 0,
        };
        let bytes = op.to_bytes();
        let back = InsertSegmentationDescriptor::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn empty_upid_round_trip() {
        let op = InsertSegmentationDescriptor {
            segmentation_upid: &[],
            ..InsertSegmentationDescriptor::default()
        };
        let bytes = op.to_bytes();
        let back = InsertSegmentationDescriptor::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = InsertSegmentationDescriptor {
            segmentation_event_id: 0x42,
            ..InsertSegmentationDescriptor::default()
        };
        let bytes = op.to_bytes();
        let mut op2 = op.clone();
        op2.segmentation_event_id = 999;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
