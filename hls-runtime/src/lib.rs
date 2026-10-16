#![doc = include_str!("../README.md")]

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod client;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub mod server;

/// The RFC this crate's client behaviour implements against.
pub const SPEC: &str = "RFC 8216bis (HTTP Live Streaming 2nd Edition, draft-pantos-hls-rfc8216bis)";
