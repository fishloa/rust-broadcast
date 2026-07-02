//! `transmux` — an any-to-any media container muxing hub (ISO/IEC 14496-12,
//! 13818-1, 23009-1, RFC 8216/3550, [MS-SSTR]).
//!
//! transmux demuxes any supported container into one neutral in-memory
//! intermediate representation ([`Media`] / [`Track`]) and muxes from it into any
//! supported container — the demux and mux spokes meet at the IR, so every
//! `{input} → {output}` combination composes. It **never encodes or decodes
//! media**: it parses codec *config/parameter headers* only to build container
//! boxes and derive metadata; coded samples pass through opaque. `no_std` +
//! `alloc`.
//!
//! The spokes are expressed through the two abstract traits in `broadcast_common`:
//! [`Unpackage`](broadcast_common::Unpackage) (container → IR) and
//! [`Package`](broadcast_common::Package) (IR → container), the inverse pair
//! mirroring `Parse` / `Serialize`.
//!
//! - **Demux (`Unpackage`) inputs:** MPEG-2 TS ([`TsDemux`]), fMP4/CMAF
//!   ([`Fmp4Demux`]), MPEG Program Stream ([`PsDemux`]), WebM/Matroska
//!   ([`WebmDemux`]).
//! - **Mux (`Package`) outputs:** CMAF/fMP4 ([`CmafMux`]), progressive single-file
//!   MP4 ([`ProgressiveMux`]), MPEG-2 TS ([`TsMux`]), CMAF-HLS ([`HlsPackager`]),
//!   TS-segment HLS ([`TsHlsPackager`]), DASH MPD ([`DashPackager`]), low-latency
//!   DASH ([`LlSegmenter`]/[`LlDashPackager`]), Microsoft Smooth Streaming
//!   ([`SmoothPackager`]).
//! - **Transforms:** resegment / trim / track-select ([`Repackage`]);
//!   streaming CMAF segmentation ([`Segmenter`]).
//! - **Crypto:** CENC (`cenc`, AES-CTR) decrypt ([`CencDecryptor`]).
//! - **RTP:** de/packetize ([`RtpPacketizer`] / [`RtpDepacketizer`]) + SDP.
//!
//! **Codec config coverage** (header parse → container box; no en/decode):
//! H.264/AVC, H.265/HEVC, H.266/VVC, AV1, VP9, VP8, MPEG-2 video (H.262); AAC,
//! AC-3, E-AC-3, AC-4, DTS, Opus, FLAC, Vorbis, MPEG-1/2 audio (MP1/2/3), MPEG-H
//! 3D Audio.
//!
//! Lower-level entry points remain: [`build_init_segment`]/[`build_media_segment`]
//! (batch CMAF), [`AvcSps::decode`] + [`AvcSps::rfc6381`] (codec metadata). See the
//! crate README for the full **feature matrix** and the `examples/` directory for
//! runnable end-to-end walkthroughs.
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
//! | `cenc`  | yes     | CENC (ISO/IEC 23001-7) AES-CTR sample decryption ([`CencDecryptor`]) via the RustCrypto `aes`/`ctr` crates |

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
#[cfg(feature = "cenc")]
pub mod cenc_decrypt;
pub mod dash;
pub mod dts;
pub mod error;
pub mod flac;
pub mod hevc_config;
pub mod hls;
pub mod init_segment;
pub mod ll_dash;
pub mod media;
pub mod movie_fragment;
pub mod mp4esds;
pub mod mpeg_legacy;
pub mod mpegh;
pub mod nalu_types;
pub mod opus;
pub mod pipeline;
pub mod progressive;
pub mod ps_demux;
pub mod repackage;
pub mod rtcp;
pub mod rtp;
pub mod sample_entries;
pub mod sample_groups;
pub mod segmenter;
pub mod segments;
pub mod smooth;
pub mod sps;
pub mod subtitle_entries;
pub mod timing;
pub mod ts_demux;
pub mod ts_hls;
pub mod ts_mux;
pub mod visual_ext;
pub mod vp9;
pub mod vvc_config;
pub mod webm_demux;

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
#[cfg(feature = "cenc")]
pub use cenc_decrypt::{CencDecryptor, CencScheme, KeyMap};
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
pub use ll_dash::{Chunk, LlDashPackager, LlSegmenter};
pub use media::{CmafMux, Fmp4Demux, HlsPackager, Media, Track};
pub use movie_fragment::{
    MovieFragmentBox, MovieFragmentHeaderBox, TrackFragmentBaseMediaDecodeTimeBox,
    TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentRunBox, TrunSample,
};
pub use mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, ObjectTypeIndication,
    SLConfigDescriptor, StreamType,
};
pub use mpeg_legacy::{
    Mpeg2SeqHeader, MpegAudioFrameHeader, MpegAudioLayer, MPEG_AUDIO_SYNCWORD, SEQUENCE_HEADER_CODE,
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
pub use ps_demux::PsDemux;
pub use repackage::{Repackage, RepackageOutput};
pub use rtcp::{
    App, Bye, CommonHeader, CompoundPacket, ReceiverReport, ReportBlock, RtcpPacket,
    RtcpPacketType, SdesChunk, SdesItem, SdesItemType, SenderReport, SourceDescription,
    APP_NAME_LEN, PT_APP, PT_BYE, PT_RECEIVER_REPORT, PT_SENDER_REPORT, PT_SOURCE_DESCRIPTION,
    REPORT_BLOCK_LEN, SDES_CNAME, SDES_EMAIL, SDES_LOC, SDES_NAME, SDES_NOTE, SDES_PHONE,
    SDES_PRIV, SDES_TOOL,
};
pub use rtp::{
    RtpDepacketizer, RtpInput, RtpInputStream, RtpMediaKind, RtpOutput, RtpPacketizer, RtpStream,
    DEFAULT_AUDIO_PT, DEFAULT_MTU, DEFAULT_VIDEO_PT, NAL_TYPE_IDR, VIDEO_CLOCK_RATE,
};
pub use sample_entries::{
    AVCSampleEntry, HEVCSampleEntry, Mp4vSampleEntry, VisualSampleEntryFields,
};
pub use sample_groups::{
    ProducerReferenceTimeBox, SampleGroupDescriptionBox, SampleToGroupBox, SbgpEntry, SgpdEntry,
    SubSampleDescriptor, SubSampleInformationBox, SubsEntry, GROUPING_TYPE_ROLL,
};
pub use segmenter::Segmenter;
pub use segments::{FileTypeBox, MediaDataBox, SegmentTypeBox};
pub use smooth::{
    SmoothFragment, SmoothOutput, SmoothPackager, SmoothStreamType, TfxdBox, FOURCC_AACL,
    FOURCC_H264, SMOOTH_TIMESCALE, TFXD_UUID,
};
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
pub use vvc_config::{
    VvcConfigurationBox, VvcDecoderConfigurationRecord, VvcNalArray, VvcNalUnitType, VvcPtlRecord,
};
pub use webm_demux::{WebmDemux, IR_TIMESCALE};
