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
    /// A SCTE-35-sourced event has no `id` (e.g. a `time_signal` cue with no
    /// `segmentation_descriptor`), so it cannot be given a `DATERANGE` `ID`
    /// (RFC 8216bis §4.4.5.1 requires unique IDs; emitting `ID=""` would
    /// silently collide across cues).
    #[error("SCTE-35 event has no id; cannot produce a DATERANGE ID")]
    MissingEventId,
    /// emsg ↔ SegmentTiming `timescale` mismatch.
    #[error("emsg timescale ({emsg}) does not match SegmentTiming timescale ({timing})")]
    EmsgTimescaleMismatch { emsg: u32, timing: u32 },
    /// v0 emsg cannot carry an event starting before the segment EPT.
    #[error("presentation time precedes earliest presentation time; cannot express as v0 delta")]
    EmsgPresentationTimeBeforeEpt,
    /// v0 `presentation_time_delta` overflowed u32.
    #[error("presentation_time_delta value {0} exceeds u32 max")]
    EmsgDeltaOverflow(u64),
    /// A v0 emsg's `earliest_presentation_time + presentation_time_delta`
    /// overflowed `u64` (ISO/IEC 23009-1 §5.10.3.3); a wrapped value would
    /// silently place the event near time zero.
    #[error("earliest presentation time + presentation_time_delta overflows u64")]
    EmsgPresentationTimeOverflow,
    /// The emsg carries a `PresentationTime` variant this crate does not
    /// know how to convert (the enum is `#[non_exhaustive]` upstream).
    #[error("unsupported emsg PresentationTime variant")]
    UnsupportedPresentationTime,
    /// A `DATERANGE` attribute value (`ID`, `CLASS`, or a `SCTE35-*` hex
    /// token) could not be represented as an RFC 8216 §4.2 quoted-string —
    /// contains `"`, CR or LF (issue #1140 / audit r12-TM-W4): these values
    /// are frequently sourced from an upstream SCTE-35 segmentation
    /// descriptor's `segmentation_upid`, which is caller/network data, not
    /// this crate's own.
    #[error(transparent)]
    HlsAttrValue(#[from] broadcast_hls::Error),
    /// `DURATION`/`PLANNED-DURATION` was NaN, infinite, or negative (issue
    /// #1140 / audit r13-BH-W4-class). RFC 8216 decimal-floating-point is
    /// non-negative and finite.
    #[error("{what} must be a finite, non-negative number of seconds, got {value}")]
    InvalidDuration {
        /// Which attribute failed (`"DURATION"` or `"PLANNED-DURATION"`).
        what: &'static str,
        /// The offending value.
        value: f64,
    },
}

/// Crate result alias.
pub type Result<T> = core::result::Result<T, Error>;
