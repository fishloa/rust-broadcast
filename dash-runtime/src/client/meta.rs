//! What the caller reports about a response beyond its body: the instants it
//! measured on its own monotonic clock and the headers the client reads.

use broadcast_common::stage::Timestamp;
use headers::{Date, ETag, LastModified};

/// An MPD response as the caller observed it.
///
/// `sent` and `received` are the caller's monotonic instants when the
/// request left and when the first response byte arrived; the client
/// places the server's `FetchTime` at their midpoint (23009-1 A.2). Every
/// header is optional: a client with none of them still plays, it just
/// refreshes unconditionally and has no `Date` clock fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResponseMeta<'a> {
    /// Monotonic instant the request was sent.
    pub sent: Timestamp,
    /// Monotonic instant the first response byte arrived.
    pub received: Timestamp,
    /// The `Date` header.
    pub date: Option<Date>,
    /// The `ETag` header.
    pub etag: Option<ETag>,
    /// The `Last-Modified` header.
    pub last_modified: Option<LastModified>,
    /// The URL the response came from after redirects, when it differs from
    /// the URL requested.
    pub final_url: Option<&'a str>,
}

impl<'a> ResponseMeta<'a> {
    /// A response exchanged between `sent` and `received`, no headers.
    #[must_use]
    pub fn new(sent: Timestamp, received: Timestamp) -> Self {
        Self {
            sent,
            received,
            date: None,
            etag: None,
            last_modified: None,
            final_url: None,
        }
    }

    /// A response with no round trip of its own: sent and received at `now`.
    #[must_use]
    pub fn at(now: Timestamp) -> Self {
        Self::new(now, now)
    }

    /// Set the `Date` header.
    #[must_use]
    pub fn with_date(mut self, date: Date) -> Self {
        self.date = Some(date);
        self
    }

    /// Set the `ETag` header.
    #[must_use]
    pub fn with_etag(mut self, etag: ETag) -> Self {
        self.etag = Some(etag);
        self
    }

    /// Set the `Last-Modified` header.
    #[must_use]
    pub fn with_last_modified(mut self, last_modified: LastModified) -> Self {
        self.last_modified = Some(last_modified);
        self
    }

    /// Set the post-redirect URL.
    #[must_use]
    pub fn with_final_url(mut self, final_url: &'a str) -> Self {
        self.final_url = Some(final_url);
        self
    }
}

/// A `UTCTiming` response as the caller observed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct UtcResponse<'a> {
    /// Monotonic instant the request was sent.
    pub sent: Timestamp,
    /// Monotonic instant the first response byte arrived.
    pub received: Timestamp,
    /// The `Date` header (the time of an `http-head` exchange).
    pub date: Option<Date>,
    /// The body, empty for a `HEAD`.
    pub body: &'a [u8],
}

impl<'a> UtcResponse<'a> {
    /// A response with a body and no `Date`.
    #[must_use]
    pub fn new(sent: Timestamp, received: Timestamp, body: &'a [u8]) -> Self {
        Self {
            sent,
            received,
            date: None,
            body,
        }
    }

    /// Set the `Date` header.
    #[must_use]
    pub fn with_date(mut self, date: Date) -> Self {
        self.date = Some(date);
        self
    }
}
