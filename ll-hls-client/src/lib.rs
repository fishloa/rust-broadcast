//! `ll-hls-client` — a sans-IO Low-Latency HLS (RFC 8216bis) playback client
//! engine (issue #717, slices 2-4).
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
//! This crate defines **no playlist model of its own**. Parsing is
//! [`transmux::hls::MediaPlaylist::parse`] (issue #717 slice 1 — the
//! symmetric inverse of the LL-HLS origin's own `to_m3u8()` renderer, so
//! origin and client share one wire model); demuxing a fetched CMAF part or
//! segment into access units is [`transmux::Fmp4Demux`]. `ll-hls-client`
//! holds only the client **engine**: the reload scheduler, the fetch
//! pipeline (prefetch/byte-range/dedup), and the output ordering — see
//! [`LlHlsClient`]'s docs for the full behaviour.
//!
//! # Zero IO in the core
//!
//! No `tokio`/`reqwest`/socket dependency anywhere in this crate. A real
//! deployment wraps [`LlHlsClient`] in an HTTP-fetching loop (planned as
//! issue #717 slice 5, mirroring `srt-runtime`'s `io` module / `rtsp-runtime`'s
//! `tokio` feature) — until then, drive it by hand (see the crate's
//! `tests/origin_loop.rs` for a complete in-process example against a real
//! `transmux::ll_hls::LlHlsSegmenter` origin).
//!
//! # Example
//!
//! ```
//! use ll_hls_client::{Action, LlHlsClient};
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

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

mod action;
mod client;
mod error;
mod output;
mod url;

pub use action::{Action, BlockingReload, ResourceId};
pub use client::LlHlsClient;
pub use error::{Error, Result};
pub use output::Output;

/// The RFC this crate's client behaviour implements against.
pub const SPEC: &str = "RFC 8216bis (HTTP Live Streaming 2nd Edition, draft-pantos-hls-rfc8216bis)";
