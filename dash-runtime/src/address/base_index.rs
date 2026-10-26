//! SegmentBase + `sidx`: the on-demand profile's segment ranges
//! (ISO/IEC 23009-1 5.3.9.5.4; ISO/IEC 14496-12 8.16.3).
//!
//! The Representation is one file; `@indexRange` locates its `sidx` box. The
//! sidx offsets are anchored at the first byte after the box (14496-12
//! 8.16.3), so each reference's byte range follows from the box alone.

use broadcast_common::Parse;
use transmux::SegmentIndexBox;

use super::AddressError;
use crate::error::Unsupported;
use crate::mpd::model::ByteRange;

/// `reference_type` of a reference that points at another `sidx`.
const REFERENCE_TYPE_SIDX: u8 = 1;

/// One subsegment described by a `sidx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct IndexedSegment {
    /// Earliest presentation time on the sidx timescale.
    pub time: u64,
    /// Duration in sidx timescale ticks.
    pub duration: u64,
    /// The byte range of the subsegment in the file.
    pub range: ByteRange,
    /// `starts_with_SAP`.
    pub starts_with_sap: bool,
}

/// The anchor offset: the first byte after the index box, i.e.
/// `indexRange.last + 1`.
///
/// # Errors
/// [`AddressError::Unsupported`] for an open-ended range.
pub fn anchor_after(index_range: &ByteRange) -> Result<u64, AddressError> {
    index_range
        .last
        .and_then(|l| l.checked_add(1))
        .ok_or(AddressError::Unsupported(Unsupported::OpenEndedRange))
}

/// Parse the bytes of `@indexRange` (the whole `sidx` box, header included).
///
/// # Errors
/// [`AddressError::Sidx`] when the bytes are not a `sidx` box.
pub fn parse_index(bytes: &[u8]) -> Result<SegmentIndexBox, AddressError> {
    SegmentIndexBox::parse(bytes).map_err(|e| AddressError::Sidx(e.to_string()))
}

/// The subsegments of `sidx`, with byte ranges relative to the file start.
/// `anchor` is [`anchor_after`] of the `@indexRange`.
///
/// # Errors
/// [`AddressError::Unsupported`] for a hierarchical index;
/// [`AddressError::Overflow`] for sizes that overflow a file offset.
pub fn segments(sidx: &SegmentIndexBox, anchor: u64) -> Result<Vec<IndexedSegment>, AddressError> {
    let mut offset = anchor
        .checked_add(sidx.first_offset)
        .ok_or(AddressError::Overflow)?;
    let mut time = sidx.earliest_presentation_time;
    let mut out = Vec::with_capacity(sidx.references.len());
    for r in &sidx.references {
        if r.reference_type == REFERENCE_TYPE_SIDX {
            return Err(AddressError::Unsupported(Unsupported::HierarchicalSidx));
        }
        let size = u64::from(r.referenced_size);
        let last = offset
            .checked_add(size)
            .and_then(|e| e.checked_sub(1))
            .ok_or(AddressError::Overflow)?;
        let duration = u64::from(r.subsegment_duration);
        out.push(IndexedSegment {
            time,
            duration,
            range: ByteRange::new(offset, last),
            starts_with_sap: r.starts_with_sap != 0,
        });
        offset = last.checked_add(1).ok_or(AddressError::Overflow)?;
        time = time.checked_add(duration).ok_or(AddressError::Overflow)?;
    }
    Ok(out)
}
