//! multimux — a multi-input, multi-output just-in-time repackaging HTTP
//! origin.
//!
//! Pull or receive live media over any of nine ingest transports —
//! [`config::InputSpec`]: RTSP pull, raw RTP/UDP, MPEG-TS/UDP, MPEG-TS/HTTP,
//! SRT, HLS pull, DASH pull, Smooth pull, and RTMP push — and serve each
//! ingested stream as any combination of [`output::OutputKind`]: Low-Latency
//! HLS, DASH, or LL-DASH, from one in-process tokio + axum HTTP origin. One
//! ingest, many outputs, no per-output re-mux. Muxing only — samples are
//! never transcoded.
//!
//! Every input runs on **one** ingest architecture: `media-plane`'s ingress
//! contracts (`Dialer` for the eight that dial out, `Listener` for RTMP's
//! push accept) driven by its `IngestDriver`/`ListenDriver`, publishing
//! samples into a per-program `Trunk` that egress serves from. Issue #805
//! converged the last holdout onto this, so there is no second path a sample
//! can take from socket to segment.
//!
//! Also built on `rtsp-runtime` (RTSP), `rtmp-runtime` (RTMP), `srt-runtime`
//! (SRT), `hls-runtime` (LL-HLS client/server engine + HLS pull),
//! `broadcast-auth` (client and server auth), and `transmux` (RTP/TS
//! depayload + CMAF segmentation + DASH packaging).
//!
//! Third-party crates can add a new input/output/output-auth scheme without
//! editing this crate at all — see [`registry`] (issue #663 external scheme
//! plugin registry) and [`origin::serve_with_registry`].

pub(crate) mod catchup;
pub mod config;
pub mod dvr;
pub mod error;
mod http;
pub mod origin;
pub mod output;
pub mod prometheus;
pub mod push;
mod redact;
pub mod registry;
pub mod route;
pub mod source;
#[cfg(test)]
pub(crate) mod testutil;

pub use error::{MultimuxError, Result};
// Re-exported so an external crate wiring up a `crate::registry::SchemeRegistry`
// factory (issue #663) has everything it needs at the crate root, without
// reaching into internal module paths:
/// The shared multi-scheme HTTP/RTSP auth model — re-exported so a
/// registered `AuthFactory` can build a `broadcast_auth::Verifier` (its
/// return type) without an external crate needing its own direct dependency
/// on `broadcast-auth`.
pub use broadcast_auth;
pub use origin::serve;
/// See [`origin::serve_config_file`] (issue #749: needed so
/// `POST /admin/reload` has a config file path to re-read).
pub use origin::serve_config_file;
/// See [`origin::serve_config_file_with_registry`].
pub use origin::serve_config_file_with_registry;
pub use origin::serve_with_registry;
/// [`supervisor::supervise_driver`](origin::supervisor::supervise_driver) is
/// the one supported way to drive a [`registry::InputFactory`]'s ingest task
/// (issue #805 task 5 deleted the old `SourceConnector`/`supervise` pair, the
/// last user of which was a `Custom`-scheme factory) — see
/// `examples/custom_scheme.rs` for a complete external input scheme built on
/// it over a small `media_plane::ingress::Dialer`/`IngestSession` of its own.
pub use origin::supervisor::{Backoff, supervise_driver};
pub use output::Output;
pub use registry::{
    AuthCtx, AuthFactory, InputCtx, InputFactory, OutputCtx, OutputFactory, SchemeRegistry,
};
pub use route::{HealthState, RouteHandle};
pub use source::Source;
