//! Smooth Streaming live origin engine (issue #2453): the client Manifest and
//! fragment *decision* logic, driven directly from a shared
//! `media_plane::Trunk` — the same shape as `hls-runtime`'s `server` module.
//!
//! Nothing here ever `.await`s or opens a socket. [`SmoothOrigin::manifest`]
//! renders the current live Manifest and [`SmoothOrigin::resolve`] answers one
//! parsed [`SmoothRequest`] with an [`EgressResponse`] (`Ready`, `Wait`,
//! `BadRequest`, `NotFound`) — never blocking the caller. The manifest is
//! built as a `broadcast-smooth` model and rendered by
//! [`SmoothManifest::to_xml`]; no XML is written here. Each track's codec
//! parameters come from `transmux`'s [`resolve_codec`](transmux::resolve_codec),
//! the same resolver the packager uses, so the manifest advertises exactly what
//! the fragments carry.
//!
//! # One cursor, not a cache of the window
//!
//! A `Trunk` is the single copy of the media. This origin keeps **one**
//! [`SegmentCursor`] (a cursor's cost is per consumer, never per peer) and a
//! window of the entries already drained through it, so a fragment request is
//! answered from bytes the trunk produced rather than a second segment store.
//!
//! # The caller-driven wait loop
//!
//! A Manifest cannot be rendered before the trunk has an init segment and at
//! least one closed segment. [`SmoothOrigin::resolve`] answers
//! [`EgressResponse::Wait`] until then; an adapter registers
//! `Trunk::listen` *before* retrying so no publish is missed:
//!
//! ```text
//! loop {
//!     let listener = trunk.listen();
//!     match origin.resolve(request, now, await_policy) {
//!         EgressResponse::Wait { .. } => { /* bounded wait on `listener` */ }
//!         answer => return answer,
//!     }
//! }
//! ```
//!
//! # `std`-only
//!
//! Like `media_plane::Trunk` itself, this module needs `std::sync::Mutex`, so
//! it is compiled only when this crate's `std` feature is enabled (the
//! default). A `--no-default-features` build gets [`crate::client`] alone.

mod engine;

pub use engine::{
    FragmentLocation, SmoothBody, SmoothOrigin, SmoothOriginError, SmoothRequest, StreamKey,
};
