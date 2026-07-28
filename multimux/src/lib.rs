//! multimux — a multi-input, multi-output just-in-time repackaging HTTP
//! origin.
//!
//! Pull/receive live media from any of several ingest transports —
//! [`config::InputSpec`]: RTSP pull, raw RTP/UDP, MPEG-TS/UDP, MPEG-TS/HTTP,
//! or HLS-pull — and serve each ingested stream as any combination of
//! [`output::OutputKind`]: Low-Latency HLS, DASH, or LL-DASH, from one
//! in-process tokio + axum HTTP origin. One ingest, many outputs, no
//! per-output re-mux. Built on `rtsp-runtime` (RTSP), `ll-hls-runtime`
//! (LL-HLS client/server engine + HLS-pull), `broadcast-auth` (client and
//! server auth), and `transmux` (RTP/TS depayload + CMAF segmentation +
//! DASH packaging). Muxing only — samples are never transcoded.
//!
//! Third-party crates can add a new input/output/output-auth scheme without
//! editing this crate at all — see [`registry`] (issue #663 external scheme
//! plugin registry) and [`origin::serve_with_registry`].

pub mod config;
pub mod error;
mod http;
pub mod origin;
pub mod output;
pub mod prometheus;
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
