//! Representation to Segment plan (ISO/IEC 23009-1 5.3.9.2-5.3.9.6).
//!
//! [`plan::SegmentPlan`] is a lazy, ordered view of a Representation's Media
//! Segments: number, media time, duration, URL reference, byte range and, for
//! dynamic MPDs, availability window. Nothing is materialised per segment
//! except where the MPD itself lists them (`SegmentList`, `sidx`).

use thiserror::Error;

use crate::error::Unsupported;

pub mod base_index;
pub mod list;
pub mod plan;
pub mod representation;
pub mod template;
pub mod timeline;

pub use representation::{PlanError, PlanOptions, PlannedRepresentation, plan_representation};
pub use plan::{DynamicContext, PlanInput, ResourceRef, SegmentPlan, SegmentRef};

/// Why a Representation's segments cannot be addressed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum AddressError {
    /// A template string is invalid (5.3.9.4.4); the Representation is
    /// dropped.
    #[error("invalid template `{template}`: {fault}")]
    InvalidTemplate {
        /// The template text.
        template: String,
        /// What is wrong with it.
        fault: template::TemplateFault,
    },
    /// A feature outside the 0.1 scope.
    #[error("unsupported: {0}")]
    Unsupported(Unsupported),
    /// A template Representation with no `@media`.
    #[error("SegmentTemplate has no @media")]
    MissingMediaTemplate,
    /// A multi-segment list with neither `@duration` nor a timeline.
    #[error("no @duration or SegmentTimeline to time the segments")]
    MissingDuration,
    /// `S@t` smaller than the end of the previous run.
    #[error("S@t {t} is before the end of the previous run ({floor})")]
    TimelineNotIncreasing {
        /// The offending `@t`.
        t: u64,
        /// The previous end.
        floor: u64,
    },
    /// `S@n` smaller than the next expected number.
    #[error("S@n {n} is below the expected next number {expected}")]
    NumberNotIncreasing {
        /// The offending `@n`.
        n: u64,
        /// The expected number.
        expected: u64,
    },
    /// The timeline describes more segments than the bound allows.
    #[error("timeline describes at least {count_hint} segments")]
    TimelineTooLong {
        /// Count reached.
        count_hint: u64,
    },
    /// A `SegmentList` whose URL count and timeline disagree.
    #[error("SegmentList has {urls} SegmentURL entries but its timeline describes {segments}")]
    ListTimelineMismatch {
        /// `SegmentURL` entries.
        urls: usize,
        /// Timeline segments.
        segments: u64,
    },
    /// The index bytes are not a usable `sidx`.
    #[error("sidx: {0}")]
    Sidx(String),
    /// Arithmetic overflow on hostile values.
    #[error("segment arithmetic overflow")]
    Overflow,
    /// A SegmentBase Representation whose `sidx` has not been supplied yet.
    #[error("the Representation index (sidx) is required before its segments can be addressed")]
    IndexRequired,
    /// A `SegmentList` entry with no `@media` and no `@mediaRange`.
    #[error("SegmentURL has neither @media nor @mediaRange")]
    EmptySegmentUrl,
}
