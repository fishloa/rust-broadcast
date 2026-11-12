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
pub mod descriptors;
pub mod dvb_ta;
pub mod error;
pub mod section;
pub mod time;
pub mod tracker;
pub mod traits;

pub use error::{Error, Result};
pub use section::{ClearPayload, SpliceInfoSection};
pub use tracker::{MAX_OPEN_EVENTS, SpliceObservation, SpliceTracker};
pub use traits::{CommandDef, SpliceDescriptorDef};
