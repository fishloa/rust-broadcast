//! insert_segmentation_descriptor_request_data() — ANSI/SCTE 104 2023 §9.8.7, Table 9-29 (opID 0x010B).
//!
//! Supplemental usage. Creates a segmentation descriptor in the resulting
//! SCTE 35 section.

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::len::{need_in, need_out};
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
/// The fixed-size portion after `segmentation_upid` — the 12 one-byte fields
/// `segmentation_type_id` through `sub_segments_expected` (§9.8.7, Table 9-29).
pub const TAIL_LEN: usize = 12;

// Field byte offsets before the variable `segmentation_upid` (§9.8.7, Table 9-29).
const SEGMENTATION_EVENT_ID_OFFSET: usize = 0;
const SEGMENTATION_EVENT_CANCEL_INDICATOR_OFFSET: usize = 4;
const DURATION_OFFSET: usize = 5;
const SEGMENTATION_UPID_TYPE_OFFSET: usize = 7;
const SEGMENTATION_UPID_LENGTH_OFFSET: usize = 8;
const SEGMENTATION_UPID_OFFSET: usize = 9;
/// Offset of the second 16-bit word within a 4-byte big-endian word.
const U32_SECOND_WORD_OFFSET: usize = core::mem::size_of::<u16>();

// Offsets within the 12-byte tail, relative to `tail_off`.
const SEGMENTATION_TYPE_ID_OFFSET: usize = 0;
const SEGMENT_NUM_OFFSET: usize = 1;
const SEGMENTS_EXPECTED_OFFSET: usize = 2;
const DURATION_EXTENSION_FRAMES_OFFSET: usize = 3;
const DELIVERY_NOT_RESTRICTED_FLAG_OFFSET: usize = 4;
const WEB_DELIVERY_ALLOWED_FLAG_OFFSET: usize = 5;
const NO_REGIONAL_BLACKOUT_FLAG_OFFSET: usize = 6;
const ARCHIVE_ALLOWED_FLAG_OFFSET: usize = 7;
const DEVICE_RESTRICTIONS_OFFSET: usize = 8;
const INSERT_SUB_SEGMENT_INFO_OFFSET: usize = 9;
const SUB_SEGMENT_NUM_OFFSET: usize = 10;
const SUB_SEGMENTS_EXPECTED_OFFSET: usize = 11;

impl<'a> Parse<'a> for InsertSegmentationDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            FIXED_LEN + TAIL_LEN,
            "insert_segmentation_descriptor (minimum)",
        )?;
        let segmentation_event_id = u32::from_be_bytes([
            bytes[SEGMENTATION_EVENT_ID_OFFSET],
            bytes[SEGMENTATION_EVENT_ID_OFFSET + 1],
            bytes[SEGMENTATION_EVENT_ID_OFFSET + U32_SECOND_WORD_OFFSET],
            bytes[SEGMENTATION_EVENT_ID_OFFSET + U32_SECOND_WORD_OFFSET + 1],
        ]);
        let segmentation_event_cancel_indicator = bytes[SEGMENTATION_EVENT_CANCEL_INDICATOR_OFFSET];
        let duration = u16::from_be_bytes([bytes[DURATION_OFFSET], bytes[DURATION_OFFSET + 1]]);
        let segmentation_upid_type = bytes[SEGMENTATION_UPID_TYPE_OFFSET];
        let upid_len = bytes[SEGMENTATION_UPID_LENGTH_OFFSET] as usize;
        let need = SEGMENTATION_UPID_OFFSET + upid_len + TAIL_LEN;
        need_in(bytes, need, "insert_segmentation_descriptor upid")?;
        let upid = &bytes[SEGMENTATION_UPID_OFFSET..SEGMENTATION_UPID_OFFSET + upid_len];
        let tail_off = SEGMENTATION_UPID_OFFSET + upid_len;
        Ok(Self {
            segmentation_event_id,
            segmentation_event_cancel_indicator,
            duration,
            segmentation_upid_type,
            segmentation_upid: upid,
            segmentation_type_id: bytes[tail_off + SEGMENTATION_TYPE_ID_OFFSET],
            segment_num: bytes[tail_off + SEGMENT_NUM_OFFSET],
            segments_expected: bytes[tail_off + SEGMENTS_EXPECTED_OFFSET],
            duration_extension_frames: bytes[tail_off + DURATION_EXTENSION_FRAMES_OFFSET],
            delivery_not_restricted_flag: bytes[tail_off + DELIVERY_NOT_RESTRICTED_FLAG_OFFSET],
            web_delivery_allowed_flag: bytes[tail_off + WEB_DELIVERY_ALLOWED_FLAG_OFFSET],
            no_regional_blackout_flag: bytes[tail_off + NO_REGIONAL_BLACKOUT_FLAG_OFFSET],
            archive_allowed_flag: bytes[tail_off + ARCHIVE_ALLOWED_FLAG_OFFSET],
            device_restrictions: bytes[tail_off + DEVICE_RESTRICTIONS_OFFSET],
            insert_sub_segment_info: bytes[tail_off + INSERT_SUB_SEGMENT_INFO_OFFSET],
            sub_segment_num: bytes[tail_off + SUB_SEGMENT_NUM_OFFSET],
            sub_segments_expected: bytes[tail_off + SUB_SEGMENTS_EXPECTED_OFFSET],
        })
    }
}

impl Serialize for InsertSegmentationDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        SEGMENTATION_UPID_OFFSET + self.segmentation_upid.len() + TAIL_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        buf[0..4].copy_from_slice(&self.segmentation_event_id.to_be_bytes());
        buf[SEGMENTATION_EVENT_CANCEL_INDICATOR_OFFSET] = self.segmentation_event_cancel_indicator;
        buf[5..7].copy_from_slice(&self.duration.to_be_bytes());
        buf[SEGMENTATION_UPID_TYPE_OFFSET] = self.segmentation_upid_type;
        buf[SEGMENTATION_UPID_LENGTH_OFFSET] = broadcast_common::len::fit_u8(
            self.segmentation_upid.len(),
            "insert_segmentation_descriptor.segmentation_upid_length",
        )?;
        let upid_len = self.segmentation_upid.len();
        buf[SEGMENTATION_UPID_OFFSET..SEGMENTATION_UPID_OFFSET + upid_len]
            .copy_from_slice(self.segmentation_upid);
        let tail_off = SEGMENTATION_UPID_OFFSET + upid_len;
        buf[tail_off + SEGMENTATION_TYPE_ID_OFFSET] = self.segmentation_type_id;
        buf[tail_off + SEGMENT_NUM_OFFSET] = self.segment_num;
        buf[tail_off + SEGMENTS_EXPECTED_OFFSET] = self.segments_expected;
        buf[tail_off + DURATION_EXTENSION_FRAMES_OFFSET] = self.duration_extension_frames;
        buf[tail_off + DELIVERY_NOT_RESTRICTED_FLAG_OFFSET] = self.delivery_not_restricted_flag;
        buf[tail_off + WEB_DELIVERY_ALLOWED_FLAG_OFFSET] = self.web_delivery_allowed_flag;
        buf[tail_off + NO_REGIONAL_BLACKOUT_FLAG_OFFSET] = self.no_regional_blackout_flag;
        buf[tail_off + ARCHIVE_ALLOWED_FLAG_OFFSET] = self.archive_allowed_flag;
        buf[tail_off + DEVICE_RESTRICTIONS_OFFSET] = self.device_restrictions;
        buf[tail_off + INSERT_SUB_SEGMENT_INFO_OFFSET] = self.insert_sub_segment_info;
        buf[tail_off + SUB_SEGMENT_NUM_OFFSET] = self.sub_segment_num;
        buf[tail_off + SUB_SEGMENTS_EXPECTED_OFFSET] = self.sub_segments_expected;
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
        let bytes = op.try_to_bytes().unwrap();
        let back = InsertSegmentationDescriptor::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn empty_upid_round_trip() {
        let op = InsertSegmentationDescriptor {
            segmentation_upid: &[],
            ..InsertSegmentationDescriptor::default()
        };
        let bytes = op.try_to_bytes().unwrap();
        let back = InsertSegmentationDescriptor::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    /// S4-W3 (#1129): `segmentation_upid_length` is an 8-bit field but used
    /// to be written with `self.segmentation_upid.len() as u8`, wrapping to
    /// 0 for 256+ bytes while the whole upid was still serialized.
    #[test]
    fn over_255_upid_bytes_rejected_not_wrapped() {
        let upid = alloc::vec![0xABu8; 256];
        let op = InsertSegmentationDescriptor {
            segmentation_upid: &upid,
            ..InsertSegmentationDescriptor::default()
        };
        assert!(matches!(
            &(op.try_to_bytes()),
            Err(crate::error::Error::FieldOverflow(_))
        ));
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = InsertSegmentationDescriptor {
            segmentation_event_id: 0x42,
            ..InsertSegmentationDescriptor::default()
        };
        let bytes = op.try_to_bytes().unwrap();
        let mut op2 = op.clone();
        op2.segmentation_event_id = 999;
        assert_ne!(op2.try_to_bytes().unwrap(), bytes);
    }
}
