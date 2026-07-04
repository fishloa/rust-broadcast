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
//! - **Demux (`Unpackage`) inputs:** MPEG-2 TS ([`TsDemux`], a thin batch
//!   wrapper over the event-driven [`StreamingTsDemux`] — issue #555),
//!   fMP4/CMAF ([`Fmp4Demux`]), progressive (non-fragmented) MP4
//!   ([`ProgressiveDemux`] — issue #561), MPEG Program Stream ([`PsDemux`]),
//!   WebM/Matroska ([`WebmDemux`]), FLV ([`FlvDemux`]), RTMP ([`RtmpDemux`]).
//! - **Mux (`Package`) outputs:** CMAF/fMP4 ([`CmafMux`]), progressive single-file
//!   MP4 ([`ProgressiveMux`]), MPEG-2 TS ([`TsMux`]), CMAF-HLS ([`HlsPackager`]),
//!   TS-segment HLS ([`TsHlsPackager`]), DASH MPD ([`DashPackager`]), low-latency
//!   DASH ([`LlSegmenter`]/[`LlDashPackager`]), Microsoft Smooth Streaming
//!   ([`SmoothPackager`]), RTMP ([`RtmpMux`]), low-latency HLS
//!   ([`LlHlsSegmenter`]).
//! - **Transforms:** resegment / trim / track-select ([`Repackage`]);
//!   streaming CMAF segmentation ([`Segmenter`]); IR timeline conditioning —
//!   PTS/DTS rebase, offset, 33-bit MPEG wrap-unroll, discontinuity-gap
//!   insertion ([`rebase_to_zero`] / [`apply_offset`] / [`unroll_33bit_wraps`] /
//!   [`insert_discontinuity_gap`], via each [`Track::start_decode_time`] anchor);
//!   timeline splice / concatenation → SSAI ([`concat`](fn@concat) / [`splice_insert`],
//!   returning a [`SpliceResult`] with discontinuity points).
//! - **Crypto:** CENC (`cenc`, AES-CTR) decrypt ([`CencDecryptor`]); HLS
//!   Sample-AES + full-segment AES-128 encrypt/decrypt (`sample-aes`,
//!   [`sample_aes`]).
//! - **RTP/RTCP:** de/packetize ([`RtpPacketizer`] / [`RtpDepacketizer`]) + SDP;
//!   RTCP control packets ([`RtcpPacket`]).
//! - **Conformance:** structural fMP4/CMAF validator ([`validate_init_segment`]
//!   / [`validate_media_segment`] / [`validate_cmaf_track`]).
//! - **Utilities:** NAL keyframe classification ([`is_keyframe_nal`] /
//!   [`nal_unit_type`]); I-frame trick-play track derivation ([`derive_iframe_track`]).
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
//! | `sample-aes` | no | HLS Sample-AES + full-segment AES-128 (AES-128-CBC) content protection ([`sample_aes`]); implies `cenc`, adds the RustCrypto `cbc` crate |
//! | `cli`   | no      | the `transmux` command-line packager binary (`clap`; implies `std`) — see [`cli`] and `docs/CLI-STANDARD.md` |

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
#[cfg(feature = "cli")]
pub mod cli;
pub mod dash;
pub mod drm;
pub mod dts;
pub mod error;
pub mod flac;
pub mod flv;
pub mod hevc_config;
pub mod hls;
pub mod init_segment;
pub mod klv;
pub mod ll_dash;
pub mod ll_hls;
pub mod media;
pub mod movie_fragment;
pub mod mp4esds;
pub mod mpeg_legacy;
pub mod mpegh;
pub mod nal;
pub mod nalu_types;
pub mod opus;
pub mod pipeline;
pub mod progressive;
pub mod progressive_demux;
pub mod ps_demux;
pub mod rebase;
pub mod repackage;
pub mod rtcp;
pub mod rtmp;
pub mod rtp;
#[cfg(feature = "sample-aes")]
pub mod sample_aes;
pub mod sample_entries;
pub mod sample_groups;
pub mod segmenter;
pub mod segments;
pub mod smooth;
pub mod splice;
pub mod sps;
pub mod subtitle_entries;
pub mod timing;
pub mod trickplay;
pub mod ts_demux;
pub mod ts_hls;
pub mod ts_mux;
pub mod validate;
pub mod visual_ext;
pub mod vp9;
pub mod vvc_config;
pub mod webm_demux;

pub use aac_asc::{
    AdtsHeader, AudioObjectType, AudioSpecificConfig, ChannelConfiguration, HeAacSignaling,
    SamplingFrequencyIndex, build_adts_header, parse_adts_header,
};
pub use ac3::{Ac3SpecificBox, Ac3SyncframeInfo, Ec3SpecificBox, Ec3Substream, Ec3SyncframeInfo};
pub use ac4::{AC4_FOURCC, Ac4SpecificBox, DAC4_FOURCC};
pub use annexb::{
    NAL_LENGTH_SIZE, annexb_to_length_prefixed, iter_annexb_nals, iter_length_prefixed_nals,
    length_prefixed_to_annexb,
};
pub use av1::{AV01_FOURCC, AV1C_FOURCC, Av1ConfigurationBox, Av1SampleEntry};
pub use avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
pub use box_types::{BoxHeader, BoxIter, BoxRef, BoxType, FullBoxHeader, box_iter, parse_box};
pub use cenc::{
    OriginalFormatBox, ProtectionSchemeInfoBox, ProtectionSystemSpecificHeaderBox,
    SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION, SampleAuxInfoOffsetsBox, SampleAuxInfoSizesBox,
    SampleEncryptionBox, SampleEncryptionEntry, SchemeInformationBox, SchemeTypeBox,
    SubSampleEntry, TrackEncryptionBox,
};
#[cfg(feature = "cenc")]
pub use cenc_decrypt::{CencDecryptor, CencScheme, KeyMap};
pub use dash::{
    DashPackager, MPD_NAMESPACE, MediaKind, PROFILE_ISOFF_LIVE, TRICKMODE_SCHEME,
    TrickModeAdaptationSet, TrickModeRepr,
};
pub use drm::{
    COMMON_SYSTEM_ID, FAIRPLAY_SYSTEM_ID, PLAYREADY_SYSTEM_ID, WIDEVINE_SYSTEM_ID,
    cenc_kid_to_playready, fairplay_pssh, fairplay_pssh_data, playready_kid_to_cenc,
    playready_kid_value_decode, playready_pro, playready_pssh, playready_wrmheader, widevine_pssh,
    widevine_pssh_data,
};
pub use dts::{
    DDTS_BODY_LEN, DDTS_FOURCC, DTSC_FOURCC, DTSE_FOURCC, DTSH_FOURCC, DTSL_FOURCC, DtsSpecificBox,
};
pub use error::{Error, Result};
pub use flac::{
    BLOCK_TYPE_STREAMINFO, DFLA_FOURCC, FLAC_FOURCC, FlacMetadataBlock, FlacSpecificBox,
};
pub use flv::{FlvDemux, FlvError, FlvMux};
pub use hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
pub use hls::{
    IFrameVariant, LowLatencyConfig, MasterPlaylist, MediaPlaylist, MediaSegment, PartSpec,
    Variant, mark_init_discontinuities,
};
pub use init_segment::{
    Ac3SampleEntry, Ac4SampleEntry, ChunkLargeOffsetBox, ChunkOffsetBox, DataEntryUrlBox,
    DataInformationBox, DataReferenceBox, DtsSampleEntry, Ec3SampleEntry, EditBox, FlacSampleEntry,
    HandlerBox, MediaBox, MediaHeaderBox, MediaInformationBox, MhaSampleEntry, MovieBox,
    MovieExtendsBox, MovieHeaderBox, Mp4aSampleEntry, OpaqueBox, OpusSampleEntry,
    SampleDescriptionBox, SampleEntryVariant, SampleSizeBox, SampleTableBox, SampleToChunkBox,
    SoundMediaHeaderBox, StblChild, StscEntry, SyncSampleBox, TrackBox, TrackExtendsBox,
    TrackHeaderBox, VideoMediaHeaderBox,
};
pub use klv::{
    CHECKSUM_LEN, KlvItem, LocalSetItem, PRECISION_TIMESTAMP_LEN, TAG_CHECKSUM,
    TAG_PRECISION_TIMESTAMP, UAS_LS_KEY, UNIVERSAL_LABEL_LEN, UasLocalSet, UniversalLabel,
    ber_length, ber_oid, crc16_ccitt, encode_ber_length, encode_ber_oid,
};
pub use ll_dash::{Chunk, LlDashPackager, LlSegmenter};
pub use ll_hls::{LlHlsSegmenter, PartInfo, SegmentInfo};
pub use media::{CmafMux, Fmp4Demux, HlsPackager, Media, PcrSample, Track};
pub use movie_fragment::{
    MovieFragmentBox, MovieFragmentHeaderBox, TrackFragmentBaseMediaDecodeTimeBox,
    TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentRunBox, TrunSample,
};
pub use mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, ObjectTypeIndication,
    SLConfigDescriptor, StreamType,
};
pub use mpeg_legacy::{
    MPEG_AUDIO_SYNCWORD, Mpeg2SeqHeader, MpegAudioFrameHeader, MpegAudioLayer, SEQUENCE_HEADER_CODE,
};
pub use mpegh::{
    MHA1_FOURCC, MHA2_FOURCC, MHAC_CONFIGURATION_VERSION, MHAC_FOURCC, MHAC_RECORD_FIXED_LEN,
    MHADecoderConfigurationRecord, MHM1_FOURCC, MHM2_FOURCC,
};
pub use nal::{NalCodec, access_unit_is_keyframe, is_keyframe_nal, nal_unit_type};
pub use nalu_types::{AvcPps, AvcSps, AvcSpsExt, HevcNalArray, HevcNalUnit};
pub use opus::{ChannelMappingTable, DOPS_FOURCC, OPUS_FOURCC, OpusSpecificBox};
pub use pipeline::{
    CodecConfig, EmsgBox, EmsgVersion, FragmentTrackData, PresentationTime, Sample, SourceTiming,
    TrackSpec, build_init_segment, build_media_segment, build_media_segment_with_events,
};
pub use progressive::ProgressiveMux;
pub use progressive_demux::ProgressiveDemux;
pub use ps_demux::PsDemux;
pub use rebase::{
    MPEG_TS_WRAP, apply_offset, insert_discontinuity_gap, rebase_to_zero, unroll_33bit_wraps,
};
pub use repackage::{Repackage, RepackageOutput};
pub use rtcp::{
    APP_NAME_LEN, App, Bye, CommonHeader, CompoundPacket, PT_APP, PT_BYE, PT_RECEIVER_REPORT,
    PT_SENDER_REPORT, PT_SOURCE_DESCRIPTION, REPORT_BLOCK_LEN, ReceiverReport, ReportBlock,
    RtcpPacket, RtcpPacketType, SDES_CNAME, SDES_EMAIL, SDES_LOC, SDES_NAME, SDES_NOTE, SDES_PHONE,
    SDES_PRIV, SDES_TOOL, SdesChunk, SdesItem, SdesItemType, SenderReport, SourceDescription,
};
pub use rtmp::{
    AmfValue, BasicHeader, Command, DEFAULT_CHUNK_SIZE, HANDSHAKE_PACKET_LEN, Handshake0,
    Handshake1, Handshake2, Message, MessageHeader, ProtocolControl, RTMP_VERSION, RtmpDemux,
    RtmpError, RtmpMux,
};
pub use rtp::{
    DEFAULT_AUDIO_PT, DEFAULT_KLV_PT, DEFAULT_MTU, DEFAULT_VIDEO_PT, KLV_ENCODING_NAME,
    NAL_TYPE_IDR, RtpDepacketizer, RtpInput, RtpInputStream, RtpMediaKind, RtpOutput,
    RtpPacketizer, RtpStream, VIDEO_CLOCK_RATE, depacketize_klv, packetize_klv,
};
#[cfg(feature = "sample-aes")]
pub use sample_aes::{
    ExtXKey, HlsEncryptionMethod, aac_decrypt_frame, aac_encrypt_frame, ac3_decrypt_frame,
    ac3_encrypt_frame, aes128_decrypt_segment, aes128_encrypt_segment, h264_decrypt_nal,
    h264_encrypt_nal, iv_from_sequence_number,
};
pub use sample_entries::{
    AVCSampleEntry, HEVCSampleEntry, Mp4vSampleEntry, VisualSampleEntryFields,
};
pub use sample_groups::{
    GROUPING_TYPE_ROLL, ProducerReferenceTimeBox, SampleGroupDescriptionBox, SampleToGroupBox,
    SbgpEntry, SgpdEntry, SubSampleDescriptor, SubSampleInformationBox, SubsEntry,
};
pub use segmenter::{SegmentMeta, Segmenter};
pub use segments::{FileTypeBox, MediaDataBox, SegmentTypeBox};
pub use smooth::{
    FOURCC_AACL, FOURCC_H264, SMOOTH_TIMESCALE, SmoothFragment, SmoothOutput, SmoothPackager,
    SmoothStreamType, TFXD_UUID, TfxdBox,
};
pub use splice::{SplicePoint, SpliceResult, concat, snap_to_preceding_sync, splice_insert};
pub use sps::{
    AvcSpsInfo, HevcSpsInfo, decode_avc_sps, decode_hevc_sps, rfc6381_avc1, rfc6381_hvc1,
    rfc6381_mp4a,
};
pub use subtitle_entries::{
    CueIdBox, CuePayloadBox, CueSettingsBox, VttCueBox, VttEmptyCueBox, WebVttConfigurationBox,
    WvttSampleEntry, XmlSubtitleSampleEntry,
};
pub use timing::{
    CompositionOffsetBox, CompositionToDecodeBox, CttsEntry, EditListBox, EditListEntry,
    SegmentIndexBox, SidxReference, SttsEntry, TimeToSampleBox,
};
pub use trickplay::{append_iframe_track, derive_iframe_track};
pub use ts_demux::{DemuxEvent, StreamingTsDemux, TsDemux};
pub use ts_hls::{TsHlsOutput, TsHlsPackager};
pub use ts_mux::TsMux;
pub use validate::{
    ConformanceIssue, Severity, validate_cmaf_track, validate_init_segment, validate_media_segment,
};
pub use visual_ext::{CleanApertureBox, ColourInformationBox, NclxColourInfo, PixelAspectRatioBox};
pub use vp9::{VP09_FOURCC, VPCC_FOURCC, Vp9ConfigurationBox, Vp9SampleEntry};
pub use vvc_config::{
    VvcConfigurationBox, VvcDecoderConfigurationRecord, VvcNalArray, VvcNalUnitType, VvcPtlRecord,
};
pub use webm_demux::{IR_TIMESCALE, WebmDemux};
