#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p dvb-si --example <name>`).\n"]
#![doc = "\n## `build_and_parse_pat`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_and_parse_pat.rs")]
#![doc = "```\n\n## `list_services`\n\n```rust,ignore"]
#![doc = include_str!("../examples/list_services.rs")]
#![doc = "```"]
// The README is the crates.io landing page; `cfg(doctest)` is set only while
// rustdoc collects doctests, so every Rust snippet in it is compiled and run
// by `cargo test --doc` without the README text appearing on docs.rs.
#![cfg_attr(doctest, doc = include_str!("../README.md"))]

extern crate alloc;

pub mod carousel;
pub mod collect;
pub mod compatibility;
pub mod descriptor_tag;
pub mod descriptors;
#[cfg(feature = "chrono")]
pub mod epg;
pub mod error;
pub(crate) mod h222_bits;
pub mod quality_access_unit;
mod registry_names;
pub mod table_id;
pub mod tables;
pub mod text;
pub mod traits;
pub(crate) mod wire;

#[cfg(feature = "yoke")]
pub mod owned;

#[cfg(feature = "ts")]
pub mod demux;

pub use descriptor_tag::DescriptorTag;
pub use error::{Error, Result};
pub use table_id::TableId;
