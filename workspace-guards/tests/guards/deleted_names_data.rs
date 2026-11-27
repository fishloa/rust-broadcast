//! Names the streaming-first refactors deleted, with the reason a
//! reappearance is a regression.

/// One symbol that must stay gone.
pub struct Banned {
    /// The identifier as it would be spelled in source.
    pub name: &'static str,
    /// Why it may not come back.
    pub reason: &'static str,
    /// The issue that removed it.
    pub issue: &'static str,
}

/// Workspace members whose files are not scanned.
///
/// `workspace-guards` holds the name table itself.
pub const EXEMPT: &[&str] = &["workspace-guards"];

/// Every name a streaming-first slice deleted. A name is matched as a whole
/// identifier, so its replacement (`CmafMuxStage` for `CmafMux`) does not
/// trip it; a name that a live item also spells (`SplicePlan::concat`, the
/// test-side `rtp_run::RtpOutput`) is recorded in the qualified form that
/// names the deleted item instead.
pub const TABLE: &[Banned] = &[
    Banned {
        name: "broadcast_common::Parse",
        reason: "the Parse trait lives in broadcast_core::traits (re-exported at its root)",
        issue: "streaming-first P6 S8",
    },
    Banned {
        name: "broadcast_common::Serialize",
        reason: "the Serialize trait lives in broadcast_core::traits (re-exported at its root)",
        issue: "streaming-first P6 S8",
    },
    Banned {
        name: "broadcast_common::CencScheme",
        reason: "the CENC scheme identity lives in broadcast_core::cenc",
        issue: "streaming-first P6 S8",
    },
    Banned {
        name: "broadcast_common::traits",
        reason: "the Parse/Serialize traits live in broadcast_core::traits",
        issue: "streaming-first P6 S8",
    },
    Banned {
        name: "broadcast_common::len",
        reason: "the checked field-width helpers live in broadcast_core::len",
        issue: "streaming-first P6 S8",
    },
    Banned {
        name: "broadcast_common::bits",
        reason: "bit I/O lives in broadcast_core::bits",
        issue: "streaming-first P6 S8",
    },
    Banned {
        name: "broadcast_common::error",
        reason: "the shared error kinds live in broadcast_core::error",
        issue: "streaming-first P6 S8",
    },
    Banned {
        name: "broadcast_common::reserved",
        reason: "typed reserved-bit fields live in broadcast_core::reserved",
        issue: "streaming-first P6 S8",
    },
    Banned {
        name: "broadcast_common::timescale",
        reason: "the media timescale codec lives in broadcast_core::timescale",
        issue: "streaming-first P6 S8",
    },
    Banned {
        name: "broadcast_common::cenc",
        reason: "the CENC scheme identity lives in broadcast_core::cenc",
        issue: "streaming-first P6 S8",
    },
    Banned {
        name: "broadcast_common::wire",
        reason: "the big-endian field readers live in broadcast_core::wire",
        issue: "streaming-first wr1",
    },
    Banned {
        name: "broadcast_common::impl_spec_display",
        reason: "the Display-from-name macro is broadcast_core::impl_spec_display",
        issue: "streaming-first P6 S8",
    },
    Banned {
        name: "Fmp4Demux",
        reason: "the one MP4 demuxer is transmux::mp4_demux::Mp4Demux",
        issue: "#4057",
    },
    Banned {
        name: "ProgressiveDemux",
        reason: "the one MP4 demuxer is transmux::mp4_demux::Mp4Demux",
        issue: "#4057",
    },
    Banned {
        name: "demux_progressive",
        reason: "the whole-input progressive parse is gone with ProgressiveDemux",
        issue: "#4057",
    },
    Banned {
        name: "CmafMux",
        reason: "the CMAF muxer is the CmafMuxStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "TsMux",
        reason: "the TS muxer is the TsMuxStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "StreamingTsMux",
        reason: "the TS muxer is the TsMuxStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "TsHlsPackager",
        reason: "classic TS-HLS is the StreamingTsHlsSegmenter stage",
        issue: "#4057",
    },
    Banned {
        name: "TsHlsOutput",
        reason: "classic TS-HLS is the StreamingTsHlsSegmenter stage",
        issue: "#4057",
    },
    Banned {
        name: "TsSegment",
        reason: "a segment is a MuxOutput::Segment carrying SegmentOut",
        issue: "#4057",
    },
    Banned {
        name: "MkvMux",
        reason: "the Matroska muxer is the MkvMuxStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "FlvMux",
        reason: "the FLV muxer is the FlvMuxStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "RtmpMux",
        reason: "the RTMP muxer is the RtmpMuxStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "RtpPacketiser",
        reason: "the RTP muxer is the RtpPacketiserStage streaming stage",
        issue: "#4057",
    },
    Banned {
        name: "rtp::RtpOutput",
        reason: "RTP output is MuxOutput from RtpPacketiserStage",
        issue: "#4057",
    },
    Banned {
        name: "transmux::RtpOutput",
        reason: "RTP output is MuxOutput from RtpPacketiserStage",
        issue: "#4057",
    },
    Banned {
        name: "ProgressiveMux",
        reason: "progressive MP4 is PlanPass over ProgressiveMuxStage",
        issue: "#4057",
    },
    Banned {
        name: "CencEncryptor",
        reason: "protection is applied by the CencEncryptStage stage",
        issue: "#4057",
    },
    Banned {
        name: "CencDecryptor",
        reason: "protection is removed by the CencDecryptStage stage",
        issue: "#4057",
    },
    Banned {
        name: "TrackEncryption",
        reason: "per-track crypto defaults live on TrackSpec::protection",
        issue: "#4057",
    },
    Banned {
        name: "UnmuxableDataTrack",
        reason: "an unmapped data track is Error::DataIdentityMismatch",
        issue: "#4057",
    },
    Banned {
        name: "rebase_to_zero",
        reason: "timeline rebasing is the RebaseStage stage",
        issue: "#4057",
    },
    Banned {
        name: "apply_offset",
        reason: "a timeline offset is the OffsetStage stage",
        issue: "#4057",
    },
    Banned {
        name: "insert_discontinuity_gap",
        reason: "a timeline gap is the GapStage stage",
        issue: "#4057",
    },
    Banned {
        name: "trickplay::derive_iframe_track",
        reason: "trick-play derivation is the TrickPlayStage stage",
        issue: "#4057",
    },
    Banned {
        name: "trickplay::append_iframe_track",
        reason: "trick-play derivation is the TrickPlayStage stage",
        issue: "#4057",
    },
    Banned {
        name: "splice::concat",
        reason: "concatenation is the SpliceStage stage over a SplicePlan",
        issue: "#4057",
    },
    Banned {
        name: "splice::splice_insert",
        reason: "SSAI splicing is SplicePlan::insert over the SpliceStage stage",
        issue: "#4057",
    },
    Banned {
        name: "SpliceResult",
        reason: "a splice plan reports SplicePoints; a caller owns the output",
        issue: "#4057",
    },
    Banned {
        name: "SegmentMeta",
        reason: "a segment carries SegmentOut { bytes, summary, tracks }",
        issue: "#4057",
    },
    Banned {
        name: "transmux::SmoothOutput",
        reason: "a Smooth fragment is a SegmentOut from SmoothSegmenter",
        issue: "#4057",
    },
    Banned {
        name: "SmoothFragment",
        reason: "a Smooth fragment is a SegmentOut from SmoothSegmenter",
        issue: "#4057",
    },
    Banned {
        name: "transmux::bitreader",
        reason: "the exp-Golomb/RBSP reader lives in broadcast_bitstream::bitreader",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::rbsp",
        reason: "RBSP emulation-prevention handling lives in broadcast_bitstream::rbsp",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::nal",
        reason: "NAL classification lives in broadcast_bitstream::nal",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::annexb",
        reason: "Annex B / length-prefix conversion lives in broadcast_bitstream::annexb",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::au",
        reason: "access-unit splitting lives in broadcast_bitstream::au",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::NalCodec",
        reason: "the NAL codec family enum lives in broadcast_bitstream::NalCodec",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::nal_unit_type",
        reason: "nal_unit_type lives in broadcast_bitstream::nal",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::is_keyframe_nal",
        reason: "is_keyframe_nal lives in broadcast_bitstream::nal",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::is_idr_nal",
        reason: "is_idr_nal lives in broadcast_bitstream::nal",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::access_unit_is_idr",
        reason: "access_unit_is_idr lives in broadcast_bitstream::nal",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::access_unit_is_keyframe",
        reason: "access_unit_is_keyframe lives in broadcast_bitstream::nal",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::access_unit_is_rap",
        reason: "access_unit_is_rap lives in broadcast_bitstream::nal",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::recovery_point_sei",
        reason: "recovery_point_sei lives in broadcast_bitstream::nal",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::caption_cc_data",
        reason: "caption_cc_data lives in broadcast_bitstream::nal",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::avc_access_unit_parameter_sets",
        reason: "avc_access_unit_parameter_sets lives in broadcast_bitstream::nal (isobmff feature)",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::iter_annexb_nals",
        reason: "iter_annexb_nals lives in broadcast_bitstream::annexb",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::iter_length_prefixed_nals",
        reason: "iter_length_prefixed_nals lives in broadcast_bitstream::annexb",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::iter_length_prefixed_nals_with",
        reason: "iter_length_prefixed_nals_with lives in broadcast_bitstream::annexb",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::annexb_to_length_prefixed",
        reason: "annexb_to_length_prefixed lives in broadcast_bitstream::annexb",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::length_prefixed_to_annexb",
        reason: "length_prefixed_to_annexb lives in broadcast_bitstream::annexb",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::normalise_nal_length_size",
        reason: "normalise_nal_length_size lives in broadcast_bitstream::annexb",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::NAL_LENGTH_SIZE",
        reason: "NAL_LENGTH_SIZE lives in broadcast_bitstream::annexb",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::AccessUnitSplitter",
        reason: "AccessUnitSplitter lives in broadcast_bitstream::au",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::split_access_units",
        reason: "split_access_units lives in broadcast_bitstream::au",
        issue: "broadcast-bitstream bs1",
    },
    Banned {
        name: "transmux::nalu_ext",
        reason: "the ISOBMFF NAL-unit codec-info traits live in broadcast_bitstream::nalu_ext",
        issue: "broadcast-bitstream bs2d",
    },
    Banned {
        name: "transmux::codec_config_ext",
        reason: "the ISOBMFF record codec-info traits live in broadcast_bitstream::codec_config_ext",
        issue: "broadcast-bitstream bs2d",
    },
    Banned {
        name: "transmux::AvcSpsCodecInfo",
        reason: "AvcSpsCodecInfo lives in broadcast_bitstream::nalu_ext",
        issue: "broadcast-bitstream bs2d",
    },
    Banned {
        name: "transmux::HevcNalUnitCodecInfo",
        reason: "HevcNalUnitCodecInfo lives in broadcast_bitstream::nalu_ext",
        issue: "broadcast-bitstream bs2d",
    },
    Banned {
        name: "transmux::HevcRecordCodecInfo",
        reason: "HevcRecordCodecInfo lives in broadcast_bitstream::codec_config_ext",
        issue: "broadcast-bitstream bs2d",
    },
    Banned {
        name: "transmux::VvcRecordCodecInfo",
        reason: "VvcRecordCodecInfo lives in broadcast_bitstream::codec_config_ext",
        issue: "broadcast-bitstream bs2d",
    },
    Banned {
        name: "transmux::avc_config_from_sps_pps",
        reason: "avc_config_from_sps_pps lives in broadcast_bitstream::codec_config_ext",
        issue: "broadcast-bitstream bs2d",
    },
    Banned {
        name: "transmux::hdr10plus",
        reason: "the HDR10+ ST 2094-40 model and its SEI codec live in broadcast_bitstream::hdr10plus",
        issue: "broadcast-bitstream bs2c",
    },
    Banned {
        name: "transmux::Hdr10PlusMetadata",
        reason: "Hdr10PlusMetadata lives in broadcast_bitstream::hdr10plus",
        issue: "broadcast-bitstream bs2c",
    },
    Banned {
        name: "transmux::WindowRegion",
        reason: "WindowRegion lives in broadcast_bitstream::hdr10plus",
        issue: "broadcast-bitstream bs2c",
    },
    Banned {
        name: "transmux::ColorVolumeTransform",
        reason: "ColorVolumeTransform lives in broadcast_bitstream::hdr10plus",
        issue: "broadcast-bitstream bs2c",
    },
    Banned {
        name: "transmux::PeakLuminanceTable",
        reason: "PeakLuminanceTable lives in broadcast_bitstream::hdr10plus",
        issue: "broadcast-bitstream bs2c",
    },
    Banned {
        name: "transmux::BezierToneMapping",
        reason: "BezierToneMapping lives in broadcast_bitstream::hdr10plus",
        issue: "broadcast-bitstream bs2c",
    },
    Banned {
        name: "transmux::DistributionPoint",
        reason: "DistributionPoint lives in broadcast_bitstream::hdr10plus",
        issue: "broadcast-bitstream bs2c",
    },
    Banned {
        name: "transmux::AdditionalWindow",
        reason: "AdditionalWindow lives in broadcast_bitstream::hdr10plus",
        issue: "broadcast-bitstream bs2c",
    },
    Banned {
        name: "transmux::OverlapProcessOption",
        reason: "OverlapProcessOption lives in broadcast_bitstream::hdr10plus",
        issue: "broadcast-bitstream bs2c",
    },
    Banned {
        name: "transmux::HdrStaticMetadata",
        reason: "HdrStaticMetadata lives in broadcast_bitstream::hdr",
        issue: "broadcast-bitstream bs2c",
    },
    Banned {
        name: "transmux::hdr",
        reason: "the HDR static metadata type and its SEI codec live in broadcast_bitstream::hdr",
        issue: "broadcast-bitstream bs2c",
    },
    Banned {
        name: "transmux::VideoScan",
        reason: "the coded interlace facts live in broadcast_bitstream::interlace",
        issue: "broadcast-bitstream bs2b",
    },
    Banned {
        name: "transmux::PicStruct",
        reason: "the coded interlace facts live in broadcast_bitstream::interlace",
        issue: "broadcast-bitstream bs2b",
    },
    Banned {
        name: "transmux::SourceScanType",
        reason: "the coded interlace facts live in broadcast_bitstream::interlace",
        issue: "broadcast-bitstream bs2b",
    },
    Banned {
        name: "transmux::PictureFacts",
        reason: "the coded interlace facts live in broadcast_bitstream::interlace",
        issue: "broadcast-bitstream bs2b",
    },
    Banned {
        name: "transmux::slice_header",
        reason: "the H.264 slice-header and PPS parsers live in broadcast_bitstream::slice_header",
        issue: "broadcast-bitstream bs2b",
    },
    Banned {
        name: "transmux::AvcSliceHeader",
        reason: "AvcSliceHeader lives in broadcast_bitstream::slice_header",
        issue: "broadcast-bitstream bs2b",
    },
    Banned {
        name: "transmux::AvcSlicePps",
        reason: "AvcSlicePps lives in broadcast_bitstream::slice_header",
        issue: "broadcast-bitstream bs2b",
    },
    Banned {
        name: "transmux::AvcParamSets",
        reason: "AvcParamSets lives in broadcast_bitstream::slice_header",
        issue: "broadcast-bitstream bs2b",
    },
    Banned {
        name: "transmux::AvcPicture",
        reason: "AvcPicture lives in broadcast_bitstream::slice_header",
        issue: "broadcast-bitstream bs2b",
    },
    Banned {
        name: "transmux::AvcFieldInfo",
        reason: "AvcFieldInfo lives in broadcast_bitstream::slice_header",
        issue: "broadcast-bitstream bs2b",
    },
    Banned {
        name: "transmux::parse_avc_slice_header",
        reason: "parse_avc_slice_header lives in broadcast_bitstream::slice_header",
        issue: "broadcast-bitstream bs2b",
    },
    Banned {
        name: "transmux::decode_avc_slice_pps",
        reason: "decode_avc_slice_pps lives in broadcast_bitstream::slice_header",
        issue: "broadcast-bitstream bs2b",
    },
    Banned {
        name: "transmux::sps",
        reason: "the SPS decoders live in broadcast_bitstream::sps",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::test_bits",
        reason: "the test bit writer is test_fixtures::bits::BitWriter",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "crate::test_bits",
        reason: "the test bit writer is test_fixtures::bits::BitWriter",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::AvcSpsInfo",
        reason: "AvcSpsInfo lives in broadcast_bitstream::sps",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::HevcSpsInfo",
        reason: "HevcSpsInfo lives in broadcast_bitstream::sps",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::rfc6381_avc1",
        reason: "the per-codec RFC 6381 helpers live in broadcast_bitstream::rfc6381",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::rfc6381_hvc1",
        reason: "the per-codec RFC 6381 helpers live in broadcast_bitstream::rfc6381",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::rfc6381_mp4a",
        reason: "the per-codec RFC 6381 helpers live in broadcast_bitstream::rfc6381",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::AspectRatioIdc",
        reason: "the coded picture facts live in broadcast_bitstream::facts",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::PictureRect",
        reason: "the coded picture facts live in broadcast_bitstream::facts",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::ColourDescription",
        reason: "the coded picture facts live in broadcast_bitstream::facts",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::VideoSignalType",
        reason: "the coded picture facts live in broadcast_bitstream::facts",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::ActiveFormatDescription",
        reason: "the coded picture facts live in broadcast_bitstream::facts",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::BarData",
        reason: "the coded picture facts live in broadcast_bitstream::facts",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::ir::AspectRatioIdc",
        reason: "the coded picture facts live in broadcast_bitstream::facts",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::ir::PictureRect",
        reason: "the coded picture facts live in broadcast_bitstream::facts",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::ir::ColourDescription",
        reason: "the coded picture facts live in broadcast_bitstream::facts",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::ir::VideoSignalType",
        reason: "the coded picture facts live in broadcast_bitstream::facts",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::ir::ActiveFormatDescription",
        reason: "the coded picture facts live in broadcast_bitstream::facts",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::ir::BarData",
        reason: "the coded picture facts live in broadcast_bitstream::facts",
        issue: "broadcast-bitstream bs2a",
    },
    Banned {
        name: "transmux::aac_asc",
        reason: "the AAC AudioSpecificConfig and ADTS header live in broadcast_bitstream::aac_asc",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::AudioSpecificConfig",
        reason: "AudioSpecificConfig lives in broadcast_bitstream::aac_asc",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::AudioObjectType",
        reason: "AudioObjectType lives in broadcast_bitstream::aac_asc",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::SamplingFrequencyIndex",
        reason: "SamplingFrequencyIndex lives in broadcast_bitstream::aac_asc",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::ChannelConfiguration",
        reason: "ChannelConfiguration lives in broadcast_bitstream::aac_asc",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::HeAacProfile",
        reason: "HeAacProfile lives in broadcast_bitstream::aac_asc",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::HeAacSignaling",
        reason: "HeAacSignaling lives in broadcast_bitstream::aac_asc",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::AdtsHeader",
        reason: "AdtsHeader lives in broadcast_bitstream::aac_asc",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::parse_adts_header",
        reason: "parse_adts_header lives in broadcast_bitstream::aac_asc",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::build_adts_header",
        reason: "build_adts_header lives in broadcast_bitstream::aac_asc",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::ac3_syncframe",
        reason: "the AC-3/E-AC-3 syncframe parsers live in broadcast_bitstream::ac3_syncframe",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::Ac3SyncframeInfo",
        reason: "Ac3SyncframeInfo lives in broadcast_bitstream::ac3_syncframe",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::Ec3SyncframeInfo",
        reason: "Ec3SyncframeInfo lives in broadcast_bitstream::ac3_syncframe",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::Ec3SplitFrame",
        reason: "Ec3SplitFrame lives in broadcast_bitstream::ac3_syncframe",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::dts_core",
        reason: "the DTS core frame-header parsers live in broadcast_bitstream::dts_core",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::DtsCoreFrameInfo",
        reason: "DtsCoreFrameInfo lives in broadcast_bitstream::dts_core",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::DtsCoreFrame",
        reason: "DtsCoreFrame lives in broadcast_bitstream::dts_core",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::mhas",
        reason: "the MHAS packet framing lives in broadcast_bitstream::mhas",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::find_mpegh3da_config",
        reason: "find_mpegh3da_config lives in broadcast_bitstream::mhas",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::walk_mhas_packets",
        reason: "walk_mhas_packets lives in broadcast_bitstream::mhas",
        issue: "broadcast-bitstream bs3a",
    },
    Banned {
        name: "transmux::mpeg_legacy::MpegAudioLayer",
        reason: "the MPEG audio frame header lives in broadcast_bitstream::mpeg_audio",
        issue: "broadcast-bitstream bs5",
    },
    Banned {
        name: "transmux::mpeg_legacy::MpegAudioFrameHeader",
        reason: "the MPEG audio frame header lives in broadcast_bitstream::mpeg_audio",
        issue: "broadcast-bitstream bs5",
    },
    Banned {
        name: "transmux::MpegAudioLayer",
        reason: "the MPEG audio frame header lives in broadcast_bitstream::mpeg_audio",
        issue: "broadcast-bitstream bs5",
    },
    Banned {
        name: "transmux::MpegAudioFrameHeader",
        reason: "the MPEG audio frame header lives in broadcast_bitstream::mpeg_audio",
        issue: "broadcast-bitstream bs5",
    },
    Banned {
        name: "transmux::MPEG_AUDIO_SYNCWORD",
        reason: "the MPEG audio syncword lives in broadcast_bitstream::mpeg_audio",
        issue: "broadcast-bitstream bs5",
    },
];
