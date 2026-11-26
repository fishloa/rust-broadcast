#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p scte35-splice --example <name>`).\n"]
#![doc = "\n## `parse_splice_insert`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_splice_insert.rs")]
#![doc = "```\n\n## `round_trip_and_descriptors`\n\n```rust,ignore"]
#![doc = include_str!("../examples/round_trip_and_descriptors.rs")]
#![doc = "```"]

extern crate alloc;

pub mod commands;
pub mod descriptor_loop;
pub mod descriptors;
pub mod dvb_ta;
pub mod error;
pub mod section;
pub mod time;
pub mod tracker;
pub mod traits;

pub use error::{Error, Result};

/// Byte width of a 16-bit field.
pub(crate) const U16_LEN: usize = core::mem::size_of::<u16>();
/// Byte width of a 32-bit field.
pub(crate) const U32_LEN: usize = core::mem::size_of::<u32>();
/// Mask of a four-bit nibble.
pub(crate) const NIBBLE_MASK: u8 = 0x0F;
/// Mask of a `splice_event_cancel_indicator` bit.
pub(crate) const CANCEL_INDICATOR_MASK: u8 = 0x80;
/// Copy `N` big-endian bytes at `offset` into an array.
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
pub use section::{ClearPayload, SpliceInfoSection};
pub use tracker::{
    FeedOutcome, MAX_OPEN_EVENTS, NoCue, SpliceEvent, SpliceObservation, SpliceSanity,
    SpliceTracker, judge_splice_time, kind_label, splice_event,
};
pub use traits::{CommandDef, SpliceDescriptorDef};
