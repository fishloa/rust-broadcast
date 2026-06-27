//! **DEPRECATED — renamed to [`mp4_emsg`].**
//!
//! Thin re-export shim kept so existing `dvb-emsg` dependencies keep building.
//! New code should depend on `mp4-emsg` directly.
#![no_std]
#![allow(deprecated)]

pub use mp4_emsg::*;
