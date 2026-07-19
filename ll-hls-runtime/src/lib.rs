//! `ll-hls-runtime` — sans-IO Low-Latency HLS (RFC 8216bis) client **and**
//! server/origin engines, in one crate — mirroring `rtsp-runtime`'s
//! client+server split.
//!
//! This crate unifies what used to be the standalone `ll-hls-client` crate
//! (Stage 1: a pure rename to `ll-hls-runtime`, zero behaviour change) with
//! the LL-HLS origin engine that used to live in `multimux` (Stage 2: moved
//! into [`server`] — issue #663/#717,
//! `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`,
//! "ll-hls-runtime — client + server in one crate").
//!
//! # Module map
//!
//! - [`client`] — the LL-HLS playback client engine: [`client::LlHlsClient`],
//!   the sans-IO reload scheduler / fetch pipeline / output adapter (issue
//!   #717 slices 2-4), plus the optional `tokio`-feature
//!   [`client::TokioClient`] async IO adapter (slice 5). See the module docs
//!   for full behaviour. No_std-capable (the core needs only `alloc`).
//! - [`server`] (feature `std`) — the LL-HLS origin engine: the rolling
//!   window/store ([`server::MediaStore`]), the blocking-reload + part-
//!   availability decision logic ([`server::MediaStore::resolve_playlist`]/
//!   [`server::MediaStore::resolve_resource`]), playlist rendering, and the
//!   TARGETDURATION/msn/abuse rules — all poll/step, no tokio, no axum. See
//!   the module docs for the caller-driven wait loop an async adapter (e.g.
//!   `multimux`) builds on top.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod client;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub mod server;

/// The RFC this crate's client behaviour implements against.
pub const SPEC: &str = "RFC 8216bis (HTTP Live Streaming 2nd Edition, draft-pantos-hls-rfc8216bis)";
