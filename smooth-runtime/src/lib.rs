#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Sans-IO Smooth Streaming (MS-SSTR) client engine (issue #2370).
//!
//! One engine owns everything a Smooth pull client does between bytes: manifest
//! fetch and live refresh, `<Protection>` refusal, `StreamIndex` identity across a
//! refresh, stream/quality selection, chunk enumeration, fragment URL resolution,
//! first-fragment track-id discovery, the synthetic init segment, the `tfxd`
//! timestamp base, the encrypted-fragment probe, retry/backoff, live join
//! look-ahead and discontinuity. Manifest syntax is `broadcast-smooth`'s; codec
//! glue and fMP4 demux are `transmux`'s.
//!
//! The shape follows `dash-runtime`'s `DashClient`: the host drives it with
//! [`SmoothClient::poll`] (what to fetch next), [`SmoothClient::on_manifest`],
//! [`SmoothClient::on_fragment`] and [`SmoothClient::on_failure`], and drains
//! [`SmoothClient::next_output`]. Every instant is an argument in microseconds; no
//! I/O happens and no clock is read inside the crate.
//!
//! The wire syntax is MS-SSTR, transcribed in
//! `private/specs-verified/mpeg/transmux/smooth/ms-sstr.md`.

pub mod client;
pub mod error;

pub use client::{
    Action, Dropped, FetchFailure, ManifestFault, Output, QualityInfo, QualitySelector, RequestId,
    SmoothClient, SmoothConfig, StreamId,
};
pub use error::SmoothError;
