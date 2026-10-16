#![doc = include_str!("../README.md")]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p dvb-vbi --example <name>`.\n"]
#![doc = "\n### `build_data_field`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_data_field.rs")]
#![doc = "```\n\n### `parse_data_field`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_data_field.rs")]
#![doc = "```"]

extern crate alloc;

mod data_unit_id;
mod error;
mod line_header;
mod payload;

pub use data_unit_id::{
    DataUnitId, ID_CLOSED_CAPTIONING, ID_EBU_TELETEXT_NON_SUBTITLE, ID_EBU_TELETEXT_SUBTITLE,
    ID_INVERTED_TELETEXT, ID_MONOCHROME_422_SAMPLES, ID_STUFFING, ID_VPS, ID_WSS,
};
pub use error::{Error, Result};
pub use line_header::{LINE_HEADER_LEN, LineHeader, RESERVED_PREFIX};
pub use payload::{
    CC_FIELD_LEN, ClosedCaptioningDataField, DataField, DataUnit, DataUnitPayload,
    FRAMING_CODE_EBU, FRAMING_CODE_INVERTED, MONO_HEADER_LEN, MonochromeDataField,
    TELETEXT_DATA_UNIT_LENGTH, TELETEXT_FIELD_LEN, TXT_DATA_BLOCK_LEN, TeletextDataField,
    VPS_DATA_BLOCK_LEN, VPS_FIELD_LEN, VpsDataField, WSS_DATA_BLOCK_MASK, WSS_FIELD_LEN,
    WSS_RESERVED_TAIL, WssDataField,
};
