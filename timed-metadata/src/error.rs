//! Crate error type.
use alloc::string::String;

/// Errors produced by conversions and the [`crate::Timeline`] session.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A wall-clock conversion was attempted without a [`crate::TimeAnchor`].
    #[error("wall-clock conversion requires a TimeAnchor, but none was set")]
    MissingAnchor,
    /// An emsg presented to [`crate::convert::emsg_to_scte35`] is not a
    /// SCTE-35 carriage scheme.
    #[error("emsg scheme {scheme:?} is not a SCTE-35 carriage scheme")]
    UnsupportedScheme { scheme: String },
    /// SCTE-35 parse failure.
    #[error("SCTE-35: {0}")]
    Scte35(#[from] scte35_splice::Error),
    /// emsg parse/serialize failure.
    #[error("emsg: {0}")]
    Emsg(#[from] mp4_emsg::Error),
    /// `EXT-X-DATERANGE` tag could not be parsed.
    #[error("DATERANGE parse: {0}")]
    AttrParse(String),
    /// emsg ↔ SegmentTiming `timescale` mismatch.
    #[error("emsg timescale ({emsg}) does not match SegmentTiming timescale ({timing})")]
    EmsgTimescaleMismatch { emsg: u32, timing: u32 },
    /// v0 emsg cannot carry an event starting before the segment EPT.
    #[error("presentation time precedes earliest presentation time; cannot express as v0 delta")]
    EmsgPresentationTimeBeforeEpt,
    /// v0 `presentation_time_delta` overflowed u32.
    #[error("presentation_time_delta value {0} exceeds u32 max")]
    EmsgDeltaOverflow(u64),
}

/// Crate result alias.
pub type Result<T> = core::result::Result<T, Error>;
