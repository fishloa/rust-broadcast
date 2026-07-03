//! Pure conversion functions (the foundation layer).
mod emsg;
pub use emsg::{EmsgConfig, SCTE35_SCHEME, emsg_to_scte35, scte35_to_emsg};

mod emsg_convert;
pub use emsg_convert::{SegmentTiming, emsg_to_v0, emsg_to_v1};

mod daterange;
pub use daterange::scte35_to_daterange;
