//! LL-HLS origin engine (issue #663/#717 Stage 2): the sans-IO rolling
//! window/store, the blocking-reload + part-availability *decision* logic,
//! and playlist rendering, moved out of `multimux` so it can be shared with
//! any async runtime — not just tokio+axum.
//!
//! # Sans-IO shape
//!
//! Nothing here ever `.await`s or opens a socket. [`MediaStore`] is fed
//! synchronously (`add_part`/`add_segment`/`set_init`/`set_health`) by
//! whatever pipeline produces media; [`MediaStore::resolve_playlist`]/
//! [`MediaStore::resolve_resource`] are poll methods returning
//! [`PlaylistOutcome`]/[`ResourceOutcome`] — `Ready`, `WouldBlock`,
//! `BadRequest`, or `NotFound` — never blocking the caller. The only
//! asynchrony is [`MediaStore::listen`], which hands back a runtime-agnostic
//! `event_listener::EventListener` (a plain `Future<Output = ()>`) that *any*
//! executor can await or time out — not a `tokio::sync::watch`.
//!
//! # The caller-driven wait loop
//!
//! An async adapter (e.g. `multimux::output::llhls`) turns a `WouldBlock`
//! into an actual wait like this — the same shape as `client`'s poll/step
//! contract, mirrored on the server side:
//!
//! ```text
//! loop {
//!     let listener = store.listen(); // register BEFORE re-checking (no missed-wakeup race)
//!     match store.resolve_playlist(track_id, query) {
//!         PlaylistOutcome::Ready(body) => return Ready(body),
//!         PlaylistOutcome::BadRequest => return BadRequest,
//!         PlaylistOutcome::WouldBlock => {
//!             // caller's own bounded timeout wraps `listener.await` here
//!         }
//!     }
//! }
//! ```
//!
//! The 5 s blocking-reload cap (RFC 8216bis §6.2.5.2) and the actual
//! `.await`/`tokio::time::timeout` live entirely in the adapter — this module
//! never assumes a clock.
//!
//! # `std`-only
//!
//! Unlike [`crate::client`], this module needs `std::sync::Mutex` (and the
//! `event-listener` crate's `std` feature), so it is only compiled when the
//! crate's `std` feature is enabled (the default). A caller building
//! `--no-default-features` (e.g. an embedded playback-only client) gets
//! [`crate::client`] but not `server`.

mod engine;
mod store;

pub use engine::{
    BlockingQuery, CachePolicy, DEFAULT_TRACK_ID, PlaylistOutcome, ResourceOutcome,
    master_playlist_m3u8, media_playlist_m3u8,
};
pub use store::{HealthState, MediaStore, SegmentWindowEntry};
