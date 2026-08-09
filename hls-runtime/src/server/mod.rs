//! LL-HLS origin engine (issue #663/#717 Stage 2; plan step 4): the
//! blocking-reload + part-availability *decision* logic and playlist
//! rendering, driven directly from a shared `media_plane::Trunk` — not a
//! push-fed rolling-window store of its own.
//!
//! # Sans-IO shape
//!
//! Nothing here ever `.await`s or opens a socket. [`HlsOrigin`] implements
//! [`media_plane::egress::ServedEgress`]: its
//! [`resolve`](media_plane::egress::ServedEgress::resolve) is a poll method
//! returning [`media_plane::egress::EgressResponse`] — `Ready`, `Await`,
//! `BadRequest`, or `NotFound` — never blocking the caller. The only
//! asynchrony is `media_plane::Trunk::listen`, which hands back a
//! runtime-agnostic `event_listener::EventListener` (a plain
//! `Future<Output = ()>`) that *any* executor can await or time out — not a
//! `tokio::sync::watch`.
//!
//! # The caller-driven wait loop
//!
//! An async adapter (e.g. `multimux`'s Step 5 LL-HLS route) turns an `Await`
//! into an actual wait like this — the same shape `MediaStore`'s own
//! (now-deleted) wait loop used, unchanged in spirit:
//!
//! ```text
//! loop {
//!     let listener = trunk.listen(); // register BEFORE re-checking (no missed-wakeup race)
//!     match origin.resolve(request, now, await_policy) {
//!         EgressResponse::Ready { body, .. } => return Ready(body),
//!         EgressResponse::BadRequest { .. } => return BadRequest,
//!         EgressResponse::NotFound => return NotFound,
//!         EgressResponse::Await { .. } => {
//!             // caller's own bounded timeout wraps `listener.await` here
//!         }
//!     }
//! }
//! ```
//!
//! The blocking-reload cap is [`media_plane::egress::AwaitPolicy`]; the
//! actual `.await`/`tokio::time::timeout` lives entirely in the adapter —
//! this module never assumes a clock.
//!
//! # `std`-only
//!
//! Like [`media_plane::Trunk`] itself, this module needs `std::sync::Mutex`,
//! so it is only compiled when this crate's `std` feature is enabled (the
//! default, and the only thing that pulls in the `media-plane` dependency at
//! all — see this crate's `Cargo.toml`). A caller building
//! `--no-default-features` (e.g. an embedded playback-only client) gets
//! [`crate::client`] but not `server`.

mod engine;

pub use engine::{
    BlockingQuery, ClosedSegment, Container, DEFAULT_TRACK_ID, HlsBody, HlsOrigin,
    HlsOriginBuildError, HlsOriginBuilder, HlsRequest, master_playlist_m3u8,
};
