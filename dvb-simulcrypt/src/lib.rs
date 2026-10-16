#![doc = include_str!("../README.md")]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p dvb-simulcrypt --example <name>`.\n"]
#![doc = "\n### `build_channel_setup`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_channel_setup.rs")]
#![doc = "```\n\n### `parse_cw_provision`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_cw_provision.rs")]
#![doc = "```"]

extern crate alloc;

mod error;
mod message;
mod registry;

pub use error::{Error, Result};
pub use message::{HEADER_LEN, PARAMETER_HEADER_LEN, Parameter, SimulcryptMessage};
pub use registry::{
    CpSigMessageType, CpSigParameterType, DataType, EcmgErrorStatus, EcmgScsMessageType,
    EcmgScsParameterType, EmmgErrorStatus, EmmgMuxMessageType, EmmgMuxParameterType, Interface,
    MessageType, ParameterType, SectionTspktFlag,
};
