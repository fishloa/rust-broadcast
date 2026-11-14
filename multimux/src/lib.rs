#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub(crate) mod catchup;
pub(crate) mod clock;
pub mod config;
pub mod dvr;
pub mod error;
mod http;
#[cfg(feature = "test-seams")]
#[doc(hidden)]
pub mod ingest_golden;
pub mod origin;
pub mod output;
pub mod prometheus;
pub mod push;
pub mod redact;
pub mod registry;
pub mod route;
mod secure_random;
pub mod source;
#[cfg(test)]
pub(crate) mod testutil;
#[cfg(any(feature = "whip", feature = "whep"))]
mod webrtc_session;
#[cfg(any(feature = "test-seams", feature = "test-hooks"))]
#[doc(hidden)]
pub mod test_seam {
    #[cfg(feature = "test-seams")]
    #[doc(hidden)]
    pub use crate::source::rtmp::serve_for_test_with_read_load as serve_rtmp_for_test_with_read_load;
    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub use crate::source::whip::{
        parse_offer_for_test, render_answer_for_test, serve_for_test as serve_whip_for_test,
        serve_for_test_with_read_load as serve_whip_for_test_with_read_load,
        serve_for_test_with_read_load_and_max_sessions,
        serve_for_test_with_read_load_and_timeout,
        serve_for_test_with_read_load_timeout_and_policy, serve_for_test_with_read_timeout,
        whip_router_for_test,
    };
    #[cfg(all(any(feature = "whip", feature = "whep"), feature = "test-hooks"))]
    #[doc(hidden)]
    pub use crate::webrtc_session::restart_fragment_for_test;

    #[cfg(feature = "test-seams")]
    #[doc(hidden)]
    pub use crate::push::srt::parse_srt_url_for_test;
    #[cfg(feature = "test-seams")]
    #[doc(hidden)]
    pub use crate::push::rtmp::tc_url_for_test;
    #[cfg(feature = "test-seams")]
    #[doc(hidden)]
    pub use crate::push::rtsp::{control_url_for_test, push_media_for_test, push_sdp_for_test};
    #[cfg(feature = "whep")]
    #[doc(hidden)]
    pub use crate::output::whep;
}

pub use error::{MultimuxError, Result};
// Re-exported so an external crate wiring up a `crate::registry::SchemeRegistry`
// factory (issue #663) has everything it needs at the crate root, without
// reaching into internal module paths:
/// The shared multi-scheme HTTP/RTSP auth model — re-exported so a
/// registered `AuthFactory` can build a `broadcast_auth::Verifier` (its
/// return type) without an external crate needing its own direct dependency
/// on `broadcast-auth`.
pub use broadcast_auth;
/// What [`Server::new`] takes: the config, the registry, and (test-only)
/// pre-bound listeners.
pub use origin::ServeRequest;
/// The one way to run the origin: [`Server::new`] takes a
/// [`origin::ServeRequest`] (an in-memory config, or `from_json_file` so
/// `POST /admin/reload` keeps working, plus its [`SchemeRegistry`]).
pub use origin::Server;
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
pub use route::{AddSegmentError, HealthState, RouteConfig, RouteHandle, TsHlsIdrStatus};
pub use source::Source;
