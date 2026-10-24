//! Action, event, status and configuration types.

use alloc::string::String;
use alloc::vec::Vec;

use url::Url;

use broadcast_auth::Scheme as AuthScheme;

use transmux::{DisplayInfo, JoinDecision, Sample, TrackId, TrackSpec};

use alloc::sync::Arc;

use crate::ConnId;

/// Source kind for [`Config`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Auto-detect from the URL: a `.m3u8`/`.m3u` path is HLS, anything else TS-over-HTTP.
    Auto,
    /// TS-over-HTTP.
    TsHttp,
    /// HLS (M3U8) on `hls-runtime`.
    Hls,
}

impl SourceKind {
    /// Spec label for this kind.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::TsHttp => "ts-http",
            Self::Hls => "hls",
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

/// Buffer event mapped from `buffering_runtime::Event`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferEvent {
    /// Start playback.
    Start {
        /// Media position 27 MHz.
        at_27mhz: u64,
    },
    /// Pause playback.
    Pause {
        /// Hold position 27 MHz.
        hold_at_27mhz: u64,
        /// Starving track.
        starving_track: Option<u32>,
    },
    /// Resume playback.
    Resume {
        /// Media position 27 MHz.
        at_27mhz: u64,
    },
}

impl BufferEvent {
    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Start { .. } => "start",
            Self::Pause { .. } => "pause",
            Self::Resume { .. } => "resume",
        }
    }
}

broadcast_common::impl_spec_display!(BufferEvent);

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
        /// Join decision.
        join: Option<JoinDecision>,
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
    /// Finite resource expected byte count (VOD).
    Finite {
        /// Expected bytes.
        expected_bytes: u64,
    },
    /// Status.
    Status(Status),
    /// Starved.
    Starved(tshttp_runtime::StarveKind),
    /// Buffer event.
    Buffer(BufferEvent),
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
    /// A master playlist was parsed and a variant chosen.
    Variants(hls_runtime::client::MasterSelection),
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Tracks(a), Self::Tracks(b)) => format!("{a:?}") == format!("{b:?}"),
            (
                Self::Sample {
                    track: ta,
                    sample: sa,
                    join: ja,
                },
                Self::Sample {
                    track: tb,
                    sample: sb,
                    join: jb,
                },
            ) => *ta == *tb && format!("{sa:?}") == format!("{sb:?}") && *ja == *jb,
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
            (Self::Finite { expected_bytes: a }, Self::Finite { expected_bytes: b }) => *a == *b,
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
            (Self::Buffer(a), Self::Buffer(b)) => *a == *b,
            (Self::Variants(a), Self::Variants(b)) => *a == *b,
            _ => false,
        }
    }
}

/// Default maximum accepted HLS playlist body size.
pub const DEFAULT_MAX_PLAYLIST_BYTES: usize = 4 * 1024 * 1024;
/// Default maximum accepted HLS media resource body size.
pub const DEFAULT_MAX_SEGMENT_BYTES: usize = 64 * 1024 * 1024;
/// Default per-request timeout for HLS playlists, microseconds.
pub const DEFAULT_HLS_REQUEST_TIMEOUT_US: u64 = 20_000_000;
/// Default per-request timeout for HLS media resources, microseconds.
pub const DEFAULT_HLS_SEGMENT_TIMEOUT_US: u64 = 60_000_000;
/// Default HLS retry backoff schedule, milliseconds per attempt.
pub const DEFAULT_HLS_BACKOFF_MS: [u64; 3] = [2_000, 4_000, 8_000];
/// Default HLS retry backoff once the schedule is exhausted, milliseconds.
pub const DEFAULT_HLS_BACKOFF_CAP_MS: u64 = 10_000;

/// Session configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Source kind.
    pub kind: SourceKind,
    /// TS-over-HTTP config.
    pub ts_http: tshttp_runtime::Config,
    /// Maximum redirects per open (per request for HLS).
    pub max_redirects: u32,
    /// Extra headers (reserved).
    pub extra_headers: Vec<(String, String)>,
    /// Buffer controller config.
    pub buffer: buffering_runtime::ControllerConfig,
    /// HLS variant selection policy.
    pub variant_policy: hls_runtime::client::VariantPolicy,
    /// HLS: maximum playlist body bytes.
    pub max_playlist_bytes: usize,
    /// HLS: maximum media resource body bytes.
    pub max_segment_bytes: usize,
    /// HLS: playlist request timeout, microseconds.
    pub hls_request_timeout_us: u64,
    /// HLS: media resource request timeout, microseconds.
    pub hls_segment_timeout_us: u64,
    /// HLS: retry backoff per attempt, milliseconds.
    pub hls_backoff_ms: Vec<u64>,
    /// HLS: retry backoff after `hls_backoff_ms` is exhausted, milliseconds.
    pub hls_backoff_cap_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            kind: SourceKind::Auto,
            ts_http: tshttp_runtime::Config::default(),
            max_redirects: 5,
            extra_headers: Vec::new(),
            buffer: buffering_runtime::ControllerConfig::default(),
            variant_policy: hls_runtime::client::VariantPolicy::default(),
            max_playlist_bytes: DEFAULT_MAX_PLAYLIST_BYTES,
            max_segment_bytes: DEFAULT_MAX_SEGMENT_BYTES,
            hls_request_timeout_us: DEFAULT_HLS_REQUEST_TIMEOUT_US,
            hls_segment_timeout_us: DEFAULT_HLS_SEGMENT_TIMEOUT_US,
            hls_backoff_ms: DEFAULT_HLS_BACKOFF_MS.to_vec(),
            hls_backoff_cap_ms: DEFAULT_HLS_BACKOFF_CAP_MS,
        }
    }
}
