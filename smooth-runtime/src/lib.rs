#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Sans-IO Smooth Streaming (MS-SSTR) client engine (issue #2370).
//!
//! Caller-driven: `poll(now_us) -> Option<Action>` (fetch manifest / fetch fragment
//! with a typed request id), `on_manifest(now_us, bytes)`, `on_fragment(now_us, id, bytes)`,
//! failure entry points, and `next_output() -> Option<Output>` (tracks/init, samples,
//! discontinuity, typed drops and fatals). Modelled on `dash-runtime`'s `DashClient`
//! for naming and shape.
//!
//! No I/O and no clock reads inside the crate.

pub mod client;
pub mod error;

pub use client::SmoothClient;
pub use error::SmoothError;
