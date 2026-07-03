//! DVB subtitling (bitmap) segment parser — ETSI EN 300 743 V1.6.1.
//!
//! Parses the subtitling segments carried in a DVB subtitle PES data field:
//! display-definition, page-composition, region-composition, CLUT-definition,
//! object-data (incl. 2/4/8-bit pixel-data sub-blocks), disparity-signalling,
//! alternative-CLUT and end-of-display-set segments. Feed it a reassembled PES
//! payload (e.g. from `mpeg-pes`); it depends only on `dvb-common` and is
//! `#![no_std]` (+ `alloc`).
//!
//! ```
//! use broadcast_common::Parse;
//! use dvb_subtitle::{PesDataField, DataIdentifier, SyncByte, EndOfPesMarker};
//!
//! let bytes = [
//!     DataIdentifier,               // 0x20
//!     0x00,                         // subtitle_stream_id
//!     SyncByte,                     // 0x0F
//!     0x80, 0x00, 0x01, 0x00, 0x00, // end_of_display_set
//!     EndOfPesMarker,               // 0xFF
//! ];
//! let field = PesDataField::parse(&bytes).unwrap();
//! assert_eq!(field.segments.len(), 1);
//! ```
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p dvb-subtitle --example <name>`).\n"]
#![doc = "\n## `parse_segment`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_segment.rs")]
#![doc = "```\n\n## `parse_full_pes`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_full_pes.rs")]
#![doc = "```"]

extern crate alloc;

mod any;
mod error;
mod pes_data_field;
mod segments;
mod traits;

pub use any::AnySegment;
pub use error::{Error, Result};
pub use pes_data_field::PesDataField;
pub use traits::SegmentDef;

/// The `data_identifier` for DVB subtitle streams (0x20).
pub use pes_data_field::DATA_IDENTIFIER as DataIdentifier;
/// The `end_of_PES_data_field_marker` (0xFF).
pub use pes_data_field::END_OF_PES_MARKER as EndOfPesMarker;
/// The `subtitle_stream_id` identifying a DVB subtitle stream (0x00).
pub use pes_data_field::SUBTITLE_STREAM_ID as SubtitleStreamId;
/// The sync_byte prefixing each subtitling segmentation (0x0F).
pub use pes_data_field::SYNC_BYTE as SyncByte;

// Re-export segment types and enums for convenience.
pub use segments::alternative_clut::{
    AlternativeClutEntry, AlternativeClutSegment, ClutParameters, DynamicRangeColourGamut,
    OutputBitDepth,
};
pub use segments::clut_definition::{ClutDefinitionSegment, ClutEntry};
pub use segments::disparity_signalling::{
    DisparityRegion, DisparityShiftInterval, DisparityShiftUpdateSequence,
    DisparitySignallingSegment, Subregion,
};
pub use segments::display_definition::DisplayDefinitionSegment;
pub use segments::end_of_display_set::EndOfDisplaySetSegment;
pub use segments::object_data::{
    DataType, InterlacedPixelsData, ObjectCodingMethod, ObjectDataPayload, ObjectDataSegment,
    PixelDataSubBlock, ProgressivePixelBlock,
};
pub use segments::page_composition::{PageCompositionSegment, PageRegionEntry, PageState};
pub use segments::region_composition::{
    ObjectProviderFlag, ObjectType, RegionCompositionSegment, RegionDepth,
    RegionLevelOfCompatibility, RegionObjectEntry,
};
pub use segments::stuffing::StuffingSegment;

// Implement SegmentDef for each type
impl SegmentDef<'_> for DisplayDefinitionSegment {
    const SEGMENT_TYPE: u8 = segments::display_definition::SEGMENT_TYPE;
    const NAME: &'static str = "DISPLAY_DEFINITION";
}

impl SegmentDef<'_> for PageCompositionSegment {
    const SEGMENT_TYPE: u8 = segments::page_composition::SEGMENT_TYPE;
    const NAME: &'static str = "PAGE_COMPOSITION";
}

impl SegmentDef<'_> for RegionCompositionSegment {
    const SEGMENT_TYPE: u8 = segments::region_composition::SEGMENT_TYPE;
    const NAME: &'static str = "REGION_COMPOSITION";
}

impl SegmentDef<'_> for ClutDefinitionSegment {
    const SEGMENT_TYPE: u8 = segments::clut_definition::SEGMENT_TYPE;
    const NAME: &'static str = "CLUT_DEFINITION";
}

impl<'a> SegmentDef<'a> for ObjectDataSegment<'a> {
    const SEGMENT_TYPE: u8 = segments::object_data::SEGMENT_TYPE;
    const NAME: &'static str = "OBJECT_DATA";
}

impl SegmentDef<'_> for EndOfDisplaySetSegment {
    const SEGMENT_TYPE: u8 = segments::end_of_display_set::SEGMENT_TYPE;
    const NAME: &'static str = "END_OF_DISPLAY_SET";
}

impl SegmentDef<'_> for DisparitySignallingSegment {
    const SEGMENT_TYPE: u8 = segments::disparity_signalling::SEGMENT_TYPE;
    const NAME: &'static str = "DISPARITY_SIGNALLING";
}

impl SegmentDef<'_> for AlternativeClutSegment {
    const SEGMENT_TYPE: u8 = segments::alternative_clut::SEGMENT_TYPE;
    const NAME: &'static str = "ALTERNATIVE_CLUT";
}

impl<'a> SegmentDef<'a> for StuffingSegment<'a> {
    const SEGMENT_TYPE: u8 = segments::stuffing::SEGMENT_TYPE;
    const NAME: &'static str = "STUFFING";
}
