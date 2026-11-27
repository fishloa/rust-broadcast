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
#![doc = "```\n\n### `decode_capture_sps`\n\n```rust,ignore"]
#![doc = include_str!("../examples/decode_capture_sps.rs")]
#![doc = "```\n\n### `print_capture_slices`\n\n```rust,ignore"]
#![doc = include_str!("../examples/print_capture_slices.rs")]
#![doc = "```"]

extern crate alloc;

pub mod annexb;
pub mod au;
pub mod bitreader;
#[cfg(feature = "isobmff")]
pub mod codec_config_ext;
mod error;
pub mod facts;
#[cfg(all(feature = "hevc", feature = "isobmff"))]
pub mod hdr;
#[cfg(feature = "hevc")]
pub mod hdr10plus;
#[cfg(any(feature = "h264", feature = "hevc"))]
pub mod interlace;
pub mod nal;
#[cfg(feature = "isobmff")]
pub mod nalu_ext;
pub mod rbsp;
pub mod rfc6381;
#[cfg(feature = "h264")]
pub mod slice_header;
pub mod sps;

pub use annexb::{
    AnnexBNalIter, NAL_LENGTH_SIZE, NAL_LENGTH_SIZE_MINUS_ONE, VALID_NAL_LENGTH_SIZES,
    annexb_to_length_prefixed, iter_annexb_nals, iter_length_prefixed_nals,
    iter_length_prefixed_nals_with, length_prefixed_to_annexb, normalise_nal_length_size,
};
pub use au::{AccessUnitSplitter, split_access_units};
#[cfg(feature = "isobmff")]
pub use codec_config_ext::avc_config_from_sps_pps;
pub use error::{Error, Result};
#[cfg(all(feature = "hevc", feature = "isobmff"))]
pub use hdr::HdrStaticMetadata;
#[cfg(feature = "hevc")]
pub use hdr10plus::{
    AdditionalWindow, BezierToneMapping, ColorVolumeTransform, DistributionPoint,
    Hdr10PlusMetadata, OverlapProcessOption, PeakLuminanceTable, WindowRegion, WindowRegionBuilder,
};
#[cfg(feature = "h264")]
pub use interlace::avc_facts;
#[cfg(feature = "hevc")]
pub use interlace::hevc_facts;
#[cfg(any(feature = "h264", feature = "hevc"))]
pub use interlace::{PicStruct, PictureFacts, SourceScanType, VideoScan};
#[cfg(feature = "h264")]
pub use nal::AvcNalUnitType;
#[cfg(feature = "hevc")]
pub use nal::HevcNalUnitType;
pub use nal::{
    AvcPictureParameterSet, AvcSequenceParameterSet, NalCodec, access_unit_is_idr,
    access_unit_is_keyframe, access_unit_is_rap, avc_access_unit_parameter_sets, caption_cc_data,
    is_idr_nal, is_keyframe_nal, nal_unit_type, recovery_point_sei,
};
#[cfg(feature = "h264")]
pub use slice_header::{
    AvcFieldInfo, AvcParamSets, AvcPicture, AvcSliceHeader, AvcSlicePps, decode_avc_slice_pps,
    parse_avc_slice_header,
};
#[cfg(any(feature = "h264", feature = "hevc"))]
pub use sps::SpsDisplayFacts;
#[cfg(feature = "h264")]
pub use sps::{
    AvcPicTimingLayout, AvcReorderDepth, AvcSliceSps, AvcSpsInfo, AvcVuiTiming,
    decode_avc_display_facts, decode_avc_reorder_depth, decode_avc_slice_sps, decode_avc_sps,
    decode_avc_vui_timing, is_high_profile,
};
#[cfg(feature = "hevc")]
pub use sps::{
    HevcInterlaceSps, HevcSpsInfo, HevcVpsTiming, HevcVuiTiming, decode_hevc_display_facts,
    decode_hevc_sps, decode_hevc_vps_timing, decode_hevc_vui_timing, decode_ptl,
};
#[cfg(feature = "vvc")]
pub use sps::{VvcSpsInfo, decode_vvc_sps};
