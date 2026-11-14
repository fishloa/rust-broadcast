//! Error type for multimux.
//!
//! Field-carrying [`thiserror`] variants (workspace convention — see
//! `rtsp-runtime/src/error.rs`) so callers can match on failure *kind*
//! instead of parsing a string. **No variant ever carries a credential** —
//! call sites that build these from a `rtsp://user:pass@host/...` URL must
//! redact it first (this crate's internal `redact_url` helper, or
//! [`crate::source::rtsp`]'s userinfo-stripping) before it reaches an error
//! message, since these render into logs (`tracing`), `Display`, and process
//! exit output.

use std::path::PathBuf;
use thiserror::Error;

/// Errors from configuration, RTSP ingest, segmentation, or the HTTP origin.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MultimuxError {
    /// The config file could not be read from disk.
    #[error("failed to read config file {path:?}: {source}")]
    ConfigRead {
        /// The config file path that failed to read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The config file's bytes could not be parsed as JSON.
    #[error("failed to parse config file {path:?}: {reason}")]
    ConfigParse {
        /// The config file path that failed to parse.
        path: PathBuf,
        /// The parse failure (e.g. the `serde_json::Error` message).
        reason: String,
    },

    /// A parsed config (or CLI-built config) failed semantic validation.
    #[error("invalid config field {field:?}: {reason}")]
    ConfigInvalid {
        /// The offending field's name.
        field: &'static str,
        /// Why it's invalid.
        reason: String,
    },

    /// Transport-level connect failure: DNS, TCP connect, or TLS handshake.
    #[error("connect failed: {reason}")]
    Connect {
        /// What went wrong. Never includes URL userinfo.
        reason: String,
    },

    /// An RTSP/HTTP protocol exchange failed (e.g. DESCRIBE/SETUP/PLAY returned a
    /// non-2xx status, or the server's response was unusable).
    #[error("{phase} failed: {kind}")]
    Protocol {
        /// Which phase failed.
        phase: Phase,
        /// Why it failed.
        kind: ProtocolFailure,
    },

    /// The DESCRIBE SDP body could not be parsed, or described media this
    /// crate doesn't support.
    #[error("SDP error: {0}")]
    Sdp(
        /// Why the SDP was rejected.
        #[source]
        SdpError,
    ),

    /// Credential extraction/decoding failed, or the source rejected the
    /// configured credentials (e.g. a `401` that persisted after the
    /// engine's own challenge/response retry).
    #[error("authentication error: {0}")]
    Auth(
        /// Why authentication failed. Never includes the raw credential.
        #[source]
        broadcast_auth::Error,
    ),

    /// An ingest FLV tag stream (the `rtmp::RtmpRoute` publisher's media)
    /// failed to demux; the `transmux::FlvError` is kept as the source.
    #[error("rtmp/flv: {0}")]
    Flv(#[from] transmux::FlvError),

    /// A `transmux` error: RTP depayloading on the WHIP ingest path, or a
    /// downstream segmentation/mux failure.
    #[error("transmux: {0}")]
    Transmux(#[from] transmux::Error),

    /// An `hls_runtime::client::HlsClient` (`source::hls_pull`, plan
    /// step 5a round 3) rejected a fed playlist/resource — malformed
    /// playlist text, a demux failure, or a resource delivered for an id it
    /// never requested (a driver bug).
    #[error("hls-pull client: {0}")]
    LlHls(#[from] hls_runtime::client::Error),

    /// A `broadcast_client::Session` reported an error that retrying cannot fix (undecryptable
    /// or unsupported content, a bad key) or that ended the session.
    #[error("{what}: {kind:?}")]
    Session {
        /// Which source reported it.
        what: &'static str,
        /// The session's own classification.
        kind: broadcast_client::ErrorKind,
    },

    /// An I/O error (socket, bind) not already covered by [`Self::Connect`]
    /// or [`Self::ConfigRead`].
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// A Smooth-pull source (issue #759) declared or exhibited PlayReady/
    /// PIFF sample encryption, which this crate does not decrypt — see
    /// `crate::source::smooth_pull`'s module doc for the detection
    /// heuristics (a manifest `<Protection>` element, or a fragment
    /// carrying CENC/PIFF sample-encryption boxes) and rationale. Returned
    /// instead of silently demuxing garbage (encrypted) sample bytes into
    /// the pipeline.
    #[error("{reason}")]
    Encrypted {
        /// Names the specific limitation/signal that triggered this error.
        reason: String,
    },

    /// A config's `Custom { type_tag, .. }` variant (`InputSpec`/
    /// `OutputKind`/`OutputAuthSpec`, issue #663 external scheme plugin
    /// registry) named a `type_tag` with no matching factory registered in
    /// the [`crate::registry::SchemeRegistry`] passed to
    /// [`crate::Server`] (or in the empty registry
    /// [`crate::Server`] uses).
    #[error("unknown {kind} scheme {tag:?} — no factory registered")]
    UnknownScheme {
        /// Which registry this was: `"input"`, `"output"`, or `"auth"`.
        kind: &'static str,
        /// The unresolved `type_tag`.
        tag: String,
    },

    /// An `InputSpec::File` route's ingest (issue #748 WP5) failed to read,
    /// probe, or demux its media file — surfaced through the route's
    /// supervisor so the route retries rather than dying silently.
    #[error("file route ingest failed: {0}")]
    FileRoute(
        /// The underlying file-reader error (read / probe / demux).
        #[from]
        media_plane::file_source::FileReaderError,
    ),

    /// An output cannot carry a codec the ingested program delivered, so the
    /// output was failed instead of emitting an invalid stream (Apple HLS
    /// Authoring Spec 1.5: HEVC and AV1 are not allowed in MPEG-TS HLS
    /// segments). Known only once the program's tracks arrive.
    #[error("{output} output cannot carry {codec}: {reason}")]
    UnsupportedCodec {
        /// The output kind that rejected the program (`"ts_hls"`).
        output: &'static str,
        /// The offending codec (`"HEVC"`, `"AV1"`).
        codec: &'static str,
        /// Why the output cannot carry it.
        reason: &'static str,
    },

    /// The runtime admin API's `POST /admin/routes`
    /// (`crate::origin::admin::RouteRegistry::add_route`) named a route
    /// already present in the registry — mapped to `409 Conflict`. The
    /// existing route is left completely untouched; adding a route never
    /// implicitly replaces one already live.
    #[error("route {name:?} already exists")]
    RouteExists {
        /// The duplicate route name.
        name: String,
    },

    /// The runtime admin API's `GET`/`DELETE /admin/routes/{name}`
    /// (`crate::origin::admin::RouteRegistry`) named a route not present in
    /// the registry — mapped to `404 Not Found`.
    #[error("route {name:?} not found")]
    RouteNotFound {
        /// The unknown route name.
        name: String,
    },
    /// An [`crate::output::OutputKind`] that has no [`crate::output::Output`]
    /// trait object (push outputs, WHEP, or a `Custom` needing a registry).
    #[error("{kind} output cannot be built into an Output trait object")]
    OutputNotBuildable {
        /// The output kind's name.
        kind: String,
    },
}

/// Which phase of an ingest/protocol exchange failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Phase {
    /// RTSP DESCRIBE.
    Describe,
    /// RTSP SETUP.
    Setup,
    /// RTSP PLAY.
    Play,
    /// Receiving media bytes.
    Recv,
    /// Any other phase (carries its label).
    Other(&'static str),
}

impl Phase {
    /// The wire/label string for this phase.
    pub fn name(&self) -> &'static str {
        match self {
            Phase::Describe => "DESCRIBE",
            Phase::Setup => "SETUP",
            Phase::Play => "PLAY",
            Phase::Recv => "recv",
            Phase::Other(s) => s,
        }
    }
}

broadcast_common::impl_spec_display!(Phase);

/// Why a [`MultimuxError::Protocol`] failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProtocolFailure {
    /// A non-success HTTP/RTSP status code.
    #[error("non-success status {0}")]
    Status(http::StatusCode),
    /// No data arrived within the configured read timeout.
    #[error("no data within {0:?}")]
    RecvTimeout(std::time::Duration),
    /// An RTSP runtime error.
    #[error(transparent)]
    Rtsp(#[from] rtsp_runtime::Error),
    /// An RTMP runtime error.
    #[error(transparent)]
    Rtmp(#[from] rtmp_runtime::RtmpError),
    /// A formatted reason with no typed source.
    #[error("{0}")]
    Other(String),
}

/// Error carrying an SDP rejection reason.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SdpError(pub String);

impl From<String> for SdpError {
    fn from(s: String) -> Self {
        SdpError(s)
    }
}

impl From<&str> for SdpError {
    fn from(s: &str) -> Self {
        SdpError(s.to_string())
    }
}

/// multimux result alias.
pub type Result<T> = core::result::Result<T, MultimuxError>;
