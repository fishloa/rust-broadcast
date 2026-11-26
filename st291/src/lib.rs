#![doc = include_str!("../README.md")]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p st291 --example <name>`.\n"]
#![doc = "\n### `build_anc`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_anc.rs")]
#![doc = "```\n\n### `parse_anc`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_anc.rs")]
#![doc = "```\n\n### `build_anc_rtp`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_anc_rtp.rs")]
#![doc = "```\n\n### `parse_anc_rtp`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_anc_rtp.rs")]
#![doc = "```"]

extern crate alloc;

#[cfg(feature = "ts")]
mod anc;
mod anc_content;
#[cfg(feature = "ts")]
mod descriptor;
mod error;
#[cfg(feature = "rtp")]
mod rtp;

#[cfg(feature = "ts")]
pub use anc::{
    ANC_PES_HEADER_DATA_LENGTH, ANC_STREAM_ID, AncDataPacket, AncPacket, PACKET_START_CODE_PREFIX,
    STUFFING_BYTE,
};
pub use anc_content::{AncContent, ValidationReport};
#[cfg(feature = "ts")]
pub use descriptor::{
    ANC_DATA_DESCRIPTOR_TAG, ANC_STREAM_TYPE, AncDataDescriptor, Descriptor,
    VANC_FORMAT_IDENTIFIER, VANC_FORMAT_IDENTIFIER_BYTES,
};
pub use error::{Error, Result};
#[cfg(feature = "rtp")]
pub use rtp::{
    ANC_RTP_DEFAULT_CLOCK_RATE, ANC_RTP_MEDIA_TYPE, ANC_RTP_PAYLOAD_HEADER_LEN, AncRtpPayload,
    ExtendedSequenceNumber, FieldSense, RtpAncPacket,
};

/// Bits per byte, for bit<->byte length conversions.
#[cfg(any(feature = "ts", feature = "rtp"))]
pub(crate) const BITS_PER_BYTE: usize = u8::BITS as usize;

/// Byte width of a 16-bit field.
#[cfg(feature = "rtp")]
pub(crate) const U16_LEN: usize = core::mem::size_of::<u16>();

/// Copy `N` big-endian bytes at `offset` into an array.
#[cfg(feature = "rtp")]
pub(crate) fn be<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N]> {
    bytes
        .get(offset..offset + N)
        .and_then(|s| s.first_chunk::<N>())
        .copied()
        .ok_or(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: offset + N,
                have: bytes.len(),
                what: "fixed-width big-endian field",
            },
        ))
}
