//! Segment type modules.
//!
//! Each module implements one segment type from ETSI EN 300 743 §7.2,
//! plus an [`AnySegment`] dispatch enum and the [`SegmentDef`] trait.

pub mod alternative_clut;
pub mod clut_definition;
pub mod disparity_signalling;
pub mod display_definition;
pub mod end_of_display_set;
pub mod object_data;
pub mod page_composition;
pub mod region_composition;
pub mod stuffing;

use crate::error::Error;

/// Bytes of the generic subtitling_segment header (EN 300 743 §7.2, Table 2).
pub(crate) const HEADER_LEN: usize = 6;
/// Offset of `segment_type` within a segment header.
pub(crate) const SEGMENT_TYPE_OFFSET: usize = 1;
/// Offset of `page_id` within a segment header.
pub(crate) const PAGE_ID_OFFSET: usize = 2;
/// Bytes of `page_id`.
pub(crate) const PAGE_ID_LEN: usize = 2;
/// Offset of `segment_length` within a segment header.
pub(crate) const SEGMENT_LENGTH_OFFSET: usize = 4;
/// Bytes of `segment_length`.
pub(crate) const SEGMENT_LENGTH_LEN: usize = 2;

/// Read the big-endian 16-bit field at `offset` of `bytes`.
///
/// The segment header carries `page_id` and `segment_length` most-significant
/// byte first (EN 300 743 §7.2, Table 2), so a field is copied into one sized
/// array rather than read out one index at a time.
pub(crate) fn be_u16(bytes: &[u8], offset: usize) -> u16 {
    const FIELD_LEN: usize = core::mem::size_of::<u16>();
    let mut field = [0u8; FIELD_LEN];
    field.copy_from_slice(&bytes[offset..offset + FIELD_LEN]);
    u16::from_be_bytes(field)
}

/// Range-check a segment body length against the generic segment header's
/// 16-bit `segment_length` field before it is narrowed and written, so a
/// body of 64 KiB or more is rejected instead of silently wrapped (see
/// `private/specs-verified/dvb/dvb-subtitle/`).
pub(crate) fn check_segment_length(len: usize) -> Result<u16, Error> {
    broadcast_core::len::fit_u16(len, "segment_length").map_err(|_| Error::SegmentTooLarge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_len_is_rejected_not_wrapped() {
        // Before the fix, 65 536 silently wrapped to segment_length 0 and
        // returned Ok.
        assert_eq!(check_segment_length(65_536), Err(Error::SegmentTooLarge));
    }

    #[test]
    fn max_len_still_fits() {
        assert_eq!(check_segment_length(0xFFFF), Ok(0xFFFF));
    }
}
