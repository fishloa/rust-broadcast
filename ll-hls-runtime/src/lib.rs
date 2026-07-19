//! `ll-hls-runtime` — sans-IO Low-Latency HLS (RFC 8216bis) client **and**
//! (soon) server/origin engines, in one crate — mirroring `rtsp-runtime`'s
//! client+server split.
//!
//! This crate is Stage 1 of the ll-hls-runtime unification (issue #717 →
//! `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`, "ll-hls-runtime
//! — client + server in one crate"): a pure rename + restructure of the
//! former standalone `ll-hls-client` crate (zero behaviour change). The LL-HLS
//! origin engine currently living in `multimux` moves into [`server`] in a
//! later stage.
//!
//! # Module map
//!
//! - [`client`] — the LL-HLS playback client engine: [`client::LlHlsClient`],
//!   the sans-IO reload scheduler / fetch pipeline / output adapter (issue
//!   #717 slices 2-4), plus the optional `tokio`-feature
//!   [`client::TokioClient`] async IO adapter (slice 5). See the module docs
//!   for full behaviour.
//! - [`server`] — the LL-HLS origin engine (Stage 2 of the unification — not
//!   yet implemented here).

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod client;
pub mod server;

/// The RFC this crate's client behaviour implements against.
pub const SPEC: &str = "RFC 8216bis (HTTP Live Streaming 2nd Edition, draft-pantos-hls-rfc8216bis)";
