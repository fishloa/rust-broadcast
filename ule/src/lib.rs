#![doc = include_str!("../README.md")]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p ule --example <name>`.\n"]
#![doc = "\n### `build_sndu`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_sndu.rs")]
#![doc = "```\n\n### `receive_sndu`\n\n```rust,ignore"]
#![doc = include_str!("../examples/receive_sndu.rs")]
#![doc = "```"]

extern crate alloc;

mod error;
mod ext_header;
mod sndu;
mod ts;
mod type_field;

pub use error::{Error, Result};
pub use ext_header::{
    ExtensionHeader, H_TYPE_BRIDGED_FRAME, H_TYPE_EXT_PADDING, H_TYPE_PDU_CONCAT, H_TYPE_TEST_SNDU,
    H_TYPE_TIMESTAMP, H_TYPE_TS_CONCAT, MandatoryHType, OptionalHType, PayloadChain,
};
pub use sndu::{
    BASE_HEADER_LEN, CRC_LEN, END_INDICATOR, END_INDICATOR_LENGTH, NPA_LEN, PADDING_BYTE, Sndu,
    is_end_indicator,
};
pub use ts::{TS_PAYLOAD_LEN, UleReceiver};
pub use type_field::{ETHERTYPE_BOUNDARY, ETHERTYPE_IPV4, ETHERTYPE_IPV6, TypeField};
