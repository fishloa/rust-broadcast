//! `isobmff` — the ISO/IEC 14496-12:2026 (ISO Base Media File Format) box
//! layer: `size`/`type` box headers (including the 64-bit `largesize` and the
//! `uuid` extended type), the top-level `ftyp`/`styp`/`mdat` set, the
//! sample-table and movie-fragment boxes, the shared CENC box syntax
//! (ISO/IEC 23001-7), and the DASH Event Message Box (`emsg`, ISO/IEC 23009-1).
//!
//! This crate is deliberately below [`transmux`](https://docs.rs/transmux):
//! it knows box syntax only, never the media IR or a codec bitstream. Its one
//! workspace dependency is [`broadcast_common`], so a parser that only needs
//! to walk MP4 boxes — a validator, a segmenter, a probe — no longer pulls in
//! the whole container-muxing hub.
//!
//! Every wire structure implements the same symmetric pair as the rest of the
//! workspace: [`Parse`](broadcast_common::Parse) borrows from the input, and
//! [`Serialize`](broadcast_common::Serialize) writes it back byte-identically.
//!
//! # Layout
//!
//! - [`box_types`] — box / FullBox headers, [`BoxType`],
//!   the size-driven [`box_iter`] walker, the header
//!   writers, and the opaque round-trip carriers.
//! - [`box_support`] — the shared flat-table and container-child helpers every
//!   box module builds on.
//! - [`segments`], [`timing`], [`visual_ext`], [`cenc`], [`sample_groups`],
//!   [`movie_fragment`], [`frag_offsets`] — the box set and the §8.8.7/§8.8.8
//!   fragment sample-offset resolver.
//! - [`avc_config`], [`hevc_config`], [`vvc_config`], [`nalu_types`], [`av1`],
//!   [`vp9`], [`mp4esds`], [`ac3`], [`ac4`], [`dts`], [`flac`], [`opus`],
//!   [`mpegh`], [`dovi`], [`hdr_boxes`], [`avs2`], [`avs3`], [`avs3_audio`] —
//!   the codec decoder-configuration records and the `esds` descriptor chain.
//! - [`emsg`] — the MPEG-DASH Event Message Box (ISO/IEC 23009-1 §5.10.3.3,
//!   version 0 and 1) and its version enum.
//!
//! # Spec citations
//!
//! - **Box / FullBox**: ISO/IEC 14496-12:2026 §4.2.
//! - **Box order**: §7.3.4.
//! - **Skip-unknown rule**: §4.2 — unknown type → ignore + skip via `size`.
//! - **CENC boxes**: ISO/IEC 23001-7 §12.
//! - **`emsg`**: ISO/IEC 23009-1 §5.10.3.3; DASH-IF IOP Part 10 §6.1.
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `std`   | yes     | `std` glue; without it the crate is `no_std` + `alloc` |
//! | `serde` | no      | `serde::Serialize` on the box types |

#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
extern crate alloc;

pub mod ac3;
pub mod ac4;
pub mod av1;
pub mod avc_config;
pub mod avs2;
pub mod avs3;
pub mod avs3_audio;
pub mod box_support;
pub mod box_types;
pub mod cenc;
pub mod dovi;
pub mod dts;
pub mod emsg;
pub mod error;
pub mod flac;
pub mod frag_offsets;
pub mod hdr_boxes;
pub mod hevc_config;
pub mod movie_fragment;
pub mod mp4esds;
pub mod mpegh;
pub mod nalu_types;
pub mod opus;
pub mod sample_groups;
pub mod segments;
pub mod timing;
pub mod track_id;
pub mod visual_ext;
pub mod vp9;
pub mod vvc_config;

pub use ac3::{Ac3SpecificBox, Ec3SpecificBox, Ec3Substream, acmod_channels};
pub use ac4::{AC4_FOURCC, Ac4SpecificBox, DAC4_FOURCC};
pub use av1::{AV1C_FOURCC, Av1ConfigurationBox};
pub use avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
pub use avs2::{AV2C_FOURCC, AVST_FOURCC, Avs2ConfigurationBox, LA2C_FOURCC, LAV2_FOURCC};
pub use avs3::{
    A3LG_FOURCC, AV3C_FOURCC, Avs3ConfigurationBox, AvsFrameRateCode, LAVC_FOURCC,
    LAvsConfigurationBox, LRAP_FOURCC, LibraryDependencyIdc, LibrarySampleGroupEntry,
    LibrarySampleRef, TELG_FOURCC, TemporalLayerEntry, TemporalLayerInfo, VisualLrapEntry,
};
pub use avs3_audio::{AV3A_FOURCC, Avs3AudioContent, Ca3Config, Ca3SpecificBox, DCA3_FOURCC};
pub use box_types::{BoxHeader, BoxIter, BoxRef, BoxType, FullBoxHeader, box_iter, parse_box};
pub use dovi::{
    AVC_PROFILE, DOVI_RECORD_LEN, DOVI_RESERVED_LEN, DOVI_RESERVED_ZERO, DVA1_FOURCC, DVCC_FOURCC,
    DVH1_FOURCC, DVHE_FOURCC, DVVC_FOURCC, DoviBlSignalCompatibilityId,
    DoviDecoderConfigurationRecord, DoviSampleEntryKind, MAX_DVCC_PROFILE,
};
pub use dts::{
    DDTS_BODY_LEN, DDTS_FOURCC, DTSC_FOURCC, DTSE_FOURCC, DTSH_FOURCC, DTSL_FOURCC, DtsSpecificBox,
};
pub use error::{Error, Result};
pub use flac::{
    BLOCK_TYPE_STREAMINFO, DFLA_FOURCC, FLAC_FOURCC, FlacMetadataBlock, FlacSpecificBox,
};
pub use hdr_boxes::{
    CLLI_FOURCC, ChromaticityXy, ContentLightLevel, MDCV_FOURCC, MasteringDisplayColourVolume,
};
pub use hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
pub use mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, OTI_MPEG1_AUDIO,
    OTI_MPEG2_AUDIO, OTI_MPEG4_AUDIO, ObjectTypeIndication, SLConfigDescriptor, STREAM_TYPE_AUDIO,
    StreamType,
};
pub use mpegh::{
    MHA1_FOURCC, MHA2_FOURCC, MHAC_CONFIGURATION_VERSION, MHAC_FOURCC, MHAC_RECORD_FIXED_LEN,
    MHADecoderConfigurationRecord, MHM1_FOURCC, MHM2_FOURCC,
};
pub use nalu_types::{
    AvcNalUnit, AvcPps, AvcSps, AvcSpsExt, HEVC_PPS_NUT, HEVC_PREFIX_SEI_NUT, HEVC_SPS_NUT,
    HEVC_SUFFIX_SEI_NUT, HEVC_VPS_NUT, HevcNalArray, HevcNalUnit, VALID_LENGTH_SIZES,
};
pub use opus::{ChannelMappingTable, DOPS_FOURCC, OPUS_FOURCC, OpusSpecificBox};
pub use track_id::TrackId;
pub use vp9::{VPCC_FOURCC, Vp9ConfigurationBox};
pub use vvc_config::{
    VvcConfigurationBox, VvcDecoderConfigurationRecord, VvcNalArray, VvcNalUnitType, VvcPtlRecord,
};

/// Serialize a [`Timescale`](broadcast_common::timescale::Timescale) as the
/// plain ticks-per-second integer it wraps, so a serde-JSON consumer sees the
/// same number the wire box carries (not a `NonZeroU32` newtype object).
#[cfg(feature = "serde")]
pub fn ser_timescale<S: serde::Serializer>(
    timescale: &broadcast_common::timescale::Timescale,
    serializer: S,
) -> core::result::Result<S::Ok, S::Error> {
    serializer.serialize_u32(timescale.get())
}
