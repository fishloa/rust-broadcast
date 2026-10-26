//! What the caller reports about a response beyond its body: the instants it
//! measured on its own monotonic clock and the headers the client reads.

/// An MPD response as the caller observed it.
///
/// `sent_us` and `received_us` are the caller's monotonic microseconds when
/// the request left and when the first response byte arrived; the client
/// places the server's `FetchTime` at their midpoint (23009-1 A.2). Every
/// header is optional: a client with none of them still plays, it just
/// refreshes unconditionally and has no `Date` clock fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResponseMeta<'a> {
    /// Monotonic microseconds when the request was sent.
    pub sent_us: u64,
    /// Monotonic microseconds when the first response byte arrived.
    pub received_us: u64,
    /// The `Date` header, verbatim.
    pub date: Option<&'a str>,
    /// The `ETag` header, verbatim.
    pub etag: Option<&'a str>,
    /// The `Last-Modified` header, verbatim.
    pub last_modified: Option<&'a str>,
    /// The URL the response came from after redirects, when it differs from
    /// the URL requested.
    pub final_url: Option<&'a str>,
}

impl<'a> ResponseMeta<'a> {
    /// A response exchanged between `sent_us` and `received_us`, no headers.
    #[must_use]
    pub fn new(sent_us: u64, received_us: u64) -> Self {
        Self {
            sent_us,
            received_us,
            date: None,
            etag: None,
            last_modified: None,
            final_url: None,
        }
    }

    /// A response with no round trip of its own: sent and received at `now_us`.
    #[must_use]
    pub fn at(now_us: u64) -> Self {
        Self::new(now_us, now_us)
    }

    /// Set the `Date` header.
    #[must_use]
    pub fn with_date(mut self, date: &'a str) -> Self {
        self.date = Some(date);
        self
    }

    /// Set the `ETag` header.
    #[must_use]
    pub fn with_etag(mut self, etag: &'a str) -> Self {
        self.etag = Some(etag);
        self
    }

    /// Set the `Last-Modified` header.
    #[must_use]
    pub fn with_last_modified(mut self, last_modified: &'a str) -> Self {
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
    /// Monotonic microseconds when the request was sent.
    pub sent_us: u64,
    /// Monotonic microseconds when the first response byte arrived.
    pub received_us: u64,
    /// The `Date` header (the time of an `http-head` exchange).
    pub date: Option<&'a str>,
    /// The body, empty for a `HEAD`.
    pub body: &'a [u8],
}

impl<'a> UtcResponse<'a> {
    /// A response with a body and no `Date`.
    #[must_use]
    pub fn new(sent_us: u64, received_us: u64, body: &'a [u8]) -> Self {
        Self {
            sent_us,
            received_us,
            date: None,
            body,
        }
    }

    /// Set the `Date` header.
    #[must_use]
    pub fn with_date(mut self, date: &'a str) -> Self {
        self.date = Some(date);
        self
    }
}
