#![doc = include_str!("../README.md")]
#![doc = "\n"]
//! # Reserved-bit and trailing-byte policy
//!
//! One workspace-wide rule for every wire crate (issue #1139):
//!
//! - **Parse is tolerant.** A non-conformant reserved bit, or trailing data
//!   that real senders emit, is accepted rather than rejected (a crate may
//!   choose stricter validation, as `dvb-t2mi` does for RFU bits, but must
//!   document it).
//! - **Serialize preserves what was parsed.** Reserved bits are captured on
//!   parse in typed, spec-named fields and re-emitted verbatim, so a stream
//!   from a sender that deviates from the governing edition re-serializes
//!   **byte-identically** (`dvb-si` descriptors, `mpeg-ps`, `rmt-flute`,
//!   `cc-data` fixed bits, `rtp-packet`'s `rtcp` `profile_extensions`, `isobmff`
//!   flags). There is no raw passthrough blob.
//! - **Constructed values default to the edition value**: a struct built by
//!   hand (`new(..)` / `Default`) carries `1` for `reserved_future_use` and
//!   H.222.0 "reserved", `0` for `reserved_zero_future_use`, with any explicit
//!   per-field exception documented on the field. (A crate that has not
//!   adopted preserve-on-parse documents which fields it normalizes to the
//!   spec value on serialize; `mpeg-ts`, `mpeg-pes` and `mpeg-ps` preserve.)
//! - A caller that needs byte-for-byte preservation of reserved bits in a
//!   not-yet-migrated crate **keeps the original bytes** rather than
//!   re-serializing.
//! - Trailing bytes inside a descriptor or section body are never silently
//!   dropped without a documented decision: hold them in a typed field, or
//!   document the field as "ignored".
//! - Reserved bytes are always **written explicitly** by `serialize_into`;
//!   never rely on a pre-zeroed output buffer.
//!
//! The round-trip gate is byte-exact against the original fixture slice
//! (see `docs/CRATE-ACCEPTANCE.md`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
// The crate's runnable examples, embedded so they render on docs.rs and stay in
// sync with the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "One runnable example ships with this crate (`cargo run -p broadcast-common --example crc_and_bcd`).\n"]
#![doc = "\n## `crc_and_bcd`\n\n```rust,ignore"]
#![doc = include_str!("../examples/crc_and_bcd.rs")]
#![doc = "```"]

extern crate alloc;

pub mod backoff;
#[cfg(feature = "base64")]
pub mod base64;
pub mod bcd;
pub mod ber;
#[cfg(feature = "std")]
pub mod bounded;
pub mod clock33;
pub mod content_range;
pub mod crc32_mpeg2;
pub mod erased;
pub mod hex;
#[cfg(feature = "std")]
#[cfg(feature = "http-date")]
pub mod http_date;
pub mod klv;
#[cfg(feature = "lang")]
pub mod lang;
pub mod net;
pub mod parity;
pub mod pusi;
pub mod range;
pub mod seq;
pub mod stage;
pub mod time;
pub mod timeouts;
pub mod units;
#[cfg(feature = "url")]
pub mod url_ref;
mod wire_enum;
#[cfg(feature = "xml")]
pub mod xml;
#[cfg(feature = "xml")]
pub mod xml_chars;

#[cfg(feature = "lang")]
pub use lang::bcp47_language;
pub use stage::{Demand, Stage, Timestamp};

// WR1-BRIDGE
pub use broadcast_core::wire;
