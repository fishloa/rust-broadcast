//! Action, event, status and configuration types.

use alloc::string::String;
use alloc::vec::Vec;

use url::Url;

use broadcast_auth::Scheme as AuthScheme;

use transmux::{DisplayInfo, JoinDecision, Sample, TrackId, TrackSpec};

use alloc::sync::Arc;

use crate::ConnId;
use dash_runtime::client::config::DashConfig;
use dash_runtime::timing::clock::ClockSource;

/// Source kind for [`Config`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Auto-detect from the URL: an `rtsp://` or `rtsps://` URL is RTSP, a `.m3u8`/`.m3u` path
    /// is HLS, a `.mpd` path (ASCII case-insensitive) is DASH, anything else TS-over-HTTP.
    Auto,
    /// TS-over-HTTP.
    TsHttp,
    /// HLS (M3U8) on `hls-runtime`.
    Hls,
    /// MPEG-DASH (MPD) on `dash-runtime`, static and dynamic presentations.
    Dash,
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
            Self::Dash => "dash",
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
    /// Reconnecting. `attempt` is the number of consecutive failed attempts; `0` announces the
    /// reconnect of a connection that had played (a finished stream, a stall reconnect, a close
    /// by the server), after which [`Status::Establishing`] and [`Status::Live`] follow with the
    /// first sample. A caller-initiated [`crate::Session::reconnect`] reports no
    /// `Reconnecting`.
    Reconnecting {
        /// Consecutive failed attempts, `0` for a reconnect after a played connection.
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
///
/// Positions are in the player's sample-time domain: the presentation time stamp of a sample
/// converted to 27 MHz ticks, so `Start.at_27mhz` and `Resume.at_27mhz` are the 27 MHz clock the
/// player must set before it plays (its position in `Session::on_playback_position` uses the
/// same domain). A timeline whose first kept sample has a negative time stamp (a jump whose
/// unrolled timestamps go below zero) cannot be expressed in `u64`: its positions are measured from
/// that first kept sample, so `0` is that sample. Live TS-over-HTTP decides `Start` inside
/// `Session::on_playback_position`: a host that never calls it gets no buffer event, and one that
/// calls it less often than every display tick gets `Start` later, not never; a demuxer-level
/// discontinuity (PCR discontinuity flag, re-anchor) is invisible to the host except through
/// these events, whose `at_27mhz` may then jump.
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
    /// is the `METHOD` value, or the `KEYFORMAT` value for an unsupported key format; for a
    /// CENC/CBCS-protected track inside fragmented MP4 (this client decrypts AES-128 segments
    /// only, never sample encryption) it is the track's encrypted sample entry (`encv`, `enca`),
    /// reported once per track id as a non-fatal error, and fatal once no track is left to play.
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
    /// A Media Initialization Section (`EXT-X-MAP`) fetch failed with a non-authentication
    /// status, a transport error, a timeout, or an oversized body. The fetch is
    /// retried with backoff; [`Config::max_key_failures`] consecutive failures (no decoded
    /// samples in between) make one fatal error of this kind. Unlike a key request, a `403`
    /// counts too (it is also reported as [`Event::Forbidden`]).
    InitFetchFailed {
        /// The HTTP status, `None` for a transport error, timeout or oversized body.
        status: Option<u16>,
    },
    /// A Media Initialization Section (`EXT-X-MAP`) arrived with a success status but holds no
    /// complete `moov` box. Retried and counted exactly like [`ErrorKind::InitFetchFailed`]:
    /// [`Config::max_key_failures`] consecutive failures (no decoded samples in between) make one
    /// fatal error of this kind.
    InitDecodeFailed,
    /// An HLS media segment (or part) could not be obtained: the origin answered with a non-success
    /// status, a transport error or a timeout, an oversized body, or a `200` whose body is not
    /// media for its container (an HTML error page, random bytes, an empty body, a fragment
    /// without its `mdat`). Each failure is retried on the backoff schedule;
    /// [`Config::max_segment_failures`] consecutive failures of the same segment end it: a
    /// VOD playlist (`EXT-X-ENDLIST`) with one fatal error of this kind, a live one with one
    /// non-fatal error of this kind followed by [`Event::Discontinuity`] and playback continuing
    /// with the next segment.
    SegmentFetchFailed {
        /// The HTTP status of the last failure, `None` for a transport error, timeout, oversized
        /// body or a `2xx` body that is not media.
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
    /// The main HLS playlist carries a track whose id is at or above
    /// [`AUDIO_TRACK_ID_OFFSET`], the range reserved for separate audio renditions. That track
    /// and its samples are dropped (reported once per session); the other tracks play.
    TrackIdReserved {
        /// The first offending track id.
        track_id: u32,
    },
    /// An HLS track was left out. For a TS segment: an elementary stream that cannot be carried
    /// in the fragmented MP4 the HLS source produces (opaque data such as SCTE-35, ID3 or
    /// private PES, or a subtitle without a sample entry). For an fMP4 init: a track whose
    /// sample entry the demuxer cannot model (an unrecognised codec; an encrypted one is
    /// [`ErrorKind::UnsupportedEncryption`]). Reported once per track per source; the other
    /// tracks play, and a stream with no playable track ends fatally after these reports.
    TrackDropped {
        /// The dropped track's id (remapped for an audio rendition); `0` for a track the fMP4
        /// demuxer could not name. For RTSP it is not the id of an announced track: it is the
        /// 1-based position of the dropped `m=` line among all media lines of the DESCRIBE SDP,
        /// whatever the other lines are.
        track_id: u32,
        /// The PMT `stream_type` (ISO/IEC 13818-1 Table 2-34) when the track is opaque TS data.
        stream_type: Option<u8>,
        /// The fMP4 sample entry's four-character code (`None` for a TS stream).
        fourcc: Option<String>,
    },
    /// The HLS media is not in the container layout this client plays. `reason` is
    /// `non_fragmented_mp4` (a progressive MP4 served as init and segment — HLS fMP4 carries
    /// `moof`/`mdat` fragments; fatal, reported at the first segment) or `damaged_fragment` (a
    /// `moof` the demuxer had to skip: unparsable, or its sample data offset points outside its
    /// own segment, as with an absolute `tfhd` base-data-offset — CMAF uses
    /// default-base-is-moof — so that fragment's samples are lost; non-fatal, reported once per
    /// session).
    UnsupportedContainer {
        /// The stable reason token.
        reason: &'static str,
    },
    /// A response body outgrew the cap `field` names (`max_manifest_bytes` or
    /// `max_segment_bytes`), so the request was closed and retried. Reported once per field and
    /// session, non-fatal: the retry meets the same cap unless the origin's response shrinks, so
    /// a host that sees this should raise the cap.
    BodyTooLarge {
        /// The [`Config`] field whose cap was exceeded.
        field: &'static str,
    },
    /// [`crate::Session::set_rendered_tracks`] named a track id that no [`Event::Tracks`] has
    /// announced. The id is ignored until an announcement names it (then it takes effect), and
    /// reported once per declaration. A declaration none of whose ids is announced renders the
    /// default set meanwhile.
    UnknownRenderedTrack {
        /// The declared id.
        track_id: u32,
    },
    /// An MPD could not be read as XML or broke a size or structure bound (`reason` is a stable
    /// token). Non-fatal and reported once per kind until a good MPD arrives; the session keeps
    /// the last good MPD and retries. After [`Config::max_key_failures`] consecutive bad MPDs
    /// with no good one between, one fatal error of this kind follows.
    MpdParse {
        /// The stable reason token.
        reason: &'static str,
    },
    /// An MPD parsed but cannot be played: no Period, no playable AdaptationSet, or an
    /// essential descriptor group none of whose members is understood. Fatal when nothing is
    /// left to play.
    MpdInvalid {
        /// The stable reason token.
        reason: &'static str,
    },
    /// The MPD addresses media in a way this client does not implement: `presentation_type_list`,
    /// `data_url`, `open_ended_range`, `hierarchical_sidx` or `cross_origin` (a URL whose origin
    /// differs from the MPD's while [`DashConfig::cross_origin_urls`] is `false`). The affected
    /// Representation is dropped; fatal only when no Representation is left.
    UnsupportedAddressing {
        /// The stable reason token.
        reason: &'static str,
    },
    /// A feature the stream uses and this client ignores: `preselection`, `xlink`, `mpd_patch`,
    /// `segment_sequence`, `event_stream` or `low_latency`. Non-fatal, reported once per feature
    /// and session; playback continues without it.
    UnsupportedFeature {
        /// The stable feature token.
        feature: &'static str,
    },
    /// A live session fell behind the availability window: the segments it needed are no longer
    /// served, so it skipped ahead (an [`Event::Discontinuity`] follows). Non-fatal, reported
    /// once per run of consecutive occurrences.
    LiveEdgeLost,
    /// A dynamic MPD needs a UTC clock and there is none: no usable `UTCTiming`, no response
    /// `Date` header, and the host never called [`crate::Session::set_wall_clock`]. Fatal.
    ClockUnavailable,
}

impl ErrorKind {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::UnsupportedEncryption { .. } => "unsupported_encryption",
            Self::KeyFetchFailed { .. } => "key_fetch_failed",
            Self::InitFetchFailed { .. } => "init_fetch_failed",
            Self::SegmentFetchFailed { .. } => "segment_fetch_failed",
            Self::InitDecodeFailed => "init_decode_failed",
            Self::BadKey => "bad_key",
            Self::DecryptFailed => "decrypt_failed",
            Self::UnsupportedCodec { .. } => "unsupported_codec",
            Self::TrackIdReserved { .. } => "track_id_reserved",
            Self::TrackDropped { .. } => "track_dropped",
            Self::UnknownRenderedTrack { .. } => "unknown_rendered_track",
            Self::UnsupportedContainer { .. } => "unsupported_container",
            Self::BodyTooLarge { .. } => "body_too_large",
            Self::MpdParse { .. } => "mpd_parse",
            Self::MpdInvalid { .. } => "mpd_invalid",
            Self::UnsupportedAddressing { .. } => "unsupported_addressing",
            Self::UnsupportedFeature { .. } => "unsupported_feature",
            Self::LiveEdgeLost => "live_edge_lost",
            Self::ClockUnavailable => "clock_unavailable",
        }
    }
}

broadcast_common::impl_spec_display!(ErrorKind);

/// Host action produced by [`crate::Session`].
///
/// `Debug` prints header names, lengths and URLs but never header values or `Send` payloads,
/// which carry `Authorization`, `Cookie` and RTSP authentication in clear.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq)]
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
        /// Optional byte range as `(offset, length)`, not `(start, end)`: the host builds the
        /// `Range: bytes=offset-(offset+length-1)` header itself.
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
    /// Call [`crate::Session::poll`] at or after the given host clock time (`now_us`); the
    /// session drops a `WakeAt` whose time is not after the `now_us` of the poll that would
    /// return it, so a past value is never delivered.
    WakeAt(u64),
}

impl core::fmt::Debug for Action {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::HttpRequest {
                conn,
                method,
                url,
                headers,
                byte_range,
            } => {
                let names: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
                f.debug_struct("HttpRequest")
                    .field("conn", conn)
                    .field("method", method)
                    .field("url", url)
                    .field("header_names", &names)
                    .field("byte_range", byte_range)
                    .finish()
            }
            Self::TcpConnect {
                conn,
                host,
                port,
                tls,
            } => f
                .debug_struct("TcpConnect")
                .field("conn", conn)
                .field("host", host)
                .field("port", port)
                .field("tls", tls)
                .finish(),
            Self::Send { conn, bytes } => f
                .debug_struct("Send")
                .field("conn", conn)
                .field("len", &bytes.len())
                .finish(),
            Self::Close { conn } => f.debug_struct("Close").field("conn", conn).finish(),
            Self::WakeAt(at) => f.debug_tuple("WakeAt").field(at).finish(),
        }
    }
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
    /// A new timeline starts with the next sample: a reconnect that follows a connection that
    /// produced media, a detected timeline jump, an HLS discontinuity. After a detected jump the
    /// other tracks deliver nothing until their first sample lies on the new timeline
    /// (see `Session::stale_samples_dropped`).
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
    /// Auth required. `resource_metadata` is chosen by the server that sent the challenge and can
    /// be any `https` origin: a host that fetches it must validate it first (expected host, no
    /// private, loopback or link-local address), or the fetch is a server-side request forgery
    /// vector. The session never fetches it.
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
    /// The session has a UTC clock for the first time, or the source of its clock changed
    /// (for example a measured `UTCTiming` replaced the host clock). Not repeated for a re-sync
    /// from the same source. `uncertainty_us` is the half-width of the error bar around the
    /// session's UTC estimate, microseconds.
    ///
    /// Ordering contract: it is emitted before the first [`Event::Sample`] that depends on the
    /// clock (a live DASH join is placed with it), and never for a source that needs no clock.
    /// The DASH source that emits it lands after this surface; hosts can match on it now.
    ClockSynced {
        /// Where the clock came from.
        source: ClockSource,
        /// Half-width of the error bar, microseconds.
        uncertainty_us: u64,
    },
    /// A condition the player should tell the user about. `fatal` means the session will not
    /// recover by itself: it has closed its connections and goes silent, so the player shows a
    /// message and does not retry. Non-fatal errors are emitted once per kind until a success
    /// (decoded samples) resets them, while the session keeps retrying with backoff; after
    /// [`Config::max_key_failures`] consecutive key, decrypt or init-fetch failures one fatal
    /// error of the same kind follows.
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
                Self::ClockSynced {
                    source: sa,
                    uncertainty_us: ua,
                },
                Self::ClockSynced {
                    source: sb,
                    uncertainty_us: ub,
                },
            ) => *sa == *sb && *ua == *ub,
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
/// main playlist's track ids, which must stay below this value (a main track at or above it is
/// dropped with [`ErrorKind::TrackIdReserved`]).
pub const AUDIO_TRACK_ID_OFFSET: u32 = 1000;
/// Default maximum accepted manifest (playlist) body size.
pub const DEFAULT_MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
/// Default maximum accepted HLS media resource body size.
pub const DEFAULT_MAX_SEGMENT_BYTES: usize = 64 * 1024 * 1024;
/// Default per-request timeout for manifests (playlists), microseconds.
pub const DEFAULT_REQUEST_TIMEOUT_US: u64 = 20_000_000;
/// Default per-request timeout for HLS media resources, microseconds.
pub const DEFAULT_SEGMENT_TIMEOUT_US: u64 = 60_000_000;
/// Default per-request timeout for HLS key requests, microseconds.
pub const DEFAULT_HLS_KEY_TIMEOUT_US: u64 = 20_000_000;
/// Default maximum accepted HLS key response body size.
pub const DEFAULT_MAX_KEY_BYTES: usize = 4096;
/// Default number of consecutive key/decrypt failures before the session gives up.
pub const DEFAULT_MAX_KEY_FAILURES: u32 = 8;
/// Default number of consecutive failures of one HLS segment before it is given up on, see
/// [`Config::max_segment_failures`].
pub const DEFAULT_MAX_SEGMENT_FAILURES: u32 = 8;
/// Default HLS retry backoff schedule, milliseconds per attempt.
pub const DEFAULT_BACKOFF_MS: [u64; 3] = [2_000, 4_000, 8_000];
/// Default HLS retry backoff once the schedule is exhausted, milliseconds.
pub const DEFAULT_BACKOFF_CAP_MS: u64 = 10_000;
/// Default cap on simultaneously outstanding HLS segment and part requests, see
/// [`Config::max_inflight_segments`].
pub const DEFAULT_MAX_INFLIGHT_SEGMENTS: usize = 4;
/// Default cap on input bytes the initial-tracks hold may buffer, see
/// [`Config::tracks_hold_max_input_bytes`].
pub const DEFAULT_TRACKS_HOLD_MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;
/// Default cap on media time the initial-tracks hold may buffer, microseconds, see
/// [`Config::tracks_hold_max_media_us`].
pub const DEFAULT_TRACKS_HOLD_MAX_MEDIA_US: u64 = 1_000_000;
/// Default cap on session-clock time the initial-tracks hold may last, microseconds, see
/// [`Config::tracks_hold_max_wall_us`].
pub const DEFAULT_TRACKS_HOLD_MAX_WALL_US: u64 = 1_000_000;
/// Default input bytes without a stream resolving after which the initial-tracks hold gives up,
/// see [`Config::tracks_hold_quiet_input_bytes`].
pub const DEFAULT_TRACKS_HOLD_QUIET_INPUT_BYTES: usize = 512 * 1024;
/// Default longest a live TS `Start` waits, after the first media, for a PMT-declared audio or
/// video stream that has no track yet, microseconds, see [`Config::start_pending_track_wait_us`].
/// A live mux sends audio up to about a second behind video and the audio codec config resolves
/// only with its first frame: the zenith-tv trace (hold released on the wall backstop, audio
/// announced about 1.3 s after the first media) started on video alone and paused 100 ms later.
/// Measured resolution gaps on the committed captures are 0.16 s to 0.48 s of media, so 1.5 s
/// covers a skew three times the worst measured and is shorter than the 1.8 s default start
/// lead, which a declared but never delivering audio PID waits out anyway.
pub const DEFAULT_START_PENDING_TRACK_WAIT_US: u64 = 1_500_000;
/// Default cap on events queued for the host, see [`Config::max_queued_events`].
pub const DEFAULT_MAX_QUEUED_EVENTS: usize = 100_000;

/// Session configuration.
#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct Config {
    /// Source kind.
    pub kind: SourceKind,
    /// TS-over-HTTP config (also the RTSP timers). An inconsistent value (a zero timeout, an
    /// empty or zero-entry backoff, a stall reconnect not after the notice) is replaced by the
    /// nearest valid one, see `tshttp_runtime::Config::clamped`; it is not rejected.
    pub ts_http: tshttp_runtime::Config,
    /// Maximum redirects per open (per request for HLS). `0` follows none: a redirect response
    /// fails the connection.
    pub max_redirects: u32,
    /// Custom request headers, sent only to the session origin. `Authorization`, `Cookie`, `Host`,
    /// `Content-Length`, `Transfer-Encoding`, `Connection` and `Range` are reserved and rejected
    /// by [`crate::Session::open`].
    pub extra_headers: Vec<(String, String)>,
    /// Cookies sent to the session origin on path `/` (see [`crate::Session::cookies`]).
    pub initial_cookies: Vec<(String, String)>,
    /// Buffer controller config; a value `buffering_runtime::ControllerConfig::validate`
    /// rejects fails [`crate::Session::open`] with [`crate::Error::InvalidConfig`].
    pub buffer: buffering_runtime::ControllerConfig,
    /// Playout gate leads for HLS and finite TS (live TS-over-HTTP uses [`Config::buffer`]).
    pub playout: crate::playout::PlayoutConfig,
    /// HLS and DASH variant selection policy (`max_bandwidth` and `max_height` bound the chosen
    /// Representation, `supported_codecs` filters by codec prefix).
    pub variant_policy: hls_runtime::client::VariantPolicy,
    /// Segment-pull sources (HLS, DASH): maximum manifest (playlist) body bytes; must be non-zero.
    pub max_manifest_bytes: usize,
    /// Segment-pull sources: maximum media resource body bytes (HLS segment or part, DASH init,
    /// index or segment); must be non-zero.
    pub max_segment_bytes: usize,
    /// Segment-pull sources: manifest request timeout, microseconds; must be non-zero.
    pub request_timeout_us: u64,
    /// Segment-pull sources: media resource request timeout, microseconds; must be non-zero.
    pub segment_timeout_us: u64,
    /// Segment-pull sources: retry backoff per attempt, milliseconds; an entry of `0` is treated as `1`.
    pub backoff_ms: Vec<u64>,
    /// Segment-pull sources: retry backoff after `backoff_ms` is exhausted, milliseconds; `0` is treated as `1`.
    pub backoff_cap_ms: u64,
    /// Segment-pull sources: segment and part requests kept outstanding at once, per playlist adapter. The rest
    /// stay queued and are released as responses complete, in order; playlist, key and
    /// initialization-section requests are not counted and never wait behind them. Held bodies
    /// of completed-but-undelivered segments count against the same cap, so memory is bounded
    /// by this value times [`Config::max_segment_bytes`]. `0` is treated as `1`.
    pub max_inflight_segments: usize,
    /// HLS: key request timeout, microseconds; must be non-zero.
    pub hls_key_timeout_us: u64,
    /// HLS: maximum key response body bytes; must be non-zero.
    pub max_key_bytes: usize,
    /// HLS and DASH: consecutive key/decrypt/init-fetch failures (no decoded samples in between) after which the
    /// session emits a fatal [`Event::Error`] and stops; `0` behaves as `1` (the first failure is
    /// fatal).
    pub max_key_failures: u32,
    /// Segment-pull sources: consecutive failures of one media segment (or part) after which it is given up on: a
    /// VOD playlist (`EXT-X-ENDLIST`) ends with one fatal [`ErrorKind::SegmentFetchFailed`], a
    /// live one skips the segment with one non-fatal [`ErrorKind::SegmentFetchFailed`] followed by
    /// [`Event::Discontinuity`]. The count is per segment, grows the backoff schedule
    /// ([`Config::backoff_ms`]) and resets only when that segment is delivered as media,
    /// never on a playlist delivery. `0` makes the first failure final; `u32::MAX` retries
    /// forever. Playlist reload failures are not counted: a playlist cannot be skipped, so they
    /// retry forever.
    pub max_segment_failures: u32,
    /// HLS and DASH: BCP 47 language (prefix match, ASCII case-insensitive) preferred for the
    /// separate audio rendition (DASH: the audio AdaptationSet, never a commentary or description
    /// role).
    pub preferred_audio_language: Option<String>,
    /// HLS and DASH: pick a separate audio rendition automatically when the master playlist (HLS)
    /// or the Period's selection (DASH) arrives; `false` plays video only until
    /// [`crate::Session::select_audio_rendition`] chooses one.
    pub auto_select_audio: bool,
    /// Events the session keeps queued for the host. The host must drain
    /// [`crate::Session::next_event`]; when a host does not and the queue exceeds this cap, the
    /// oldest [`Event::Sample`]s, [`Event::Starved`] notices and transient [`Status`] values
    /// are dropped (samples are counted by [`crate::Session::dropped_events`]; the latest status
    /// always survives). One [`Event::Discontinuity`] takes the place of each burst of dropped samples
    /// so the player re-syncs. Every other event (tracks, errors, buffer events, terminal
    /// statuses) is kept, so the queue is bounded by this cap plus the undroppable events a
    /// stalled host leaves unread. Must be non-zero (a zero cap would drop every sample).
    pub max_queued_events: usize,
    /// Timeline-jump detector thresholds; `None` disables the detector. A jump of the media
    /// timeline without a TS discontinuity flag emits [`Event::Discontinuity`] before the first
    /// sample of the new timeline and re-arms the join, buffering and playout state as a
    /// demuxer discontinuity does (see [`crate::TimelineJumpConfig`]). Applies to every source.
    /// Both thresholds must be non-zero; a threshold of `u64::MAX` never fires in that direction.
    /// A threshold below the stream's frame spacing or reordering depth fires on ordinary
    /// samples and drops the other tracks' samples as stale.
    pub timeline_jump: Option<crate::timeline::TimelineJumpConfig>,
    /// Live TS-over-HTTP: the first media of each connection is held behind one
    /// [`Event::Tracks`] listing every PMT-declared stream until all have resolved or a hold
    /// limit is reached. This is the most input bytes held; must be at least
    /// [`Config::tracks_hold_quiet_input_bytes`].
    pub tracks_hold_max_input_bytes: usize,
    /// Live TS-over-HTTP hold: input bytes without any further stream resolving after which the
    /// hold gives up on the streams still unresolved. Must be non-zero.
    pub tracks_hold_quiet_input_bytes: usize,
    /// Live TS-over-HTTP hold: most media time held (the largest per-track pts span),
    /// microseconds. `0` disables the hold: tracks are announced as each stream resolves.
    pub tracks_hold_max_media_us: u64,
    /// Live TS-over-HTTP hold: longest the hold may last on the caller's microsecond clock.
    /// `0` disables the hold: tracks are announced as each stream resolves. A low-latency
    /// host whose first frame must land early lowers this (a stream whose declared PID never
    /// sends is released after this long at most).
    pub tracks_hold_max_wall_us: u64,
    /// Live TS-over-HTTP: how long the first `Start` waits, counted from the first media, for an
    /// audio or video stream the PMT declares but that has no announced track yet (a live mux
    /// sends audio up to about a second behind video). `0` does not wait: `Start` goes ahead on
    /// the tracks announced so far, which can be followed by a `Pause` when the late track
    /// arrives behind the playback position. A host with a short start lead and a tight
    /// first-frame budget lowers this; any value is accepted and `u64::MAX` waits until the
    /// first `Start`.
    pub start_pending_track_wait_us: u64,
    /// DASH-specific knobs (clock ladder, `UTCTiming` timeouts, early-request cadence, MPD refresh
    /// floor); the knobs every segment-pull source shares stay on this struct. A value
    /// [`DashConfig::validate`] rejects fails [`crate::Session::open`] with
    /// [`crate::Error::InvalidConfig`] naming `dash.<field>`, even for a session that does not
    /// play DASH.
    pub dash: DashConfig,
}

impl Config {
    pub(crate) fn validate(&self) -> Result<(), crate::Error> {
        let invalid = |field| Err(crate::Error::InvalidConfig { field });
        if self.max_queued_events == 0 {
            return invalid("max_queued_events");
        }
        if self.max_manifest_bytes == 0 {
            return invalid("max_manifest_bytes");
        }
        if self.max_segment_bytes == 0 {
            return invalid("max_segment_bytes");
        }
        if self.max_key_bytes == 0 {
            return invalid("max_key_bytes");
        }
        if self.request_timeout_us == 0 {
            return invalid("request_timeout_us");
        }
        if self.segment_timeout_us == 0 {
            return invalid("segment_timeout_us");
        }
        if self.hls_key_timeout_us == 0 {
            return invalid("hls_key_timeout_us");
        }
        if self.buffer.validate().is_err() {
            return invalid("buffer");
        }
        if let Some(jump) = &self.timeline_jump {
            if jump.backward_us == 0 {
                return invalid("timeline_jump.backward_us");
            }
            if jump.forward_slack_us == 0 {
                return invalid("timeline_jump.forward_slack_us");
            }
        }
        if let Err(e) = self.dash.validate() {
            return invalid(dash_field(e.field));
        }
        self.validate_tracks_hold()
    }

    fn validate_tracks_hold(&self) -> Result<(), crate::Error> {
        if self.tracks_hold_quiet_input_bytes == 0 {
            return Err(crate::Error::InvalidConfig {
                field: "tracks_hold_quiet_input_bytes",
            });
        }
        if self.tracks_hold_max_input_bytes < self.tracks_hold_quiet_input_bytes {
            return Err(crate::Error::InvalidConfig {
                field: "tracks_hold_max_input_bytes",
            });
        }
        Ok(())
    }
}

fn dash_field(name: &str) -> &'static str {
    match name {
        "utc_timeout_us" => "dash.utc_timeout_us",
        "utc_wait_us" => "dash.utc_wait_us",
        "utc_max_rtt_us" => "dash.utc_max_rtt_us",
        "utc_resync_us" => "dash.utc_resync_us",
        "early_retry_us" => "dash.early_retry_us",
        "min_mpd_refresh_us" => "dash.min_mpd_refresh_us",
        _ => "dash",
    }
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
            playout: crate::playout::PlayoutConfig::default(),
            variant_policy: hls_runtime::client::VariantPolicy::default(),
            max_manifest_bytes: DEFAULT_MAX_MANIFEST_BYTES,
            max_segment_bytes: DEFAULT_MAX_SEGMENT_BYTES,
            request_timeout_us: DEFAULT_REQUEST_TIMEOUT_US,
            segment_timeout_us: DEFAULT_SEGMENT_TIMEOUT_US,
            backoff_ms: DEFAULT_BACKOFF_MS.to_vec(),
            backoff_cap_ms: DEFAULT_BACKOFF_CAP_MS,
            max_inflight_segments: DEFAULT_MAX_INFLIGHT_SEGMENTS,
            hls_key_timeout_us: DEFAULT_HLS_KEY_TIMEOUT_US,
            max_key_bytes: DEFAULT_MAX_KEY_BYTES,
            max_key_failures: DEFAULT_MAX_KEY_FAILURES,
            max_segment_failures: DEFAULT_MAX_SEGMENT_FAILURES,
            preferred_audio_language: None,
            auto_select_audio: true,
            max_queued_events: DEFAULT_MAX_QUEUED_EVENTS,
            timeline_jump: Some(crate::timeline::TimelineJumpConfig::default()),
            tracks_hold_max_input_bytes: DEFAULT_TRACKS_HOLD_MAX_INPUT_BYTES,
            tracks_hold_quiet_input_bytes: DEFAULT_TRACKS_HOLD_QUIET_INPUT_BYTES,
            tracks_hold_max_media_us: DEFAULT_TRACKS_HOLD_MAX_MEDIA_US,
            tracks_hold_max_wall_us: DEFAULT_TRACKS_HOLD_MAX_WALL_US,
            start_pending_track_wait_us: DEFAULT_START_PENDING_TRACK_WAIT_US,
            dash: DashConfig::default(),
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
            .field("playout", &self.playout)
            .field("variant_policy", &self.variant_policy)
            .field("max_manifest_bytes", &self.max_manifest_bytes)
            .field("max_segment_bytes", &self.max_segment_bytes)
            .field("request_timeout_us", &self.request_timeout_us)
            .field("segment_timeout_us", &self.segment_timeout_us)
            .field("backoff_ms", &self.backoff_ms)
            .field("backoff_cap_ms", &self.backoff_cap_ms)
            .field("max_inflight_segments", &self.max_inflight_segments)
            .field("hls_key_timeout_us", &self.hls_key_timeout_us)
            .field("max_key_bytes", &self.max_key_bytes)
            .field("max_key_failures", &self.max_key_failures)
            .field("max_segment_failures", &self.max_segment_failures)
            .field("preferred_audio_language", &self.preferred_audio_language)
            .field("auto_select_audio", &self.auto_select_audio)
            .field("max_queued_events", &self.max_queued_events)
            .field("timeline_jump", &self.timeline_jump)
            .field(
                "tracks_hold_max_input_bytes",
                &self.tracks_hold_max_input_bytes,
            )
            .field(
                "tracks_hold_quiet_input_bytes",
                &self.tracks_hold_quiet_input_bytes,
            )
            .field("tracks_hold_max_media_us", &self.tracks_hold_max_media_us)
            .field("tracks_hold_max_wall_us", &self.tracks_hold_max_wall_us)
            .field(
                "start_pending_track_wait_us",
                &self.start_pending_track_wait_us,
            )
            .field("dash", &self.dash)
            .finish()
    }
}
