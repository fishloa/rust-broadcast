//! Robust media container-format detection over a byte prefix — issue
//! [#960](https://github.com/fishloa/rust-broadcast/issues/960).
//!
//! Scaffold only. The design this implements is
//! `docs/superpowers/specs/2026-08-11-container-probe-design.md`; the probers,
//! scoring model, and public API land per that spec.
//!
//! `no_std` + `alloc`.

#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;
