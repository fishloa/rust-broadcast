//! SMPTE ST 291-1 ancillary (ANC) data packets, parsed and serialized as typed
//! structs for their two transports: MPEG-2 Transport Stream carriage (SMPTE
//! ST 2038:2021, feature `ts`) and RTP carriage (RFC 8331 / SMPTE ST 2110-40,
//! feature `rtp`).
//!
//! # What it does / what it does not
//!
//! ST 291-1 defines the ANC data packet: the generic carrier for VANC/HANC
//! payloads (captions, AFD, timecode, audio metadata, ...) multiplexed into a
//! professional video signal. This crate is about that **content**, not any one
//! carriage mechanism.
//!
//! - It parses and serializes the wire structures byte-exact, and exposes the
//!   ST 291-1 10-bit parity rule and `Checksum_Word` computation as explicit
//!   helpers ([`AncContent::with_parity`], [`AncContent::compute_checksum`],
//!   [`AncContent::verify_checksum`], [`AncContent::build`]).
//! - It does **not** interpret payloads: `DID`/`SDID` and user data words are
//!   opaque (no CEA-708 / AFD / ST 12-2 / SCTE 104 decoding).
//! - It does **not** validate parity/checksum on parse; a bit-flipped packet
//!   still parses `Ok` and the caller opts in with
//!   [`AncContent::verify_checksum`].
//! - It does **not** implement the SDI wire (HANC/VANC word packing), the
//!   ST 2110-40 SDP/session layer, or the RTP fixed header (the `rtp-packet`
//!   crate's job).
//!
//! # Standards supported
//!
//! | Standard | Edition | Scope / clauses | Status | Notes |
//! |---|---|---|---|---|
//! | SMPTE ST 2038, ANC in an MPEG-2 TS | 2021 (vendored, verified) | §4.1 Table 1 `anc_data_descriptor` (tag `0xC4`), `"VANC"` identifier `0x56414E43`, `stream_type` `0x06`; §4.2 Table 2 ANC data PES packet | Full | PES header parsed inline; fixed/reserved bits validated; PTS of 2^33 or more rejected |
//! | IETF RFC 8331, RTP payload for ST 291-1 ANC | Feb 2018 | §2/§2.1 payload header + packet list, `F` field, `Length`/`ANC_Count` cross-check; §3.1/§4 media-type and clock-rate constants | Full (wire format) | Not the SDP/session layer; `F == 0b01` parses as `FieldSense::Invalid` |
//! | SMPTE ST 2110-40, ST 291-1 ANC over RTP | 2023 (vendored) | Builds normatively on RFC 8331 | Partial | Grounded in RFC 8331; the 2023 text is not line-audited against the code (unverified delta) |
//! | SMPTE ST 291-1, ANC packet and space formatting | 2011 (vendored) | 10-bit parity rule, `Checksum_Word` | Partial | Derived from RFC 8331 §2.1, which reproduces the rule; SDI ANC *space* formatting is out of scope |
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | yes | Link the standard library; without it the crate is `#![no_std]` + `alloc`. |
//! | `ts` | yes | ST 2038:2021 MPEG-2 TS transport. |
//! | `rtp` | no | RFC 8331 / ST 2110-40 RTP transport; pulls in `rtp-packet`. |
//! | `serde` | no | `serde::Serialize` derives on public types. |
//!
//! [`AncContent`] is always compiled.
//!
//! # What is implemented
//!
//! - **Shared content**: [`AncContent`] holds `DID`, `SDID`, `data_count`,
//!   `user_data_words` and `checksum` as raw 10-bit values (parity bits
//!   included). The user-data loop counter uses only the low 8 bits of
//!   `data_count`; a mismatch with `user_data_words.len()` is rejected on
//!   serialize ([`Error::InconsistentUdwLength`]).
//! - **`ts`**: [`AncDataDescriptor`] (tag [`ANC_DATA_DESCRIPTOR_TAG`], with
//!   [`VANC_FORMAT_IDENTIFIER`] and [`ANC_STREAM_TYPE`]) and
//!   [`AncDataPacket`] (PES, `stream_id` [`ANC_STREAM_ID`], PTS) holding
//!   [`AncPacket`]s plus `0xFF` stuffing.
//! - **`rtp`**: [`AncRtpPayload`] (extended sequence number, [`FieldSense`],
//!   packet list; `Length` and `ANC_Count` recomputed on serialize and
//!   cross-checked on parse) holding [`RtpAncPacket`]s. See
//!   `docs/anc_rtp_8331.md` and `docs/anc_packet_291.md`.
//!
//! # Design notes
//!
//! - Every wire structure implements the workspace `Parse`/`Serialize` pair
//!   with a byte-exact round-trip test and a committed fixture per transport.
//! - The content fields are a contiguous MSB-first 10-bit stream walked with
//!   [`broadcast_common::bits`], identical across both transports.
//! - Reserved bits are validated on parse ([`Error::BadFixedBits`],
//!   [`Error::ReservedNotZero`]); lengths and counts are written through the
//!   checked `fit_*` helpers ([`Error::FieldTooWide`]), never silently wrapped.
//! - One `thiserror` enum, [`Error`]; public enums are `#[non_exhaustive]`.
//! - `#![no_std]` + `alloc`. MSRV 1.95.0. Licence MIT OR Apache-2.0.
//!
//! # Related crates
//!
//! Depends on `broadcast-common` and, with `rtp`, `rtp-packet`; `mpeg-ts`
//! supplies the TS packetisation around the `ts` transport. The ST 2038 PES
//! header is parsed inline (every field is fixed by Table 2).
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
pub use anc_content::{AncContent, ValidationReport};
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
