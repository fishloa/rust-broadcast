//! Per-track codec configuration — [`CodecConfig`] + [`DataCarriage`].
//!
//! Moved out of `pipeline.rs` (media plane step 2a, no-op): same types, same
//! fields, same impls.

use alloc::vec::Vec;

use crate::ac3::{Ac3SpecificBox, Ec3SpecificBox};
use crate::ac4::Ac4SpecificBox;
use crate::av1::Av1ConfigurationBox;
use crate::avc_config::AVCConfigurationBox;
use crate::dts::DtsSpecificBox;
use crate::flac::FlacSpecificBox;
use crate::hevc_config::HEVCConfigurationBox;
use crate::mp4esds::EsdsBox;
use crate::opus::OpusSpecificBox;
use crate::vp9::Vp9ConfigurationBox;
use crate::vvc_config::VvcConfigurationBox;

/// Whether a [`CodecConfig::Data`] elementary stream carries PES packets or
/// PSI/private sections on its PID.
///
/// ISO/IEC 13818-1 §2.4.4.8 / Table 2-34 splits `stream_type` into two
/// carriage families: most types (subtitles, teletext, SMPTE 2038 ANC,
/// metadata, and any unrecognised value) are PES-packetised (§2.4.3.6); a
/// fixed set (`0x05` private_sections, `0x0A`-`0x0D` DSM-CC, `0x14` DSM-CC
/// synchronized download, `0x86` SCTE-35/ANSI-scoped) are carried as raw
/// PSI-style sections instead (§2.4.4), with no PES header at all.
/// Reassembling a section stream with a PES parser (or vice versa) silently
/// yields nothing, so a demuxer/muxer must dispatch on this before touching
/// the payload — see `crate::ts_demux` / `crate::ts_mux` (issue #576).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataCarriage {
    /// The elementary stream is PES-packetised (ISO/IEC 13818-1 §2.4.3.6).
    Pes,
    /// The elementary stream carries PSI/private sections directly on its
    /// PID (ISO/IEC 13818-1 §2.4.4), with no PES header at all.
    Sections,
}

impl DataCarriage {
    /// A short label for this carriage kind.
    pub fn name(&self) -> &'static str {
        match self {
            DataCarriage::Pes => "pes",
            DataCarriage::Sections => "sections",
        }
    }
}

broadcast_common::impl_spec_display!(DataCarriage);

/// Per-track codec configuration for the initialization segment.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum CodecConfig {
    /// H.264/AVC video (`avc1` sample entry with an `avcC` config box).
    Avc {
        /// The `avcC` decoder configuration record.
        config: AVCConfigurationBox,
        /// Coded width in pixels.
        width: u16,
        /// Coded height in pixels.
        height: u16,
    },
    /// H.265/HEVC video (`hvc1`/`hev1` sample entry with an `hvcC` config box) —
    /// ISO/IEC 14496-15:2017 §8.4.
    Hevc {
        /// The `hvcC` decoder configuration record box.
        config: HEVCConfigurationBox,
        /// Coded width in pixels.
        width: u16,
        /// Coded height in pixels.
        height: u16,
    },
    /// H.266/VVC video (`vvc1`/`vvi1` sample entry with a `vvcC` config box) —
    /// ISO/IEC 14496-15:2022 §11.3.3.
    Vvc {
        /// The `vvcC` decoder configuration record box.
        config: VvcConfigurationBox,
        /// Coded width in pixels.
        width: u16,
        /// Coded height in pixels.
        height: u16,
    },
    /// AAC audio (`mp4a` sample entry with an `esds` box).
    Aac {
        /// The `esds` box carrying the AudioSpecificConfig.
        esds: EsdsBox,
        /// Channel count.
        channel_count: u16,
        /// Sampling rate in Hz (stored 16.16 in the sample entry).
        sample_rate: u32,
        /// Sample size in bits (typically 16).
        sample_size: u16,
    },
    /// AC-3 audio (`ac-3` sample entry with a `dac3` box).
    Ac3 {
        /// The `dac3` config box.
        config: Ac3SpecificBox,
        /// Channel count.
        channel_count: u16,
        /// Sampling rate in Hz.
        sample_rate: u32,
        /// Sample size in bits (typically 16).
        sample_size: u16,
    },
    /// E-AC-3 audio (`ec-3` sample entry with a `dec3` box).
    Eac3 {
        /// The `dec3` config box.
        config: Ec3SpecificBox,
        /// Channel count.
        channel_count: u16,
        /// Sampling rate in Hz.
        sample_rate: u32,
        /// Sample size in bits (typically 16).
        sample_size: u16,
    },
    /// AV1 video (`av01` sample entry with an `av1C` box).
    Av1 {
        /// The `av1C` configuration box.
        config: Av1ConfigurationBox,
        /// Coded width in pixels.
        width: u16,
        /// Coded height in pixels.
        height: u16,
    },
    /// VP9 video (`vp09` sample entry with a `vpcC` box).
    Vp9 {
        /// The `vpcC` configuration box.
        config: Vp9ConfigurationBox,
        /// Coded width in pixels.
        width: u16,
        /// Coded height in pixels.
        height: u16,
    },
    /// Opus audio (`Opus` sample entry with a `dOps` box).
    Opus {
        /// The `dOps` config box.
        config: OpusSpecificBox,
        /// Channel count.
        channel_count: u16,
        /// Sampling rate in Hz (stored 16.16; per spec always 48000).
        sample_rate: u32,
        /// Sample size in bits (typically 16).
        sample_size: u16,
    },
    /// FLAC audio (`fLaC` sample entry with a `dfLa` box).
    Flac {
        /// The `dfLa` config box.
        config: FlacSpecificBox,
        /// Channel count.
        channel_count: u16,
        /// Sampling rate in Hz.
        sample_rate: u32,
        /// Sample size in bits.
        sample_size: u16,
    },
    /// AC-4 audio (`ac-4` sample entry with a `dac4` box).
    Ac4 {
        /// The `dac4` config box (opaque `ac4_dsi_v1()`).
        config: Ac4SpecificBox,
        /// Channel count.
        channel_count: u16,
        /// Sampling rate in Hz.
        sample_rate: u32,
        /// Sample size in bits (16 per spec).
        sample_size: u16,
    },
    /// MPEG-H 3D Audio (`mha1` sample entry with an `mhaC` box) — ISO/IEC 23008-3 §20.
    ///
    /// Use `mha1` for raw MHAS frames (config in `mhaC`).  For in-band MHAS
    /// (`mhm1`) the caller should convert the `codec_type` on the resulting
    /// [`MhaSampleEntry`](crate::init_segment::MhaSampleEntry) if needed;
    /// `build_trak` always emits `mha1`.
    MpegH {
        /// The `MHADecoderConfigurationRecord` carried in the `mhaC` box.
        config: crate::mpegh::MHADecoderConfigurationRecord,
        /// Channel count.
        channel_count: u16,
        /// Sampling rate in Hz.
        sample_rate: u32,
        /// Sample size in bits (typically 16).
        sample_size: u16,
    },
    /// MPEG-2 video / H.262 (`mp4v` sample entry with an `esds` box) —
    /// ISO/IEC 13818-2 (ITU-T H.262) carried per ISO/IEC 14496-1 §7.2.6.6
    /// (ObjectTypeIndication 0x60–0x65, 0x61 = MPEG-2 Main Visual).
    ///
    /// The `esds` carries the ES/decoder descriptors (its DecoderSpecificInfo
    /// optionally the `sequence_header()` bytes); the coded picture geometry is
    /// decoded from the in-band `sequence_header()` (ISO/IEC 13818-2 §6.2.2.1).
    Mpeg2Video {
        /// The `esds` box (its body is re-embedded byte-identically).
        esds: EsdsBox,
        /// Coded width in pixels (from the `sequence_header()`).
        width: u16,
        /// Coded height in pixels (from the `sequence_header()`).
        height: u16,
    },
    /// MPEG-1/2 audio, Layers I/II/III (`mp4a` sample entry with an `esds` box)
    /// — ISO/IEC 11172-3 / ISO/IEC 13818-3, carried per ISO/IEC 14496-1
    /// §7.2.6.6 (ObjectTypeIndication 0x69 = MPEG-2 audio, 0x6B = MPEG-1 audio).
    MpegAudio {
        /// The `esds` box (OTI 0x69/0x6B; DecoderSpecificInfo usually empty).
        esds: EsdsBox,
        /// Audio layer (1/2/3), from the first frame header.
        layer: crate::mpeg_legacy::MpegAudioLayer,
        /// Channel count.
        channel_count: u16,
        /// Sampling rate in Hz.
        sample_rate: u32,
        /// Sample size in bits (typically 16).
        sample_size: u16,
    },
    /// DTS audio (`dtsc`/`dtsh`/`dtsl`/`dtse` sample entry with a `ddts` box) —
    /// ETSI TS 102 114 §E.2.
    ///
    /// `codec_fourcc` selects the sample-entry FourCC:
    /// `dtsc` (core only), `dtsh` (core + extension / multi-asset),
    /// `dtsl` (LBR only), or `dtse` (extension substream only).
    /// Use [`crate::dts::DTSC_FOURCC`] etc. for the named constants.
    Dts {
        /// The `ddts` DTSSpecificBox.
        config: DtsSpecificBox,
        /// Sample-entry FourCC: one of `dtsc`, `dtsh`, `dtsl`, `dtse`.
        codec_fourcc: [u8; 4],
        /// Channel count.
        channel_count: u16,
        /// Sampling rate in Hz (48000, 44100, or 32000 per §E.2.2.2).
        sample_rate: u32,
        /// Sample size in bits (always 16 per §E.2.2.2).
        sample_size: u16,
    },
    /// VP8 video (WebM-native; RFC 6386).
    ///
    /// VP8 has **no** out-of-band configuration box — the coded dimensions are
    /// carried in the key-frame header (RFC 6386 §9.1) and decoded from the first
    /// key frame at demux time (see `transmux/docs/codec/vp8-vorbis-webm.md`).
    /// Carried in the IR for `{WebM} → IR → {WebM}` / inspection; there is no
    /// ISOBMFF sample entry for VP8 in this crate (out of scope), so it does not
    /// participate in the fMP4 mux path.
    Vp8 {
        /// Coded width in pixels (key-frame header, masked `& 0x3FFF`).
        width: u16,
        /// Coded height in pixels (key-frame header, masked `& 0x3FFF`).
        height: u16,
    },
    /// Vorbis audio (WebM-native; Vorbis I specification, xiph.org).
    ///
    /// The three setup headers (Identification / Comment / Setup) are carried
    /// verbatim in `codec_private` (Xiph-laced, exactly as WebM `CodecPrivate`
    /// stores them — see `transmux/docs/codec/vp8-vorbis-webm.md`); `channels`
    /// and `sample_rate` are decoded from the Identification header (Vorbis I
    /// §4.2.2). Carried in the IR for `{WebM} → IR → {WebM}` / inspection; there
    /// is no ISOBMFF sample entry for Vorbis in this crate (out of scope), so it
    /// does not participate in the fMP4 mux path.
    Vorbis {
        /// The Xiph-laced 3-header `CodecPrivate`, verbatim.
        codec_private: Vec<u8>,
        /// Channel count (`audio_channels`, Vorbis I §4.2.2).
        channels: u16,
        /// Sampling rate in Hz (`audio_sample_rate`, Vorbis I §4.2.2).
        sample_rate: u32,
    },
    /// Opaque data track (issue #557/#576): a PMT-listed elementary stream
    /// whose `stream_type` is not a codec this crate decodes (DVB subtitles
    /// EN 300 743, teletext EN 300 472, SMPTE 2038 ANC, ID3/KLV metadata,
    /// SCTE-35, DSM-CC, private sections, and any other/unrecognised
    /// `stream_type`) — carried losslessly rather than dropped. `stream_type`
    /// per ISO/IEC 13818-1 Table 2-34; `descriptors` is the raw PMT ES_info
    /// descriptor loop so consumers can classify it (e.g. with dvb-si's
    /// parsers); `carriage` records whether the samples are PES payloads or
    /// whole PSI/private sections — see [`DataCarriage`].
    ///
    /// Carried in the IR for `{TS} → IR → {TS}` / inspection; there is no
    /// ISOBMFF sample entry for an opaque stream in this crate (out of
    /// scope), so it does not participate in the fMP4 mux path (mirrors
    /// [`CodecConfig::Vp8`] / [`CodecConfig::Vorbis`]) — the fMP4/CMAF mux
    /// omits such tracks entirely rather than erroring (issue #576).
    Data {
        /// PMT `stream_type` (ISO/IEC 13818-1 Table 2-34).
        stream_type: u8,
        /// The raw PMT ES_info descriptor-loop bytes for this stream.
        descriptors: Vec<u8>,
        /// Whether this elementary stream carries PES packets or PSI/private
        /// sections (ISO/IEC 13818-1 §2.4.4.8) — determines how a demuxer
        /// reassembles it and how a muxer re-emits it.
        carriage: DataCarriage,
    },
}

impl CodecConfig {
    pub(crate) fn is_audio(&self) -> bool {
        matches!(
            self,
            CodecConfig::Aac { .. }
                | CodecConfig::Ac3 { .. }
                | CodecConfig::Eac3 { .. }
                | CodecConfig::Opus { .. }
                | CodecConfig::Flac { .. }
                | CodecConfig::Ac4 { .. }
                | CodecConfig::MpegH { .. }
                | CodecConfig::Dts { .. }
                | CodecConfig::MpegAudio { .. }
                | CodecConfig::Vorbis { .. }
        )
    }

    /// True for the opaque [`CodecConfig::Data`] variant (issue #557/#576): a
    /// PMT-carried elementary stream with no ISOBMFF sample entry in this
    /// crate. The fMP4/CMAF mux path (init segment + every packager built on
    /// it) omits such tracks entirely rather than erroring — mirrors how the
    /// TS mux path, unlike this one, *can* carry them verbatim.
    pub(crate) fn is_opaque_data(&self) -> bool {
        matches!(self, CodecConfig::Data { .. })
    }

    /// True if `self` is a video codec (issue #628) — codec-family-complete
    /// regardless of which output container actually carries it (e.g. the TS
    /// mux path does not carry every variant matched here; see
    /// `ts_mux::EsKind::from_config`). Mirrors [`CodecConfig::is_audio`]. Used
    /// to pick the anchor/segmentation track (`ts_hls::choose_anchor`,
    /// `Segmenter::new`).
    pub(crate) fn is_video(&self) -> bool {
        matches!(
            self,
            CodecConfig::Avc { .. }
                | CodecConfig::Hevc { .. }
                | CodecConfig::Vvc { .. }
                | CodecConfig::Av1 { .. }
                | CodecConfig::Vp9 { .. }
                | CodecConfig::Vp8 { .. }
                | CodecConfig::Mpeg2Video { .. }
        )
    }
}
