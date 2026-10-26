//! Failure vocabulary: what went wrong with a fetch ([`Failure`]), how the
//! core classifies it ([`FailureClass`]) and the errors the `on_*` calls
//! return ([`Error`]).

use broadcast_common::impl_spec_display;
use thiserror::Error;

use super::action::{ResourceId, UtcId};
use crate::error::MpdError;

/// How a fetch failed, as the caller observed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Failure {
    /// The server answered with this HTTP status (an error status, or a
    /// success status the caller could not use).
    Status(u16),
    /// Connection refused, reset, or closed before the response completed.
    Transport,
    /// No (complete) response within the caller's deadline.
    Timeout,
    /// The body exceeded the caller's size cap.
    TooLarge,
}

/// The cause the core attributes to a counted failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailureKind {
    /// 404 or 410: the resource is not there.
    Missing,
    /// 425 (RFC 8470 Too Early).
    TooEarly,
    /// Any other error status.
    Server,
    /// Transport error or timeout.
    Transport,
    /// A body arrived that is not usable (not media, wrong length, rejected
    /// by the demuxer, oversize).
    Invalid,
}

impl FailureKind {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::TooEarly => "too_early",
            Self::Server => "server",
            Self::Transport => "transport",
            Self::Invalid => "invalid",
        }
    }
}
impl_spec_display!(FailureKind);

/// What the caller does after a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailureClass {
    /// Back off (the adapter owns the arithmetic, `Retry-After` included) and
    /// call [`super::DashClient::retry`]. `attempt` is the 1-based count of
    /// consecutive failures of this resource.
    Retry {
        /// Consecutive failures including this one.
        attempt: u32,
        /// Why.
        kind: FailureKind,
    },
    /// The failure count reached the configured cap: the presentation has
    /// ended with an [`super::Output::Fatal`].
    GiveUp,
    /// A live segment requested before it exists (a `425`, or a `404`/`410`
    /// inside the early-grace window): not counted. The client asks for it
    /// again from [`super::DashClient::poll`] at `retry_at_us`; the caller
    /// does nothing.
    NotYetAvailable {
        /// Monotonic microseconds at which the segment is requested again.
        retry_at_us: u64,
    },
    /// A live segment that left the time-shift window before it was fetched:
    /// not counted and not retried. The client continues at the oldest
    /// segment still available and marks the gap with a `Discontinuity`.
    Expired,
    /// A live segment given up on after the configured number of consecutive
    /// failures: [`super::Output::SegmentLost`] and a `Discontinuity` follow.
    SegmentSkipped,
    /// The resource is the missing tail of a Period (DESIGN section 5.5 and
    /// the multi-period decision in the crate README): nothing to retry,
    /// nothing counted, playback continues.
    EndTail,
}

/// Why a delivered body was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Rejection {
    /// Not a media fragment (`moof` then `mdat`): an error page, a truncated
    /// transfer or garbage.
    NotMedia,
    /// An init without a complete `moov`.
    InitUndecodable,
    /// The demuxer refused the init and segment.
    DemuxFailed,
    /// The `sidx` bytes are not a usable index.
    BadIndex,
    /// A ranged response whose length differs from the requested range.
    WrongLength,
    /// Larger than `DashConfig::max_segment_bytes`.
    TooLarge,
    /// A progressive MP4 served as a media segment.
    NonFragmentedMp4,
}

impl Rejection {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotMedia => "not_media",
            Self::InitUndecodable => "init_undecodable",
            Self::DemuxFailed => "demux_failed",
            Self::BadIndex => "bad_index",
            Self::WrongLength => "wrong_length",
            Self::TooLarge => "too_large",
            Self::NonFragmentedMp4 => "non_fragmented_mp4",
        }
    }
}
impl_spec_display!(Rejection);

/// Why the presentation cannot continue.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FatalKind {
    /// A dynamic MPD and no UTC source at all: no `UTCTiming` could be read,
    /// the host never supplied a wall clock and the MPD response carried no
    /// usable `Date`. Segment availability cannot be decided (DESIGN 3.4).
    ClockUnavailable,
    /// The MPD has nothing a client can play.
    MpdInvalid {
        /// What is missing.
        reason: &'static str,
    },
    /// A URL derived from the MPD was refused.
    UnsupportedAddressing {
        /// Which rule (`cross_origin`, `unusable_url`).
        reason: &'static str,
    },
    /// A segment failed the configured number of times in a row.
    SegmentFetchFailed {
        /// The segment.
        id: ResourceId,
        /// Status of the last failure, when it was an HTTP status.
        status: Option<u16>,
    },
    /// An init or index failed the configured number of times in a row.
    InitFetchFailed {
        /// The resource.
        id: ResourceId,
        /// Status of the last failure, when it was an HTTP status.
        status: Option<u16>,
    },
    /// A progressive MP4 was served as a media segment of a fragmented
    /// presentation.
    NonFragmentedMp4 {
        /// The segment.
        id: ResourceId,
    },
}

impl FatalKind {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ClockUnavailable => "clock_unavailable",
            Self::MpdInvalid { .. } => "mpd_invalid",
            Self::UnsupportedAddressing { .. } => "unsupported_addressing",
            Self::SegmentFetchFailed { .. } => "segment_fetch_failed",
            Self::InitFetchFailed { .. } => "init_fetch_failed",
            Self::NonFragmentedMp4 { .. } => "non_fragmented_mp4",
        }
    }
}
impl_spec_display!(FatalKind);

/// Errors returned by the client's `on_*` and `select_*` calls.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum Error {
    /// A resource was delivered for an id the client did not request, or one
    /// it no longer waits for.
    #[error("resource {id:?} was not requested")]
    UnrequestedResource {
        /// The id.
        id: ResourceId,
    },
    /// A second delivery of a resource already accepted.
    #[error("resource {id:?} was delivered twice")]
    DuplicateResource {
        /// The id.
        id: ResourceId,
    },
    /// The MPD was not requested, or already accepted.
    #[error("no MPD is awaited")]
    MpdNotAwaited,
    /// The MPD bytes were rejected; request it again with
    /// [`super::DashClient::retry_mpd`].
    #[error("MPD rejected: {0}")]
    Mpd(MpdError),
    /// A `UTCTiming` answer for a request the client does not wait for
    /// (answered twice, abandoned after its timeout, or never made).
    #[error("UTC request {id:?} was not requested")]
    UnrequestedUtc {
        /// The id.
        id: UtcId,
    },
    /// A body was rejected and counted as a failure; `class` says what to do.
    #[error("resource {id:?} rejected ({reason}): {class:?}")]
    Rejected {
        /// The resource.
        id: ResourceId,
        /// Why.
        reason: Rejection,
        /// What the caller does next.
        class: FailureClass,
    },
    /// The presentation has ended or failed; everything is ignored.
    #[error("the client has stopped")]
    Stopped,
    /// [`super::DashClient::select_representation`] with an index outside the
    /// current variant list.
    #[error("no such variant")]
    NoSuchVariant,
    /// A switch is only possible while the Period being played is the one
    /// being fetched.
    #[error("the Representation cannot be switched now")]
    NotSwitchable,
}

/// Result alias.
pub type Result<T> = core::result::Result<T, Error>;
