//! Pure conversion functions (the foundation layer).
mod emsg;
pub use emsg::{emsg_to_scte35, scte35_to_emsg, EmsgConfig, SCTE35_SCHEME};

mod daterange;
pub use daterange::scte35_to_daterange;
