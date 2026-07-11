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
//! - **`rtp`** — RFC 8331 / ST 2110-40 carriage of ANC data packets over RTP
//!   (issue #648). [`AncRtpPayload`] is the §2.1 payload (`Extended Sequence
//!   Number`/`Length`/`ANC_Count`/[`FieldSense`]/`reserved` + a list of
//!   [`RtpAncPacket`]s), riding on an `rtp_packet::RtpPacket`'s payload (the
//!   RTP fixed header, RFC 3550, is the `rtp-packet` crate's responsibility).
//!   See `docs/anc_rtp_8331.md`.
//!
//! The per-ANC-packet content — `DID`/`SDID`/`data_count`/`user_data_word`/
//! `checksum_word` — is a **contiguous MSB-first 10-bit bit stream**, walked
//! with [`broadcast_common::bits`], and is byte-for-byte **identical across
//! both transports**: it lives in the always-compiled [`AncContent`] type
//! (gated behind neither `ts` nor `rtp`), wrapped by each transport's own
//! placement fields ([`AncPacket`]'s three for ST 2038, [`RtpAncPacket`]'s
//! five for RFC 8331). Per §4.2.1/§2.1 the `user_data_word` loop counter uses
//! only the **low 8 bits** of `data_count`; the full 10-bit values are stored
//! verbatim and ST 291-1 parity/checksum is **not** validated (deferred to
//! ST 291-1, which is not vendored — see `docs/anc_packet_291.md`).
//!
//! Depends only on `broadcast-common` (plus `rtp-packet`, optionally, for the
//! `rtp` feature) and is `#![no_std]` (+ `alloc`). The ST 2038 PES header is
//! parsed inline (every field is fixed by ST 2038 Table 2, so the dedicated
//! `mpeg-pes` parser adds a dependency without simplifying the bit-packed
//! payload walk).
//!
//! # Examples
//!
//! Build an ANC PES packet from typed fields and round-trip it:
//!
//! ```
//! # #[cfg(feature = "ts")]
//! # fn main() {
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
//! # }
//! # #[cfg(not(feature = "ts"))]
//! # fn main() {}
//! ```
//!
//! Build an ANC-over-RTP payload from typed fields and round-trip it:
//!
//! ```
//! # #[cfg(feature = "rtp")]
//! # fn main() {
//! use broadcast_common::{Parse, Serialize};
//! use st291::{AncContent, AncRtpPayload, FieldSense, RtpAncPacket};
//!
//! let payload = AncRtpPayload {
//!     extended_sequence_number: 0,
//!     field_sense: FieldSense::ProgressiveOrUnspecified,
//!     anc_packets: vec![RtpAncPacket {
//!         c: false,
//!         line_number: 9,
//!         horizontal_offset: 0,
//!         s: false,
//!         stream_num: 0,
//!         content: AncContent {
//!             did: 0x161,
//!             sdid: 0x101,
//!             data_count: 0x002,
//!             user_data_words: vec![0x2CF, 0x101],
//!             checksum: 0x233,
//!         },
//!     }],
//! };
//! let bytes = {
//!     let mut b = vec![0u8; payload.serialized_len()];
//!     payload.serialize_into(&mut b).unwrap();
//!     b
//! };
//! assert_eq!(AncRtpPayload::parse(&bytes).unwrap(), payload);
//! # }
//! # #[cfg(not(feature = "rtp"))]
//! # fn main() {}
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
pub use anc_content::AncContent;
#[cfg(feature = "ts")]
pub use descriptor::{
    ANC_DATA_DESCRIPTOR_TAG, ANC_STREAM_TYPE, AncDataDescriptor, VANC_FORMAT_IDENTIFIER,
    VANC_FORMAT_IDENTIFIER_BYTES,
};
pub use error::{Error, Result};
#[cfg(feature = "rtp")]
pub use rtp::{
    ANC_RTP_DEFAULT_CLOCK_RATE, ANC_RTP_MEDIA_TYPE, ANC_RTP_PAYLOAD_HEADER_LEN, AncRtpPayload,
    FieldSense, RtpAncPacket,
};
