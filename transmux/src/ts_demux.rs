//! MPEG-2 Transport Stream demuxer → hub [`Media`] IR.
//!
//! [`StreamingTsDemux`] (issue #555) is the **one** demux core: an
//! event-driven, incremental engine that consumes TS bytes of any size or
//! alignment and emits [`DemuxEvent`]s (`TrackAdded`/`TrackUpdated`/`Sample`/
//! `Pcr`/`Discontinuity`) as soon as they are known. [`TsDemux`] — the
//! **input** side of the any-to-any container hub, implementing the abstract
//! [`broadcast_common::Unpackage`] trait so `{TS} → IR → {any}` composes with
//! the existing [`CmafMux`](crate::media::CmafMux) /
//! [`HlsPackager`](crate::media::HlsPackager) packagers — is now a thin batch
//! wrapper over it: feed the whole buffer, call `finish()`, fold the event
//! stream into a [`Media`]. There is no separate whole-buffer implementation;
//! every behaviour below is produced by the streaming core.
//!
//! Pipeline: TS packet layer ([`mpeg_ts`], resynchronised via
//! [`mpeg_ts::resync::TsResync`]) → follow PAT → PMT → per-PID PES
//! reassembly ([`mpeg_pes`]) → codec-config recovery (H.264 SPS/PPS → `avcC`,
//! H.265 VPS/SPS/PPS → `hvcC`, MPEG-2 video `sequence_header()` → `esds`,
//! ADTS → AudioSpecificConfig →
//! `esds`, MPEG-1/2 audio frame header → `esds`, AC-3/E-AC-3 syncframe BSI →
//! `dac3`/`dec3`) → length-prefixed video / raw audio samples.
//!
//! Config recovery happens incrementally, access unit by access unit, and is
//! **single-shot and permanent**: the first successfully-recovered config for
//! a PID is used for the rest of the stream (identical to the old whole-file
//! `find_map` scans this replaces), so a track's `DemuxEvent::TrackAdded`
//! fires once config is known — with an opaque [`CodecConfig::Data`] track
//! (issue #557) firing on its very first access unit, since its config needs
//! no in-band header at all.
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
//! survive into the IR. Video/data-track sample durations are resolved
//! **one access unit behind**: the timestamp delta to the *next* access unit
//! (33-bit-unwrapped DTS for video, PTS for data — ISO/IEC 13818-1 §2.4.3.7)
//! finalizes the *previous* sample's duration, with the final sample of a
//! finished stream reusing the previous duration ([`finish`](StreamingTsDemux::finish)).
//!
//! PMT `stream_type` 0x06 (PES private data — DVB subtitles/teletext/SMPTE
//! 2038/etc.) and 0x15 (metadata in PES) are carried as opaque
//! [`CodecConfig::Data`] tracks (issue #557): the demuxer does not decode the
//! payload, each `Sample` is one verbatim PES payload, and `descriptors`
//! preserves the raw PMT ES_info descriptor loop for the caller to classify.
//! The demuxer also collects every PCR observation from the TS adaptation
//! fields, both into [`Media`]'s `pcr` field (batch) and as
//! [`DemuxEvent::Pcr`] (streaming).
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
//! - **Byte-stream resynchronisation**: ISO/IEC 13818-1 §2.4.3.2, via
//!   [`mpeg_ts::resync::TsResync`] (also strips 204-byte Reed-Solomon FEC).

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Serialize, Unpackage};
use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::resync::TsResync;
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};

use crate::aac_asc::{AudioSpecificConfig, parse_adts_header};
use crate::ac3::{
    AC3_SAMPLES_PER_SYNCFRAME, Ac3SyncframeInfo, Ec3SyncframeInfo, split_ac3_syncframes,
    split_eac3_syncframes,
};
use crate::annexb::{annexb_to_length_prefixed, iter_annexb_nals};
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
use crate::media::{Media, PcrSample, Track};
use crate::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, ObjectTypeIndication,
    SLConfigDescriptor, StreamType as EsdsStreamType,
};
use crate::mpeg_legacy::{Mpeg2SeqHeader, MpegAudioFrameHeader};
use crate::nal::{NalCodec, is_keyframe_nal, nal_unit_type};
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
/// The null packet PID — always stuffing, never meaningful payload
/// (ISO/IEC 13818-1 §2.4.3.2 Table 2-3) — excluded from the
/// `unattributed`-payload replay buffer.
const NULL_PACKET_PID: u16 = 0x1FFF;
/// Hard cap on the total bytes retained across all pre-PMT `unattributed` PID
/// buffers before the oldest payloads are evicted (FIFO). Bounds memory on a
/// full-multiplex feed whose unrelated-service PIDs never appear in the
/// followed PMT (live ingest); comfortably above any real capture's pre-PMT
/// lead-in (a PID's PMT entry resolves within the first PES cycle), so a
/// legitimately-claimed PID's buffered payloads are never evicted in practice.
const MAX_UNATTRIBUTED_BYTES: usize = 4 * 1024 * 1024;

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
/// `esds` `ES_ID` assigned to the single video elementary stream.
const ESDS_VIDEO_ES_ID: u16 = 2;
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

/// MPEG-2 video `picture_start_code` (0x00000100) — ISO/IEC 13818-2 §6.2.3.
const MPEG2_PICTURE_START_CODE: u8 = 0x00;
/// `picture_coding_type` value for an intra-coded (I) picture — §6.3.9 Table 6-12.
const MPEG2_PICTURE_CODING_TYPE_I: u8 = 0x01;

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

// ── Streaming core (issue #555) ─────────────────────────────────────────────

/// One buffered access unit awaiting codec-config recovery, held until the
/// owning [`ConfigProbe`] finds enough header data to build a [`CodecConfig`]
/// (mirrors the old whole-file `find_map` scans this replaces, just applied
/// incrementally — see the module docs' bounded-memory note).
struct BufferedAu {
    data: Vec<u8>,
    pts_uw: i128,
    dts_uw: i128,
}

/// Per-PID state accumulated while scanning access units for the codec
/// config. Resolution is single-shot and permanent: the moment enough header
/// data is seen, [`finalize_probe`] returns the finished [`CodecConfig`] and
/// the owning [`TrackState`] moves to `Parked` (backlog carried over as-is,
/// still accumulating — see [`TrackState`]).
enum ConfigProbe {
    H264 {
        sps: Option<Vec<u8>>,
        pps: Option<Vec<u8>>,
    },
    Hevc {
        vps: Option<Vec<u8>>,
        sps: Option<Vec<u8>>,
        pps: Option<Vec<u8>>,
    },
    Mpeg2Video,
    MpegAudio {
        is_mpeg2: bool,
    },
    Aac,
    Ac3,
    Eac3,
    /// Opaque PES data (#557): the config (`stream_type` + descriptors) is
    /// already fully known from the PMT, so this probe finalizes on the very
    /// first access unit — no header scan needed.
    Data,
    /// DTS (recognized in the PMT but not decoded from TS — issue #467): the
    /// hub IR carries no DTS-from-ES audio config, so this probe never
    /// resolves; access units are dropped silently (skip, never fatal).
    Never,
}

fn initial_probe(codec: Codec) -> ConfigProbe {
    match codec {
        Codec::H264 => ConfigProbe::H264 {
            sps: None,
            pps: None,
        },
        Codec::Hevc => ConfigProbe::Hevc {
            vps: None,
            sps: None,
            pps: None,
        },
        Codec::Mpeg2Video => ConfigProbe::Mpeg2Video,
        Codec::MpegAudio(is_mpeg2) => ConfigProbe::MpegAudio { is_mpeg2 },
        Codec::Aac => ConfigProbe::Aac,
        Codec::Ac3 => ConfigProbe::Ac3,
        Codec::Eac3 => ConfigProbe::Eac3,
        Codec::Dts => ConfigProbe::Never,
        Codec::Data(_) => ConfigProbe::Data,
    }
}

/// Video codec family for a [`LiveKind::Video`] track — selects the sample
/// byte transform (Annex B → length-prefixed, or raw ES bytes for MPEG-2) and
/// the keyframe classification.
#[derive(Clone, Copy)]
enum VideoCodec {
    H264,
    Hevc,
    Mpeg2,
}

/// Split-frame family for a [`LiveKind::Audio`] track — a PES access unit may
/// carry more than one coded frame (issue #556); each is emitted immediately
/// with its intrinsic duration (no lookahead needed, unlike video/data).
enum AudioKind {
    Aac,
    Ac3,
    Eac3,
    MpegAudio { samples_per_frame: u32 },
}

/// A completed-but-not-yet-durationed sample, held until the *next* access
/// unit resolves its duration (video: DTS delta; data: PTS delta — mirrors
/// the old batch demuxer's "duration = delta to the next access unit, last
/// sample reuses the previous duration" rule).
struct PendingOneBehind {
    data: Vec<u8>,
    is_sync: bool,
    composition_offset: i32,
    pts_uw: i128,
    dts_uw: i128,
}

/// Per-track live (config-known) processing state.
enum LiveKind {
    /// H.264/HEVC/MPEG-2 video: one `Sample` per access unit.
    Video {
        pending: Option<PendingOneBehind>,
        last_duration: u32,
        codec: VideoCodec,
    },
    /// AAC/AC-3/E-AC-3/MPEG audio: zero-lookahead, intrinsic-duration frames.
    Audio { sample_rate: u32, kind: AudioKind },
    /// Opaque PES data (#557): one `Sample` per access unit.
    Data {
        pending: Option<PendingOneBehind>,
        last_duration: u32,
    },
}

struct LiveTrack {
    track_id: u32,
    kind: LiveKind,
}

/// A [`StreamState`]'s codec-config **and** PMT-declaration-order lifecycle.
///
/// Track IDs and `DemuxEvent::TrackAdded` order must match the PMT's
/// declaration order (codec tracks first, then data tracks, each group in
/// PMT order — the old batch demuxer's invariant), which need not be the
/// order each PID's config happens to resolve in. So a PID whose config is
/// already known still waits, `Parked`, until every earlier-ranked PID has
/// itself resolved (see [`StreamingTsDemux::try_promote_ready`]) — at which
/// point it becomes `Live` and its whole backlog replays as a burst of
/// `DemuxEvent::Sample`s.
enum TrackState {
    /// No config recovered yet; `backlog` accumulates every access unit seen
    /// so far (replayed once config resolves and it's this PID's turn).
    Probing {
        probe: ConfigProbe,
        backlog: Vec<BufferedAu>,
    },
    /// Config resolved, but an earlier-ranked PID hasn't resolved yet.
    /// `backlog` keeps accumulating every access unit that arrives while
    /// parked.
    Parked {
        config: CodecConfig,
        timescale: u32,
        kind: LiveKind,
        backlog: Vec<BufferedAu>,
    },
    /// Config resolved and this PID's turn has come: `TrackAdded` has fired
    /// and samples stream directly.
    Live(LiveTrack),
}

/// Incremental 33-bit PTS/DTS wrap-unroll, one access unit at a time —
/// produces the identical sequence the old whole-stream unroll would, applied
/// access-unit-by-access-unit (ISO/IEC 13818-1 §2.4.3.7). A raw value of
/// exactly `0` before any genuine value has been observed is always the
/// caller's fallback for a PES with no header timing at all (never a real
/// 90 kHz wire timestamp landing on tick 0 in practice — e.g. a sparse opaque
/// data-stream "heartbeat" access unit preceding the first timestamped one,
/// issue #557): wrap-jump detection does not run against it.
#[derive(Default)]
struct WrapState {
    initialized: bool,
    dts_seen_real: bool,
    pts_seen_real: bool,
    prev_dts_raw: u64,
    prev_dts_uw: i128,
    prev_pts_raw: u64,
    prev_pts_uw: i128,
}

impl WrapState {
    /// Feed the next access unit's raw 33-bit `(pts, dts)`, returning the
    /// unwrapped `(pts, dts)`.
    fn push(&mut self, raw_pts: u64, raw_dts: u64) -> (i128, i128) {
        if !self.initialized {
            self.initialized = true;
            self.dts_seen_real = raw_dts != 0;
            self.pts_seen_real = raw_pts != 0;
            self.prev_dts_raw = raw_dts;
            self.prev_dts_uw = raw_dts as i128;
            self.prev_pts_raw = raw_pts;
            self.prev_pts_uw = raw_pts as i128;
            return (self.prev_pts_uw, self.prev_dts_uw);
        }
        let dts_uw = if self.dts_seen_real {
            unwrap_ts(self.prev_dts_uw, self.prev_dts_raw, raw_dts)
        } else {
            self.dts_seen_real = raw_dts != 0;
            raw_dts as i128
        };
        let pts_uw = if self.pts_seen_real {
            unwrap_ts(self.prev_pts_uw, self.prev_pts_raw, raw_pts)
        } else {
            self.pts_seen_real = raw_pts != 0;
            raw_pts as i128
        };
        self.prev_dts_raw = raw_dts;
        self.prev_dts_uw = dts_uw;
        self.prev_pts_raw = raw_pts;
        self.prev_pts_uw = pts_uw;
        (pts_uw, dts_uw)
    }
}

/// Per-PID (elementary stream) engine state.
struct StreamState {
    codec: Codec,
    descriptors: Vec<u8>,
    assembler: PesAssembler,
    /// Previous access unit's resolved `(pts, dts)` — the fallback used when
    /// a PES carries neither (mirrors the old `push_access_unit` fallback).
    fallback: (u64, u64),
    has_any: bool,
    wrap: WrapState,
    /// The very first access unit's unwrapped DTS — every track kind anchors
    /// its `Track::start_decode_time` here (verified equivalent to every old
    /// batch anchor formula: video/data/audio all reduce to "first AU's DTS").
    first_dts_uw: Option<i128>,
    /// Always `Some` except transiently inside [`advance_track`].
    track: Option<TrackState>,
}

/// Advance a one-behind (video/data) pending slot with a newly-built sample,
/// emitting the *previous* pending sample now that its duration is known
/// (`duration_from_pts` selects the PTS delta for data tracks, DTS delta for
/// video).
#[allow(clippy::too_many_arguments)]
fn advance_one_behind(
    pending: &mut Option<PendingOneBehind>,
    last_duration: &mut u32,
    data: Vec<u8>,
    is_sync: bool,
    composition_offset: i32,
    pts_uw: i128,
    dts_uw: i128,
    duration_from_pts: bool,
    track_id: u32,
    events: &mut VecDeque<DemuxEvent>,
) {
    if let Some(prev) = pending.take() {
        let duration = if duration_from_pts {
            (pts_uw - prev.pts_uw).max(0) as u32
        } else {
            (dts_uw - prev.dts_uw).max(0) as u32
        };
        *last_duration = duration;
        events.push_back(DemuxEvent::Sample {
            track_id,
            sample: Sample {
                data: prev.data,
                duration,
                is_sync: prev.is_sync,
                composition_offset: prev.composition_offset,
                source_timing: Some(SourceTiming {
                    pts: prev.pts_uw.max(0) as u64,
                    dts: prev.dts_uw.max(0) as u64,
                }),
            },
        });
    }
    *pending = Some(PendingOneBehind {
        data,
        is_sync,
        composition_offset,
        pts_uw,
        dts_uw,
    });
}

/// Flush a trailing one-behind pending sample at end of stream, reusing the
/// last-known duration (mirrors the batch tail rule: the final sample repeats
/// the previous sample's duration, or `0` if there was only ever one sample).
fn flush_one_behind(
    pending: &mut Option<PendingOneBehind>,
    last_duration: u32,
    track_id: u32,
    events: &mut VecDeque<DemuxEvent>,
) {
    if let Some(p) = pending.take() {
        events.push_back(DemuxEvent::Sample {
            track_id,
            sample: Sample {
                data: p.data,
                duration: last_duration,
                is_sync: p.is_sync,
                composition_offset: p.composition_offset,
                source_timing: Some(SourceTiming {
                    pts: p.pts_uw.max(0) as u64,
                    dts: p.dts_uw.max(0) as u64,
                }),
            },
        });
    }
}

/// Build a video sample's coded bytes + sync flag from one Annex B (or raw
/// MPEG-2) access unit.
fn video_sample_bytes(codec: VideoCodec, au_data: &[u8]) -> (Vec<u8>, bool) {
    match codec {
        VideoCodec::H264 => {
            let mut idr = false;
            for nal in iter_annexb_nals(au_data) {
                if is_keyframe_nal(NalCodec::Avc, nal) {
                    idr = true;
                }
            }
            (annexb_to_length_prefixed(au_data), idr)
        }
        VideoCodec::Hevc => {
            let mut irap = false;
            for nal in iter_annexb_nals(au_data) {
                if is_keyframe_nal(NalCodec::Hevc, nal) {
                    irap = true;
                }
            }
            (annexb_to_length_prefixed(au_data), irap)
        }
        VideoCodec::Mpeg2 => (au_data.to_vec(), mpeg2_is_sync(au_data)),
    }
}

/// Split one access unit into its coded frames and emit each immediately
/// (audio needs no lookahead: duration is intrinsic per split-frame family).
fn emit_audio_au(
    kind: &AudioKind,
    sample_rate: u32,
    au_data: &[u8],
    pts_uw: i128,
    dts_uw: i128,
    track_id: u32,
    events: &mut VecDeque<DemuxEvent>,
) {
    let mut elapsed = 0u64;
    match kind {
        AudioKind::Aac => {
            for frame in split_adts_frames(au_data) {
                if frame.len() > ADTS_HEADER_SIZE {
                    events.push_back(DemuxEvent::Sample {
                        track_id,
                        sample: Sample::from_raw(
                            frame[ADTS_HEADER_SIZE..].to_vec(),
                            AAC_SAMPLES_PER_FRAME,
                        )
                        .with_source_timing(SourceTiming {
                            pts: interpolate_ts(pts_uw, elapsed, sample_rate),
                            dts: interpolate_ts(dts_uw, elapsed, sample_rate),
                        }),
                    });
                }
                elapsed += AAC_SAMPLES_PER_FRAME as u64;
            }
        }
        AudioKind::Ac3 => {
            for frame in split_ac3_syncframes(au_data) {
                events.push_back(DemuxEvent::Sample {
                    track_id,
                    sample: Sample::from_raw(frame.to_vec(), AC3_SAMPLES_PER_SYNCFRAME)
                        .with_source_timing(SourceTiming {
                            pts: interpolate_ts(pts_uw, elapsed, sample_rate),
                            dts: interpolate_ts(dts_uw, elapsed, sample_rate),
                        }),
                });
                elapsed += AC3_SAMPLES_PER_SYNCFRAME as u64;
            }
        }
        AudioKind::Eac3 => {
            for split in split_eac3_syncframes(au_data) {
                let duration = split.info.samples_per_frame();
                events.push_back(DemuxEvent::Sample {
                    track_id,
                    sample: Sample::from_raw(split.data, duration).with_source_timing(
                        SourceTiming {
                            pts: interpolate_ts(pts_uw, elapsed, sample_rate),
                            dts: interpolate_ts(dts_uw, elapsed, sample_rate),
                        },
                    ),
                });
                elapsed += duration as u64;
            }
        }
        AudioKind::MpegAudio { samples_per_frame } => {
            for frame in split_mpeg_audio_frames(au_data) {
                events.push_back(DemuxEvent::Sample {
                    track_id,
                    sample: Sample::from_raw(frame.to_vec(), *samples_per_frame)
                        .with_source_timing(SourceTiming {
                            pts: interpolate_ts(pts_uw, elapsed, sample_rate),
                            dts: interpolate_ts(dts_uw, elapsed, sample_rate),
                        }),
                });
                elapsed += *samples_per_frame as u64;
            }
        }
    }
}

/// Apply one access unit to an already-live track, emitting whatever
/// [`DemuxEvent::Sample`]s it resolves.
fn push_live_au(
    live: &mut LiveTrack,
    data: &[u8],
    pts_uw: i128,
    dts_uw: i128,
    events: &mut VecDeque<DemuxEvent>,
) {
    let track_id = live.track_id;
    match &mut live.kind {
        LiveKind::Video {
            pending,
            last_duration,
            codec,
        } => {
            let (bytes, is_sync) = video_sample_bytes(*codec, data);
            let composition_offset = (pts_uw - dts_uw) as i32;
            advance_one_behind(
                pending,
                last_duration,
                bytes,
                is_sync,
                composition_offset,
                pts_uw,
                dts_uw,
                false,
                track_id,
                events,
            );
        }
        LiveKind::Data {
            pending,
            last_duration,
        } => {
            advance_one_behind(
                pending,
                last_duration,
                data.to_vec(),
                true,
                0,
                pts_uw,
                dts_uw,
                true,
                track_id,
                events,
            );
        }
        LiveKind::Audio { sample_rate, kind } => {
            emit_audio_au(kind, *sample_rate, data, pts_uw, dts_uw, track_id, events);
        }
    }
}

/// Feed the latest access unit (`backlog.last()`, already pushed by the
/// caller) into a probing [`ConfigProbe`], returning the finished config the
/// moment it becomes recoverable. `backlog` (every access unit seen on this
/// PID so far) is read-only here — the caller owns transferring it into
/// [`TrackState::Parked`].
fn finalize_probe(
    codec: Codec,
    descriptors: &[u8],
    probe: &mut ConfigProbe,
    backlog: &[BufferedAu],
) -> Option<(CodecConfig, u32, LiveKind)> {
    let latest = backlog
        .last()
        .expect("finalize_probe is only called after pushing the latest AU");
    match probe {
        ConfigProbe::Never => None,
        ConfigProbe::Data => {
            let Codec::Data(stream_type) = codec else {
                unreachable!("ConfigProbe::Data is only created for Codec::Data")
            };
            Some((
                CodecConfig::Data {
                    stream_type,
                    descriptors: descriptors.to_vec(),
                },
                VIDEO_TIMESCALE,
                LiveKind::Data {
                    pending: None,
                    last_duration: 0,
                },
            ))
        }
        ConfigProbe::H264 { sps, pps } => {
            for nal in iter_annexb_nals(&latest.data) {
                match nal[0] & H264_NAL_TYPE_MASK {
                    H264_NAL_SPS if sps.is_none() => *sps = Some(nal.to_vec()),
                    H264_NAL_PPS if pps.is_none() => *pps = Some(nal.to_vec()),
                    _ => {}
                }
            }
            let (sps_bytes, pps_bytes) = (sps.as_ref()?, pps.as_ref()?);
            if sps_bytes.len() < 4 {
                return None;
            }
            // Coded dimensions from the SPS (ISO/IEC 14496-10 §7.3.2.1.1) — the
            // TS in-band parameter set carries them (0 if undecodable).
            let (width, height) = crate::sps::decode_avc_sps(sps_bytes)
                .map(|i| (i.width as u16, i.height as u16))
                .unwrap_or((0, 0));
            let record = AVCDecoderConfigurationRecord {
                configuration_version: 1,
                // profile_idc / constraint_flags / level_idc live at SPS bytes
                // 1..=3 (after the 1-byte NAL header) — ISO/IEC 14496-15 §5.3.3.1.
                profile_indication: sps_bytes[1],
                profile_compatibility: sps_bytes[2],
                level_indication: sps_bytes[3],
                length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
                sps: alloc::vec![AvcSps(sps_bytes.clone())],
                pps: alloc::vec![AvcPps(pps_bytes.clone())],
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext: alloc::vec![],
            };
            Some((
                CodecConfig::Avc {
                    config: AVCConfigurationBox::new(record),
                    width,
                    height,
                },
                VIDEO_TIMESCALE,
                LiveKind::Video {
                    pending: None,
                    last_duration: 0,
                    codec: VideoCodec::H264,
                },
            ))
        }
        ConfigProbe::Hevc { vps, sps, pps } => {
            for nal in iter_annexb_nals(&latest.data) {
                match nal_unit_type(NalCodec::Hevc, nal) {
                    Some(H265_NAL_VPS) if vps.is_none() => *vps = Some(nal.to_vec()),
                    Some(H265_NAL_SPS) if sps.is_none() => *sps = Some(nal.to_vec()),
                    Some(H265_NAL_PPS) if pps.is_none() => *pps = Some(nal.to_vec()),
                    _ => {}
                }
            }
            // Decode the SPS for geometry + profile/tier/level/chroma/bit-depth.
            // Without it the hvcC PTL fields cannot be filled — stay probing
            // (never fatal — issue #467). VPS/PPS are optional: whichever have
            // been seen by the time SPS resolves are included (real encoders
            // always bundle VPS+SPS+PPS in the same access unit).
            let sps_bytes = sps.as_ref()?;
            let info = crate::sps::decode_hevc_sps(sps_bytes).ok()?;
            let width = info.width.min(u16::MAX as u32) as u16;
            let height = info.height.min(u16::MAX as u32) as u16;

            let mut arrays: Vec<HevcNalArray> = Vec::new();
            if let Some(vps_nal) = vps.clone() {
                arrays.push(HevcNalArray::new(
                    true,
                    H265_NAL_VPS,
                    alloc::vec![HevcNalUnit::new(vps_nal)],
                ));
            }
            arrays.push(HevcNalArray::new(
                true,
                H265_NAL_SPS,
                alloc::vec![HevcNalUnit::new(sps_bytes.clone())],
            ));
            if let Some(pps_nal) = pps.clone() {
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
                // hvcC stores bit_depth_{luma,chroma}_minus8; the SPS decode
                // returns the absolute bit depth (minus8 + 8), so subtract 8
                // back out (saturating — an ES reporting < 8 would be malformed).
                bit_depth_luma_minus8: info.bit_depth_luma.saturating_sub(8),
                bit_depth_chroma_minus8: info.bit_depth_chroma.saturating_sub(8),
                avg_frame_rate: HVCC_AVG_FRAME_RATE_UNSPEC,
                constant_frame_rate: HVCC_CONSTANT_FRAME_RATE_UNSPEC,
                num_temporal_layers: HVCC_NUM_TEMPORAL_LAYERS,
                temporal_id_nested: false,
                length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
                arrays,
            };
            Some((
                CodecConfig::Hevc {
                    config: HEVCConfigurationBox::new(record),
                    width,
                    height,
                },
                VIDEO_TIMESCALE,
                LiveKind::Video {
                    pending: None,
                    last_duration: 0,
                    codec: VideoCodec::Hevc,
                },
            ))
        }
        ConfigProbe::Mpeg2Video => {
            // Geometry from the first sequence_header() seen in the stream.
            let seq = backlog
                .iter()
                .find_map(|au| Mpeg2SeqHeader::find(&au.data).ok())?;
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
            Some((
                CodecConfig::Mpeg2Video {
                    esds,
                    width: seq.width,
                    height: seq.height,
                },
                VIDEO_TIMESCALE,
                LiveKind::Video {
                    pending: None,
                    last_duration: 0,
                    codec: VideoCodec::Mpeg2,
                },
            ))
        }
        ConfigProbe::MpegAudio { is_mpeg2 } => {
            let first = backlog
                .iter()
                .find_map(|au| MpegAudioFrameHeader::parse(&au.data).ok())?;
            let sample_rate = first.sample_rate;
            let channel_count = first.channels;
            let samples_per_frame = first.samples_per_frame;
            let oti = if *is_mpeg2 {
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
            Some((
                CodecConfig::MpegAudio {
                    esds,
                    layer: first.layer,
                    channel_count,
                    sample_rate,
                    sample_size: AUDIO_SAMPLE_SIZE_BITS,
                },
                sample_rate,
                LiveKind::Audio {
                    sample_rate,
                    kind: AudioKind::MpegAudio { samples_per_frame },
                },
            ))
        }
        ConfigProbe::Aac => {
            let first_hdr = backlog
                .iter()
                .find_map(|au| parse_adts_header(&au.data).ok())?;
            let asc = AudioSpecificConfig::from_adts_header(&first_hdr);
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
                    decoder_specific_info: Some(DecoderSpecificInfo {
                        data: asc.to_bytes(),
                    }),
                }),
                sl_config: Some(SLConfigDescriptor {
                    body: alloc::vec![SL_CONFIG_PREDEFINED_MP4],
                }),
            });
            Some((
                CodecConfig::Aac {
                    esds,
                    channel_count,
                    sample_rate,
                    sample_size: AUDIO_SAMPLE_SIZE_BITS,
                },
                sample_rate,
                LiveKind::Audio {
                    sample_rate,
                    kind: AudioKind::Aac,
                },
            ))
        }
        ConfigProbe::Ac3 => {
            let info = backlog
                .iter()
                .find_map(|au| Ac3SyncframeInfo::from_es(&au.data).ok())?;
            let sample_rate = info.sample_rate;
            let channel_count = info.channel_count() as u16;
            let config = info.into_dac3();
            Some((
                CodecConfig::Ac3 {
                    config,
                    channel_count,
                    sample_rate,
                    sample_size: AUDIO_SAMPLE_SIZE_BITS,
                },
                sample_rate,
                LiveKind::Audio {
                    sample_rate,
                    kind: AudioKind::Ac3,
                },
            ))
        }
        ConfigProbe::Eac3 => {
            let info = backlog
                .iter()
                .find_map(|au| Ec3SyncframeInfo::from_es(&au.data).ok())?;
            let sample_rate = info.sample_rate;
            let channel_count = info.channel_count() as u16;
            let config = info.into_dec3();
            Some((
                CodecConfig::Eac3 {
                    config,
                    channel_count,
                    sample_rate,
                    sample_size: AUDIO_SAMPLE_SIZE_BITS,
                },
                sample_rate,
                LiveKind::Audio {
                    sample_rate,
                    kind: AudioKind::Eac3,
                },
            ))
        }
    }
}

/// Advance a [`StreamState`]'s track lifecycle by one access unit: apply it
/// directly if already live, append it to the backlog if parked, or feed the
/// probe (transitioning `Probing` → `Parked` the moment config becomes
/// recoverable) otherwise. Never assigns a track ID or emits
/// [`DemuxEvent::TrackAdded`] itself — that is
/// [`StreamingTsDemux::try_promote_ready`]'s job, since a `Parked` track must
/// still wait for its PMT-declaration-order turn.
fn advance_track(
    stream: &mut StreamState,
    data: Vec<u8>,
    pts_uw: i128,
    dts_uw: i128,
    events: &mut VecDeque<DemuxEvent>,
) {
    let track = stream
        .track
        .take()
        .expect("StreamState.track is always populated outside this function");
    let new_track = match track {
        TrackState::Live(mut live) => {
            push_live_au(&mut live, &data, pts_uw, dts_uw, events);
            TrackState::Live(live)
        }
        TrackState::Parked {
            config,
            timescale,
            kind,
            mut backlog,
        } => {
            backlog.push(BufferedAu {
                data,
                pts_uw,
                dts_uw,
            });
            TrackState::Parked {
                config,
                timescale,
                kind,
                backlog,
            }
        }
        TrackState::Probing {
            mut probe,
            mut backlog,
        } => {
            backlog.push(BufferedAu {
                data,
                pts_uw,
                dts_uw,
            });
            match finalize_probe(stream.codec, &stream.descriptors, &mut probe, &backlog) {
                Some((config, timescale, kind)) => TrackState::Parked {
                    config,
                    timescale,
                    kind,
                    backlog,
                },
                None => TrackState::Probing { probe, backlog },
            }
        }
    };
    stream.track = Some(new_track);
}

/// Resolve a completed PES packet's `(pts, dts)` (mirrors the old
/// `push_access_unit` fallback rule) and drive it through [`advance_track`]
/// (parked/probing) or [`push_live_au`] (already live).
fn on_completed_pes(stream: &mut StreamState, pes_bytes: &[u8], events: &mut VecDeque<DemuxEvent>) {
    let Ok(pes) = PesPacket::parse(pes_bytes) else {
        return;
    };
    if pes.payload.is_empty() {
        return;
    }
    let fallback = if stream.has_any {
        stream.fallback
    } else {
        (0, 0)
    };
    let (pts, dts) = match pes.header.as_ref() {
        Some(h) => {
            let hp = h.pts.map(|p| p.0);
            let hd = h.dts.map(|d| d.0);
            // DTS defaults to PTS when absent; PTS defaults to DTS; else the
            // fallback above.
            let pts = hp.or(hd).unwrap_or(fallback.0);
            let dts = hd.unwrap_or(pts);
            (pts, dts)
        }
        None => fallback,
    };
    stream.fallback = (pts, dts);
    stream.has_any = true;
    let (pts_uw, dts_uw) = stream.wrap.push(pts, dts);
    if stream.first_dts_uw.is_none() {
        stream.first_dts_uw = Some(dts_uw);
    }
    advance_track(stream, pes.payload.to_vec(), pts_uw, dts_uw, events);
}

/// One incremental demux event from [`StreamingTsDemux`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum DemuxEvent {
    /// New track discovered (PAT/PMT parsed, or a PMT version change added a
    /// PID). The codec config is fully recovered by the time this fires,
    /// mirroring the old batch demuxer's per-track "skip until recoverable"
    /// gate (issue #467) — an opaque [`CodecConfig::Data`] track (issue #557)
    /// fires on its very first access unit, since its config needs no
    /// in-band header at all.
    TrackAdded(Track),
    /// Track's codec config changed after having already been added. Config
    /// recovery in this engine is single-shot and permanent (first-found
    /// wins, exactly mirroring the old batch builders), so this variant is
    /// never emitted today; it is part of the event API for a future
    /// incremental config upgrade (e.g. a mid-stream SPS change).
    TrackUpdated(Track),
    /// A completed access unit / audio frame, with per-sample
    /// [`SourceTiming`] (issue #556 semantics preserved exactly).
    Sample {
        /// The owning track's ID (matches a prior [`DemuxEvent::TrackAdded`]).
        track_id: u32,
        /// The coded sample.
        sample: Sample,
    },
    /// PCR observed in an adaptation field (27 MHz) — the same data collected
    /// into [`Media::pcr`](crate::media::Media::pcr) by the batch wrapper.
    Pcr(PcrSample),
    /// Discontinuity indicator seen on a PID's adaptation field
    /// (ISO/IEC 13818-1 §2.4.3.5), independent of whether that packet also
    /// carried a PCR.
    Discontinuity {
        /// The PID the discontinuity was observed on.
        pid: u16,
    },
}

/// Event-driven, incremental MPEG-2 Transport Stream demuxer (issue #555) —
/// the one demux core [`TsDemux`] is a thin batch wrapper over.
///
/// Feed TS bytes of any size/alignment with [`feed`](Self::feed) (backed by
/// [`mpeg_ts::resync::TsResync`], so mid-packet chunk boundaries — down to a
/// single byte at a time — and 204-byte RS-coded input are both handled
/// transparently); drain [`DemuxEvent`]s with [`poll_event`](Self::poll_event);
/// call [`finish`](Self::finish) once, at end of input, to flush trailing
/// partial access units.
///
/// # Memory
///
/// Bounded, independent of stream length: per-PID PES reassembly + PSI
/// section-reassembly state, one pending (duration-incomplete) sample per
/// live video/data track, and — until a track's codec config first becomes
/// recoverable — a small backlog of that PID's buffered access units. In real
/// broadcast streams parameter sets / frame headers appear in the first
/// access unit or two, so this backlog is tiny in practice. The one caveat:
/// a PMT-listed codec PID whose config is *never* recoverable (e.g. no SPS
/// ever arrives on that PID) holds that PID's own backlog for the life of the
/// stream — exactly mirroring the old batch demuxer, which also needed the
/// whole file to reach the same "never recoverable, skip" conclusion; it does
/// not delay or affect any other PID's event delivery.
///
/// One more source has the same shape: a captured excerpt need not start at
/// a clean PAT/PMT boundary, so a PID's own payload can arrive on the wire
/// before its PMT registration has finished reassembling (observed in a
/// committed real DVB capture). Those payloads are held in `unattributed`
/// (keyed by PID) and replayed the instant that PID's PMT entry resolves —
/// restoring the full-file view the old two-pass batch demuxer had "for
/// free". A PID that never appears in any PMT (e.g. an unrelated service's
/// traffic in a full-multiplex capture) is FIFO-evicted once the total
/// buffered size exceeds a fixed byte cap (`MAX_UNATTRIBUTED_BYTES`), keeping
/// this buffer bounded regardless of stream length; null packets (PID `0x1FFF`)
/// are excluded from it entirely.
///
/// Track IDs / `TrackAdded` order follow PMT declaration order (codec tracks
/// first, then data tracks, each group in PMT order — the old batch
/// demuxer's invariant, see `TrackState`), tracked via `codec_order` /
/// `data_order` / `resolved`; these hold one `u16` PID per known ES, not
/// per-sample data, so they stay tiny regardless of stream length.
pub struct StreamingTsDemux {
    resync: TsResync,
    packet_index: u64,
    pat_reasm: SectionReassembler,
    pmt_reasm: BTreeMap<u16, SectionReassembler>,
    es_seen: BTreeSet<u16>,
    streams: BTreeMap<u16, StreamState>,
    /// Payloads for a PID not yet classified as PAT/PMT/a known ES — a real
    /// capture excerpt need not start at a clean PAT/PMT boundary, so an ES's
    /// own packets can arrive before its PMT registration completes (see the
    /// module-level `# Memory` note). Replayed into the new [`StreamState`]
    /// the moment that PID is discovered in a PMT, restoring the same
    /// full-file view the old two-pass batch demuxer had for free. FIFO-bounded
    /// by [`MAX_UNATTRIBUTED_BYTES`] (see `unattributed_order` /
    /// `unattributed_bytes`).
    unattributed: BTreeMap<u16, VecDeque<(bool, Vec<u8>)>>,
    /// One entry per buffered `unattributed` payload, in insertion order — the
    /// FIFO eviction queue backing [`MAX_UNATTRIBUTED_BYTES`]. Stale entries
    /// (for a PID already replayed into `streams`) are skipped harmlessly when
    /// popped.
    unattributed_order: VecDeque<u16>,
    /// Running total of bytes held in `unattributed`, kept in sync on push,
    /// eviction, and replay to enforce [`MAX_UNATTRIBUTED_BYTES`].
    unattributed_bytes: usize,
    /// Codec-track PIDs, in PMT discovery order.
    codec_order: Vec<u16>,
    /// Data-track (opaque PES, issue #557) PIDs, in PMT discovery order.
    data_order: Vec<u16>,
    /// PIDs that have reached a final disposition: promoted to `Live` (a
    /// track_id assigned and `TrackAdded` fired) or abandoned (config never
    /// recoverable / no access units ever arrived, concluded at `finish()`).
    resolved: BTreeSet<u16>,
    next_track_id: u32,
    events: VecDeque<DemuxEvent>,
}

impl Default for StreamingTsDemux {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingTsDemux {
    /// Create a new streaming demuxer with empty state.
    pub fn new() -> Self {
        Self {
            resync: TsResync::new(),
            packet_index: 0,
            pat_reasm: SectionReassembler::default(),
            pmt_reasm: BTreeMap::new(),
            es_seen: BTreeSet::new(),
            streams: BTreeMap::new(),
            unattributed: BTreeMap::new(),
            unattributed_order: VecDeque::new(),
            unattributed_bytes: 0,
            codec_order: Vec::new(),
            data_order: Vec::new(),
            resolved: BTreeSet::new(),
            next_track_id: 1,
            events: VecDeque::new(),
        }
    }

    /// Feed `data` — any size, any alignment (mid-packet chunk boundaries are
    /// legal, including one byte at a time). Internally resynchronises to
    /// `0x47` TS packet boundaries via [`mpeg_ts::resync::TsResync`] and
    /// processes every newly-aligned packet.
    pub fn feed(&mut self, data: &[u8]) {
        let packets = self.resync.feed(data);
        for raw in &packets {
            self.process_packet(raw);
        }
    }

    fn process_packet(&mut self, raw: &[u8; TS_PACKET_SIZE]) {
        let idx = self.packet_index;
        self.packet_index += 1;
        let Ok(pkt) = TsPacket::parse(raw) else {
            return;
        };

        // PCR / discontinuity — independent of PID classification, matches
        // every packet's adaptation field regardless of payload routing.
        if let Some(Ok(af)) = pkt.adaptation_field() {
            if af.discontinuity_indicator {
                self.events.push_back(DemuxEvent::Discontinuity {
                    pid: pkt.header.pid,
                });
            }
            if let Some(pcr) = af.pcr {
                self.events.push_back(DemuxEvent::Pcr(PcrSample {
                    pcr_27mhz: pcr.as_27mhz(),
                    pid: pkt.header.pid,
                    packet_index: idx,
                    discontinuity: af.discontinuity_indicator,
                }));
            }
        }

        let pid = pkt.header.pid;
        let pusi = pkt.header.pusi;
        let Some(payload) = pkt.payload else {
            return;
        };

        if pid == PAT_PID {
            self.pat_reasm.feed(payload, pusi);
            while let Some(section) = self.pat_reasm.pop_section() {
                if let Ok(pmt_pids) = parse_pat(&section) {
                    for pmt_pid in pmt_pids {
                        self.pmt_reasm.entry(pmt_pid).or_default();
                    }
                }
            }
            return;
        }

        if let Some(reasm) = self.pmt_reasm.get_mut(&pid) {
            reasm.feed(payload, pusi);
            let mut newly = Vec::new();
            while let Some(section) = reasm.pop_section() {
                if let Ok(es_list) = parse_pmt(&section) {
                    newly.extend(es_list);
                }
            }
            for (es_pid, codec, descriptors) in newly {
                if self.es_seen.insert(es_pid) {
                    if matches!(codec, Codec::Data(_)) {
                        self.data_order.push(es_pid);
                    } else {
                        self.codec_order.push(es_pid);
                    }
                    // DTS is statically known to never resolve a track (issue
                    // #467) — mark it resolved immediately so it never blocks
                    // a later-ranked PID's promotion.
                    if codec == Codec::Dts {
                        self.resolved.insert(es_pid);
                    }
                    let mut stream = StreamState {
                        codec,
                        descriptors,
                        assembler: PesAssembler::new(),
                        fallback: (0, 0),
                        has_any: false,
                        wrap: WrapState::default(),
                        first_dts_uw: None,
                        track: Some(TrackState::Probing {
                            probe: initial_probe(codec),
                            backlog: Vec::new(),
                        }),
                    };
                    // Replay any payloads that arrived on this PID before its
                    // PMT registration completed (see `unattributed`'s doc).
                    if let Some(buffered) = self.unattributed.remove(&es_pid) {
                        for (buf_pusi, buf_payload) in buffered {
                            self.unattributed_bytes =
                                self.unattributed_bytes.saturating_sub(buf_payload.len());
                            if let Some(completed) = stream.assembler.feed(buf_pusi, &buf_payload) {
                                on_completed_pes(&mut stream, &completed, &mut self.events);
                            }
                        }
                    }
                    self.streams.insert(es_pid, stream);
                }
            }
            self.try_promote_ready();
            return;
        }

        if let Some(stream) = self.streams.get_mut(&pid) {
            if let Some(completed) = stream.assembler.feed(pusi, payload) {
                on_completed_pes(stream, &completed, &mut self.events);
            }
        } else if pid != NULL_PACKET_PID {
            self.unattributed
                .entry(pid)
                .or_default()
                .push_back((pusi, payload.to_vec()));
            self.unattributed_order.push_back(pid);
            self.unattributed_bytes += payload.len();
            self.evict_unattributed();
        }
        self.try_promote_ready();
    }

    /// Enforce [`MAX_UNATTRIBUTED_BYTES`] by FIFO-evicting the oldest buffered
    /// `unattributed` payloads. Order entries whose PID has already been
    /// replayed into `streams` (and thus removed from the map) are stale and
    /// skipped without touching the byte counter.
    fn evict_unattributed(&mut self) {
        while self.unattributed_bytes > MAX_UNATTRIBUTED_BYTES {
            let Some(pid) = self.unattributed_order.pop_front() else {
                break;
            };
            if let Some(buf) = self.unattributed.get_mut(&pid) {
                if let Some((_, payload)) = buf.pop_front() {
                    self.unattributed_bytes = self.unattributed_bytes.saturating_sub(payload.len());
                }
                if buf.is_empty() {
                    self.unattributed.remove(&pid);
                }
            }
        }
    }

    /// Promote every `Parked` PID that has reached its PMT-declaration-order
    /// turn to `Live`: assign the next sequential track ID, emit
    /// `DemuxEvent::TrackAdded`, and replay its accumulated backlog as a
    /// burst of `DemuxEvent::Sample`s — repeating while the *next*-ranked PID
    /// is also already `Parked`. Stops at the first PID that is still
    /// `Probing` (blocked) or not yet known at all.
    fn try_promote_ready(&mut self) {
        loop {
            let Some(&next_pid) = self
                .codec_order
                .iter()
                .chain(self.data_order.iter())
                .find(|p| !self.resolved.contains(p))
            else {
                return;
            };
            let Some(stream) = self.streams.get_mut(&next_pid) else {
                return;
            };
            let track = stream
                .track
                .take()
                .expect("StreamState.track is always populated outside this function");
            match track {
                TrackState::Parked {
                    config,
                    timescale,
                    kind,
                    backlog,
                } => {
                    let track_id = self.next_track_id;
                    self.next_track_id += 1;
                    let anchor = stream.first_dts_uw.unwrap_or(0).max(0) as u64;
                    self.events.push_back(DemuxEvent::TrackAdded(Track::new_at(
                        TrackSpec {
                            track_id,
                            timescale,
                            config,
                        },
                        Vec::new(),
                        anchor,
                    )));
                    let mut live = LiveTrack { track_id, kind };
                    for au in backlog {
                        push_live_au(&mut live, &au.data, au.pts_uw, au.dts_uw, &mut self.events);
                    }
                    stream.track = Some(TrackState::Live(live));
                    self.resolved.insert(next_pid);
                    // loop again: the next-ranked PID may also already be parked
                }
                other @ TrackState::Probing { .. } => {
                    stream.track = Some(other);
                    return; // blocked — an earlier-ranked PID isn't ready yet
                }
                other @ TrackState::Live(_) => {
                    // Already resolved; `resolved` should already contain it,
                    // but stay consistent defensively and keep scanning.
                    stream.track = Some(other);
                    self.resolved.insert(next_pid);
                }
            }
        }
    }

    /// Drain the next pending event, if any (FIFO).
    pub fn poll_event(&mut self) -> Option<DemuxEvent> {
        self.events.pop_front()
    }

    /// Flush trailing partial access units (no more input coming): completes
    /// every PID's buffered PES payload, definitively abandons any PID whose
    /// config never became recoverable (unblocking later-ranked `Parked`
    /// PIDs — mirrors the old batch demuxer's own "never resolved, skip"
    /// conclusion, which likewise needed the whole file), and emits the
    /// final one-behind pending sample for every live video/data track.
    pub fn finish(&mut self) {
        for stream in self.streams.values_mut() {
            if let Some(completed) = stream.assembler.flush() {
                on_completed_pes(stream, &completed, &mut self.events);
            }
        }
        self.try_promote_ready();

        loop {
            let Some(&next_pid) = self
                .codec_order
                .iter()
                .chain(self.data_order.iter())
                .find(|p| !self.resolved.contains(p))
            else {
                break;
            };
            match self.streams.get(&next_pid).and_then(|s| s.track.as_ref()) {
                Some(TrackState::Probing { .. }) => {
                    self.resolved.insert(next_pid);
                    self.try_promote_ready();
                }
                _ => break,
            }
        }

        for stream in self.streams.values_mut() {
            if let Some(TrackState::Live(live)) = &mut stream.track {
                match &mut live.kind {
                    LiveKind::Video {
                        pending,
                        last_duration,
                        ..
                    } => {
                        flush_one_behind(pending, *last_duration, live.track_id, &mut self.events);
                    }
                    LiveKind::Data {
                        pending,
                        last_duration,
                    } => {
                        flush_one_behind(pending, *last_duration, live.track_id, &mut self.events);
                    }
                    LiveKind::Audio { .. } => {}
                }
            }
        }
    }
}

// ── Batch wrapper ────────────────────────────────────────────────────────────

/// Demux an MPEG-2 Transport Stream byte slice into a [`Media`].
///
/// A thin wrapper over [`StreamingTsDemux`] (issue #555): follows the PAT to
/// every PMT, enumerates each program's elementary streams into IR [`Track`]s,
/// reassembles per-PID PES into access units with PTS/DTS, recovers codec
/// config from the in-band headers, and emits length-prefixed video / raw
/// audio samples in decode order — by feeding the whole input to a
/// [`StreamingTsDemux`], calling `finish()`, and folding the resulting
/// [`DemuxEvent`]s into a [`Media`].
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
        let mut demux = StreamingTsDemux::new();
        demux.feed(input);
        demux.finish();

        let mut tracks: Vec<Track> = Vec::new();
        let mut index_by_id: BTreeMap<u32, usize> = BTreeMap::new();
        let mut pcr: Vec<PcrSample> = Vec::new();
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(track) => {
                    index_by_id.insert(track.spec.track_id, tracks.len());
                    tracks.push(track);
                }
                DemuxEvent::TrackUpdated(track) => {
                    if let Some(&i) = index_by_id.get(&track.spec.track_id) {
                        let samples = core::mem::take(&mut tracks[i].samples);
                        tracks[i] = track;
                        tracks[i].samples = samples;
                    }
                }
                DemuxEvent::Sample { track_id, sample } => {
                    if let Some(&i) = index_by_id.get(&track_id) {
                        tracks[i].samples.push(sample);
                    }
                }
                DemuxEvent::Pcr(sample) => pcr.push(sample),
                DemuxEvent::Discontinuity { .. } => {}
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes of TS payload each crafted payload-only packet contributes
    /// (188 − 4-byte TS header, adaptation_field_control = payload-only).
    const PACKET_PAYLOAD_LEN: usize = TS_PACKET_SIZE - 4;

    /// One valid payload-only TS packet on `pid` (no adaptation field), payload
    /// filled with stuffing. `cc` is the 4-bit continuity counter.
    fn payload_only_packet(pid: u16, cc: u8) -> [u8; TS_PACKET_SIZE] {
        let mut p = [0xFFu8; TS_PACKET_SIZE];
        p[0] = 0x47; // sync_byte
        p[1] = ((pid >> 8) as u8) & PID_HI_MASK; // pusi=0, priority=0, PID hi
        p[2] = (pid & 0xFF) as u8; // PID lo
        p[3] = 0x10 | (cc & 0x0F); // AFC=01 (payload only) + continuity counter
        p
    }

    /// A PID whose payload floods in but which never appears in any PAT/PMT
    /// (the full-multiplex unrelated-service case) must not grow the
    /// `unattributed` buffer without bound: it is FIFO-capped at
    /// `MAX_UNATTRIBUTED_BYTES` regardless of how much arrives.
    #[test]
    fn unattributed_buffer_is_bounded_for_never_claimed_pid() {
        // Enough packets that the raw payload total is several times the cap,
        // so eviction must have run.
        let target_bytes = MAX_UNATTRIBUTED_BYTES * 3;
        let packet_count = target_bytes / PACKET_PAYLOAD_LEN + 1;
        let unclaimed_pid: u16 = 0x0123; // never introduced via PAT/PMT

        let mut demux = StreamingTsDemux::new();
        for i in 0..packet_count {
            demux.feed(&payload_only_packet(unclaimed_pid, i as u8));
        }

        // The counter is capped …
        assert!(
            demux.unattributed_bytes <= MAX_UNATTRIBUTED_BYTES,
            "unattributed_bytes {} exceeded cap {}",
            demux.unattributed_bytes,
            MAX_UNATTRIBUTED_BYTES
        );
        // … eviction genuinely fired (we fed far more than the cap) …
        assert!(
            demux.unattributed_bytes > 0,
            "expected the never-claimed PID's payload to be buffered"
        );
        // … and the counter matches the bytes actually retained in the map
        // (accounting stays consistent through eviction).
        let actual: usize = demux
            .unattributed
            .values()
            .flat_map(|q| q.iter())
            .map(|(_, payload)| payload.len())
            .sum();
        assert_eq!(
            actual, demux.unattributed_bytes,
            "unattributed_bytes drifted from the real retained size"
        );
    }
}
