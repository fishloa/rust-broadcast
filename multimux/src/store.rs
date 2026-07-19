//! Re-export of the LL-HLS origin engine's rolling in-RAM window.
//!
//! [`MediaStore`]/[`HealthState`] moved into `ll_hls_runtime::server` (issue
//! #663/#717 Stage 2 —
//! `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`,
//! "ll-hls-runtime — client + server in one crate"), shared with the sans-IO
//! LL-HLS server engine rather than living only in multimux. This module
//! exists so existing `crate::store::...` call sites (the pipeline, the
//! supervisor, `origin`) keep working unchanged; the playlist
//! rendering/blocking-reload decision logic that used to sit alongside
//! `MediaStore` here now lives in `ll_hls_runtime::server` too — see
//! [`crate::output::llhls`] for the thin tokio+axum adapter over it.
pub use ll_hls_runtime::server::{HealthState, MediaStore};
