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
mod damage;
pub mod error;
pub mod flv;
pub mod handshake;
pub mod ids;
#[cfg(feature = "tokio")]
#[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
pub mod io;
pub mod message;
pub mod server;
#[cfg(feature = "target")]
#[cfg_attr(docsrs, doc(cfg(feature = "target")))]
pub mod target;

pub use error::RtmpError;
pub use message::ChunkSize;

/// The Adobe RTMP specification version this engine implements.
pub const RTMP_VERSION: &str = "RTMP 1.0";

/// Read a big-endian 32-bit field at `offset` of `bytes`.
pub(crate) fn be_u32(bytes: &[u8], offset: usize) -> u32 {
    const FIELD_LEN: usize = core::mem::size_of::<u32>();
    let mut field = [0_u8; FIELD_LEN];
    field.copy_from_slice(&bytes[offset..offset + FIELD_LEN]);
    u32::from_be_bytes(field)
}

/// Read a little-endian 32-bit field at `offset` of `bytes`.
pub(crate) fn le_u32(bytes: &[u8], offset: usize) -> u32 {
    const FIELD_LEN: usize = core::mem::size_of::<u32>();
    let mut field = [0_u8; FIELD_LEN];
    field.copy_from_slice(&bytes[offset..offset + FIELD_LEN]);
    u32::from_le_bytes(field)
}
