//! Hostile-input bounds for MPD parsing and segment expansion.
//!
//! An MPD is fetched from a server the player does not control, so every
//! dimension of it that costs memory or CPU has a named ceiling and a typed
//! error ([`crate::error::MpdError::TooLarge`]) instead of an allocation.

/// Largest MPD document accepted, in bytes.
pub const MAX_MPD_BYTES: usize = 4 * 1024 * 1024;
/// Deepest element nesting accepted.
pub const MAX_XML_DEPTH: usize = 32;
/// Most elements (of any namespace) accepted in one document.
pub const MAX_XML_ELEMENTS: usize = 100_000;
/// Longest single attribute value accepted, in bytes.
pub const MAX_ATTRIBUTE_BYTES: usize = 8 * 1024;
/// Most attributes accepted on one element.
pub const MAX_ATTRIBUTES_PER_ELEMENT: usize = 256;
/// Most `Period` elements accepted.
pub const MAX_PERIODS: usize = 1_000;
/// Most `AdaptationSet` elements accepted in one `Period`.
pub const MAX_ADAPTATION_SETS_PER_PERIOD: usize = 1_000;
/// Most `Representation` elements accepted in one `AdaptationSet`.
pub const MAX_REPRESENTATIONS_PER_SET: usize = 1_000;
/// Most `S` entries accepted in one `SegmentTimeline`.
pub const MAX_TIMELINE_ENTRIES: usize = 100_000;
/// Most `BaseURL` alternates accepted on one level.
pub const MAX_BASE_URLS_PER_LEVEL: usize = 64;
/// Most `SegmentURL` entries accepted in one `SegmentList`.
pub const MAX_SEGMENT_URLS: usize = 100_000;
/// Most Media Segments one `SegmentTimeline` may describe once `@r` is
/// expanded. The expansion is lazy (run table, never one entry per segment),
/// so this bounds the arithmetic, not the memory.
pub const MAX_TIMELINE_SEGMENTS: u64 = 1_000_000;
/// Most candidate base locations kept when `BaseURL` alternates are combined
/// across levels.
pub const MAX_BASE_CANDIDATES: usize = 64;

/// The bounds applied by [`crate::mpd::Mpd::parse_with`]; `Default` is the
/// `MAX_*` constants above.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// See [`MAX_MPD_BYTES`].
    pub max_bytes: usize,
    /// See [`MAX_XML_DEPTH`].
    pub max_depth: usize,
    /// See [`MAX_XML_ELEMENTS`].
    pub max_elements: usize,
    /// See [`MAX_ATTRIBUTE_BYTES`].
    pub max_attribute_bytes: usize,
    /// See [`MAX_ATTRIBUTES_PER_ELEMENT`].
    pub max_attributes: usize,
    /// See [`MAX_PERIODS`].
    pub max_periods: usize,
    /// See [`MAX_ADAPTATION_SETS_PER_PERIOD`].
    pub max_adaptation_sets: usize,
    /// See [`MAX_REPRESENTATIONS_PER_SET`].
    pub max_representations: usize,
    /// See [`MAX_TIMELINE_ENTRIES`].
    pub max_timeline_entries: usize,
    /// See [`MAX_BASE_URLS_PER_LEVEL`].
    pub max_base_urls: usize,
    /// See [`MAX_SEGMENT_URLS`].
    pub max_segment_urls: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: MAX_MPD_BYTES,
            max_depth: MAX_XML_DEPTH,
            max_elements: MAX_XML_ELEMENTS,
            max_attribute_bytes: MAX_ATTRIBUTE_BYTES,
            max_attributes: MAX_ATTRIBUTES_PER_ELEMENT,
            max_periods: MAX_PERIODS,
            max_adaptation_sets: MAX_ADAPTATION_SETS_PER_PERIOD,
            max_representations: MAX_REPRESENTATIONS_PER_SET,
            max_timeline_entries: MAX_TIMELINE_ENTRIES,
            max_base_urls: MAX_BASE_URLS_PER_LEVEL,
            max_segment_urls: MAX_SEGMENT_URLS,
        }
    }
}
