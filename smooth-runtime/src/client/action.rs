//! What the engine asks its host to fetch, and how a host reports a failed fetch.

use url::Url;

/// Identifies one outstanding fetch; echoed back in `on_*` calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(pub u64);

/// Identifies a selected stream by its position in the session's stream order,
/// stable for the session. Track `n + 1` carries stream `n`'s samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamId(pub usize);

/// One GET the host must perform.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Action {
    /// GET the client manifest (MS-SSTR §2.2.2).
    FetchManifest {
        /// The id to echo back.
        id: RequestId,
        /// Absolute manifest URL.
        url: Url,
    },
    /// GET one fragment (MS-SSTR §2.2.4).
    FetchFragment {
        /// The id to echo back.
        id: RequestId,
        /// The stream the fragment belongs to.
        stream: StreamId,
        /// Fragment start time, in the stream's timescale.
        start: u64,
        /// Fragment duration, in the stream's timescale.
        duration: u64,
        /// Absolute fragment URL.
        url: Url,
    },
}

/// A fetch that produced no usable body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FetchFailure {
    /// The HTTP status when the server answered with a non-success one; `None`
    /// for a transport error or a timeout.
    pub status: Option<http::StatusCode>,
    /// The wait the server asked for with `Retry-After` (RFC 9110 §10.2.3), in
    /// microseconds; `0` when it asked for none. The retry waits at least this
    /// long, never less than the configured backoff.
    pub retry_after_us: u64,
}

impl FetchFailure {
    /// The HTTP statuses that say a resource is not (yet) there.
    pub(crate) fn is_not_found(self) -> bool {
        matches!(
            self.status,
            Some(http::StatusCode::NOT_FOUND | http::StatusCode::GONE)
        )
    }
}
