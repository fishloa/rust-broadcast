//! Closed-caption `cc_data()` carriage (CEA-608/708), per ETSI TS 101 154 §B.5, Table B.9.
//!
//! Parses the closed-caption carriage structure carried in MPEG-2 / AVC / HEVC
//! picture `user_data` (the DVB-native, normative form of the ATSC/CEA `cc_data()`).
//! Exposes the typed caption triplets (`cc_valid`, `cc_type`, `cc_data_1/2`) and a
//! CEA-608 vs CEA-708 split by `cc_type`.
//!
//! Feed it the `cc_data()` bytes (the caller extracts them from the picture
//! user_data / SEI). Depends only on `dvb-common`, `#![no_std]` (+ `alloc`).
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
