//! Deprecated — this crate has been renamed to [`ule`].
//!
//! Add `ule` to your `Cargo.toml` instead of `dvb-ule`.
//! This shim re-exports everything from [`ule`] and will be
//! removed in a future release.
#![no_std]
#![allow(deprecated)]
#![deprecated(since = "0.1.1", note = "renamed to `ule`; update your Cargo.toml")]

pub use ule::*;
