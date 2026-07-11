//! RTP fixed header + CSRC list + generic header extension — RFC 3550 §5.1 /
//! §5.3.1, spec-complete (not just a happy-path subset).
//!
//! This crate implements exactly the wire structures described in the curated
//! spec transcription at `rtp-packet/docs/rtp-header.md` (fetched directly
//! from [RFC 3550](https://www.rfc-editor.org/rfc/rfc3550.txt)) — cite that
//! file, not this doc comment, as the field-semantics oracle.
//!
//! - [`RtpPacket`] — the §5.1 fixed header (version/padding/extension bit/
//!   CSRC-count/marker/payload-type/sequence-number/timestamp/SSRC), the CSRC
//!   identifier list (0–15 entries), the optional §5.3.1 header extension, an
//!   optional trailing padding region, and the payload.
//! - [`HeaderExtension`] — the §5.3.1 generic header extension: a 16-bit
//!   profile-specific identifier + opaque profile-specific data.
//!
//! `version`/`P`/`X`/`CC` are never stored as independent fields that could
//! disagree with the typed data: `version` is fixed at 2 by the spec (checked
//! on parse, always written on serialize), and `P`/`X`/`CC` are derived from
//! `padding.is_some()` / `extension.is_some()` / `csrc.len()` respectively —
//! see [`RtpPacket`]'s doc for the reasoning.
//!
//! Depends only on `broadcast-common`. `#![no_std]` (+ `alloc`) when the
//! `std` feature is disabled.
//!
//! The optional `rfc8285` feature adds [`rfc8285`], a decoder for [RFC
//! 8285](https://www.rfc-editor.org/rfc/rfc8285.txt)'s one-byte/two-byte
//! multiplexed extension elements that a profile may pack into
//! [`HeaderExtension::data`] — see `rtp-packet/docs/rfc8285_header_ext.md`
//! for the curated transcription. It is additive and off by default: most
//! RTP consumers only need the RFC 3550 fixed header.
//!
//! # Examples
//!
//! Build a simple packet (no padding/CSRC/extension) and round-trip it:
//!
//! ```
//! use broadcast_common::{Parse, Serialize};
//! use rtp_packet::RtpPacket;
//!
//! let pkt = RtpPacket {
//!     marker: true,
//!     payload_type: 96,
//!     sequence_number: 1,
//!     timestamp: 3600,
//!     ssrc: 0x1234_5678,
//!     csrc: vec![],
//!     extension: None,
//!     padding: None,
//!     payload: &[0xDE, 0xAD, 0xBE, 0xEF],
//! };
//! let mut bytes = vec![0u8; pkt.serialized_len()];
//! pkt.serialize_into(&mut bytes).unwrap();
//! assert_eq!(RtpPacket::parse(&bytes).unwrap(), pkt);
//! ```
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p rtp-packet --example <name>`.\n"]
#![doc = "\n### `build_packet`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_packet.rs")]
#![doc = "```\n\n### `parse_packet`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_packet.rs")]
#![doc = "```\n\n### `rfc8285_extensions` (requires `--features rfc8285`)\n\n```rust,ignore"]
#![doc = include_str!("../examples/rfc8285_extensions.rs")]
#![doc = "```"]

extern crate alloc;

mod error;
mod header;
#[cfg(feature = "rfc8285")]
pub mod rfc8285;

pub use error::{Error, Result};
pub use header::{
    FIXED_HEADER_LEN, HeaderExtension, MAX_CSRC_COUNT, MAX_PADDING_COUNT, MAX_PAYLOAD_TYPE,
    RTP_VERSION, RtpPacket,
};
