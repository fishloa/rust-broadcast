//! **DEPRECATED — renamed to [`scte35_splice`](https://crates.io/crates/scte35-splice).**
//!
//! This crate is a thin re-export shim kept so existing `dvb-scte35` dependencies
//! keep building. New code should depend on `scte35-splice` directly. No further
//! feature work lands here.
#![no_std]
#![allow(deprecated)]

pub use scte35_splice::*;
