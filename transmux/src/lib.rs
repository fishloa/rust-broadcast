//! `transmux` — MPEG-2 TS → CMAF/fMP4 container remux (ISO/IEC 14496-12:2015).
//!
//! A **samples-in** container multiplexer: the caller supplies demuxed *coded*
//! access units + codec config, and `transmux` produces CMAF initialization and
//! media segments plus HLS playlists. It parses codec config/parameter headers
//! (SPS/PPS/VPS, AAC ASC, AC-3/E-AC-3 syncframe BSI) only to build the container
//! boxes and derive metadata — it never encodes or decodes media; coded samples
//! pass through opaque. `no_std` + `alloc`.
//!
//! See the crate README for the full **feature matrix** (implemented boxes and
//! codecs plus planned coverage). Key entry points: [`build_init_segment`] and
//! [`build_media_segment`] (batch), [`Segmenter`] (streaming), [`AvcSps::decode`]
//! with [`AvcSps::rfc6381`] (codec metadata), and the HLS playlist builders.
//!
//! This crate root also exposes the low-level ISOBMFF framing it is built on:
//! [`BoxHeader`] (optional 64-bit `largesize` / `uuid` extended type),
//! [`FullBoxHeader`] (`version` + `flags`), and a size-driven box walker
//! ([`BoxIter`]) that skips unknown box types by advancing by `size`.
//!
//! # Quick start
//!
//! ```rust
//! use transmux::{box_iter, parse_box, BoxHeader, BoxType};
//! use broadcast_common::Parse;
//!
//! // Parse a simple ftyp box header.
//! let bytes = [0, 0, 0, 12, b'f', b't', b'y', b'p', 0, 0, 0, 0];
//! let header = BoxHeader::parse(&bytes).unwrap();
//! assert_eq!(header.box_type.to_string(), "ftyp");
//! ```
//!
//! # Spec citations
//!
//! - **Box / FullBox**: ISO/IEC 14496-12:2015 §4.2 (fulltext L1254–L1304).
//! - **Box order**: §6.2.3 (recommendations; only `ftyp` before variable-length and
//!   `moof` sequence_number are mandatory).
//! - **Skip-unknown rule**: §4.2 L1294 — unknown type → ignore + skip via size.
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `std`   | yes     | `std::error::Error` impls |
//! | `serde` | yes     | `serde::Serialize` for box types |

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
extern crate alloc;

pub mod aac_asc;
pub mod ac3;
pub mod ac4;
pub mod annexb;
pub mod av1;
pub mod avc_config;
pub mod bitreader;
pub mod box_types;
pub mod cenc;
pub mod dash;
pub mod dts;
pub mod error;
pub mod flac;
pub mod hevc_config;
pub mod hls;
pub mod init_segment;
pub mod media;
pub mod movie_fragment;
pub mod mp4esds;
pub mod mpegh;
pub mod nalu_types;
pub mod opus;
pub mod pipeline;
pub mod progressive;
pub mod repackage;
pub mod sample_entries;
pub mod sample_groups;
pub mod segmenter;
pub mod segments;
pub mod sps;
pub mod subtitle_entries;
pub mod timing;
pub mod ts_demux;
pub mod ts_hls;
pub mod ts_mux;
pub mod visual_ext;
pub mod vp9;

pub use aac_asc::{
    build_adts_header, parse_adts_header, AdtsHeader, AudioObjectType, AudioSpecificConfig,
    ChannelConfiguration, HeAacSignaling, SamplingFrequencyIndex,
};
pub use ac3::{Ac3SpecificBox, Ac3SyncframeInfo, Ec3SpecificBox, Ec3Substream, Ec3SyncframeInfo};
pub use ac4::{Ac4SpecificBox, AC4_FOURCC, DAC4_FOURCC};
pub use annexb::{
    annexb_to_length_prefixed, iter_annexb_nals, iter_length_prefixed_nals,
    length_prefixed_to_annexb, NAL_LENGTH_SIZE,
};
pub use av1::{Av1ConfigurationBox, Av1SampleEntry, AV01_FOURCC, AV1C_FOURCC};
pub use avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
pub use box_types::{box_iter, parse_box, BoxHeader, BoxIter, BoxRef, BoxType, FullBoxHeader};
pub use cenc::{
    OriginalFormatBox, ProtectionSchemeInfoBox, ProtectionSystemSpecificHeaderBox,
    SampleAuxInfoOffsetsBox, SampleAuxInfoSizesBox, SampleEncryptionBox, SampleEncryptionEntry,
    SchemeInformationBox, SchemeTypeBox, SubSampleEntry, TrackEncryptionBox,
    SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION,
};
pub use dash::{DashPackager, MediaKind, MPD_NAMESPACE, PROFILE_ISOFF_LIVE};
pub use dts::{
    DtsSpecificBox, DDTS_BODY_LEN, DDTS_FOURCC, DTSC_FOURCC, DTSE_FOURCC, DTSH_FOURCC, DTSL_FOURCC,
};
pub use error::{Error, Result};
pub use flac::{
    FlacMetadataBlock, FlacSpecificBox, BLOCK_TYPE_STREAMINFO, DFLA_FOURCC, FLAC_FOURCC,
};
pub use hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
pub use hls::{MasterPlaylist, MediaPlaylist, MediaSegment, Variant};
pub use init_segment::{
    Ac3SampleEntry, Ac4SampleEntry, ChunkLargeOffsetBox, ChunkOffsetBox, DataEntryUrlBox,
    DataInformationBox, DataReferenceBox, DtsSampleEntry, Ec3SampleEntry, EditBox, FlacSampleEntry,
    HandlerBox, MediaBox, MediaHeaderBox, MediaInformationBox, MhaSampleEntry, MovieBox,
    MovieExtendsBox, MovieHeaderBox, Mp4aSampleEntry, OpaqueBox, OpusSampleEntry,
    SampleDescriptionBox, SampleEntryVariant, SampleSizeBox, SampleTableBox, SampleToChunkBox,
    SoundMediaHeaderBox, StblChild, StscEntry, SyncSampleBox, TrackBox, TrackExtendsBox,
    TrackHeaderBox, VideoMediaHeaderBox,
};
pub use media::{CmafMux, Fmp4Demux, HlsPackager, Media, Track};
pub use movie_fragment::{
    MovieFragmentBox, MovieFragmentHeaderBox, TrackFragmentBaseMediaDecodeTimeBox,
    TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentRunBox, TrunSample,
};
pub use mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, ObjectTypeIndication,
    SLConfigDescriptor, StreamType,
};
pub use mpegh::{
    MHADecoderConfigurationRecord, MHA1_FOURCC, MHA2_FOURCC, MHAC_CONFIGURATION_VERSION,
    MHAC_FOURCC, MHAC_RECORD_FIXED_LEN, MHM1_FOURCC, MHM2_FOURCC,
};
pub use nalu_types::{AvcPps, AvcSps, AvcSpsExt, HevcNalArray, HevcNalUnit};
pub use opus::{ChannelMappingTable, OpusSpecificBox, DOPS_FOURCC, OPUS_FOURCC};
pub use pipeline::{
    build_init_segment, build_media_segment, CodecConfig, FragmentTrackData, Sample, TrackSpec,
};
pub use progressive::ProgressiveMux;
pub use repackage::{Repackage, RepackageOutput};
pub use sample_entries::{AVCSampleEntry, HEVCSampleEntry, VisualSampleEntryFields};
pub use sample_groups::{
    ProducerReferenceTimeBox, SampleGroupDescriptionBox, SampleToGroupBox, SbgpEntry, SgpdEntry,
    SubSampleDescriptor, SubSampleInformationBox, SubsEntry, GROUPING_TYPE_ROLL,
};
pub use segmenter::Segmenter;
pub use segments::{FileTypeBox, MediaDataBox, SegmentTypeBox};
pub use sps::{
    decode_avc_sps, decode_hevc_sps, rfc6381_avc1, rfc6381_hvc1, rfc6381_mp4a, AvcSpsInfo,
    HevcSpsInfo,
};
pub use subtitle_entries::{
    CueIdBox, CuePayloadBox, CueSettingsBox, VttCueBox, VttEmptyCueBox, WebVttConfigurationBox,
    WvttSampleEntry, XmlSubtitleSampleEntry,
};
pub use timing::{
    CompositionOffsetBox, CompositionToDecodeBox, CttsEntry, EditListBox, EditListEntry,
    SegmentIndexBox, SidxReference, SttsEntry, TimeToSampleBox,
};
pub use ts_demux::TsDemux;
pub use ts_hls::{TsHlsOutput, TsHlsPackager};
pub use ts_mux::TsMux;
pub use visual_ext::{CleanApertureBox, ColourInformationBox, NclxColourInfo, PixelAspectRatioBox};
pub use vp9::{Vp9ConfigurationBox, Vp9SampleEntry, VP09_FOURCC, VPCC_FOURCC};
