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

impl From<broadcast_common::xml_chars::XmlLimit> for Limit {
    fn from(limit: broadcast_common::xml_chars::XmlLimit) -> Self {
        use broadcast_common::xml_chars::XmlLimit;
        match limit {
            XmlLimit::Depth => Self::Depth,
            XmlLimit::Elements => Self::Elements,
            XmlLimit::AttributeBytes => Self::AttributeBytes,
            XmlLimit::Attributes => Self::Attributes,
            _ => Self::InputBytes,
        }
    }
}

impl From<broadcast_common::xml_chars::XmlCharError> for MpdError {
    /// A character-data or namespace failure, at the position the caller
    /// that raised it reports (the pull reader's own offset).
    fn from(e: broadcast_common::xml_chars::XmlCharError) -> Self {
        Self::Xml {
            pos: 0,
            message: e.to_string(),
        }
    }
}

impl From<broadcast_common::xml::XmlError> for MpdError {
    fn from(e: broadcast_common::xml::XmlError) -> Self {
        use broadcast_common::xml::XmlError;
        match e {
            XmlError::InvalidUtf8 { valid_up_to } => Self::InvalidUtf8 { valid_up_to },
            XmlError::NotWellFormed { position, message } => Self::Xml {
                pos: position,
                message,
            },
            XmlError::WrongRoot { found } => Self::NotMpd { found },
            XmlError::Limit(exceeded) => Self::TooLarge {
                limit: exceeded.limit.into(),
                max: exceeded.max,
            },
            _ => Self::Invalid {
                what: "document",
                detail: "unrecognised XML reader failure".to_string(),
            },
        }
    }
}

/// A feature outside the read-only core's scope.
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
    /// An `Event` whose content is an XML payload rather than text.
    EventXmlPayload,
    /// A `Preselection` child the model does not carry: `Switching`,
    /// `RandomAccess`, `ContentPopularityRate` or `SegmentSequenceProperties`.
    RepresentationBaseChild,
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
            Self::EventXmlPayload => "event_xml_payload",
            Self::RepresentationBaseChild => "representation_base_child",
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
    /// Serialising the model to XML failed.
    #[error("MPD write failed: {message}")]
    Write {
        /// What went wrong.
        message: String,
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
