//! What the caller must fetch ([`Action`]) and how it names a fetch
//! ([`ResourceId`]).

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

/// One unit of I/O the caller performs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Action {
    /// GET the MPD.
    FetchMpd {
        /// The MPD URL.
        url: String,
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
            Self::FetchMpd { .. } => None,
            Self::FetchInit { id, .. }
            | Self::FetchIndex { id, .. }
            | Self::FetchSegment { id, .. } => Some(*id),
        }
    }

    /// The URL to fetch.
    #[must_use]
    pub fn url(&self) -> &str {
        match self {
            Self::FetchMpd { url }
            | Self::FetchInit { url, .. }
            | Self::FetchIndex { url, .. }
            | Self::FetchSegment { url, .. } => url,
        }
    }

    /// The byte range, if the request is ranged.
    #[must_use]
    pub fn byte_range(&self) -> Option<(u64, u64)> {
        match self {
            Self::FetchMpd { .. } => None,
            Self::FetchInit { byte_range, .. }
            | Self::FetchIndex { byte_range, .. }
            | Self::FetchSegment { byte_range, .. } => *byte_range,
        }
    }
}
