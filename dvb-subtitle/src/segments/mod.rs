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
