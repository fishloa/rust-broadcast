//! Subtitling-specific traits. `Parse` is provided by `broadcast_common`.

use broadcast_common::Parse;

/// Implemented by every typed subtitling segment; drives
/// [`crate::any::AnySegment`] dispatch. `SEGMENT_TYPE` is the wire
/// segment_type this type parses.
pub trait SegmentDef<'a>: Parse<'a, Error = crate::error::Error> {
    /// Wire segment_type.
    const SEGMENT_TYPE: u8;
    /// Diagnostic name. Convention: SCREAMING_SNAKE, suffix-free —
    /// `PAGE_COMPOSITION`, `REGION_COMPOSITION`, `OBJECT_DATA`.
    const NAME: &'static str;
}
