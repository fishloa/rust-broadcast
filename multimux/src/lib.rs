//! multimux — a live RTSP -> LL-HLS just-in-time repackaging HTTP origin.
//!
//! A thin client+server wrap over `rtsp-runtime` (RTSP pull) and `transmux`
//! (RTP depayload + LL-HLS CMAF segmentation): pull one or more live RTSP
//! sources and serve each as LL-HLS from an in-process tokio + axum origin.
//! Muxing only — samples are never transcoded.

pub mod config;
pub mod error;
pub mod origin;
pub mod output;
pub mod pipeline;
mod redact;
pub mod source;
pub mod store;

pub use error::{MultimuxError, Result};
