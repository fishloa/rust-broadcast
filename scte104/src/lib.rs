#![doc = include_str!("../README.md")]
#![doc = "\n## Examples\n\nTwo runnable examples ship with this crate (`cargo run -p scte104 --example <name>`).\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_splice.rs")]
#![doc = "```\n\n```rust,ignore"]
#![doc = include_str!("../examples/multi_op_round_trip.rs")]
#![doc = "```"]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

extern crate alloc;

pub mod error;
pub mod multi;
pub mod operations;
pub mod single;
pub mod time;
pub mod traits;

pub use error::{Error, Result};
pub use multi::MultipleOperationMessage;
pub use single::SingleOperationMessage;
