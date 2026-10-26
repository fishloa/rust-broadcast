//! Typed errors for MPD parsing.

use broadcast_common::impl_spec_display;
use thiserror::Error;

/// Which bound of [`crate::limits::Limits`] was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Limit {
    /// Document size in bytes.
    InputBytes,
    /// Element nesting depth.
    Depth,
    /// Total element count.
    Elements,
    /// Length of one attribute value.
    AttributeBytes,
    /// Attributes on one element.
    Attributes,
    /// `Period` count.
    Periods,
    /// `AdaptationSet` count in one Period.
    AdaptationSets,
    /// `Representation` count in one AdaptationSet.
    Representations,
    /// `S` entries in one `SegmentTimeline`.
    TimelineEntries,
    /// `BaseURL` alternates on one level.
    BaseUrls,
    /// `SegmentURL` entries in one `SegmentList`.
    SegmentUrls,
}

impl Limit {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::InputBytes => "input_bytes",
            Self::Depth => "depth",
            Self::Elements => "elements",
            Self::AttributeBytes => "attribute_bytes",
            Self::Attributes => "attributes",
            Self::Periods => "periods",
            Self::AdaptationSets => "adaptation_sets",
            Self::Representations => "representations",
            Self::TimelineEntries => "timeline_entries",
            Self::BaseUrls => "base_urls",
            Self::SegmentUrls => "segment_urls",
        }
    }
}
impl_spec_display!(Limit);

/// A feature the 0.1 core recognises but deliberately does not implement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unsupported {
    /// `MPD@type="list"` (ISO/IEC 23009-1 Ed.6).
    PresentationTypeList,
    /// Segment sequences: `S@k` above 1 or a `$SubNumber$` template.
    SegmentSequence,
    /// A `data:` URL used as a Segment address (5.3.9.1 b).
    DataUrl,
    /// A byte range with no last byte (`a-`).
    OpenEndedRange,
    /// A `sidx` reference that points at another `sidx`.
    HierarchicalSidx,
}

impl Unsupported {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::PresentationTypeList => "presentation_type_list",
            Self::SegmentSequence => "segment_sequence",
            Self::DataUrl => "data_url",
            Self::OpenEndedRange => "open_ended_range",
            Self::HierarchicalSidx => "hierarchical_sidx",
        }
    }
}
impl_spec_display!(Unsupported);

/// Why an MPD document was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum MpdError {
    /// The bytes are not UTF-8.
    #[error("MPD is not valid UTF-8 (valid up to byte {valid_up_to})")]
    InvalidUtf8 {
        /// Length of the valid prefix.
        valid_up_to: usize,
    },
    /// The document is not well-formed XML (or uses a construct this parser
    /// refuses, such as a DOCTYPE).
    #[error("XML error at byte {pos}: {message}")]
    Xml {
        /// Byte offset where the reader stopped.
        pos: usize,
        /// What was wrong.
        message: String,
    },
    /// The root element is not `MPD` in the MPD namespace.
    #[error("document root is not an MPD in the MPD namespace (found `{found}`)")]
    NotMpd {
        /// Local name of the root element.
        found: String,
    },
    /// An attribute or element violates the schema rules the core relies on.
    #[error("invalid MPD: {what}: {detail}")]
    Invalid {
        /// The attribute or rule.
        what: &'static str,
        /// The offending value or an explanation.
        detail: String,
    },
    /// A hostile-input bound was exceeded.
    #[error("MPD exceeds the {limit} limit of {max}")]
    TooLarge {
        /// Which bound.
        limit: Limit,
        /// Its value.
        max: usize,
    },
    /// A feature outside the 0.1 scope.
    #[error("unsupported: {0}")]
    Unsupported(Unsupported),
}
