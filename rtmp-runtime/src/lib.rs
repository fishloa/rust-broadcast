#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(any(feature = "tokio", feature = "target", test)), no_std)]

extern crate alloc;

pub mod amf0;
pub mod chunk;
pub mod client;
#[cfg(feature = "tokio")]
mod codec;
pub mod error;
pub mod flv;
pub mod handshake;
#[cfg(feature = "tokio")]
#[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
pub mod io;
pub mod message;
pub mod server;
#[cfg(feature = "target")]
#[cfg_attr(docsrs, doc(cfg(feature = "target")))]
pub mod target;

pub use error::RtmpError;

/// The Adobe RTMP specification version this engine implements.
pub const RTMP_VERSION: &str = "RTMP 1.0";
