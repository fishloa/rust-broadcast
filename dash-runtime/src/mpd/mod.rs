//! BaseURL alternates, URL resolution and attribute inheritance
//! (ISO/IEC 23009-1 5.6, 5.3.9.1).
//!
//! The model, the parser and the SegmentTemplate grammar live in
//! [`broadcast_dash`]; this module keeps what a player adds on top of them.

pub mod base_url;
pub mod urls;

pub use broadcast_dash::descriptors;
pub use broadcast_dash::inherit;
pub use broadcast_dash::model;
pub use broadcast_dash::template;

pub use broadcast_dash::{
    AdaptationSet, BaseInfo, BaseUrl, ByteRange, CommonAttributes, Descriptor, Mpd, MpdError,
    MpdFeatures, Period, PresentationType, Representation, SegmentLayer, SegmentList,
    SegmentTemplate, SegmentTimeline, SegmentUrl, TimelineEntry, UtcTiming,
};
