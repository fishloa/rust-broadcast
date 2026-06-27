//! Deprecated — this crate has been renamed to [`smpte2038`].
//!
//! Add `smpte2038` to your `Cargo.toml` instead of `dvb-smpte2038`.
//! This shim re-exports everything from [`smpte2038`] and will be
//! removed in a future release.
#![no_std]
#![allow(deprecated)]
#![deprecated(
    since = "0.1.1",
    note = "renamed to `smpte2038`; update your Cargo.toml"
)]

pub use smpte2038::*;
