//! Crate error type.

use alloc::string::String;

/// Errors produced by session, decision, splice-conditioning, and playlist
/// rendering operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// [`crate::splice::condition_splice_point`] found no candidate boundary
    /// within the caller's tolerance.
    #[error(
        "no candidate splice boundary within {tolerance_ticks} ticks of requested pts \
         {requested_pts} (nearest delta {nearest_delta_ticks} ticks)"
    )]
    NoAlignedBoundary {
        /// The cue's nominal target.
        requested_pts: u64,
        /// The caller's maximum acceptable drift.
        tolerance_ticks: u64,
        /// The delta of the nearest candidate actually found.
        nearest_delta_ticks: u64,
    },
    /// [`crate::splice::condition_splice_point`] was called with an empty
    /// candidate slice.
    #[error("no candidate splice boundaries supplied")]
    NoCandidates,
    /// A PTS value (or the modulus itself) passed to
    /// [`crate::splice::condition_splice_point_wrapping`] was not below a
    /// non-zero modulus (issue #1125 / audit r14-SSAI-W2).
    #[error("pts {pts} is not below the clock modulus {modulus} (or the modulus is zero)")]
    PtsOutOfRange {
        /// The offending value.
        pts: u64,
        /// The wrap modulus supplied.
        modulus: u64,
    },
    /// The base playlist carries no `#EXT-X-PROGRAM-DATE-TIME` tag, which
    /// RFC 8216bis §4.4.5.1 requires of any playlist containing an
    /// `EXT-X-DATERANGE` (issue #1125 / audit r14-SSAI-W3).
    #[error("base playlist has no EXT-X-PROGRAM-DATE-TIME; an EXT-X-DATERANGE requires one")]
    MissingProgramDateTime,
    /// An interstitial asset source was neither `X-ASSET-URI` nor
    /// `X-ASSET-LIST` — Appendix D §D.2 requires exactly one.
    #[error("interstitial asset source must be exactly one of X-ASSET-URI or X-ASSET-LIST")]
    InvalidAssetSource,
    /// [`crate::session::SessionStore`] has no record for the session.
    #[error("session {0:?} has no active ad break")]
    NoActiveBreak(String),
    /// [`crate::session::SessionStore::begin_break`] was called for a
    /// session that already has an unresumed break.
    #[error("session {0:?} already has an active ad break")]
    BreakAlreadyActive(String),
    /// An Interstitial `EXT-X-DATERANGE` tag line failed to parse.
    #[error("interstitial DATERANGE parse: {0}")]
    TagParse(String),
    /// An attribute value supplied by the [`crate::decision::AdDecisionProvider`]
    /// (a `URI`, an `ID`, or any other ad-server-controlled string) could not
    /// be represented as an RFC 8216 §4.2 quoted-string — contains `"`, CR or
    /// LF (issue #1140 / audit r14-SSAI-W1): rendering an ad-decision
    /// response straight into the playlist without validation let a `"` or
    /// a newline in, e.g., an ad URI terminate the attribute list and inject
    /// arbitrary tag lines into every viewer's session playlist.
    #[error(transparent)]
    HlsAttrValue(#[from] broadcast_hls::Error),
    /// A `DURATION`/`X-RESUME-OFFSET`/`X-PLAYOUT-LIMIT` value was NaN,
    /// infinite, or negative (issue #1140 / audit r14-SSAI-W4). RFC 8216
    /// decimal-floating-point is non-negative and finite; `NaN as i64 == 0`
    /// let a NaN duration render as the bare token `NaN`.
    #[error("{what} must be a finite, non-negative number of seconds, got {value}")]
    InvalidDuration {
        /// Which attribute failed (`"DURATION"`, `"X-RESUME-OFFSET"`, …).
        what: &'static str,
        /// The offending value.
        value: f64,
    },
}

/// Crate result alias.
pub type Result<T> = core::result::Result<T, Error>;
