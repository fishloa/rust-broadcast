//! Action, event, status and configuration types.

use alloc::string::String;
use alloc::vec::Vec;

use url::Url;

use broadcast_auth::Scheme as AuthScheme;

use transmux::{DisplayInfo, Sample, TrackId, TrackSpec};

use alloc::sync::Arc;

use crate::ConnId;

/// Source kind for [`Config`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Auto-detect from the URL (BC-1: `http(s)` -> TS-over-HTTP).
    Auto,
    /// TS-over-HTTP.
    TsHttp,
}

impl SourceKind {
    /// Spec label for this kind.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::TsHttp => "ts-http",
        }
    }
}

broadcast_common::impl_spec_display!(SourceKind);

/// Status values carried by [`Event::Status`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Connection being established.
    Establishing,
    /// Live playback.
    Live,
    /// Stalled for `secs` seconds.
    Stalled {
        /// Stalled seconds.
        secs: u64,
    },
    /// Reconnecting, attempt number.
    Reconnecting {
        /// Attempt number.
        attempt: u64,
    },
    /// Not found.
    NotFound,
    /// Waiting for credentials.
    WaitingForCredentials,
    /// Ended.
    Ended,
}

impl Status {
    /// Spec label for this status.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Establishing => "establishing",
            Self::Live => "live",
            Self::Stalled { .. } => "stalled",
            Self::Reconnecting { .. } => "reconnecting",
            Self::NotFound => "not-found",
            Self::WaitingForCredentials => "waiting-for-credentials",
            Self::Ended => "ended",
        }
    }
}

broadcast_common::impl_spec_display!(Status);

/// Host action produced by [`crate::Session`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// HTTP request to issue.
    HttpRequest {
        /// Connection id.
        conn: ConnId,
        /// HTTP method.
        method: String,
        /// URL to request.
        url: String,
        /// Headers to send.
        headers: Vec<(String, String)>,
        /// Optional byte range.
        byte_range: Option<(u64, u64)>,
    },
    /// Drop a connection.
    Close {
        /// Connection id.
        conn: ConnId,
    },
    /// Wake the session at the given time.
    WakeAt(u64),
}

/// Session event.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum Event {
    /// Full track set after a change.
    Tracks(Vec<TrackSpec>),
    /// A sample.
    Sample {
        /// Track id.
        track: TrackId,
        /// Sample.
        sample: Sample,
    },
    /// Display changed.
    DisplayChanged {
        /// Track id.
        track: TrackId,
        /// PTS.
        pts: Option<i64>,
        /// Display info.
        display: Arc<DisplayInfo>,
    },
    /// Discontinuity.
    Discontinuity,
    /// Status.
    Status(Status),
    /// Starved.
    Starved(tshttp_runtime::StarveKind),
    /// Auth required.
    AuthRequired {
        /// Scheme.
        scheme: AuthScheme,
        /// Realm.
        realm: Option<String>,
        /// Origin.
        origin: Url,
        /// Resource metadata.
        resource_metadata: Option<Url>,
    },
    /// Auth rejected.
    AuthRejected {
        /// HTTP status.
        status: u16,
    },
    /// Forbidden.
    Forbidden {
        /// HTTP status.
        status: u16,
    },
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Tracks(a), Self::Tracks(b)) => format!("{a:?}") == format!("{b:?}"),
            (
                Self::Sample {
                    track: ta,
                    sample: sa,
                },
                Self::Sample {
                    track: tb,
                    sample: sb,
                },
            ) => *ta == *tb && format!("{sa:?}") == format!("{sb:?}"),
            (
                Self::DisplayChanged {
                    track: ta,
                    pts: pa,
                    display: da,
                },
                Self::DisplayChanged {
                    track: tb,
                    pts: pb,
                    display: db,
                },
            ) => *ta == *tb && *pa == *pb && *da == *db,
            (Self::Discontinuity, Self::Discontinuity) => true,
            (Self::Status(a), Self::Status(b)) => *a == *b,
            (Self::Starved(a), Self::Starved(b)) => *a == *b,
            (
                Self::AuthRequired {
                    scheme: sa,
                    realm: ra,
                    origin: oa,
                    resource_metadata: ma,
                },
                Self::AuthRequired {
                    scheme: sb,
                    realm: rb,
                    origin: ob,
                    resource_metadata: mb,
                },
            ) => *sa == *sb && *ra == *rb && *oa == *ob && *ma == *mb,
            (Self::AuthRejected { status: a }, Self::AuthRejected { status: b }) => *a == *b,
            (Self::Forbidden { status: a }, Self::Forbidden { status: b }) => *a == *b,
            _ => false,
        }
    }
}

/// Session configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Source kind.
    pub kind: SourceKind,
    /// TS-over-HTTP config.
    pub ts_http: tshttp_runtime::Config,
    /// Maximum redirects per open.
    pub max_redirects: u32,
    /// Extra headers (reserved).
    pub extra_headers: Vec<(String, String)>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            kind: SourceKind::Auto,
            ts_http: tshttp_runtime::Config::default(),
            max_redirects: 5,
            extra_headers: Vec::new(),
        }
    }
}
