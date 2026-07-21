//! LL-HLS playback client engine — a sans-IO Low-Latency HLS (RFC 8216bis)
//! client (issue #717, slices 2-4; formerly the standalone `ll-hls-client`
//! crate, folded in here as `ll-hls-runtime`'s `client` module — see
//! `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`).
//!
//! [`LlHlsClient`] is a driveable, caller-driven state machine — the same
//! sans-IO shape as `srt-runtime` (issue #565): the core never opens a socket
//! or reads a clock. The caller drains [`Action`]s (what to fetch, and how),
//! performs the IO itself, and feeds the response back in via
//! [`LlHlsClient::on_playlist`] / [`LlHlsClient::on_resource`] /
//! [`LlHlsClient::on_error`]; decoded [`Output`]s (the init segment, then
//! ordered access units) drain out via [`LlHlsClient::next_output`].
//!
//! # Reuse, not re-description
//!
//! This module defines **no playlist model of its own**. Parsing is
//! [`transmux::hls::MediaPlaylist::parse`] (issue #717 slice 1 — the
//! symmetric inverse of the LL-HLS origin's own `to_m3u8()` renderer, so
//! origin and client share one wire model); demuxing a fetched CMAF part or
//! segment into access units is [`transmux::Fmp4Demux`]. `client` holds only
//! the client **engine**: the reload scheduler, the fetch pipeline
//! (prefetch/byte-range/dedup), and the output ordering — see
//! [`LlHlsClient`]'s docs for the full behaviour.
//!
//! # Zero IO in the core
//!
//! No `tokio`/`reqwest`/socket dependency in [`LlHlsClient`] itself, ever —
//! it is driveable by hand (see the crate's `tests/origin_loop.rs` for a
//! complete in-process example against a real
//! `transmux::ll_hls::LlHlsSegmenter` origin) with zero IO dependencies at
//! all (verified by the `--no-default-features` gate).
//!
//! # The `tokio` feature (issue #717 slice 5)
//!
//! Enabling the `tokio` cargo feature (NOT default) adds
//! [`tokio_client::TokioClient`], a thin async shell (tokio + reqwest/rustls)
//! that drives [`LlHlsClient`] over real HTTP — blocking-reload/preload-hint
//! query params, byte-range `Range` headers, per-request timeouts, and
//! retry/backoff on transient failures. See [`tokio_client`]'s module docs
//! for the full behaviour and its `tests/glass_to_glass.rs` for a
//! loopback-HTTP, sub-second glass-to-glass proof against a real
//! `multimux`-served LL-HLS origin. This feature is entirely additive — the
//! sans-IO core above is completely unaffected by it either way.
//!
//! # Example
//!
//! ```
//! use ll_hls_runtime::client::{Action, LlHlsClient};
//!
//! let mut client = LlHlsClient::new("http://origin/live/stream.m3u8");
//! // The caller performs this GET; here we just inspect the first action.
//! match client.poll() {
//!     Some(Action::FetchPlaylist { url, blocking, .. }) => {
//!         assert_eq!(url, "http://origin/live/stream.m3u8");
//!         assert!(blocking.is_none()); // nothing fetched yet, so no LL info.
//!     }
//!     other => panic!("unexpected first action: {other:?}"),
//! }
//! ```

mod action;
mod engine;
mod error;
mod output;
#[cfg(feature = "tokio")]
pub mod tokio_client;
mod url;

pub use action::{Action, BlockingReload, ResourceId};
pub use engine::LlHlsClient;
pub use error::{Error, Result};
pub use output::Output;
#[cfg(feature = "tokio")]
pub use tokio_client::{TokioClient, TokioClientConfig, TokioClientStats, TokioError};
