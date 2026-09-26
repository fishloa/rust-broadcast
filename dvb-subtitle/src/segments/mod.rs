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

/// Range-check a segment body length against the generic segment header's
/// 16-bit `segment_length` field before it is narrowed and written, so a
/// body of 64 KiB or more is rejected instead of silently wrapped (see
/// `docs/w3-rules.md`).
pub(crate) fn check_segment_length(len: usize) -> Result<u16, Error> {
    broadcast_common::len::fit_u16(len, "segment_length").map_err(|_| Error::SegmentTooLarge)
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
