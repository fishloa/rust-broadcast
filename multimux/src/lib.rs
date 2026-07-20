//! multimux — a live RTSP -> LL-HLS just-in-time repackaging HTTP origin.
//!
//! A thin client+server wrap over `rtsp-runtime` (RTSP pull) and `transmux`
//! (RTP depayload + LL-HLS CMAF segmentation): pull one or more live RTSP
//! sources and serve each as LL-HLS from an in-process tokio + axum origin.
//! Muxing only — samples are never transcoded.
//!
//! Third-party crates can add a new input/output/output-auth scheme without
//! editing this crate at all — see [`registry`] (issue #663 external scheme
//! plugin registry) and [`origin::serve_with_registry`].

pub mod config;
pub mod error;
pub mod origin;
pub mod output;
pub mod pipeline;
pub mod prometheus;
mod redact;
pub mod registry;
pub mod source;
pub mod store;
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
pub use origin::supervisor::{Backoff, SourceConnector, supervise};
pub use output::Output;
pub use registry::{
    AuthCtx, AuthFactory, InputCtx, InputFactory, OutputCtx, OutputFactory, SchemeRegistry,
};
pub use source::Source;
pub use store::MediaStore;
