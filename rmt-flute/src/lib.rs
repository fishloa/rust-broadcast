#![doc = include_str!("../README.md")]
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p rmt-flute --example <name>`.\n"]
#![doc = "\n### `build_lct`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_lct.rs")]
#![doc = "```\n\n### `parse_flute`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_flute.rs")]
#![doc = "```"]

extern crate alloc;

mod alc;
mod error;
mod ext;
mod fec;
mod flute;
mod lct;
mod lct_ext;
mod norm;

pub use alc::{
    AlcPacket, FEC_PAYLOAD_ID_128_LEN, FecPayloadId128, HET_EXT_FTI as ALC_HET_EXT_FTI, PSI_SPI,
};
pub use error::{Error, Result};
pub use ext::{FIXED_HET_MIN, HeaderExtension, WORD, chain_len, parse_chain, serialize_chain};
pub use fec::SourceBlockPartition;
pub use flute::{
    CencAlgorithm, ExtCenc, ExtFdt, FDT_INSTANCE_ID_MAX, FLUTE_VERSION, HET_EXT_CENC, HET_EXT_FDT,
    TOI_FDT,
};
pub use lct::{FIXED_HEADER_LEN, LCT_VERSION, LctHeader};
pub use lct_ext::{
    ExtTime, HET_EXT_AUTH as LCT_HET_EXT_AUTH, HET_EXT_NOP, HET_EXT_TIME, LctExtType, USE_ERT,
    USE_SCT_HIGH, USE_SCT_LOW, USE_SLC,
};
pub use norm::{
    COMMON_HEADER_LEN, FEEDBACK_FIXED_LEN, HET_EXT_AUTH as NORM_HET_EXT_AUTH, HET_EXT_CC,
    HET_EXT_FTI as NORM_HET_EXT_FTI, HET_EXT_RATE, NORM_FLAG_EXPLICIT, NORM_FLAG_FILE,
    NORM_FLAG_INFO, NORM_FLAG_REPAIR, NORM_FLAG_STREAM, NORM_FLAG_UNRELIABLE, NORM_INFO_FIXED_LEN,
    NORM_NODE_ANY, NORM_NODE_NONE, NORM_VERSION, NormAckType, NormCmd, NormCmdBody, NormCmdType,
    NormCommonHeader, NormData, NormFeedback, NormInfo, NormMessageType, SENDER_WORD_LEN,
    SenderWord,
};
