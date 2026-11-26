//! Codec bitstream parsers and stream-format conversion for `rust-broadcast`.
//!
//! The crate reads codec **facts** only — it never decodes or encodes media.
//! This first layer is what everything else stands on:
//!
//! - [`bitreader`] — the exp-Golomb (`ue(v)`/`se(v)`) reader over an RBSP byte
//!   stream, built on [`broadcast_core::bits`].
//! - [`rbsp`] — RBSP emulation-prevention byte removal and insertion
//!   (H.264 §7.4.1 / H.265 §7.4.2).
//!
//! `no_std` + `alloc`. It depends on `broadcast-core` and `thiserror` only;
//! later codec layers (parameter sets, slice headers, audio headers) arrive one
//! feature at a time on top of it.

#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod bitreader;
mod error;
pub mod rbsp;

pub use error::{Error, Result};
