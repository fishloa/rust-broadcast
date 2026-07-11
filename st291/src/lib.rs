//! SMPTE ST 291-1 — ancillary (ANC) data content.
//!
//! ST 291-1 defines the ANC data packet: the generic carrier for VANC/HANC
//! payloads (captions, AFD, timecode, audio metadata, …) multiplexed into a
//! professional video signal. This crate is about that **content**, not any
//! one carriage mechanism — ST 291-1 packets can be conveyed over more than
//! one transport, and this crate grows to cover each as it is added.
//!
//! ## Transports
//!
//! - **`ts`** (default) — SMPTE ST 2038:2021 carriage of ANC data packets in
//!   an MPEG-2 Transport Stream. ST 2038 provides a transparent pipe so
//!   ST 291-1 ANC data packets can be conveyed frame-accurately alongside the
//!   video they belong to (ST 2038 §1); it is **not** for audio carriage and
//!   **not** for EDH packets (Introduction). This crate implements the two
//!   wire structures defined in ST 2038 §4:
//!   - [`AncDataDescriptor`] — the `anc_data_descriptor` (tag `0xC4`) used in
//!     the PMT ES loop, plus the `"VANC"` `registration_descriptor`
//!     [`format_identifier`](VANC_FORMAT_IDENTIFIER) `0x56414E43` and the
//!     [`ANC_STREAM_TYPE`] `0x06` (§4.1, Table 1).
//!   - [`AncDataPacket`] — the ANC data PES packet (`stream_id == 0xBD`, PTS,
//!     `PES_header_data_length == 0x05`) carrying a list of bit-packed
//!     [`AncPacket`] records + trailing `0xFF` stuffing (§4.2, Table 2).
//! - A future `rtp` transport (ST 2110-40 / RFC 8331) will carry the same
//!   [`AncPacket`] content over RTP; see issue #648.
//!
//! The per-ANC-packet fields (`DID`/`SDID`/`data_count`/`user_data_word`/
//! `checksum_word`) are a **contiguous MSB-first 10-bit bit stream**, walked
//! with [`broadcast_common::bits`]. Per ST 2038 §4.2.1 the `user_data_word`
//! loop counter uses only the **low 8 bits** of `data_count`; the full 10-bit
//! values are stored verbatim and ST 291-1 parity/checksum is **not**
//! validated here (ST 2038 defers it to ST 291-1, which is not vendored).
//!
//! Depends only on `broadcast-common` and is `#![no_std]` (+ `alloc`). The PES
//! header is parsed inline (every field is fixed by ST 2038 Table 2, so the
//! dedicated `mpeg-pes` parser adds a dependency without simplifying the
//! bit-packed payload walk).
//!
//! # Examples
//!
//! Build an ANC PES packet from typed fields and round-trip it:
//!
//! ```
//! use st291::{AncDataPacket, AncPacket};
//!
//! let pkt = AncDataPacket {
//!     pes_priority: false,
//!     copyright: false,
//!     original_or_copy: false,
//!     pts: 90_000,
//!     anc_packets: vec![AncPacket {
//!         c_not_y_channel_flag: false,
//!         line_number: 9,
//!         horizontal_offset: 0,
//!         did: 0x161,
//!         sdid: 0x101,
//!         data_count: 0x002,
//!         user_data_words: vec![0x2CF, 0x101],
//!         checksum: 0x233,
//!     }],
//!     stuffing_bytes: 0,
//! };
//! let bytes = {
//!     let mut b = vec![0u8; pkt.serialized_len()];
//!     pkt.serialize_into(&mut b).unwrap();
//!     b
//! };
//! assert_eq!(AncDataPacket::parse(&bytes).unwrap(), pkt);
//! ```
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
#![doc = "```"]

extern crate alloc;

#[cfg(feature = "ts")]
mod anc;
#[cfg(feature = "ts")]
mod descriptor;
mod error;

#[cfg(feature = "ts")]
pub use anc::{
    ANC_PES_HEADER_DATA_LENGTH, ANC_STREAM_ID, AncDataPacket, AncPacket, PACKET_START_CODE_PREFIX,
    STUFFING_BYTE,
};
#[cfg(feature = "ts")]
pub use descriptor::{
    ANC_DATA_DESCRIPTOR_TAG, ANC_STREAM_TYPE, AncDataDescriptor, VANC_FORMAT_IDENTIFIER,
    VANC_FORMAT_IDENTIFIER_BYTES,
};
pub use error::{Error, Result};
