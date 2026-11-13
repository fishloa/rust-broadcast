//! Smooth Streaming live origin engine (issue #2453): the client Manifest and
//! fragment *decision* logic, driven from a shared `media_plane::Trunk` — the
//! same shape as `hls-runtime`'s `server` module.
//!
//! Nothing here ever `.await`s or opens a socket. [`SmoothOrigin::manifest`]
//! renders the current live Manifest and the `ServedEgress` implementation
//! answers one parsed [`SmoothRequest`] with an
//! [`EgressResponse`](media_plane::egress::EgressResponse) (`Ready`, `Await`,
//! `BadRequest`, `NotFound`) — never blocking the caller. The manifest is
//! built as a `broadcast-smooth` model and rendered by its own `to_xml`; no
//! XML is written here. Each track's codec parameters come from `transmux`'s
//! [`resolve_codec`](transmux::resolve_codec), the same resolver the packager
//! uses, so the manifest advertises exactly what the fragments carry.
//!
//! # The host's bookkeeping, not the origin's
//!
//! Which segments and tracks a program presents is the host's own
//! bookkeeping (the set its DASH window and HLS origin already render from),
//! so the origin asks for both through [`SegmentProvider`] and
//! [`TrackProvider`] rather than caching the media itself. The `Trunk` stays
//! the single copy.
//!
//! # The caller-driven wait loop
//!
//! A Manifest cannot be rendered before the trunk has an init segment, a
//! track set and at least one published segment. The origin answers
//! `EgressResponse::Await` until then; an adapter registers `Trunk::listen`
//! *before* retrying so no publish is missed:
//!
//! ```text
//! loop {
//!     let listener = trunk.listen();
//!     match origin.resolve(request, now, await_policy) {
//!         EgressResponse::Await { .. } => { /* bounded wait on `listener` */ }
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
    FragmentLocation, SegmentProvider, SmoothBody, SmoothOrigin, SmoothOriginError, SmoothRequest,
    StreamKey, TrackProvider,
};
