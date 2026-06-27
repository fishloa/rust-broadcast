//! **Deprecated** — use [`cc_data`] instead.
//!
//! This crate is a thin compatibility shim. Everything it exported is now
//! published under the crate name [`cc-data`](https://crates.io/crates/cc-data).
//! The API is 100 % identical; migrate by replacing `dvb-cc` with `cc-data` in
//! your `Cargo.toml` and renaming `dvb_cc::` to `cc_data::` in your source.
#![no_std]
#![allow(deprecated)]
#![doc(hidden)]

pub use cc_data::*;
