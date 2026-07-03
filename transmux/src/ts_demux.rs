//! MPEG-2 Transport Stream demuxer → hub [`Media`] IR.
//!
//! `TsDemux` is the **input** side of the any-to-any container hub: it consumes
//! raw MPEG-2 TS bytes and produces the neutral [`Media`] IR (one [`Track`] per
//! elementary stream, coded samples in decode order), implementing the abstract
//! [`broadcast_common::Unpackage`] trait so `{TS} → IR → {any}` composes with the
//! existing [`CmafMux`](crate::media::CmafMux) / [`HlsPackager`](crate::media::HlsPackager)
//! packagers.
//!
//! Pipeline: TS packet layer ([`mpeg_ts`]) → follow PAT → PMT → per-PID PES
//! reassembly ([`mpeg_pes`]) → codec-config recovery (H.264 SPS/PPS → `avcC`,
//! H.265 VPS/SPS/PPS → `hvcC`, MPEG-2 video `sequence_header()` → `esds`,
//! ADTS → AudioSpecificConfig →
//! `esds`, MPEG-1/2 audio frame header → `esds`, AC-3/E-AC-3 syncframe BSI →
//! `dac3`/`dec3`) → length-prefixed video / raw audio samples.
//!
//! HEVC (H.265) elementary streams are carried into the IR: the in-band
//! VPS/SPS/PPS NAL units are gathered from the Annex-B access units, decoded
//! into an `hvcC` [`HEVCConfigurationBox`], and emitted as a `hvc1`
//! [`CodecConfig::Hevc`] track — identical to the config `Fmp4Demux` recovers
//! from an fMP4 `hvcC` (issue #467). DTS elementary streams are still recognized
//! in the PMT but not carried (the TS DTS-ES → `ddts` recovery is not yet implemented) — such
//! tracks are skipped, never fatal.
//!
//! Every video and audio sample additionally carries a [`SourceTiming`]
//! recovered from the PES clock (issue #556): video/AAC/MPEG-audio samples get the unwrapped
//! PTS/DTS of the access unit they were decoded from (with per-frame
//! interpolation when a PES payload splits into several frames); AC-3/E-AC-3
//! elementary streams are additionally split into individual syncframes
//! (rather than one zero-duration `Sample` per PES access unit — see
//! [`crate::ac3`]) so real durations and exact PES-boundary timestamps
//! survive into the IR.
//!
//! PMT `stream_type` 0x06 (PES private data — DVB subtitles/teletext/SMPTE
//! 2038/etc.) and 0x15 (metadata in PES) are carried as opaque
//! [`CodecConfig::Data`] tracks (issue #557): the demuxer does not decode the
//! payload, each `Sample` is one verbatim PES payload, and `descriptors`
//! preserves the raw PMT ES_info descriptor loop for the caller to classify.
//! Data tracks are ordered after every codec track, in PMT order. The
//! demuxer also collects every PCR observation from the TS adaptation fields
//! into [`Media`]'s `pcr` field.
//!
//! [`CodecConfig`]: crate::pipeline::CodecConfig
//!
//! # Spec
//!
//! - **PAT / PMT section syntax**: ITU-T H.222.0 (= ISO/IEC 13818-1) §2.4.4.3 /
//!   §2.4.4.8 — see `docs/codec/ts-demux-13818-1.md`.
//! - **stream_type → codec**: ISO/IEC 13818-1 Table 2-34 + ETSI TS 101 154 §G
//!   (DVB user-private AC-3/E-AC-3/DTS assignments).
//! - **PES-over-TS reassembly + PTS/DTS**: ISO/IEC 13818-1 §2.4.3.6 / §2.4.3.7
//!   (via [`mpeg_pes`], 33-bit @ 90 kHz).
//! - **PCR**: ISO/IEC 13818-1 §2.4.3.4 (adaptation field) / §2.4.3.5 (PCR encoding).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Serialize, Unpackage};
use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};

use crate::aac_asc::{AudioSpecificConfig, parse_adts_header};
use crate::ac3::{
    AC3_SAMPLES_PER_SYNCFRAME, Ac3SyncframeInfo, Ec3SyncframeInfo, split_ac3_syncframes,
    split_eac3_syncframes,
};
use crate::annexb::iter_annexb_nals;
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
use crate::media::{Media, PcrSample, Track};
use crate::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, ObjectTypeIndication,
    SLConfigDescriptor, StreamType as EsdsStreamType,
};
use crate::mpeg_legacy::MpegAudioFrameHeader;
use crate::nalu_types::{AvcPps, AvcSps, HevcNalArray, HevcNalUnit};
use crate::pipeline::{CodecConfig, Sample, SourceTiming, TrackSpec};

// ── PSI constants (ISO/IEC 13818-1 §2.4.4) ──────────────────────────────────

/// PID carrying the Program Association Table (§2.4.4.3).
const PAT_PID: u16 = 0x0000;
/// `table_id` of a PAT section (§2.4.4.3, Table 2-31).
const TABLE_ID_PAT: u8 = 0x00;
/// `table_id` of a PMT section (§2.4.4.8, Table 2-31).
const TABLE_ID_PMT: u8 = 0x02;
/// Long-form section header length before the table body: `table_id`(1) +
/// flags/`section_length`(2) + `table_id_extension`(2) + version/cni(1) +
/// `section_number`(1) + `last_section_number`(1) = 8 (§2.4.4.1).
const SECTION_HEADER_LEN: usize = 8;
/// Trailing `CRC_32` on every long-form PSI section (§2.4.4.1).
const CRC32_LEN: usize = 4;
/// Mask for the 12-bit `section_length` high nibble (byte 1 of a section).
const SECTION_LENGTH_HI_MASK: u8 = 0x0F;
/// Mask for the 13-bit PID low byte's high 5 bits.
const PID_HI_MASK: u8 = 0x1F;
/// Bytes per PAT program-loop entry: `program_number`(2) + reserved/PID(2).
const PAT_ENTRY_LEN: usize = 4;
/// Mask for the 12-bit `program_info_length` / `ES_info_length` high nibble.
const INFO_LENGTH_HI_MASK: u8 = 0x0F;
/// A PAT entry with `program_number == 0` gives the network PID, not a PMT PID.
const NETWORK_PROGRAM_NUMBER: u16 = 0x0000;

// ── stream_type → codec (ISO/IEC 13818-1 Table 2-34 + ETSI TS 101 154) ──────

/// MPEG-2 video (ITU-T H.262 / ISO/IEC 13818-2) — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_MPEG2_VIDEO: u8 = 0x02;
/// MPEG-1 audio (ISO/IEC 11172-3) — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_MPEG1_AUDIO: u8 = 0x03;
/// MPEG-2 audio (ISO/IEC 13818-3, LSF) — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_MPEG2_AUDIO: u8 = 0x04;
/// AVC (H.264) video — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_AVC: u8 = 0x1B;
/// HEVC (H.265) video — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_HEVC: u8 = 0x24;
/// ISO/IEC 13818-7 AAC in ADTS — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_AAC_ADTS: u8 = 0x0F;
/// AC-3 (ATSC/DVB user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_AC3: u8 = 0x81;
/// E-AC-3 (user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_EAC3: u8 = 0x87;
/// DTS (user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_DTS_82: u8 = 0x82;
/// DTS-HD (user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_DTS_85: u8 = 0x85;
/// DTS (user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_DTS_8A: u8 = 0x8A;
/// PES packets carrying private data (DVB subtitles EN 300 743, teletext
/// EN 300 472, SMPTE 2038 ANC, …) — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_PES_PRIVATE: u8 = 0x06;
/// ISO/IEC 15938-1 metadata carried in PES packets — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_METADATA_PES: u8 = 0x15;

// ── Codec-config recovery constants ─────────────────────────────────────────

/// NAL length-field width for `mdat` samples: 4-byte prefixes → `lengthSizeMinusOne = 3`.
const NAL_LENGTH_SIZE_MINUS_ONE: u8 = 3;
/// H.264 `nal_unit_type` for SPS (ISO/IEC 14496-10 Table 7-1).
const H264_NAL_SPS: u8 = 7;
/// H.264 `nal_unit_type` for PPS (Table 7-1).
const H264_NAL_PPS: u8 = 8;
/// Mask for the H.264 5-bit `nal_unit_type` in the NAL header byte.
const H264_NAL_TYPE_MASK: u8 = 0x1F;

/// H.265 `nal_unit_type` for VPS (`VPS_NUT`) — ITU-T H.265 Table 7-1 (type 32).
const H265_NAL_VPS: u8 = 32;
/// H.265 `nal_unit_type` for SPS (`SPS_NUT`) — ITU-T H.265 Table 7-1 (type 33).
const H265_NAL_SPS: u8 = 33;
/// H.265 `nal_unit_type` for PPS (`PPS_NUT`) — ITU-T H.265 Table 7-1 (type 34).
const H265_NAL_PPS: u8 = 34;
/// `configurationVersion` for an `hvcC` record (ISO/IEC 14496-15:2017 §8.3.3.1.1).
const HVCC_CONFIGURATION_VERSION: u8 = 1;
/// `constantFrameRate = 0` (not-constant / unspecified) — §8.3.3.1.2.
const HVCC_CONSTANT_FRAME_RATE_UNSPEC: u8 = 0;
/// `numTemporalLayers = 1` when unknown from the ES (single temporal layer).
const HVCC_NUM_TEMPORAL_LAYERS: u8 = 1;
/// `parallelismType = 0` (mixed/unknown) — §8.3.3.1.2.
const HVCC_PARALLELISM_TYPE_UNKNOWN: u8 = 0;
/// `avgFrameRate = 0` (unspecified) — §8.3.3.1.2.
const HVCC_AVG_FRAME_RATE_UNSPEC: u16 = 0;
/// `min_spatial_segmentation_idc = 0` (no constraint) — §8.3.3.1.2.
const HVCC_MIN_SPATIAL_SEGMENTATION_UNSPEC: u16 = 0;

/// `esds` `objectTypeIndication` for MPEG-4 Audio (ISO/IEC 14496-1 Table 5).
const OTI_MPEG4_AUDIO: u8 = 0x40;
/// `esds` `objectTypeIndication` for MPEG-2 Main Visual (ISO/IEC 14496-1 Table 5).
const OTI_MPEG2_VIDEO_MAIN: u8 = 0x61;
/// `esds` `objectTypeIndication` for MPEG-1 Audio, ISO/IEC 11172-3 (Table 5).
const OTI_MPEG1_AUDIO: u8 = 0x6B;
/// `esds` `objectTypeIndication` for MPEG-2 Audio, ISO/IEC 13818-3 (Table 5).
const OTI_MPEG2_AUDIO: u8 = 0x69;
/// `esds` `streamType` for an AudioStream (ISO/IEC 14496-1 Table 6).
const STREAM_TYPE_AUDIO: u8 = 0x05;
/// `esds` `streamType` for a VisualStream (ISO/IEC 14496-1 Table 6).
const STREAM_TYPE_VISUAL: u8 = 0x04;
/// `esds` `ES_ID` assigned to the single audio elementary stream.
const ESDS_ES_ID: u16 = 1;
/// `SLConfigDescriptor` predefined body for MP4 file SL packaging
/// (ISO/IEC 14496-1 §7.3.2.3 — `predefined = 0x02`).
const SL_CONFIG_PREDEFINED_MP4: u8 = 0x02;

/// Audio sample size in bits carried in the sample entry (PCM-equivalent; 16).
const AUDIO_SAMPLE_SIZE_BITS: u16 = 16;
/// Video media timescale (90 kHz — the TS/PES timestamp clock).
const VIDEO_TIMESCALE: u32 = 90_000;
/// Samples per AAC access unit (ISO/IEC 14496-3 — one frame = 1024 samples).
const AAC_SAMPLES_PER_FRAME: u32 = 1024;
/// ADTS fixed header length (bytes) — `crate::aac_asc` `ADTS_HEADER_SIZE`.
const ADTS_HEADER_SIZE: usize = 7;

/// 33-bit PTS/DTS modulus, for wrap-around unrolling (§2.4.3.7, 90 kHz clock).
const TS_WRAP: u64 = 1 << 33;
/// Half the 33-bit range — the threshold used to detect a backward wrap.
const TS_WRAP_HALF: u64 = TS_WRAP / 2;

/// Codec class recovered from a PMT `stream_type` (used to pick the sample /
/// config-recovery path). Data-carrying dispatch discriminant, not a spec label
/// enum — hence no `name()`/`Display` (see `tests/label_coverage.rs` policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Codec {
    H264,
    Hevc,
    Mpeg2Video,
    /// MPEG-1/2 audio; the bool is `true` for MPEG-2 audio (stream_type 0x04,
    /// OTI 0x69), `false` for MPEG-1 audio (stream_type 0x03, OTI 0x6B).
    MpegAudio(bool),
    Aac,
    Ac3,
    Eac3,
    Dts,
    /// Opaque PES data stream (stream_type 0x06/0x15 — issue #557); the field
    /// is the PMT `stream_type` itself, carried through into
    /// [`CodecConfig::Data`].
    Data(u8),
}

impl Codec {
    /// Map a PMT `stream_type` to a supported [`Codec`], or `None` if the demuxer
    /// does not carry it (skipped, never fatal — issue #467).
    fn from_stream_type(stream_type: u8) -> Option<Self> {
        match stream_type {
            STREAM_TYPE_MPEG2_VIDEO => Some(Codec::Mpeg2Video),
            STREAM_TYPE_MPEG1_AUDIO => Some(Codec::MpegAudio(false)),
            STREAM_TYPE_MPEG2_AUDIO => Some(Codec::MpegAudio(true)),
            STREAM_TYPE_AVC => Some(Codec::H264),
            STREAM_TYPE_HEVC => Some(Codec::Hevc),
            STREAM_TYPE_AAC_ADTS => Some(Codec::Aac),
            STREAM_TYPE_AC3 => Some(Codec::Ac3),
            STREAM_TYPE_EAC3 => Some(Codec::Eac3),
            STREAM_TYPE_DTS_82 | STREAM_TYPE_DTS_85 | STREAM_TYPE_DTS_8A => Some(Codec::Dts),
            STREAM_TYPE_PES_PRIVATE | STREAM_TYPE_METADATA_PES => Some(Codec::Data(stream_type)),
            _ => None,
        }
    }
}

/// One elementary stream discovered in the PMT, plus its reassembled access
/// units in wire (decode) order.
struct ElementaryStream {
    codec: Codec,
    /// Raw PMT ES_info descriptor-loop bytes for this stream (ISO/IEC
    /// 13818-1 §2.4.4.8); only meaningful for [`Codec::Data`] streams, carried
    /// verbatim into [`CodecConfig::Data::descriptors`].
    descriptors: Vec<u8>,
    /// PES reassembly buffer for this PID.
    assembler: PesAssembler,
    /// Access units: `(coded bytes, pts, dts)` — Annex B for video, raw frames
    /// otherwise. Filled as complete PES packets arrive, in stream order.
    access_units: Vec<AccessUnit>,
}

/// A single reassembled access unit with its presentation/decoding timestamps.
struct AccessUnit {
    /// Coded bytes: an Annex B AU for video, or the raw ES payload for audio.
    data: Vec<u8>,
    /// 33-bit PTS @ 90 kHz (defaults to DTS/0 when the PES carried none).
    pts: u64,
    /// 33-bit DTS @ 90 kHz (defaults to PTS when the PES carried none).
    dts: u64,
}

/// Demux an MPEG-2 Transport Stream byte slice into a [`Media`].
///
/// Follows the PAT to every PMT, enumerates each program's elementary streams
/// into IR [`Track`]s, reassembles per-PID PES into access units with PTS/DTS,
/// recovers codec config from the in-band headers, and emits length-prefixed
/// video / raw audio samples in decode order.
///
/// The `'a` parameter ties the demuxer to the byte-slice lifetime it consumes
/// via [`Unpackage::Input`]; construct one per call with [`TsDemux::new`].
#[derive(Debug, Default, Clone)]
pub struct TsDemux<'a> {
    _marker: PhantomData<&'a [u8]>,
}

impl<'a> TsDemux<'a> {
    /// Create a new demuxer.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }

    /// Demux `input` (a whole MPEG-2 TS byte stream) into a [`Media`].
    ///
    /// This is the inherent form of [`Unpackage::unpackage`]; both produce the
    /// same result. See the type-level docs for the pipeline.
    pub fn demux(&mut self, input: &'a [u8]) -> Result<Media> {
        // ── Pass 1: PSI — follow PAT → PMT to discover elementary streams ──
        // A single pass over the packet stream collects PSI sections; we resolve
        // the PMT PID set from the PAT, then read every PMT to enumerate the ESs.
        let mut pat_reasm = SectionReassembler::default();
        let mut pmt_reasm: BTreeMap<u16, SectionReassembler> = BTreeMap::new();
        let mut pmt_pids: Vec<u16> = Vec::new();
        // Ordered elementary-stream table: (pid, codec, ES_info descriptors),
        // PMT order preserved.
        let mut es_defs: Vec<(u16, Codec, Vec<u8>)> = Vec::new();
        let mut es_seen: alloc::collections::BTreeSet<u16> = alloc::collections::BTreeSet::new();

        for pkt in iter_ts_packets(input) {
            let pid = pkt.header.pid;
            let pusi = pkt.header.pusi;
            let Some(payload) = pkt.payload else {
                continue;
            };

            if pid == PAT_PID {
                pat_reasm.feed(payload, pusi);
                while let Some(section) = pat_reasm.pop_section() {
                    for pmt_pid in parse_pat(&section)? {
                        if !pmt_pids.contains(&pmt_pid) {
                            pmt_pids.push(pmt_pid);
                            pmt_reasm.entry(pmt_pid).or_default();
                        }
                    }
                }
            } else if let Some(reasm) = pmt_reasm.get_mut(&pid) {
                reasm.feed(payload, pusi);
                while let Some(section) = reasm.pop_section() {
                    for (es_pid, codec, descriptors) in parse_pmt(&section)? {
                        if es_seen.insert(es_pid) {
                            es_defs.push((es_pid, codec, descriptors));
                        }
                    }
                }
            }
        }

        // ── Pass 2: per-ES PES reassembly into access units ──
        let mut streams: BTreeMap<u16, ElementaryStream> = BTreeMap::new();
        for (pid, codec, descriptors) in &es_defs {
            streams.entry(*pid).or_insert_with(|| ElementaryStream {
                codec: *codec,
                descriptors: descriptors.clone(),
                assembler: PesAssembler::new(),
                access_units: Vec::new(),
            });
        }

        for pkt in iter_ts_packets(input) {
            let pid = pkt.header.pid;
            let pusi = pkt.header.pusi;
            let Some(payload) = pkt.payload else {
                continue;
            };
            if let Some(es) = streams.get_mut(&pid) {
                if let Some(completed) = es.assembler.feed(pusi, payload) {
                    push_access_unit(es, &completed);
                }
            }
        }
        for es in streams.values_mut() {
            if let Some(completed) = es.assembler.flush() {
                push_access_unit(es, &completed);
            }
        }

        // ── Pass 3: build tracks (config recovery + samples in decode order) ──
        // Codec tracks first, then data tracks (issue #557), each group in PMT
        // order; track_id numbering continues across both groups.
        let mut tracks: Vec<Track> = Vec::new();
        let mut track_id: u32 = 1;
        let codec_pids = es_defs
            .iter()
            .filter(|(_, codec, _)| !matches!(codec, Codec::Data(_)))
            .map(|(pid, _, _)| *pid);
        let data_pids = es_defs
            .iter()
            .filter(|(_, codec, _)| matches!(codec, Codec::Data(_)))
            .map(|(pid, _, _)| *pid);
        for pid in codec_pids.chain(data_pids) {
            let Some(es) = streams.get(&pid) else {
                continue;
            };
            if es.access_units.is_empty() {
                continue;
            }
            // Config could not be recovered (e.g. no in-band SPS/PPS, or an
            // unsupported-but-recognized codec) → `None`: skip, never fail.
            if let Some(track) = build_track(es, track_id) {
                tracks.push(track);
                track_id += 1;
            }
        }

        // ── PCR timeline (issue #557) — every adaptation-field PCR, in packet order.
        let pcr = collect_pcr(input);

        Ok(Media::new(tracks, VIDEO_TIMESCALE).with_pcr(pcr))
    }
}

impl<'a> Unpackage for TsDemux<'a> {
    type Input = &'a [u8];
    type Media = Media;
    type Error = Error;

    fn unpackage(&mut self, input: &'a [u8]) -> Result<Media> {
        self.demux(input)
    }
}

/// Iterate the 188-byte TS packets in `data`, yielding each that parses.
///
/// Packets with a bad sync byte or short tail are silently skipped (the caller
/// is assumed to feed byte-aligned TS; resync lives in `mpeg_ts::resync`).
fn iter_ts_packets(data: &[u8]) -> impl Iterator<Item = TsPacket<'_>> {
    data.chunks_exact(TS_PACKET_SIZE)
        .filter_map(|chunk| TsPacket::parse(chunk).ok())
}

/// Collect every PCR observation from `data`'s TS adaptation fields, in
/// packet order (ISO/IEC 13818-1 §2.4.3.4/§2.4.3.5). `packet_index` is the
/// 0-based index of the 188-byte packet within `data` (i.e. `byte_offset /
/// 188`), counted over every chunk regardless of whether it parses, so it
/// stays a stable position reference even across malformed packets. Packets
/// with no adaptation field, no PCR, or an unparseable adaptation field are
/// skipped (never fatal).
fn collect_pcr(data: &[u8]) -> Vec<PcrSample> {
    let mut out = Vec::new();
    for (idx, chunk) in data.chunks_exact(TS_PACKET_SIZE).enumerate() {
        let Ok(pkt) = TsPacket::parse(chunk) else {
            continue;
        };
        let Some(Ok(af)) = pkt.adaptation_field() else {
            continue;
        };
        let Some(pcr) = af.pcr else {
            continue;
        };
        out.push(PcrSample {
            pcr_27mhz: pcr.as_27mhz(),
            pid: pkt.header.pid,
            packet_index: idx as u64,
            discontinuity: af.discontinuity_indicator,
        });
    }
    out
}

/// Parse a PAT section, returning every `program_map_PID` it lists (network
/// entries — `program_number == 0` — are skipped). ISO/IEC 13818-1 §2.4.4.3.
fn parse_pat(section: &[u8]) -> Result<Vec<u16>> {
    if section.first().copied() != Some(TABLE_ID_PAT) {
        return Ok(Vec::new());
    }
    let body = section_body(section, "PAT")?;
    let mut pids = Vec::new();
    let mut off = 0usize;
    while off + PAT_ENTRY_LEN <= body.len() {
        let program_number = u16::from_be_bytes([body[off], body[off + 1]]);
        let pid = (((body[off + 2] & PID_HI_MASK) as u16) << 8) | body[off + 3] as u16;
        if program_number != NETWORK_PROGRAM_NUMBER {
            pids.push(pid);
        }
        off += PAT_ENTRY_LEN;
    }
    Ok(pids)
}

/// Parse a PMT section, returning `(elementary_PID, codec, ES_info
/// descriptors)` for every stream whose `stream_type` maps to a supported
/// codec. ISO/IEC 13818-1 §2.4.4.8. `descriptors` is the raw ES_info
/// descriptor-loop bytes for that stream (empty when `ES_info_length` is 0);
/// consumers that don't need it (every codec but [`Codec::Data`]) simply
/// ignore it.
fn parse_pmt(section: &[u8]) -> Result<Vec<(u16, Codec, Vec<u8>)>> {
    if section.first().copied() != Some(TABLE_ID_PMT) {
        return Ok(Vec::new());
    }
    let body = section_body(section, "PMT")?;
    // PMT body prefix: reserved(3)+PCR_PID(13) = 2 bytes, then
    // reserved(4)+program_info_length(12) = 2 bytes, then the descriptor loop.
    if body.len() < 4 {
        return Err(Error::BufferTooShort {
            need: 4,
            have: body.len(),
            what: "PMT program-info prefix",
        });
    }
    let program_info_length = (((body[2] & INFO_LENGTH_HI_MASK) as usize) << 8) | body[3] as usize;
    let mut off = 4 + program_info_length;
    let mut out = Vec::new();
    // Each ES entry: stream_type(1) + reserved(3)/elementary_PID(13) [2] +
    // reserved(4)/ES_info_length(12) [2] + descriptor()×ES_info_length.
    while off + 5 <= body.len() {
        let stream_type = body[off];
        let es_pid = (((body[off + 1] & PID_HI_MASK) as u16) << 8) | body[off + 2] as u16;
        let es_info_length =
            (((body[off + 3] & INFO_LENGTH_HI_MASK) as usize) << 8) | body[off + 4] as usize;
        let desc_start = off + 5;
        let desc_end = (desc_start + es_info_length).min(body.len());
        if let Some(codec) = Codec::from_stream_type(stream_type) {
            let descriptors = body[desc_start..desc_end].to_vec();
            out.push((es_pid, codec, descriptors));
        }
        off += 5 + es_info_length;
    }
    Ok(out)
}

/// Slice a long-form PSI section's table body: the bytes between the 8-byte
/// section header and the trailing 4-byte CRC_32 (ISO/IEC 13818-1 §2.4.4.1),
/// bounded by the declared `section_length`.
fn section_body<'a>(section: &'a [u8], what: &'static str) -> Result<&'a [u8]> {
    if section.len() < SECTION_HEADER_LEN + CRC32_LEN {
        return Err(Error::BufferTooShort {
            need: SECTION_HEADER_LEN + CRC32_LEN,
            have: section.len(),
            what,
        });
    }
    // section_length counts the bytes AFTER the 3-byte header, i.e. through CRC.
    let section_length =
        (((section[1] & SECTION_LENGTH_HI_MASK) as usize) << 8) | section[2] as usize;
    let total = 3 + section_length;
    let end = total.min(section.len());
    if end < SECTION_HEADER_LEN + CRC32_LEN {
        return Err(Error::BufferTooShort {
            need: SECTION_HEADER_LEN + CRC32_LEN,
            have: end,
            what,
        });
    }
    Ok(&section[SECTION_HEADER_LEN..end - CRC32_LEN])
}

/// Parse a completed PES packet's bytes and append its access unit (payload +
/// resolved PTS/DTS) to the elementary stream. Malformed PES / empty payloads
/// are dropped rather than failing the whole demux.
fn push_access_unit(es: &mut ElementaryStream, pes_bytes: &[u8]) {
    let Ok(pes) = PesPacket::parse(pes_bytes) else {
        return;
    };
    if pes.payload.is_empty() {
        return;
    }
    // Fallback when this PES carries neither PTS nor DTS: the previous access
    // unit's already-resolved timestamps (or `(0, 0)` for the very first AU in
    // the stream, when nothing has ever been seen). Falling back to the
    // *previous* value rather than a hardcoded 0 matters for opaque PES data
    // streams (issue #557), whose sparse "heartbeat" access units can
    // legitimately carry no PES header timing at all: defaulting such an AU to
    // literal 0 would hand `unwrap_ts` a fake near-zero raw timestamp, which a
    // real subsequent AU (anywhere in the 33-bit range) would then look like a
    // spurious backward wrap relative to.
    let fallback = es
        .access_units
        .last()
        .map(|au| (au.pts, au.dts))
        .unwrap_or((0, 0));
    let (pts, dts) = pes
        .header
        .as_ref()
        .map(|h| {
            let pts = h.pts.map(|p| p.0);
            let dts = h.dts.map(|d| d.0);
            // DTS defaults to PTS when absent; PTS defaults to DTS; else the
            // fallback above.
            let pts = pts.or(dts).unwrap_or(fallback.0);
            let dts = dts.unwrap_or(pts);
            (pts, dts)
        })
        .unwrap_or(fallback);
    es.access_units.push(AccessUnit {
        data: pes.payload.to_vec(),
        pts,
        dts,
    });
}

/// Build one IR [`Track`] for an elementary stream: recover the codec config
/// into a [`TrackSpec`] and convert access units into decode-ordered samples.
///
/// Returns `None` when the config cannot be recovered (skip, never fatal).
fn build_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    match es.codec {
        Codec::H264 => build_h264_track(es, track_id),
        Codec::Hevc => build_h265_track(es, track_id),
        Codec::Mpeg2Video => build_mpeg2_video_track(es, track_id),
        Codec::MpegAudio(is_mpeg2) => build_mpeg_audio_track(es, track_id, is_mpeg2),
        Codec::Aac => build_aac_track(es, track_id),
        Codec::Ac3 => build_ac3_track(es, track_id),
        Codec::Eac3 => build_eac3_track(es, track_id),
        // DTS is recognized in the PMT but the hub IR ([`CodecConfig`]) does not
        // yet carry a DTS-from-ES audio config, so no track is built (issue #467:
        // skip, do not fail). DTS-from-TS remains unimplemented (no fixture).
        Codec::Dts => None,
        Codec::Data(stream_type) => build_data_track(es, track_id, stream_type),
    }
}

/// Extend a running unwrapped timestamp by the delta to the next raw 33-bit
/// value, correcting for a single 90 kHz wrap in either direction (§2.4.3.7).
///
/// The delta is computed on the wrapped clock (a signed value in
/// `(-2^32, 2^32]`), then applied to the unwrapped accumulator — so ordinary
/// B-frame reordering (small backward deltas within an epoch) is preserved and
/// only a near-full-range jump is treated as a wrap.
fn unwrap_ts(prev_unwrapped: i128, prev_raw: u64, raw: u64) -> i128 {
    let mut delta = raw as i128 - prev_raw as i128;
    if delta > TS_WRAP_HALF as i128 {
        delta -= TS_WRAP as i128; // wrapped backward across 2^33
    } else if delta < -(TS_WRAP_HALF as i128) {
        delta += TS_WRAP as i128; // wrapped forward across 2^33
    }
    prev_unwrapped + delta
}

/// Unwrap every access unit's `(pts, dts)` across the 33-bit 90 kHz wrap, in
/// wire (stream) order — the shared basis for both [`decode_order`] (video,
/// which additionally reorders by DTS) and the audio/data track builders
/// (which need no reordering: PES access units already arrive in
/// presentation/decode order for those elementary streams).
fn unwrap_all(units: &[AccessUnit]) -> Vec<(i128, i128)> {
    if units.is_empty() {
        return Vec::new();
    }
    let mut unwrapped: Vec<(i128, i128)> = Vec::with_capacity(units.len());
    // A raw value of exactly 0 before any genuine (non-placeholder) value has
    // been observed is always `push_access_unit`'s fallback for a PES with no
    // header timing at all (never a real 90 kHz wire timestamp landing on
    // tick 0 in practice) — e.g. a sparse opaque-data-stream "heartbeat" AU
    // preceding the very first timestamped one (issue #557). Until a genuine
    // value appears, don't run wrap-jump detection against it (a large
    // forward jump away from a placeholder zero is not a wrap): just
    // re-baseline on whatever arrives.
    let mut dts_seen_real = units[0].dts != 0;
    let mut pts_seen_real = units[0].pts != 0;
    let (mut prev_dts_raw, mut prev_dts_uw) = (units[0].dts, units[0].dts as i128);
    let (mut prev_pts_raw, mut prev_pts_uw) = (units[0].pts, units[0].pts as i128);
    unwrapped.push((prev_pts_uw, prev_dts_uw));

    for au in &units[1..] {
        let dts_uw = if dts_seen_real {
            unwrap_ts(prev_dts_uw, prev_dts_raw, au.dts)
        } else {
            dts_seen_real = au.dts != 0;
            au.dts as i128
        };
        let pts_uw = if pts_seen_real {
            unwrap_ts(prev_pts_uw, prev_pts_raw, au.pts)
        } else {
            pts_seen_real = au.pts != 0;
            au.pts as i128
        };
        prev_dts_raw = au.dts;
        prev_dts_uw = dts_uw;
        prev_pts_raw = au.pts;
        prev_pts_uw = pts_uw;
        unwrapped.push((pts_uw, dts_uw));
    }
    unwrapped
}

/// Sort access-unit indices into decode order (ascending unwrapped DTS) and
/// return, per index, the unwrapped `(pts, dts)`. Preserves input order for
/// equal DTS (stable). Timestamps are unwrapped across the 33-bit 90 kHz wrap
/// using the stream (wire) order.
fn decode_order(units: &[AccessUnit]) -> Vec<(usize, i128, i128)> {
    let unwrapped = unwrap_all(units);
    if unwrapped.is_empty() {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..units.len()).collect();
    order.sort_by_key(|&i| unwrapped[i].1); // stable sort by unwrapped DTS
    order
        .into_iter()
        .map(|i| (i, unwrapped[i].0, unwrapped[i].1))
        .collect()
}

/// Interpolated 90 kHz PES-clock timestamp for a sample `elapsed_samples`
/// into a source access unit anchored at the unwrapped `anchor_uw` PTS/DTS
/// (ISO/IEC 13818-1 §2.4.3.7): `anchor + elapsed_samples * 90000 /
/// sample_rate`, floored (u128 math to avoid overflow on a full 33-bit
/// anchor). `elapsed_samples == 0` returns `anchor` exactly — the PES-boundary
/// sample's timestamp is never touched by interpolation (issue #556).
fn interpolate_ts(anchor_uw: i128, elapsed_samples: u64, sample_rate: u32) -> u64 {
    let base = anchor_uw.max(0) as u128;
    let offset = (elapsed_samples as u128 * VIDEO_TIMESCALE as u128) / sample_rate.max(1) as u128;
    (base + offset) as u64
}

/// Per-sample duration from decode-ordered unwrapped DTS deltas; the final
/// sample reuses the previous delta (no successor to measure against).
fn durations_from_dts(ordered: &[(usize, i128, i128)]) -> Vec<u32> {
    let n = ordered.len();
    let mut durs = alloc::vec![0u32; n];
    for i in 0..n {
        let dur = if i + 1 < n {
            (ordered[i + 1].2 - ordered[i].2).max(0) as u64
        } else if i > 0 {
            durs[i - 1] as u64
        } else {
            0
        };
        durs[i] = dur as u32;
    }
    durs
}

/// Absolute decode-time anchor for an audio track: the first access unit's
/// DTS (in the 90 kHz PES clock, [`VIDEO_TIMESCALE`]) rescaled to the audio
/// track's own media timescale (`sample_rate` ticks/s). Audio access units are
/// not reordered, so the first AU carries the earliest DTS.
fn audio_start_decode_time(es: &ElementaryStream, sample_rate: u32) -> u64 {
    let Some(first) = es.access_units.first() else {
        return 0;
    };
    // dts is in 90 kHz ticks: anchor = dts * sample_rate / 90000 (u128 to avoid
    // overflow on a full 33-bit dts).
    (first.dts as u128 * sample_rate as u128 / VIDEO_TIMESCALE as u128) as u64
}

/// Recover H.264 config + build video samples (Annex B → length-prefixed).
fn build_h264_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;
    let mut is_idr: Vec<bool> = Vec::with_capacity(es.access_units.len());
    for au in &es.access_units {
        let mut idr = false;
        for nal in iter_annexb_nals(&au.data) {
            match nal[0] & H264_NAL_TYPE_MASK {
                H264_NAL_SPS if sps.is_none() => sps = Some(nal.to_vec()),
                H264_NAL_PPS if pps.is_none() => pps = Some(nal.to_vec()),
                _ => {}
            }
            // IDR/keyframe classification is delegated to the shared helper
            // (single source of truth across the demuxers — issue #517).
            if crate::nal::is_keyframe_nal(crate::nal::NalCodec::Avc, nal) {
                idr = true;
            }
        }
        is_idr.push(idr);
    }
    let sps = sps?;
    let pps = pps?;
    if sps.len() < 4 {
        return None;
    }
    // Coded dimensions from the SPS (ISO/IEC 14496-10 §7.3.2.1.1) — the TS in-band
    // parameter set carries them; decode into the track spec (0 if undecodable).
    let (width, height) = crate::sps::decode_avc_sps(&sps)
        .map(|i| (i.width as u16, i.height as u16))
        .unwrap_or((0, 0));
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        // profile_idc / constraint_flags / level_idc live at SPS bytes 1..=3
        // (after the 1-byte NAL header) — ISO/IEC 14496-15 §5.3.3.1.
        profile_indication: sps[1],
        profile_compatibility: sps[2],
        level_indication: sps[3],
        length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
        sps: alloc::vec![AvcSps(sps)],
        pps: alloc::vec![AvcPps(pps)],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: alloc::vec![],
    };
    let config = AVCConfigurationBox::new(record);

    let ordered = decode_order(&es.access_units);
    let durations = durations_from_dts(&ordered);
    let samples: Vec<Sample> = ordered
        .iter()
        .enumerate()
        .map(|(pos, &(i, pts, dts))| {
            let composition_offset = (pts - dts) as i32;
            Sample::from_annexb(
                &es.access_units[i].data,
                durations[pos],
                is_idr[i],
                composition_offset,
            )
            .with_source_timing(SourceTiming {
                pts: pts.max(0) as u64,
                dts: dts.max(0) as u64,
            })
        })
        .collect();

    // Absolute decode-time anchor: the first sample's DTS in decode order,
    // already 33-bit-unwrapped by `decode_order`, in the 90 kHz media timescale.
    let start_decode_time = ordered.first().map(|&(_, _, dts)| dts.max(0) as u64);
    Some(Track::new_at(
        TrackSpec {
            track_id,
            timescale: VIDEO_TIMESCALE,
            config: CodecConfig::Avc {
                config,
                width,
                height,
            },
        },
        samples,
        start_decode_time.unwrap_or(0),
    ))
}

/// Recover H.265/HEVC config + build video samples (Annex B → length-prefixed).
///
/// Gathers the first in-band VPS/SPS/PPS from the Annex-B access units, decodes
/// the SPS ([`crate::sps::decode_hevc_sps`]) for the coded geometry + PTL /
/// chroma / bit-depth fields, assembles an `hvcC`
/// ([`HEVCDecoderConfigurationRecord`], ISO/IEC 14496-15:2017 §8.3.3) with one
/// NAL array per parameter-set type, and emits a [`CodecConfig::Hevc`] track
/// (identical to the config `Fmp4Demux` recovers from an fMP4 `hvcC`).
///
/// Per-sample `is_sync` marks an IRAP access unit (HEVC NAL types 16..=23), via
/// the shared [`crate::nal::is_keyframe_nal`] helper. Returns `None` when no
/// SPS (or no VPS/PPS) is present, so the config cannot be built (skip, never
/// fatal — issue #467).
fn build_h265_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    let mut vps: Option<Vec<u8>> = None;
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;
    let mut is_irap: Vec<bool> = Vec::with_capacity(es.access_units.len());
    for au in &es.access_units {
        let mut irap = false;
        for nal in iter_annexb_nals(&au.data) {
            match crate::nal::nal_unit_type(crate::nal::NalCodec::Hevc, nal) {
                Some(H265_NAL_VPS) if vps.is_none() => vps = Some(nal.to_vec()),
                Some(H265_NAL_SPS) if sps.is_none() => sps = Some(nal.to_vec()),
                Some(H265_NAL_PPS) if pps.is_none() => pps = Some(nal.to_vec()),
                _ => {}
            }
            // IRAP (random-access) classification via the shared helper (single
            // source of truth across the demuxers — issue #517).
            if crate::nal::is_keyframe_nal(crate::nal::NalCodec::Hevc, nal) {
                irap = true;
            }
        }
        is_irap.push(irap);
    }
    let sps_nal = sps?;
    // Decode the SPS for geometry + profile/tier/level/chroma/bit-depth. Without
    // it the hvcC PTL fields cannot be filled — skip the track (never fatal).
    let info = crate::sps::decode_hevc_sps(&sps_nal).ok()?;
    let width = info.width.min(u16::MAX as u32) as u16;
    let height = info.height.min(u16::MAX as u32) as u16;

    // Assemble the hvcC NAL arrays: one array per parameter-set type present
    // (VPS 32, SPS 33, PPS 34), in that spec-conventional order. Each carries
    // the raw NAL unit (with its 2-byte NAL header), array_completeness = true.
    let mut arrays: Vec<HevcNalArray> = Vec::new();
    if let Some(vps_nal) = vps {
        arrays.push(HevcNalArray::new(
            true,
            H265_NAL_VPS,
            alloc::vec![HevcNalUnit::new(vps_nal)],
        ));
    }
    arrays.push(HevcNalArray::new(
        true,
        H265_NAL_SPS,
        alloc::vec![HevcNalUnit::new(sps_nal)],
    ));
    if let Some(pps_nal) = pps {
        arrays.push(HevcNalArray::new(
            true,
            H265_NAL_PPS,
            alloc::vec![HevcNalUnit::new(pps_nal)],
        ));
    }

    let record = HEVCDecoderConfigurationRecord {
        configuration_version: HVCC_CONFIGURATION_VERSION,
        general_profile_space: info.general_profile_space,
        general_tier_flag: info.general_tier_flag,
        general_profile_idc: info.general_profile_idc,
        general_profile_compatibility_flags: info.general_profile_compatibility_flags,
        general_constraint_indicator_flags: info.general_constraint_indicator_flags,
        general_level_idc: info.general_level_idc,
        min_spatial_segmentation_idc: HVCC_MIN_SPATIAL_SEGMENTATION_UNSPEC,
        parallelism_type: HVCC_PARALLELISM_TYPE_UNKNOWN,
        chroma_format_idc: info.chroma_format_idc,
        // hvcC stores bit_depth_{luma,chroma}_minus8; the SPS decode returns the
        // absolute bit depth (minus8 + 8), so subtract 8 back out (saturating —
        // an ES reporting < 8 would be malformed).
        bit_depth_luma_minus8: info.bit_depth_luma.saturating_sub(8),
        bit_depth_chroma_minus8: info.bit_depth_chroma.saturating_sub(8),
        avg_frame_rate: HVCC_AVG_FRAME_RATE_UNSPEC,
        constant_frame_rate: HVCC_CONSTANT_FRAME_RATE_UNSPEC,
        num_temporal_layers: HVCC_NUM_TEMPORAL_LAYERS,
        temporal_id_nested: false,
        length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
        arrays,
    };
    let config = HEVCConfigurationBox::new(record);

    let ordered = decode_order(&es.access_units);
    let durations = durations_from_dts(&ordered);
    let samples: Vec<Sample> = ordered
        .iter()
        .enumerate()
        .map(|(pos, &(i, pts, dts))| {
            let composition_offset = (pts - dts) as i32;
            Sample::from_annexb(
                &es.access_units[i].data,
                durations[pos],
                is_irap[i],
                composition_offset,
            )
            .with_source_timing(SourceTiming {
                pts: pts.max(0) as u64,
                dts: dts.max(0) as u64,
            })
        })
        .collect();

    // Absolute decode-time anchor: the first sample's DTS in decode order,
    // already 33-bit-unwrapped by `decode_order`, in the 90 kHz media timescale
    // (the #476 anchor — mirrors the AVC path).
    let start_decode_time = ordered.first().map(|&(_, _, dts)| dts.max(0) as u64);
    Some(Track::new_at(
        TrackSpec {
            track_id,
            timescale: VIDEO_TIMESCALE,
            config: CodecConfig::Hevc {
                config,
                width,
                height,
            },
        },
        samples,
        start_decode_time.unwrap_or(0),
    ))
}

/// MPEG-2 video `picture_start_code` (0x00000100) — ISO/IEC 13818-2 §6.2.3.
const MPEG2_PICTURE_START_CODE: u8 = 0x00;
/// `picture_coding_type` value for an intra-coded (I) picture — §6.3.9 Table 6-12.
const MPEG2_PICTURE_CODING_TYPE_I: u8 = 0x01;
/// `esds` `ES_ID` assigned to the single video elementary stream.
const ESDS_VIDEO_ES_ID: u16 = 2;

/// Whether an MPEG-2 video access unit is a random-access point: it carries a
/// `sequence_header()` (0x000001B3) or its `picture_header()` codes an I-frame
/// (`picture_coding_type == 1`) — ISO/IEC 13818-2 §6.2.2.1 / §6.3.9.
fn mpeg2_is_sync(au: &[u8]) -> bool {
    let mut i = 0usize;
    while i + 4 <= au.len() {
        if au[i] == 0x00 && au[i + 1] == 0x00 && au[i + 2] == 0x01 {
            let code = au[i + 3];
            if code == crate::mpeg_legacy::SEQUENCE_HEADER_CODE[3] {
                return true;
            }
            if code == MPEG2_PICTURE_START_CODE && i + 6 <= au.len() {
                // picture_coding_type = bits [5:3] of the byte after temporal_ref
                // high byte: header = temporal_reference(10) + coding_type(3).
                let pct = (au[i + 5] >> 3) & 0x07;
                return pct == MPEG2_PICTURE_CODING_TYPE_I;
            }
        }
        i += 1;
    }
    false
}

/// Build an MPEG-2 video (H.262) track: recover geometry from the in-band
/// `sequence_header()` into an `esds` (OTI 0x61), one sample per PES access
/// unit (raw ES bytes, start codes preserved), decode-ordered by DTS.
fn build_mpeg2_video_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    // Geometry from the first sequence_header() seen in the stream.
    let seq = es
        .access_units
        .iter()
        .find_map(|au| crate::mpeg_legacy::Mpeg2SeqHeader::find(&au.data).ok())?;

    let esds = EsdsBox::new(ESDescriptor {
        es_id: ESDS_VIDEO_ES_ID,
        stream_dependence_flag: false,
        url_flag: false,
        ocr_stream_flag: false,
        stream_priority: 0,
        depends_on_es_id: None,
        url: None,
        ocr_es_id: None,
        decoder_config: Some(DecoderConfigDescriptor {
            object_type_indication: ObjectTypeIndication(OTI_MPEG2_VIDEO_MAIN),
            stream_type: EsdsStreamType(STREAM_TYPE_VISUAL),
            up_stream: false,
            buffer_size_db: 0,
            max_bitrate: 0,
            avg_bitrate: 0,
            decoder_specific_info: None,
        }),
        sl_config: Some(SLConfigDescriptor {
            body: alloc::vec![SL_CONFIG_PREDEFINED_MP4],
        }),
    });

    let is_sync: Vec<bool> = es
        .access_units
        .iter()
        .map(|au| mpeg2_is_sync(&au.data))
        .collect();
    let ordered = decode_order(&es.access_units);
    let durations = durations_from_dts(&ordered);
    let samples: Vec<Sample> = ordered
        .iter()
        .enumerate()
        .map(|(pos, &(i, pts, dts))| Sample {
            data: es.access_units[i].data.clone(),
            duration: durations[pos],
            is_sync: is_sync[i],
            composition_offset: (pts - dts) as i32,
            source_timing: Some(SourceTiming {
                pts: pts.max(0) as u64,
                dts: dts.max(0) as u64,
            }),
        })
        .collect();

    // Absolute decode-time anchor: first-in-decode-order unwrapped DTS (90 kHz).
    let start_decode_time = ordered.first().map(|&(_, _, dts)| dts.max(0) as u64);
    Some(Track::new_at(
        TrackSpec {
            track_id,
            timescale: VIDEO_TIMESCALE,
            config: CodecConfig::Mpeg2Video {
                esds,
                width: seq.width,
                height: seq.height,
            },
        },
        samples,
        start_decode_time.unwrap_or(0),
    ))
}

/// Split a concatenated MPEG audio payload into individual frames using the
/// frame-header length field (ISO/IEC 11172-3 §2.4.1.3). Stops at the first
/// bad sync / over-run so a partial tail does not lose earlier frames.
fn split_mpeg_audio_frames(payload: &[u8]) -> Vec<&[u8]> {
    let mut frames = Vec::new();
    let mut off = 0usize;
    while off + 4 <= payload.len() {
        let Ok(hdr) = MpegAudioFrameHeader::parse(&payload[off..]) else {
            break;
        };
        let flen = hdr.frame_length;
        if flen < 4 || off + flen > payload.len() {
            break;
        }
        frames.push(&payload[off..off + flen]);
        off += flen;
    }
    frames
}

/// Build an MPEG-1/2 audio track: recover config from the first frame header
/// into an `esds` (OTI 0x6B / 0x69), one raw sample per audio frame.
fn build_mpeg_audio_track(es: &ElementaryStream, track_id: u32, is_mpeg2: bool) -> Option<Track> {
    let first = es
        .access_units
        .iter()
        .find_map(|au| MpegAudioFrameHeader::parse(&au.data).ok())?;
    let sample_rate = first.sample_rate;
    let channel_count = first.channels;
    let samples_per_frame = first.samples_per_frame;
    let oti = if is_mpeg2 {
        OTI_MPEG2_AUDIO
    } else {
        OTI_MPEG1_AUDIO
    };

    let esds = EsdsBox::new(ESDescriptor {
        es_id: ESDS_ES_ID,
        stream_dependence_flag: false,
        url_flag: false,
        ocr_stream_flag: false,
        stream_priority: 0,
        depends_on_es_id: None,
        url: None,
        ocr_es_id: None,
        decoder_config: Some(DecoderConfigDescriptor {
            object_type_indication: ObjectTypeIndication(oti),
            stream_type: EsdsStreamType(STREAM_TYPE_AUDIO),
            up_stream: false,
            buffer_size_db: 0,
            max_bitrate: 0,
            avg_bitrate: 0,
            decoder_specific_info: None,
        }),
        sl_config: Some(SLConfigDescriptor {
            body: alloc::vec![SL_CONFIG_PREDEFINED_MP4],
        }),
    });

    // One sample per MPEG audio frame, with interpolated per-frame timing when
    // a PES access unit carries more than one frame (issue #556): the first
    // frame in each AU carries the AU's unwrapped PTS/DTS exactly, subsequent
    // frames are interpolated from the elapsed sample count.
    let unwrapped = unwrap_all(&es.access_units);
    let mut samples: Vec<Sample> = Vec::new();
    for (au, &(pts_uw, dts_uw)) in es.access_units.iter().zip(unwrapped.iter()) {
        let mut elapsed = 0u64;
        for frame in split_mpeg_audio_frames(&au.data) {
            samples.push(
                Sample::from_raw(frame.to_vec(), samples_per_frame).with_source_timing(
                    SourceTiming {
                        pts: interpolate_ts(pts_uw, elapsed, sample_rate),
                        dts: interpolate_ts(dts_uw, elapsed, sample_rate),
                    },
                ),
            );
            elapsed += samples_per_frame as u64;
        }
    }
    if samples.is_empty() {
        return None;
    }

    Some(Track::new_at(
        TrackSpec {
            track_id,
            timescale: sample_rate,
            config: CodecConfig::MpegAudio {
                esds,
                layer: first.layer,
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            },
        },
        samples,
        audio_start_decode_time(es, sample_rate),
    ))
}

/// Split a concatenated ADTS payload into individual frames (header + raw data).
fn split_adts_frames(payload: &[u8]) -> Vec<&[u8]> {
    let mut frames = Vec::new();
    let mut off = 0usize;
    while off + ADTS_HEADER_SIZE <= payload.len() {
        let Ok(hdr) = parse_adts_header(&payload[off..]) else {
            break;
        };
        let frame_len = hdr.frame_length as usize;
        if frame_len < ADTS_HEADER_SIZE || off + frame_len > payload.len() {
            break;
        }
        frames.push(&payload[off..off + frame_len]);
        off += frame_len;
    }
    frames
}

/// Recover AAC config (ADTS → ASC → `esds`) + build one raw sample per ADTS frame.
fn build_aac_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    // The ADTS header of the first frame gives profile/rate/channels → ASC.
    let first_hdr = es
        .access_units
        .iter()
        .find_map(|au| parse_adts_header(&au.data).ok())?;
    let asc = AudioSpecificConfig::from_adts_header(&first_hdr);
    let asc_bytes = asc.to_bytes();
    let sample_rate = sfi_to_hz(first_hdr.sampling_frequency_index)?;
    let channel_count = first_hdr.channel_configuration as u16;

    let esds = EsdsBox::new(ESDescriptor {
        es_id: ESDS_ES_ID,
        stream_dependence_flag: false,
        url_flag: false,
        ocr_stream_flag: false,
        stream_priority: 0,
        depends_on_es_id: None,
        url: None,
        ocr_es_id: None,
        decoder_config: Some(DecoderConfigDescriptor {
            object_type_indication: ObjectTypeIndication(OTI_MPEG4_AUDIO),
            stream_type: EsdsStreamType(STREAM_TYPE_AUDIO),
            up_stream: false,
            buffer_size_db: 0,
            max_bitrate: 0,
            avg_bitrate: 0,
            decoder_specific_info: Some(DecoderSpecificInfo { data: asc_bytes }),
        }),
        sl_config: Some(SLConfigDescriptor {
            body: alloc::vec![SL_CONFIG_PREDEFINED_MP4],
        }),
    });

    // One sample per ADTS frame, with its 7-byte header stripped (raw AAC AU).
    // Audio AUs are all sync samples; duration is 1024 samples @ the ES rate.
    // Interpolated per-frame timing when a PES AU carries more than one ADTS
    // frame (issue #556): the first frame carries the AU's unwrapped PTS/DTS
    // exactly, subsequent frames are interpolated from the elapsed samples.
    let unwrapped = unwrap_all(&es.access_units);
    let mut samples: Vec<Sample> = Vec::new();
    for (au, &(pts_uw, dts_uw)) in es.access_units.iter().zip(unwrapped.iter()) {
        let mut elapsed = 0u64;
        for frame in split_adts_frames(&au.data) {
            if frame.len() > ADTS_HEADER_SIZE {
                samples.push(
                    Sample::from_raw(frame[ADTS_HEADER_SIZE..].to_vec(), AAC_SAMPLES_PER_FRAME)
                        .with_source_timing(SourceTiming {
                            pts: interpolate_ts(pts_uw, elapsed, sample_rate),
                            dts: interpolate_ts(dts_uw, elapsed, sample_rate),
                        }),
                );
            }
            elapsed += AAC_SAMPLES_PER_FRAME as u64;
        }
    }
    if samples.is_empty() {
        return None;
    }

    Some(Track::new_at(
        TrackSpec {
            track_id,
            timescale: sample_rate,
            config: CodecConfig::Aac {
                esds,
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            },
        },
        samples,
        audio_start_decode_time(es, sample_rate),
    ))
}

/// Recover AC-3 config (syncframe BSI → `dac3`) and split each PES payload
/// into individual syncframes (issue #556: a 44.1 kHz AC-3 cadence never
/// divides the 90 kHz PES clock, so one Sample per PES AU with `duration = 0`
/// cannot reproduce PES PTS exactly). Each syncframe gets `duration =
/// `[`AC3_SAMPLES_PER_SYNCFRAME`] and interpolated per-syncframe timing: the
/// first syncframe in a PES AU carries that AU's unwrapped PTS/DTS exactly,
/// subsequent syncframes are interpolated from the elapsed sample count.
fn build_ac3_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    let info = es
        .access_units
        .iter()
        .find_map(|au| Ac3SyncframeInfo::from_es(&au.data).ok())?;
    let sample_rate = info.sample_rate;
    let channel_count = info.channel_count() as u16;
    let config = info.into_dac3();

    let unwrapped = unwrap_all(&es.access_units);
    let mut samples: Vec<Sample> = Vec::new();
    for (au, &(pts_uw, dts_uw)) in es.access_units.iter().zip(unwrapped.iter()) {
        let mut elapsed = 0u64;
        for frame in split_ac3_syncframes(&au.data) {
            samples.push(
                Sample::from_raw(frame.to_vec(), AC3_SAMPLES_PER_SYNCFRAME).with_source_timing(
                    SourceTiming {
                        pts: interpolate_ts(pts_uw, elapsed, sample_rate),
                        dts: interpolate_ts(dts_uw, elapsed, sample_rate),
                    },
                ),
            );
            elapsed += AC3_SAMPLES_PER_SYNCFRAME as u64;
        }
    }
    if samples.is_empty() {
        return None;
    }
    Some(Track::new_at(
        TrackSpec {
            track_id,
            timescale: sample_rate,
            config: CodecConfig::Ac3 {
                config,
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            },
        },
        samples,
        audio_start_decode_time(es, sample_rate),
    ))
}

/// Recover E-AC-3 config (syncframe BSI → `dec3`) and split each PES payload
/// into individual access units (issue #556): each independent syncframe
/// (`strmtyp != 0x1`) starts a new access unit; a dependent-substream
/// syncframe immediately following is concatenated into it
/// ([`split_eac3_syncframes`]). `duration = numblks * 256` from the
/// independent frame; timing is interpolated the same way as AC-3.
fn build_eac3_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    let info = es
        .access_units
        .iter()
        .find_map(|au| Ec3SyncframeInfo::from_es(&au.data).ok())?;
    let sample_rate = info.sample_rate;
    let channel_count = info.channel_count() as u16;
    let config = info.into_dec3();

    let unwrapped = unwrap_all(&es.access_units);
    let mut samples: Vec<Sample> = Vec::new();
    for (au, &(pts_uw, dts_uw)) in es.access_units.iter().zip(unwrapped.iter()) {
        let mut elapsed = 0u64;
        for split in split_eac3_syncframes(&au.data) {
            let duration = split.info.samples_per_frame();
            samples.push(
                Sample::from_raw(split.data, duration).with_source_timing(SourceTiming {
                    pts: interpolate_ts(pts_uw, elapsed, sample_rate),
                    dts: interpolate_ts(dts_uw, elapsed, sample_rate),
                }),
            );
            elapsed += duration as u64;
        }
    }
    if samples.is_empty() {
        return None;
    }
    Some(Track::new_at(
        TrackSpec {
            track_id,
            timescale: sample_rate,
            config: CodecConfig::Eac3 {
                config,
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            },
        },
        samples,
        audio_start_decode_time(es, sample_rate),
    ))
}

/// Build an opaque PES data track (issue #557): one Sample per PES access
/// unit, verbatim payload bytes, `timescale = 90 kHz`, `is_sync = true`,
/// `composition_offset = 0`, with `source_timing` from the unwrapped PES
/// PTS/DTS. `duration` is the delta to the next AU's unwrapped PTS (the last
/// sample reuses the previous duration — mirrors [`durations_from_dts`], but
/// on PTS since these streams are not guaranteed to carry a distinct DTS).
fn build_data_track(es: &ElementaryStream, track_id: u32, stream_type: u8) -> Option<Track> {
    if es.access_units.is_empty() {
        return None;
    }
    let unwrapped = unwrap_all(&es.access_units);
    let n = unwrapped.len();
    let mut durations = alloc::vec![0u32; n];
    for i in 0..n {
        let dur = if i + 1 < n {
            (unwrapped[i + 1].0 - unwrapped[i].0).max(0) as u64
        } else if i > 0 {
            durations[i - 1] as u64
        } else {
            0
        };
        durations[i] = dur as u32;
    }
    let samples: Vec<Sample> = es
        .access_units
        .iter()
        .zip(unwrapped.iter())
        .enumerate()
        .map(|(i, (au, &(pts_uw, dts_uw)))| {
            Sample::from_raw(au.data.clone(), durations[i]).with_source_timing(SourceTiming {
                pts: pts_uw.max(0) as u64,
                dts: dts_uw.max(0) as u64,
            })
        })
        .collect();
    let start_decode_time = unwrapped.first().map(|&(_, dts)| dts.max(0) as u64);
    Some(Track::new_at(
        TrackSpec {
            track_id,
            timescale: VIDEO_TIMESCALE,
            config: CodecConfig::Data {
                stream_type,
                descriptors: es.descriptors.clone(),
            },
        },
        samples,
        start_decode_time.unwrap_or(0),
    ))
}

/// Convert an ADTS `sampling_frequency_index` to Hz (ISO/IEC 14496-3 Table 1.16).
fn sfi_to_hz(sfi: u8) -> Option<u32> {
    Some(match sfi {
        0 => 96000,
        1 => 88200,
        2 => 64000,
        3 => 48000,
        4 => 44100,
        5 => 32000,
        6 => 24000,
        7 => 22050,
        8 => 16000,
        9 => 12000,
        10 => 11025,
        11 => 8000,
        12 => 7350,
        _ => return None,
    })
}
