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
    /// Auto-detect from the URL: an `rtsp://` or `rtsps://` URL is RTSP, a `.m3u8`/`.m3u` path
    /// is HLS, anything else TS-over-HTTP.
    Auto,
    /// TS-over-HTTP.
    TsHttp,
    /// HLS (M3U8) on `hls-runtime`.
    Hls,
    /// RTSP 1.0 with RTP interleaved over the RTSP TCP connection (`rtsp://` and `rtsps://`).
    Rtsp,
}

impl SourceKind {
    /// Spec label for this kind.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::TsHttp => "ts-http",
            Self::Hls => "hls",
            Self::Rtsp => "rtsp",
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

/// Why a session reported [`Event::Error`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// The stream is encrypted in a way this client does not decrypt: SAMPLE-AES, another
    /// non-AES-128 method, a non-`identity` `KEYFORMAT` (FairPlay, Widevine, ...), an encrypted
    /// `EXT-X-MAP`, encrypted I-frames-only segments, or an `EXT-X-KEY` without a `URI`. `method`
    /// is the `METHOD` value, or the `KEYFORMAT` value for an unsupported key format.
    UnsupportedEncryption {
        /// The `METHOD` (or unsupported `KEYFORMAT`) value.
        method: String,
    },
    /// A key request failed with a non-authentication status, a transport error, a timeout, or
    /// an oversized body. 401/407/403 on a key request surface as `AuthRequired`/`AuthRejected`/
    /// `Forbidden` instead.
    KeyFetchFailed {
        /// The HTTP status, `None` for a transport error, timeout or oversized body.
        status: Option<u16>,
    },
    /// The key response was not a valid 16-byte AES-128 key.
    BadKey,
    /// A segment did not decrypt (bad padding or length), typically because the key is wrong.
    DecryptFailed,
    /// The stream uses a codec the client cannot handle. Defined for players; not emitted by the
    /// BC-3 sources yet.
    UnsupportedCodec {
        /// The codec's four-character code.
        fourcc: String,
    },
}

impl ErrorKind {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::UnsupportedEncryption { .. } => "unsupported_encryption",
            Self::KeyFetchFailed { .. } => "key_fetch_failed",
            Self::BadKey => "bad_key",
            Self::DecryptFailed => "decrypt_failed",
            Self::UnsupportedCodec { .. } => "unsupported_codec",
        }
    }
}

broadcast_common::impl_spec_display!(ErrorKind);

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
    /// Open a TCP connection (RTSP). The host reports the outcome with
    /// [`crate::Session::on_connected`], or [`crate::Session::on_closed`] with an error.
    TcpConnect {
        /// Connection id.
        conn: ConnId,
        /// Host name or IP literal (IPv6 without brackets).
        host: String,
        /// TCP port.
        port: u16,
        /// The host performs TLS and certificate validation before reporting the connection
        /// established; the session only ever exchanges plaintext bytes.
        tls: bool,
    },
    /// Write bytes to an open TCP connection (RTSP).
    Send {
        /// Connection id.
        conn: ConnId,
        /// Bytes to write, in order.
        bytes: Vec<u8>,
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
    /// The active separate-audio selection changed. `index` is the position in the chosen
    /// variant's audio renditions (see [`crate::Session::select_audio_rendition`]); `None` means
    /// no rendition is active. A rendition carried inside the variant itself (no `URI`) is
    /// reported as active without any separate audio being fetched.
    AudioRendition {
        /// Index of the active rendition, `None` when none is active.
        index: Option<usize>,
        /// `NAME` of the active rendition.
        name: Option<String>,
        /// `LANGUAGE` of the active rendition.
        language: Option<String>,
    },
    /// A condition the player should tell the user about. `fatal` means the session will not
    /// recover by itself: it has closed its connections and goes silent, so the player shows a
    /// message and does not retry. Non-fatal errors are emitted once per kind until a success
    /// (decoded samples) resets them, while the session keeps retrying with backoff; after
    /// [`Config::max_key_failures`] consecutive key or decrypt failures one fatal error of the
    /// same kind follows.
    Error {
        /// What went wrong.
        kind: ErrorKind,
        /// Whether the session gave up.
        fatal: bool,
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
            (
                Self::AudioRendition {
                    index: ia,
                    name: na,
                    language: la,
                },
                Self::AudioRendition {
                    index: ib,
                    name: nb,
                    language: lb,
                },
            ) => *ia == *ib && *na == *nb && *la == *lb,
            (
                Self::Error {
                    kind: ka,
                    fatal: fa,
                },
                Self::Error {
                    kind: kb,
                    fatal: fb,
                },
            ) => *ka == *kb && *fa == *fb,
            _ => false,
        }
    }
}

/// Offset added to the track ids of a separate HLS audio rendition so they never collide with the
/// main playlist's track ids (which must stay below this value).
pub const AUDIO_TRACK_ID_OFFSET: u32 = 1000;
/// Default maximum accepted HLS playlist body size.
pub const DEFAULT_MAX_PLAYLIST_BYTES: usize = 4 * 1024 * 1024;
/// Default maximum accepted HLS media resource body size.
pub const DEFAULT_MAX_SEGMENT_BYTES: usize = 64 * 1024 * 1024;
/// Default per-request timeout for HLS playlists, microseconds.
pub const DEFAULT_HLS_REQUEST_TIMEOUT_US: u64 = 20_000_000;
/// Default per-request timeout for HLS media resources, microseconds.
pub const DEFAULT_HLS_SEGMENT_TIMEOUT_US: u64 = 60_000_000;
/// Default per-request timeout for HLS key requests, microseconds.
pub const DEFAULT_HLS_KEY_TIMEOUT_US: u64 = 20_000_000;
/// Default maximum accepted HLS key response body size.
pub const DEFAULT_MAX_KEY_BYTES: usize = 4096;
/// Default number of consecutive key/decrypt failures before the session gives up.
pub const DEFAULT_MAX_KEY_FAILURES: u32 = 8;
/// Default HLS retry backoff schedule, milliseconds per attempt.
pub const DEFAULT_HLS_BACKOFF_MS: [u64; 3] = [2_000, 4_000, 8_000];
/// Default HLS retry backoff once the schedule is exhausted, milliseconds.
pub const DEFAULT_HLS_BACKOFF_CAP_MS: u64 = 10_000;

/// Session configuration.
#[derive(Clone, PartialEq)]
pub struct Config {
    /// Source kind.
    pub kind: SourceKind,
    /// TS-over-HTTP config.
    pub ts_http: tshttp_runtime::Config,
    /// Maximum redirects per open (per request for HLS).
    pub max_redirects: u32,
    /// Custom request headers, sent only to the session origin. `Authorization`, `Cookie`, `Host`,
    /// `Content-Length`, `Transfer-Encoding`, `Connection` and `Range` are reserved and rejected
    /// by [`crate::Session::open`].
    pub extra_headers: Vec<(String, String)>,
    /// Cookies sent to the session origin on path `/` (see [`crate::Session::cookies`]).
    pub initial_cookies: Vec<(String, String)>,
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
    /// HLS: key request timeout, microseconds.
    pub hls_key_timeout_us: u64,
    /// HLS: maximum key response body bytes.
    pub max_key_bytes: usize,
    /// HLS: consecutive key/decrypt failures (no decoded samples in between) after which the
    /// session emits a fatal [`Event::Error`] and stops.
    pub max_key_failures: u32,
    /// HLS: BCP 47 language (prefix match, ASCII case-insensitive) preferred for the separate
    /// audio rendition.
    pub preferred_audio_language: Option<String>,
    /// HLS: pick a separate audio rendition automatically when the master playlist arrives.
    pub auto_select_audio: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            kind: SourceKind::Auto,
            ts_http: tshttp_runtime::Config::default(),
            max_redirects: 5,
            extra_headers: Vec::new(),
            initial_cookies: Vec::new(),
            buffer: buffering_runtime::ControllerConfig::default(),
            variant_policy: hls_runtime::client::VariantPolicy::default(),
            max_playlist_bytes: DEFAULT_MAX_PLAYLIST_BYTES,
            max_segment_bytes: DEFAULT_MAX_SEGMENT_BYTES,
            hls_request_timeout_us: DEFAULT_HLS_REQUEST_TIMEOUT_US,
            hls_segment_timeout_us: DEFAULT_HLS_SEGMENT_TIMEOUT_US,
            hls_backoff_ms: DEFAULT_HLS_BACKOFF_MS.to_vec(),
            hls_backoff_cap_ms: DEFAULT_HLS_BACKOFF_CAP_MS,
            hls_key_timeout_us: DEFAULT_HLS_KEY_TIMEOUT_US,
            max_key_bytes: DEFAULT_MAX_KEY_BYTES,
            max_key_failures: DEFAULT_MAX_KEY_FAILURES,
            preferred_audio_language: None,
            auto_select_audio: true,
        }
    }
}

impl core::fmt::Debug for Config {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let header_names: Vec<&str> = self.extra_headers.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("Config")
            .field("kind", &self.kind)
            .field("ts_http", &self.ts_http)
            .field("max_redirects", &self.max_redirects)
            .field("extra_header_names", &header_names)
            .field("initial_cookies", &self.initial_cookies.len())
            .field("buffer", &self.buffer)
            .field("variant_policy", &self.variant_policy)
            .field("max_playlist_bytes", &self.max_playlist_bytes)
            .field("max_segment_bytes", &self.max_segment_bytes)
            .field("hls_request_timeout_us", &self.hls_request_timeout_us)
            .field("hls_segment_timeout_us", &self.hls_segment_timeout_us)
            .field("hls_backoff_ms", &self.hls_backoff_ms)
            .field("hls_backoff_cap_ms", &self.hls_backoff_cap_ms)
            .field("hls_key_timeout_us", &self.hls_key_timeout_us)
            .field("max_key_bytes", &self.max_key_bytes)
            .field("max_key_failures", &self.max_key_failures)
            .field("preferred_audio_language", &self.preferred_audio_language)
            .field("auto_select_audio", &self.auto_select_audio)
            .finish()
    }
}
