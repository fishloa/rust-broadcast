//! **DEPRECATED — renamed to [`mpeg_pes`].**
//!
//! This crate is a thin re-export shim. Switch your dependency to
//! `mpeg-pes = "0.1"` and replace `dvb_pes::` with `mpeg_pes::`.
#![no_std]
#![allow(deprecated)]
#[deprecated(
    since = "0.1.2",
    note = "renamed to `mpeg-pes`; use `mpeg_pes::` instead"
)]
pub use mpeg_pes::*;
