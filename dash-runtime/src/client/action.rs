//! What the caller must fetch ([`Action`]) and how it names a fetch
//! ([`ResourceId`]).

use broadcast_common::impl_spec_display;

use crate::timing::utc_timing::UtcScheme;

/// Client-assigned identity of one Representation in one Period. Stable for
/// the life of the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepId(pub(crate) u32);

impl RepId {
    /// The numeric value.
    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }
}

/// One fetchable resource; correlates an [`Action`] with the `on_*` call that
/// delivers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ResourceId {
    /// The Initialization Segment of a Representation. Representations whose
    /// resolved init URL and range are identical share one fetch, named by
    /// the first of them.
    Init {
        /// The Representation that asked first.
        rep: RepId,
    },
    /// The `sidx` of a `SegmentBase` Representation.
    Index {
        /// The Representation.
        rep: RepId,
    },
    /// One Media Segment.
    Segment {
        /// The Representation.
        rep: RepId,
        /// Its `$Number$` (the 1-based position for a `sidx`).
        number: u64,
    },
}

impl ResourceId {
    /// The Representation the resource belongs to.
    #[must_use]
    pub fn rep(&self) -> RepId {
        match self {
            Self::Init { rep } | Self::Index { rep } | Self::Segment { rep, .. } => *rep,
        }
    }
}

/// Identity of one `UTCTiming` request; correlates an
/// [`Action::FetchUtcTiming`] with the `on_utc_*` call that answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcId(pub(crate) u32);

impl UtcId {
    /// The numeric value.
    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }
}

/// The HTTP method of a `UTCTiming` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UtcMethod {
    /// `GET`; the body (or, for `http-head` re-issued after a refused `HEAD`,
    /// only the `Date` header) is the time.
    Get,
    /// `HEAD`; the `Date` header is the time.
    Head,
}

impl UtcMethod {
    /// The method token.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
        }
    }
}
impl_spec_display!(UtcMethod);

/// The validators of the MPD the client holds; send them as `If-None-Match`
/// and `If-Modified-Since` so an unchanged MPD answers `304` (IOP 4.4.4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct MpdValidators {
    /// The last `ETag`.
    pub etag: Option<String>,
    /// The last `Last-Modified`.
    pub last_modified: Option<String>,
}

/// One unit of I/O the caller performs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Action {
    /// GET the MPD (conditional when `validators` are set). A `304` is
    /// reported with [`super::DashClient::on_mpd_unchanged`].
    FetchMpd {
        /// The MPD URL: the first `Location`, else the redirect target, else
        /// the original (23009-1 A.11).
        url: String,
        /// Validators of the MPD held now.
        validators: MpdValidators,
    },
    /// Fetch the time from a `UTCTiming` server (23009-1 5.8.5.7). Report with
    /// [`super::DashClient::on_utc_response`] or
    /// [`super::DashClient::on_utc_failure`]. Carry no credentials: the server
    /// is chosen by the MPD.
    FetchUtcTiming {
        /// Correlates the answer.
        id: UtcId,
        /// The scheme the MPD announced.
        scheme: UtcScheme,
        /// Absolute URL.
        url: String,
        /// `GET` or `HEAD`.
        method: UtcMethod,
    },
    /// GET an Initialization Segment.
    FetchInit {
        /// Correlates [`super::DashClient::on_init`].
        id: ResourceId,
        /// Absolute URL.
        url: String,
        /// `(offset, length)` of a byte-range request.
        byte_range: Option<(u64, u64)>,
    },
    /// GET a `sidx` (SegmentBase index).
    FetchIndex {
        /// Correlates [`super::DashClient::on_index`].
        id: ResourceId,
        /// Absolute URL.
        url: String,
        /// `(offset, length)` of a byte-range request.
        byte_range: Option<(u64, u64)>,
    },
    /// GET a Media Segment.
    FetchSegment {
        /// Correlates [`super::DashClient::on_segment`].
        id: ResourceId,
        /// Absolute URL.
        url: String,
        /// `(offset, length)` of a byte-range request.
        byte_range: Option<(u64, u64)>,
    },
}

impl Action {
    /// The resource a fetch action names; `None` for [`Action::FetchMpd`].
    #[must_use]
    pub fn id(&self) -> Option<ResourceId> {
        match self {
            Self::FetchMpd { .. } | Self::FetchUtcTiming { .. } => None,
            Self::FetchInit { id, .. }
            | Self::FetchIndex { id, .. }
            | Self::FetchSegment { id, .. } => Some(*id),
        }
    }

    /// The URL to fetch.
    #[must_use]
    pub fn url(&self) -> &str {
        match self {
            Self::FetchMpd { url, .. }
            | Self::FetchUtcTiming { url, .. }
            | Self::FetchInit { url, .. }
            | Self::FetchIndex { url, .. }
            | Self::FetchSegment { url, .. } => url,
        }
    }

    /// The byte range, if the request is ranged.
    #[must_use]
    pub fn byte_range(&self) -> Option<(u64, u64)> {
        match self {
            Self::FetchMpd { .. } | Self::FetchUtcTiming { .. } => None,
            Self::FetchInit { byte_range, .. }
            | Self::FetchIndex { byte_range, .. }
            | Self::FetchSegment { byte_range, .. } => *byte_range,
        }
    }
}
