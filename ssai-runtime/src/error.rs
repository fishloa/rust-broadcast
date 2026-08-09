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
}

/// Crate result alias.
pub type Result<T> = core::result::Result<T, Error>;
