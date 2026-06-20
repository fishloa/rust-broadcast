//! DVB closed-caption carriage — `cc_data()` per ETSI TS 101 154 §B.5, Table B.9.
//!
//! Parses the closed-caption carriage structure carried in MPEG-2 / AVC / HEVC
//! picture `user_data` (the DVB-native, normative form of the ATSC/CEA `cc_data()`).
//! Exposes the typed caption triplets (`cc_valid`, `cc_type`, `cc_data_1/2`) and a
//! CEA-608 vs CEA-708 split by `cc_type`. The *meaning* of the caption byte pair
//! (the CEA-708-E character/control decode) is a layer above this carriage and is
//! out of scope.
//!
//! Feed it the `cc_data()` bytes (the caller extracts them from the picture
//! user_data / SEI). Depends only on `dvb-common`, `#![no_std]` (+ `alloc`).
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p dvb-cc --example <name>`).\n"]
#![doc = "\n## `parse_cc_data`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_cc_data.rs")]
#![doc = "```\n\n## `build_cc_data`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_cc_data.rs")]
#![doc = "```"]

extern crate alloc;

mod cc_data;
mod error;

pub use cc_data::{CcData, CcTriplet, CcType};
pub use error::{Error, Result};
