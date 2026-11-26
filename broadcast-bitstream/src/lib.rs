#![doc = include_str!("../README.md")]
#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p broadcast-bitstream --example <name>`.\n"]
#![doc = "\n### `split_annexb`\n\n```rust,ignore"]
#![doc = include_str!("../examples/split_annexb.rs")]
#![doc = "```\n\n### `annexb_to_avcc`\n\n```rust,ignore"]
#![doc = include_str!("../examples/annexb_to_avcc.rs")]
#![doc = "```"]

extern crate alloc;

pub mod annexb;
pub mod au;
pub mod bitreader;
mod error;
pub mod nal;
pub mod rbsp;

pub use annexb::{
    AnnexBNalIter, NAL_LENGTH_SIZE, NAL_LENGTH_SIZE_MINUS_ONE, VALID_NAL_LENGTH_SIZES,
    annexb_to_length_prefixed, iter_annexb_nals, iter_length_prefixed_nals,
    iter_length_prefixed_nals_with, length_prefixed_to_annexb, normalise_nal_length_size,
};
pub use au::{AccessUnitSplitter, split_access_units};
pub use error::{Error, Result};
#[cfg(feature = "h264")]
pub use nal::AvcNalUnitType;
#[cfg(feature = "hevc")]
pub use nal::HevcNalUnitType;
pub use nal::{
    AvcPictureParameterSet, AvcSequenceParameterSet, NalCodec, access_unit_is_idr,
    access_unit_is_keyframe, access_unit_is_rap, avc_access_unit_parameter_sets, caption_cc_data,
    is_idr_nal, is_keyframe_nal, nal_unit_type, recovery_point_sei,
};
