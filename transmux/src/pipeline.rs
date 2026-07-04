//! TS → CMAF/fMP4 remux pipeline — the muxer above the box builders.
//!
//! This is a **samples-in** API (transcode/demux stay in the caller): feed
//! already-demuxed *encoded* access units plus their per-track [`CodecConfig`],
//! and get back a CMAF **initialization segment** (`ftyp` + fragmented-init
//! `moov`) and **media segments** (`styp` + `moof` + `mdat`), per
//! ISO/IEC 14496-12:2015 §8.8 (movie fragments) and the CMAF structure.
//!
//! Video access units are carried length-prefixed in `mdat`
//! ([`crate::annexb`]); the `moof` `trun` carries per-sample size / duration /
//! flags / composition offset, `tfdt` the base media decode time, and each
//! `trun.data_offset` is computed from the finished `moof` size so the samples
//! resolve against the `default-base-is-moof` base.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use broadcast_common::Serialize;

use crate::ac3::{Ac3SpecificBox, Ec3SpecificBox};
use crate::ac4::{Ac4SpecificBox, DAC4_FOURCC};
use crate::annexb::annexb_to_length_prefixed;
use crate::av1::{Av1ConfigurationBox, Av1SampleEntry};
use crate::avc_config::AVCConfigurationBox;
use crate::dts::{DDTS_FOURCC, DtsSpecificBox};
use crate::error::Result;
use crate::flac::{DFLA_FOURCC, FlacSpecificBox};
use crate::hevc_config::HEVCConfigurationBox;
use crate::init_segment::{
    Ac3SampleEntry, Ac4SampleEntry, ChunkOffsetBox, DataEntryUrlBox, DataInformationBox,
    DataReferenceBox, DtsSampleEntry, Ec3SampleEntry, FlacSampleEntry, HandlerBox, MediaBox,
    MediaHeaderBox, MediaInformationBox, MhaSampleEntry, MovieBox, MovieExtendsBox, MovieHeaderBox,
    Mp4aSampleEntry, OpaqueBox, OpusSampleEntry, SampleDescriptionBox, SampleEntryVariant,
    SampleSizeBox, SampleTableBox, SampleToChunkBox, SoundMediaHeaderBox, StblChild, TrackBox,
    TrackExtendsBox, TrackHeaderBox, VideoMediaHeaderBox,
};
use crate::movie_fragment::{
    MovieFragmentBox, MovieFragmentHeaderBox, TFHD_DEFAULT_BASE_IS_MOOF, TRUN_DATA_OFFSET_PRESENT,
    TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT,
    TRUN_SAMPLE_FLAGS_PRESENT, TRUN_SAMPLE_SIZE_PRESENT, TrackFragmentBaseMediaDecodeTimeBox,
    TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentRunBox, TrunSample,
};
use crate::mp4esds::EsdsBox;
use crate::mpegh::MHAC_FOURCC;
use crate::opus::{DOPS_FOURCC, OpusSpecificBox};
use crate::sample_entries::{
    AVCSampleEntry, HEVCSampleEntry, Mp4vSampleEntry, VVCSampleEntry, VisualSampleEntryFields,
};
use crate::segments::{FileTypeBox, MediaDataBox, SegmentTypeBox};

use crate::timing::TimeToSampleBox;
use crate::vp9::{Vp9ConfigurationBox, Vp9SampleEntry};
use crate::vvc_config::VvcConfigurationBox;
pub use mp4_emsg::{EmsgBox, EmsgVersion, PresentationTime};

// --- sample_flags (ISO/IEC 14496-12:2015 §8.8.3.1) --------------------------
/// Sample flags for a sync sample (I-frame): `sample_depends_on = 2` (does not
/// depend on others), `sample_is_non_sync_sample = 0`.
const SAMPLE_FLAGS_SYNC: u32 = 0x0200_0000;
/// Sample flags for a non-sync sample: `sample_depends_on = 1`,
/// `sample_is_non_sync_sample = 1`.
const SAMPLE_FLAGS_NON_SYNC: u32 = 0x0101_0000;

/// ISO-639-2 packed language code `und` (undetermined).
const LANG_UND: u16 = 0x55C4;
/// The ISOBMFF identity transformation matrix (§6.2.2), unity + `[2,30]` fixed.
const IDENTITY_MATRIX: [i32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];
/// `tkhd` flags: track_enabled | in_movie | in_preview.
const TKHD_ENABLED_IN_MOVIE: u32 = 0x0000_0007;

/// Whether a [`CodecConfig::Data`] elementary stream carries PES packets or
/// PSI/private sections on its PID.
///
/// ISO/IEC 13818-1 §2.4.4.8 / Table 2-34 splits `stream_type` into two
/// carriage families: most types (subtitles, teletext, SMPTE 2038 ANC,
/// metadata, and any unrecognised value) are PES-packetized (§2.4.3.6); a
/// fixed set (`0x05` private_sections, `0x0A`-`0x0D` DSM-CC, `0x14` DSM-CC
/// synchronized download, `0x86` SCTE-35/ANSI-scoped) are carried as raw
/// PSI-style sections instead (§2.4.4), with no PES header at all.
/// Reassembling a section stream with a PES parser (or vice versa) silently
/// yields nothing, so a demuxer/muxer must dispatch on this before touching
/// the payload — see `crate::ts_demux` / `crate::ts_mux` (issue #576).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataCarriage {
    /// The elementary stream is PES-packetized (ISO/IEC 13818-1 §2.4.3.6).
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
    /// [`MhaSampleEntry`] if needed; `build_trak` always emits `mha1`.
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
    fn is_audio(&self) -> bool {
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
}

/// A track's identity + codec config, used to build the init segment.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TrackSpec {
    /// Track ID (1-based, unique within the movie).
    pub track_id: u32,
    /// Media timescale (ticks per second, e.g. 90000 for video).
    pub timescale: u32,
    /// Codec configuration + dimensions.
    pub config: CodecConfig,
    /// Source elementary-stream PID for TS-demuxed tracks; `None` for non-TS
    /// sources (fMP4/FLV/WebM/PS/RTP). (issue #582)
    pub source_pid: Option<u16>,
    /// Raw PMT ES_info descriptor-loop bytes for this elementary stream
    /// (ISO/IEC 13818-1 §2.4.4.8), verbatim; empty for non-TS sources.
    /// transmux does not parse these — consumers use dvb-si. (issue #582)
    pub es_info_descriptors: Vec<u8>,
}

impl TrackSpec {
    /// A track spec with no TS provenance (`source_pid = None`, no ES_info
    /// descriptors) — the common case for every non-TS demuxer/transform.
    pub fn new(track_id: u32, timescale: u32, config: CodecConfig) -> Self {
        Self {
            track_id,
            timescale,
            config,
            source_pid: None,
            es_info_descriptors: Vec::new(),
        }
    }

    /// Attach TS provenance (issue #582): the source elementary-stream PID
    /// and its verbatim PMT ES_info descriptor-loop bytes.
    pub fn with_source(mut self, source_pid: u16, es_info_descriptors: Vec<u8>) -> Self {
        self.source_pid = Some(source_pid);
        self.es_info_descriptors = es_info_descriptors;
        self
    }
}

/// Explicit per-sample timestamps recovered from the source container, in the
/// source's own clock — for TS/PES sources the 33-bit-unwrapped 90 kHz PES
/// clock (ISO/IEC 13818-1 §2.4.3.7). `None` when the source carries no
/// per-sample timestamps or the sample's time was synthesized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceTiming {
    /// Decode timestamp (90 kHz for TS sources), unwrapped.
    pub dts: u64,
    /// Presentation timestamp (90 kHz for TS sources), unwrapped.
    pub pts: u64,
}

/// A single coded sample (access unit) fed to [`build_media_segment`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Sample {
    /// Coded bytes: **length-prefixed** NAL data for AVC/HEVC, or the raw frame
    /// for AAC. Use [`Sample::from_annexb`] to convert an Annex B access unit.
    pub data: Vec<u8>,
    /// Sample duration in the track's media timescale.
    pub duration: u32,
    /// Whether this is a sync sample (random-access point / keyframe).
    pub is_sync: bool,
    /// Composition time offset (`pts − dts`) in media-timescale ticks.
    pub composition_offset: i32,
    /// Explicit source-container timestamps, when the source carries them
    /// per-sample (see [`SourceTiming`]). All mux paths in this crate ignore
    /// this field — fMP4 output timing stays duration-based
    /// ([`FragmentTrackData::base_media_decode_time`] + running `duration` sum).
    pub source_timing: Option<SourceTiming>,
}

impl Sample {
    /// Build a sample from already-encoded bytes with every field explicit
    /// (issue #580: the general-purpose constructor now that `Sample` is
    /// `#[non_exhaustive]` and cannot be struct-literal-constructed outside
    /// this crate). `data` must already be in this crate's wire form
    /// (length-prefixed for AVC/HEVC) — use [`Sample::from_annexb`] to
    /// convert an Annex B access unit instead.
    pub fn new(data: Vec<u8>, duration: u32, is_sync: bool, composition_offset: i32) -> Self {
        Self {
            data,
            duration,
            is_sync,
            composition_offset,
            source_timing: None,
        }
    }

    /// Build a video sample from an Annex B access unit, converting its NAL
    /// units to the length-prefixed `mdat` form.
    pub fn from_annexb(
        annexb: &[u8],
        duration: u32,
        is_sync: bool,
        composition_offset: i32,
    ) -> Self {
        Self {
            data: annexb_to_length_prefixed(annexb),
            duration,
            is_sync,
            composition_offset,
            source_timing: None,
        }
    }

    /// Build an audio sample from a raw coded frame (e.g. an AAC access unit).
    pub fn from_raw(data: Vec<u8>, duration: u32) -> Self {
        Self {
            data,
            duration,
            is_sync: true,
            composition_offset: 0,
            source_timing: None,
        }
    }

    /// Attach explicit [`SourceTiming`] recovered from the source container,
    /// returning `self` (builder style).
    pub fn with_source_timing(mut self, t: SourceTiming) -> Self {
        self.source_timing = Some(t);
        self
    }
}

/// One track's samples for a single media segment.
pub struct FragmentTrackData<'a> {
    /// Track ID matching a [`TrackSpec`] from the init segment.
    pub track_id: u32,
    /// The decode time of the first sample, in media-timescale ticks.
    pub base_media_decode_time: u64,
    /// The samples for this fragment, in decode order.
    pub samples: &'a [Sample],
}

/// Build a CMAF initialization segment (`ftyp` + fragmented-init `moov`) for the
/// given tracks. The `moov` carries empty sample tables (`stts`/`stsc`/`stsz`/
/// `stco`) plus an `mvex`/`trex` per track, marking the movie as fragmented.
pub fn build_init_segment(tracks: &[TrackSpec], movie_timescale: u32) -> Result<Vec<u8>> {
    let ftyp = FileTypeBox {
        major_brand: *b"iso5",
        minor_version: 512,
        compatible_brands: vec![*b"iso5", *b"iso6", *b"mp41"],
    };

    let next_track_id = tracks.iter().map(|t| t.track_id).max().unwrap_or(0) + 1;
    let mvhd = MovieHeaderBox {
        version: 0,
        flags: 0,
        creation_time: 0,
        modification_time: 0,
        timescale: movie_timescale,
        duration: 0,
        rate: 0x0001_0000,
        volume: 0x0100,
        matrix: IDENTITY_MATRIX,
        next_track_id,
    };

    let mut trak_boxes = Vec::with_capacity(tracks.len());
    let mut trex = Vec::with_capacity(tracks.len());
    for t in tracks {
        // Opaque data tracks have no ISOBMFF sample entry in this crate
        // (issue #557/#576) — omit them from the fMP4/CMAF mux path entirely
        // rather than erroring, so a Media containing e.g. a DVB
        // subtitle/DSM-CC/SCTE-35 track still produces a valid init segment
        // for its carriable (video/audio) tracks.
        if t.config.is_opaque_data() {
            continue;
        }
        trak_boxes.push(build_trak(t)?);
        trex.push(TrackExtendsBox {
            version: 0,
            flags: 0,
            track_id: t.track_id,
            default_sample_description_index: 1,
            default_sample_duration: 0,
            default_sample_size: 0,
            default_sample_flags: 0,
        });
    }

    let moov = MovieBox {
        mvhd,
        tracks: trak_boxes,
        mvex: Some(MovieExtendsBox {
            trex,
            opaque: vec![],
        }),
        opaque: vec![],
    };

    let mut out = vec![0u8; ftyp.serialized_len() + moov.serialized_len()];
    let n1 = ftyp.serialize_into(&mut out)?;
    let n2 = moov.serialize_into(&mut out[n1..])?;
    out.truncate(n1 + n2);
    Ok(out)
}

/// Serialize a typed config box body and wrap it as an [`OpaqueBox`] under the
/// given four-CC (mirrors the `esds`/`dac3` embedding for audio sample entries).
fn config_box<C: Serialize<Error = crate::error::Error>>(
    fourcc: &[u8; 4],
    config: &C,
) -> Result<OpaqueBox> {
    let mut body = vec![0u8; config.serialized_len()];
    let n = config.serialize_into(&mut body)?;
    body.truncate(n);
    Ok(OpaqueBox::new(*fourcc, body))
}

/// Build one fragmented-init `trak` (empty sample tables + a single `stsd` entry).
fn build_trak(t: &TrackSpec) -> Result<TrackBox> {
    let audio = t.config.is_audio();

    let (stsd_entry, vmhd, smhd, handler_type, handler_name, tkhd_w, tkhd_h) = match &t.config {
        CodecConfig::Avc {
            config,
            width,
            height,
        } => {
            let visual = VisualSampleEntryFields {
                data_reference_index: 1,
                width: *width,
                height: *height,
                ..VisualSampleEntryFields::default()
            };
            let entry = SampleEntryVariant::Avc1(AVCSampleEntry {
                codec_type: *b"avc1",
                visual,
                config: config.clone(),
                extra_boxes: vec![],
            });
            (
                entry,
                Some(VideoMediaHeaderBox {
                    version: 0,
                    flags: 1,
                    graphicsmode: 0,
                    opcolor: [0, 0, 0],
                }),
                None,
                *b"vide",
                b"VideoHandler\0".to_vec(),
                (*width as u32) << 16,
                (*height as u32) << 16,
            )
        }
        CodecConfig::Hevc {
            config,
            width,
            height,
        } => {
            let visual = VisualSampleEntryFields {
                data_reference_index: 1,
                width: *width,
                height: *height,
                ..VisualSampleEntryFields::default()
            };
            // Always emit `hvc1` (parameter sets in the sample entry) —
            // ISO/IEC 14496-15:2017 §8.4.1.
            let entry = SampleEntryVariant::Hevc1(HEVCSampleEntry {
                codec_type: *b"hvc1",
                visual,
                config: config.clone(),
                extra_boxes: vec![],
            });
            (
                entry,
                Some(VideoMediaHeaderBox {
                    version: 0,
                    flags: 1,
                    graphicsmode: 0,
                    opcolor: [0, 0, 0],
                }),
                None,
                *b"vide",
                b"VideoHandler\0".to_vec(),
                (*width as u32) << 16,
                (*height as u32) << 16,
            )
        }
        CodecConfig::Vvc {
            config,
            width,
            height,
        } => {
            let visual = VisualSampleEntryFields {
                data_reference_index: 1,
                width: *width,
                height: *height,
                ..VisualSampleEntryFields::default()
            };
            // Always emit `vvc1` (parameter sets in the sample entry) —
            // ISO/IEC 14496-15:2022 §11.3.3.
            let entry = SampleEntryVariant::Vvc(Box::new(VVCSampleEntry {
                codec_type: *b"vvc1",
                visual,
                config: config.clone(),
                extra_boxes: vec![],
            }));
            (
                entry,
                Some(VideoMediaHeaderBox {
                    version: 0,
                    flags: 1,
                    graphicsmode: 0,
                    opcolor: [0, 0, 0],
                }),
                None,
                *b"vide",
                b"VideoHandler\0".to_vec(),
                (*width as u32) << 16,
                (*height as u32) << 16,
            )
        }
        CodecConfig::Aac {
            esds,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            // Serialize the esds to a full box, then carry its body opaquely.
            let mut esds_full = vec![0u8; esds.serialized_len()];
            let n = esds.serialize_into(&mut esds_full)?;
            let esds_opaque = OpaqueBox::new(*b"esds", esds_full[8..n].to_vec());
            let entry = SampleEntryVariant::Mp4a(Box::new(Mp4aSampleEntry {
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                samplerate: sample_rate << 16,
                config_boxes: vec![esds_opaque],
            }));
            (
                entry,
                None,
                Some(SoundMediaHeaderBox {
                    version: 0,
                    flags: 0,
                    balance: 0,
                }),
                *b"soun",
                b"SoundHandler\0".to_vec(),
                0,
                0,
            )
        }
        CodecConfig::Ac3 {
            config,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let mut dac3_full = vec![0u8; config.serialized_len() + 8];
            dac3_full[4..8].copy_from_slice(b"dac3");
            let n = config.serialize_into(&mut dac3_full[8..])?;
            let dac3_opaque = OpaqueBox::new(*b"dac3", dac3_full[8..8 + n].to_vec());
            let entry = SampleEntryVariant::Ac3(Box::new(Ac3SampleEntry {
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                samplerate: (*sample_rate) << 16,
                config_boxes: vec![dac3_opaque],
            }));
            (
                entry,
                None,
                Some(SoundMediaHeaderBox {
                    version: 0,
                    flags: 0,
                    balance: 0,
                }),
                *b"soun",
                b"SoundHandler\0".to_vec(),
                0,
                0,
            )
        }
        CodecConfig::Eac3 {
            config,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let mut dec3_full = vec![0u8; config.serialized_len() + 8];
            dec3_full[4..8].copy_from_slice(b"dec3");
            let n = config.serialize_into(&mut dec3_full[8..])?;
            let dec3_opaque = OpaqueBox::new(*b"dec3", dec3_full[8..8 + n].to_vec());
            let entry = SampleEntryVariant::Ec3(Box::new(Ec3SampleEntry {
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                samplerate: (*sample_rate) << 16,
                config_boxes: vec![dec3_opaque],
            }));
            (
                entry,
                None,
                Some(SoundMediaHeaderBox {
                    version: 0,
                    flags: 0,
                    balance: 0,
                }),
                *b"soun",
                b"SoundHandler\0".to_vec(),
                0,
                0,
            )
        }
        CodecConfig::Av1 {
            config,
            width,
            height,
        } => {
            let visual = VisualSampleEntryFields {
                data_reference_index: 1,
                width: *width,
                height: *height,
                ..VisualSampleEntryFields::default()
            };
            let entry = SampleEntryVariant::Av01(Box::new(Av1SampleEntry {
                visual,
                config: config.clone(),
            }));
            (
                entry,
                Some(VideoMediaHeaderBox {
                    version: 0,
                    flags: 1,
                    graphicsmode: 0,
                    opcolor: [0, 0, 0],
                }),
                None,
                *b"vide",
                b"VideoHandler\0".to_vec(),
                (*width as u32) << 16,
                (*height as u32) << 16,
            )
        }
        CodecConfig::Vp9 {
            config,
            width,
            height,
        } => {
            let visual = VisualSampleEntryFields {
                data_reference_index: 1,
                width: *width,
                height: *height,
                ..VisualSampleEntryFields::default()
            };
            let entry = SampleEntryVariant::Vp09(Box::new(Vp9SampleEntry {
                visual,
                config: config.clone(),
            }));
            (
                entry,
                Some(VideoMediaHeaderBox {
                    version: 0,
                    flags: 1,
                    graphicsmode: 0,
                    opcolor: [0, 0, 0],
                }),
                None,
                *b"vide",
                b"VideoHandler\0".to_vec(),
                (*width as u32) << 16,
                (*height as u32) << 16,
            )
        }
        CodecConfig::Opus {
            config,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let dops_opaque = config_box(&DOPS_FOURCC, config)?;
            let entry = SampleEntryVariant::Opus(Box::new(OpusSampleEntry {
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                samplerate: (*sample_rate) << 16,
                config_boxes: vec![dops_opaque],
            }));
            (
                entry,
                None,
                Some(SoundMediaHeaderBox {
                    version: 0,
                    flags: 0,
                    balance: 0,
                }),
                *b"soun",
                b"SoundHandler\0".to_vec(),
                0,
                0,
            )
        }
        CodecConfig::Flac {
            config,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let dfla_opaque = config_box(&DFLA_FOURCC, config)?;
            let entry = SampleEntryVariant::Flac(Box::new(FlacSampleEntry {
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                samplerate: (*sample_rate) << 16,
                config_boxes: vec![dfla_opaque],
            }));
            (
                entry,
                None,
                Some(SoundMediaHeaderBox {
                    version: 0,
                    flags: 0,
                    balance: 0,
                }),
                *b"soun",
                b"SoundHandler\0".to_vec(),
                0,
                0,
            )
        }
        CodecConfig::Ac4 {
            config,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let dac4_opaque = config_box(&DAC4_FOURCC, config)?;
            let entry = SampleEntryVariant::Ac4(Box::new(Ac4SampleEntry {
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                samplerate: (*sample_rate) << 16,
                config_boxes: vec![dac4_opaque],
            }));
            (
                entry,
                None,
                Some(SoundMediaHeaderBox {
                    version: 0,
                    flags: 0,
                    balance: 0,
                }),
                *b"soun",
                b"SoundHandler\0".to_vec(),
                0,
                0,
            )
        }
        CodecConfig::MpegH {
            config,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let mhac_opaque = config_box(&MHAC_FOURCC, config)?;
            let entry = SampleEntryVariant::Mha(Box::new(MhaSampleEntry {
                codec_type: crate::mpegh::MHA1_FOURCC,
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                samplerate: (*sample_rate) << 16,
                config_boxes: vec![mhac_opaque],
            }));
            (
                entry,
                None,
                Some(SoundMediaHeaderBox {
                    version: 0,
                    flags: 0,
                    balance: 0,
                }),
                *b"soun",
                b"SoundHandler\0".to_vec(),
                0,
                0,
            )
        }
        CodecConfig::Mpeg2Video {
            esds,
            width,
            height,
        } => {
            let visual = VisualSampleEntryFields {
                data_reference_index: 1,
                width: *width,
                height: *height,
                ..VisualSampleEntryFields::default()
            };
            // Carry the esds as the mp4v's first child box (body only, no header).
            let mut esds_full = vec![0u8; esds.serialized_len()];
            let n = esds.serialize_into(&mut esds_full)?;
            let esds_opaque = OpaqueBox::new(*b"esds", esds_full[8..n].to_vec());
            let entry = SampleEntryVariant::Mp4v(Box::new(Mp4vSampleEntry {
                visual,
                config_boxes: vec![esds_opaque],
            }));
            (
                entry,
                Some(VideoMediaHeaderBox {
                    version: 0,
                    flags: 1,
                    graphicsmode: 0,
                    opcolor: [0, 0, 0],
                }),
                None,
                *b"vide",
                b"VideoHandler\0".to_vec(),
                (*width as u32) << 16,
                (*height as u32) << 16,
            )
        }
        CodecConfig::MpegAudio {
            esds,
            layer: _,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let mut esds_full = vec![0u8; esds.serialized_len()];
            let n = esds.serialize_into(&mut esds_full)?;
            let esds_opaque = OpaqueBox::new(*b"esds", esds_full[8..n].to_vec());
            let entry = SampleEntryVariant::Mp4a(Box::new(Mp4aSampleEntry {
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                samplerate: sample_rate << 16,
                config_boxes: vec![esds_opaque],
            }));
            (
                entry,
                None,
                Some(SoundMediaHeaderBox {
                    version: 0,
                    flags: 0,
                    balance: 0,
                }),
                *b"soun",
                b"SoundHandler\0".to_vec(),
                0,
                0,
            )
        }
        CodecConfig::Dts {
            config,
            codec_fourcc,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            let ddts_opaque = config_box(&DDTS_FOURCC, config)?;
            let entry = SampleEntryVariant::Dts(Box::new(DtsSampleEntry {
                codec_type: *codec_fourcc,
                data_reference_index: 1,
                channelcount: *channel_count,
                samplesize: *sample_size,
                samplerate: (*sample_rate) << 16,
                config_boxes: vec![ddts_opaque],
            }));
            (
                entry,
                None,
                Some(SoundMediaHeaderBox {
                    version: 0,
                    flags: 0,
                    balance: 0,
                }),
                *b"soun",
                b"SoundHandler\0".to_vec(),
                0,
                0,
            )
        }
        // WebM-native codecs: carried in the IR (see [`crate::webm_demux`]) but
        // with no ISOBMFF sample entry in this crate — the fMP4 mux path is out
        // of scope (see `docs/codec/vp8-vorbis-webm.md`).
        CodecConfig::Vp8 { .. } => {
            return Err(crate::error::Error::UnsupportedCodec { codec: "VP8" });
        }
        CodecConfig::Vorbis { .. } => {
            return Err(crate::error::Error::UnsupportedCodec { codec: "Vorbis" });
        }
        // Opaque PES data track: no ISOBMFF sample entry in this crate — the
        // fMP4 mux path is out of scope (mirrors Vp8/Vorbis above).
        CodecConfig::Data { .. } => {
            return Err(crate::error::Error::UnsupportedCodec { codec: "Data" });
        }
    };

    let stbl = SampleTableBox {
        children: vec![
            StblChild::Stsd(SampleDescriptionBox {
                version: 0,
                flags: 0,
                entries: vec![stsd_entry],
            }),
            StblChild::Stts(TimeToSampleBox {
                version: 0,
                flags: 0,
                entries: vec![],
            }),
            StblChild::Stsc(SampleToChunkBox {
                version: 0,
                flags: 0,
                entries: vec![],
            }),
            StblChild::Stsz(SampleSizeBox {
                version: 0,
                flags: 0,
                sample_size: 0,
                entries: vec![],
            }),
            StblChild::Stco(ChunkOffsetBox {
                version: 0,
                flags: 0,
                entries: vec![],
            }),
        ],
    };

    let dinf = DataInformationBox {
        dref: Some(DataReferenceBox {
            version: 0,
            flags: 0,
            // A single self-contained URL entry (flags bit 0 = media in this file).
            entries: vec![DataEntryUrlBox {
                version: 0,
                flags: 1,
                location: vec![],
            }],
        }),
        opaque: vec![],
    };

    let minf = MediaInformationBox {
        vmhd,
        smhd,
        dinf: Some(dinf),
        stbl: Some(stbl),
        opaque: vec![],
    };

    let mdhd = MediaHeaderBox {
        version: 0,
        flags: 0,
        creation_time: 0,
        modification_time: 0,
        timescale: t.timescale,
        duration: 0,
        language: LANG_UND,
    };

    let hdlr = HandlerBox {
        version: 0,
        flags: 0,
        handler_type,
        name: handler_name,
    };

    let mdia = MediaBox {
        mdhd: Some(mdhd),
        hdlr: Some(hdlr),
        minf: Some(minf),
        opaque: vec![],
    };

    let tkhd = TrackHeaderBox {
        version: 0,
        flags: TKHD_ENABLED_IN_MOVIE,
        creation_time: 0,
        modification_time: 0,
        track_id: t.track_id,
        duration: 0,
        layer: 0,
        alternate_group: 0,
        volume: if audio { 0x0100 } else { 0 },
        matrix: IDENTITY_MATRIX,
        width: tkhd_w,
        height: tkhd_h,
    };

    Ok(TrackBox {
        tkhd,
        edts: None,
        mdia: Some(mdia),
        opaque: vec![],
    })
}

/// Build one CMAF media segment (`styp` + `moof` + `mdat`) carrying the given
/// per-track samples. Each `trun.data_offset` is computed from the finished
/// `moof` size (the tracks' sample data are concatenated into a single `mdat`
/// in track order, resolved against the `default-base-is-moof` base).
///
/// Delegates to [`build_media_segment_with_events`] with an empty `emsgs`
/// slice (identical output).
pub fn build_media_segment(
    sequence_number: u32,
    tracks: &[FragmentTrackData<'_>],
) -> Result<Vec<u8>> {
    build_media_segment_with_events(sequence_number, tracks, &[])
}

/// Build one CMAF media segment (`styp` + one or more `emsg` boxes + `moof` +
/// `mdat`), optionally carrying inband timed-metadata events.
///
/// # Box order
///
/// Per **ISO/IEC 14496-12 §8.8** (movie fragments) and the DASH-IF Interoperability
/// Point Part 10 §6.1 (Events and Timed Metadata in MPEG-DASH / CMAF), each
/// [`EmsgBox`] must appear **at the start of the segment**, after the `styp`
/// segment-type box but *before* the `moof`.  The boxes in `emsgs` are emitted
/// in the order given.  A common consumer is SCTE 35 in-band ad-insertion
/// signalling (`scheme_id_uri = "urn:scte:scte35:2013:bin"`, ANSI/SCTE 214-3 /
/// DASH-IF Part 10 §7.3).
///
/// # Timing
///
/// [`PresentationTime::Delta`] (version 0) expresses the event time relative to
/// the segment's earliest presentation time in `timescale` ticks.
/// [`PresentationTime::Absolute`] (version 1) expresses an absolute
/// representation-timeline position, per ISO/IEC 23009-1 §5.10.3.3.
///
/// # Empty slice
///
/// When `emsgs` is empty this function produces byte-identical output to
/// [`build_media_segment`].
pub fn build_media_segment_with_events(
    sequence_number: u32,
    tracks: &[FragmentTrackData<'_>],
    emsgs: &[EmsgBox<'_>],
) -> Result<Vec<u8>> {
    let styp = SegmentTypeBox {
        major_brand: *b"msdh",
        minor_version: 0,
        compatible_brands: vec![*b"msdh", *b"msix"],
    };

    // Build each traf with a placeholder data_offset (size is offset-value
    // independent, so the moof size is stable once structured).
    let mut traf_boxes = Vec::with_capacity(tracks.len());
    for ft in tracks {
        let any_cts = ft.samples.iter().any(|s| s.composition_offset != 0);
        let samples: Vec<TrunSample> = ft
            .samples
            .iter()
            .map(|s| TrunSample {
                sample_duration: Some(s.duration),
                sample_size: Some(s.data.len() as u32),
                sample_flags: Some(if s.is_sync {
                    SAMPLE_FLAGS_SYNC
                } else {
                    SAMPLE_FLAGS_NON_SYNC
                }),
                sample_composition_time_offset: if any_cts {
                    Some(s.composition_offset)
                } else {
                    None
                },
            })
            .collect();

        let mut tr_flags = TRUN_DATA_OFFSET_PRESENT
            | TRUN_SAMPLE_DURATION_PRESENT
            | TRUN_SAMPLE_SIZE_PRESENT
            | TRUN_SAMPLE_FLAGS_PRESENT;
        // Version 1 carries a signed composition offset (needed for B-frames).
        let version = if any_cts {
            tr_flags |= TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT;
            1u8
        } else {
            0u8
        };

        let trun = TrackFragmentRunBox {
            version,
            tr_flags,
            data_offset: Some(0),
            first_sample_flags: None,
            samples,
        };
        let tfhd = TrackFragmentHeaderBox {
            flags: TFHD_DEFAULT_BASE_IS_MOOF,
            track_id: ft.track_id,
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: None,
            default_sample_size: None,
            default_sample_flags: None,
        };
        let tfdt = TrackFragmentBaseMediaDecodeTimeBox::new_v1(ft.base_media_decode_time);
        traf_boxes.push(TrackFragmentBox {
            tfhd,
            tfdt: Some(tfdt),
            trun: vec![trun],
        });
    }

    let mut moof = MovieFragmentBox {
        mfhd: MovieFragmentHeaderBox::new(sequence_number),
        traf: traf_boxes,
    };

    // With default-base-is-moof, data_offset is measured from the moof start.
    // The mdat payload begins at moof_size + 8 (the mdat header).
    let moof_size = moof.serialized_len();
    let mut cursor = moof_size + 8;
    let mut mdat_data = Vec::new();
    for (i, ft) in tracks.iter().enumerate() {
        moof.traf[i].trun[0].data_offset = Some(cursor as i32);
        for s in ft.samples {
            mdat_data.extend_from_slice(&s.data);
            cursor += s.data.len();
        }
    }

    let mdat = MediaDataBox { data: mdat_data };

    // Serialize each emsg box to an owned Vec so we know its exact size before
    // allocating the output buffer.  The `?` coerces `mp4_emsg::Error` to
    // `crate::error::Error` via the `From` impl in `error.rs`.
    let emsg_vecs: Vec<Vec<u8>> = emsgs
        .iter()
        .map(|e| Ok(e.to_vec()?))
        .collect::<Result<_>>()?;
    let emsg_total: usize = emsg_vecs.iter().map(|v| v.len()).sum();

    let total = styp.serialized_len() + emsg_total + moof.serialized_len() + mdat.serialized_len();
    let mut out = vec![0u8; total];
    let mut c = 0usize;
    c += styp.serialize_into(&mut out[c..])?;
    // ISO/IEC 14496-12 §8.8 + DASH-IF IOP Part 10 §6.1: each `emsg` box
    // follows `styp` and precedes `moof` so DASH clients can process events
    // before media decoding begins.
    for ev in &emsg_vecs {
        out[c..c + ev.len()].copy_from_slice(ev);
        c += ev.len();
    }
    c += moof.serialize_into(&mut out[c..])?;
    c += mdat.serialize_into(&mut out[c..])?;
    out.truncate(c);
    Ok(out)
}
