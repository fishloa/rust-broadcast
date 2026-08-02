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

    /// An RTSP protocol exchange failed (e.g. DESCRIBE/SETUP/PLAY returned a
    /// non-2xx status, or the server's response was unusable).
    #[error("{phase} failed: {reason}")]
    Protocol {
        /// Which RTSP phase failed (e.g. `"DESCRIBE"`, `"SETUP"`, `"PLAY"`).
        phase: &'static str,
        /// Why it failed.
        reason: String,
    },

    /// The DESCRIBE SDP body could not be parsed, or described media this
    /// crate doesn't support.
    #[error("SDP error: {reason}")]
    Sdp {
        /// Why the SDP was rejected.
        reason: String,
    },

    /// Credential extraction/decoding failed, or the source rejected the
    /// configured credentials (e.g. a `401` that persisted after the
    /// engine's own challenge/response retry).
    #[error("authentication error: {reason}")]
    Auth {
        /// Why authentication failed. Never includes the raw credential.
        reason: String,
    },

    /// Ingest-side framing of samples out of the wire container failed:
    /// either RTP depayloading into access units, or an ingest container
    /// demux (e.g. the FLV tag stream a `rtmp::RtmpRoute` publisher
    /// carries — `transmux::flv::FlvError`) — distinct from a *downstream*
    /// `transmux` segmentation/mux error (see [`Self::Transmux`]), which
    /// this crate never produces on the ingest path.
    #[error("depayload error: {reason}")]
    Depay {
        /// The underlying depayload/demux failure.
        reason: String,
    },

    /// A `transmux` segmentation/mux error.
    #[error("transmux: {0}")]
    Transmux(#[from] transmux::Error),

    /// An `hls_runtime::client::HlsClient` (`source::hls_pull`, plan
    /// step 5a round 3) rejected a fed playlist/resource — malformed
    /// playlist text, a demux failure, or a resource delivered for an id it
    /// never requested (a driver bug).
    #[error("hls-pull client: {0}")]
    LlHls(#[from] hls_runtime::client::Error),

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
    /// [`crate::origin::serve_with_registry`] (or in the empty registry
    /// [`crate::origin::serve`] uses).
    #[error("unknown {kind} scheme {tag:?} — no factory registered")]
    UnknownScheme {
        /// Which registry this was: `"input"`, `"output"`, or `"auth"`.
        kind: &'static str,
        /// The unresolved `type_tag`.
        tag: String,
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
}

/// multimux result alias.
pub type Result<T> = core::result::Result<T, MultimuxError>;
