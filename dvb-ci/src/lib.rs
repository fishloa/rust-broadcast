#![doc = include_str!("../README.md")]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p dvb-ci --example <name>`).\n"]
#![doc = "\n## `build_ca_pmt`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_ca_pmt.rs")]
#![doc = "```\n\n## `parse_apdu`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_apdu.rs")]
#![doc = "```"]

extern crate alloc;

pub mod any;
pub mod builder;
pub mod ci_ext;
pub mod ci_plus;
mod dispatch;
pub mod error;
pub mod length;
pub mod objects;
pub mod reserved;
pub mod resource;
#[cfg(test)]
mod roundtrip_property;
pub mod spdu;
#[cfg(test)]
mod strict_tests;
pub mod tag;
pub mod tpdu;
pub mod traits;

pub use any::AnyApdu;
pub use error::{Error, Result};
pub use resource::ResourceId;
pub use tag::ApduTag;
pub use traits::ApduDef;
