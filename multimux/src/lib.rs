#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub(crate) mod catchup;
pub(crate) mod clock;
pub mod config;
pub mod dashif_ingest;
pub mod dvr;
pub mod error;
mod http;
pub mod origin;
pub mod output;
pub mod prometheus;
pub mod push;
pub mod reconnect;
pub mod redact;
pub mod registry;
pub mod route;
mod secure_random;
pub mod source;
#[cfg(test)]
pub(crate) mod testutil;
#[cfg(any(feature = "whip", feature = "whep"))]
mod webrtc_session;

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
/// See [`origin::serve_config_file_with_registry_on_admin`] — the port-0
/// (SP7.1) variant: a caller binds the listeners and passes them in.
pub use origin::serve_config_file_with_registry_on_admin;
/// See [`origin::serve_config_file_with_registry_on_admin_prebound`] — the
/// same, plus caller-bound route sockets for a *file-defined* route
/// (`test-seams` only).
#[cfg(feature = "test-seams")]
pub use origin::serve_config_file_with_registry_on_admin_prebound;
pub use origin::serve_with_registry;
/// See [`origin::serve_with_registry_on`] — the port-0 (SP7.1) variant.
pub use origin::serve_with_registry_on;
/// See [`origin::serve_with_registry_on_admin`] — the port-0 (SP7.1) variant
/// for a media + admin listener pair.
pub use origin::serve_with_registry_on_admin;
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
pub use route::{AddSegmentError, HealthState, RouteHandle, TsHlsIdrStatus};
pub use source::Source;
