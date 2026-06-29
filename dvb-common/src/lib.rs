//! **DEPRECATED — renamed to [`broadcast_common`](https://crates.io/crates/broadcast-common).**
//!
//! This crate is a thin re-export shim kept so existing `dvb-common` dependencies
//! keep building. New code should depend on `broadcast-common` directly. No further
//! feature work lands here.
#![no_std]
#![allow(deprecated)]

pub use broadcast_common::*;
