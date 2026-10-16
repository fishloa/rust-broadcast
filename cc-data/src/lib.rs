//! Closed-caption `cc_data()` carriage (CEA-608/708), per ETSI TS 101 154 §B.5, Table B.9.
//!
//! Parses the closed-caption carriage structure carried in MPEG-2 / AVC / HEVC
//! picture `user_data` (the DVB-native, normative form of the ATSC/CEA `cc_data()`).
//! Exposes the typed caption triplets (`cc_valid`, `cc_type`, `cc_data_1/2`) and a
//! CEA-608 vs CEA-708 split by `cc_type`.
//!
//! Feed it the `cc_data()` bytes (the caller extracts them from the picture
//! user_data / SEI). Depends only on `broadcast-common`, `#![no_std]` (+ `alloc`).
//!
//! # Caption decode (`decode` feature)
//!
//! With the default `decode` feature, this crate also *interprets* the demuxed
//! caption byte pairs — the [`decode`] module's [`Cea608Decoder`](decode::Cea608Decoder)
//! (line-21 pop-on / roll-up / paint-on, PACs, mid-row codes, the standard /
//! special / extended character sets, channels CC1–CC4) and
//! [`Cea708Decoder`](decode::Cea708Decoder) (DTVCC packet / service-block
//! reassembly + the C0/C1/G0/G1/G2/G3 window / pen command interpreter, services
//! 1–6) — and exposes the decoded caption screen / window text. Grounded in
//! ANSI/CTA-608-E, ANSI/CTA-708-E and 47 CFR §79.102 (see `cc-data/docs/decode/`).
//!
//! # What it does / what it does not
//!
//! Parses and builds `cc_data()` byte-exactly, splits CEA-608 (`cc_type` 0/1)
//! from CEA-708 (`cc_type` 2/3), and (with `decode`) interprets the pairs into
//! caption text. It does **not** locate `cc_data()` in picture `user_data` / SEI,
//! render pixels, or decode the XDS metadata catalogue (field-2 XDS is detected
//! and skipped). Conversion to WebVTT / SRT is in `timed-metadata` and
//! `caption-convert`.
//!
//! # Standards supported
//!
//! | Standard | Edition | Scope | Status |
//! |---|---|---|---|
//! | ETSI TS 101 154 | V2.6.1 (transcribed in `docs/ts_101_154/`) | §B.5 / Table B.9 `cc_data()` | Full |
//! | ANSI/CTA-608-E | -E; CTA-608-E is not vendored (no copy in `private/specs` or `specs/`), so byte tables cannot be re-verified against it | line-21 decode, CC1-CC4, PACs, character sets | Partial (decode); XDS skipped |
//! | ANSI/CTA-708-E | -E; CTA-708-E is not vendored; cross-source: GY/T 270-2013 (vendored, a Chinese adaptation; differences from CTA must be checked) | DTVCC packets, service blocks, windows, pens; services 1-6 | Partial (decode) |
//! | 47 CFR §79.102 | public domain (`docs/decode/cea708-conformance.md`) | 708 semantic model | Full as a grounding source |
//!
//! # Feature flags
//!
//! | Feature | Default | Effect |
//! |---|---|---|
//! | `std` | on | link `std`; off = `no_std` + `alloc` |
//! | `decode` | on | [`decode`](crate::decode) module: 608 / 708 decoders |
//! | `serde` | off | `serde::Serialize` on public types |
//!
//! # Quick start
//!
//! ```
//! use broadcast_common::{Parse, Serialize};
//! use cc_data::{CcData, CcTriplet, CcType};
//!
//! // Build a cc_data() with one 608 field-1 pair and one 708 start triplet.
//! let cc = CcData {
//!     process_em_data_flag: true,
//!     process_cc_data_flag: true,
//!     additional_data_flag: false,
//!     reserved_byte1: 0xFF,
//!     triplets: vec![
//!         CcTriplet {
//!             cc_valid: true,
//!             cc_type: CcType::Dtvcc708Start,
//!             cc_data_1: 0xC1,
//!             cc_data_2: 0x02,
//!         },
//!         CcTriplet {
//!             cc_valid: true,
//!             cc_type: CcType::Ntsc608Field1,
//!             cc_data_1: 0x94,
//!             cc_data_2: 0x2C,
//!         },
//!     ],
//! };
//!
//! // Serialize, parse back byte-exactly, and split 608 from 708.
//! let bytes = cc.try_to_bytes().unwrap();
//! let back = CcData::parse(&bytes).unwrap();
//! assert_eq!(back, cc);
//! assert_eq!(back.cea608().count(), 1);
//! assert_eq!(back.cea708().count(), 1);
//! ```
//!
//! Decoding (feature `decode`):
//!
//! ```
//! # #[cfg(feature = "decode")]
//! # {
//! use cc_data::decode::{Cea608Channel, Cea608Decoder};
//!
//! let mut dec = Cea608Decoder::new();
//! // RCL, PAC row 15, "HI", EOC: a pop-on caption on CC1 (field 1).
//! dec.push_pair(false, 0x14, 0x20); // RCL (field 1 -> CC1)
//! dec.push_pair(false, 0x14, 0x20); // doubled control, ignored
//! dec.push_pair(false, 0x14, 0x70); // PAC row 15 indent 0
//! dec.push_pair(false, b'H', b'I');
//! dec.push_pair(false, 0x14, 0x2F); // EOC -> flip
//! assert_eq!(dec.channel_text(Cea608Channel::Cc1), "HI");
//! # }
//! ```
//!
//! # Design notes and related crates
//!
//! Symmetric `Parse` / `Serialize` with a byte-identical carriage round trip
//! (flag bits and `reserved_byte1` preserved); [`Error`] is `#[non_exhaustive]`;
//! both decoders are one-way and panic-free on hostile input. Depends only on
//! `broadcast-common`; used by `timed-metadata` and `caption-convert`. MSRV
//! 1.95.0; MIT OR Apache-2.0.
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Runnable examples ship with this crate (`cargo run -p cc-data --example <name>`).\n"]
#![doc = "\n## `parse_cc_data`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_cc_data.rs")]
#![doc = "```\n\n## `build_cc_data`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_cc_data.rs")]
#![cfg_attr(
    feature = "decode",
    doc = "```\n\n## `decode_cea608`\n\n```rust,ignore"
)]
#![cfg_attr(feature = "decode", doc = include_str!("../examples/decode_cea608.rs"))]
#![cfg_attr(
    feature = "decode",
    doc = "```\n\n## `decode_cea708`\n\n```rust,ignore"
)]
#![cfg_attr(feature = "decode", doc = include_str!("../examples/decode_cea708.rs"))]
#![doc = "```"]

extern crate alloc;

mod cc_data;
#[cfg(feature = "decode")]
#[cfg_attr(docsrs, doc(cfg(feature = "decode")))]
pub mod decode;
mod error;

pub use cc_data::{CcData, CcTriplet, CcType};
pub use error::{Error, Result};
