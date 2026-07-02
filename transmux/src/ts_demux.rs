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
//! ADTS → AudioSpecificConfig → `esds`, AC-3/E-AC-3 syncframe BSI →
//! `dac3`/`dec3`) → length-prefixed video / raw audio samples.
//!
//! HEVC and DTS elementary streams are recognized in the PMT but not yet carried
//! into the IR (the hub [`CodecConfig`] enum has no HEVC-video /
//! DTS-from-ES-audio variant) — such tracks are skipped, never fatal (issue #467).
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

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Serialize, Unpackage};
use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::ts::{SectionReassembler, TsPacket, TS_PACKET_SIZE};

use crate::aac_asc::{parse_adts_header, AudioSpecificConfig};
use crate::ac3::{Ac3SyncframeInfo, Ec3SyncframeInfo};
use crate::annexb::iter_annexb_nals;
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::media::{Media, Track};
use crate::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, ObjectTypeIndication,
    SLConfigDescriptor, StreamType as EsdsStreamType,
};
use crate::nalu_types::{AvcPps, AvcSps};
use crate::pipeline::{CodecConfig, Sample, TrackSpec};

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

// ── Codec-config recovery constants ─────────────────────────────────────────

/// NAL length-field width for `mdat` samples: 4-byte prefixes → `lengthSizeMinusOne = 3`.
const NAL_LENGTH_SIZE_MINUS_ONE: u8 = 3;
/// H.264 `nal_unit_type` for SPS (ISO/IEC 14496-10 Table 7-1).
const H264_NAL_SPS: u8 = 7;
/// H.264 `nal_unit_type` for PPS (Table 7-1).
const H264_NAL_PPS: u8 = 8;
/// H.264 `nal_unit_type` for a coded slice of an IDR picture (Table 7-1).
const H264_NAL_IDR: u8 = 5;
/// Mask for the H.264 5-bit `nal_unit_type` in the NAL header byte.
const H264_NAL_TYPE_MASK: u8 = 0x1F;

/// `esds` `objectTypeIndication` for MPEG-4 Audio (ISO/IEC 14496-1 Table 5).
const OTI_MPEG4_AUDIO: u8 = 0x40;
/// `esds` `streamType` for an AudioStream (ISO/IEC 14496-1 Table 6).
const STREAM_TYPE_AUDIO: u8 = 0x05;
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
    Aac,
    Ac3,
    Eac3,
    Dts,
}

impl Codec {
    /// Map a PMT `stream_type` to a supported [`Codec`], or `None` if the demuxer
    /// does not carry it (skipped, never fatal — issue #467).
    fn from_stream_type(stream_type: u8) -> Option<Self> {
        match stream_type {
            STREAM_TYPE_AVC => Some(Codec::H264),
            STREAM_TYPE_HEVC => Some(Codec::Hevc),
            STREAM_TYPE_AAC_ADTS => Some(Codec::Aac),
            STREAM_TYPE_AC3 => Some(Codec::Ac3),
            STREAM_TYPE_EAC3 => Some(Codec::Eac3),
            STREAM_TYPE_DTS_82 | STREAM_TYPE_DTS_85 | STREAM_TYPE_DTS_8A => Some(Codec::Dts),
            _ => None,
        }
    }
}

/// One elementary stream discovered in the PMT, plus its reassembled access
/// units in wire (decode) order.
struct ElementaryStream {
    codec: Codec,
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
        // Ordered elementary-stream table: (pid, codec), PMT order preserved.
        let mut es_defs: Vec<(u16, Codec)> = Vec::new();
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
                    for (es_pid, codec) in parse_pmt(&section)? {
                        if es_seen.insert(es_pid) {
                            es_defs.push((es_pid, codec));
                        }
                    }
                }
            }
        }

        // ── Pass 2: per-ES PES reassembly into access units ──
        let mut streams: BTreeMap<u16, ElementaryStream> = BTreeMap::new();
        for &(pid, codec) in &es_defs {
            streams.entry(pid).or_insert_with(|| ElementaryStream {
                codec,
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
        let mut tracks: Vec<Track> = Vec::new();
        let mut track_id: u32 = 1;
        for &(pid, _) in &es_defs {
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

        Ok(Media::new(tracks, VIDEO_TIMESCALE))
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

/// Parse a PMT section, returning `(elementary_PID, codec)` for every stream
/// whose `stream_type` maps to a supported codec. ISO/IEC 13818-1 §2.4.4.8.
fn parse_pmt(section: &[u8]) -> Result<Vec<(u16, Codec)>> {
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
        if let Some(codec) = Codec::from_stream_type(stream_type) {
            out.push((es_pid, codec));
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
    let (pts, dts) = pes
        .header
        .as_ref()
        .map(|h| {
            let pts = h.pts.map(|p| p.0);
            let dts = h.dts.map(|d| d.0);
            // DTS defaults to PTS when absent; PTS defaults to DTS; else 0.
            let pts = pts.or(dts).unwrap_or(0);
            let dts = dts.unwrap_or(pts);
            (pts, dts)
        })
        .unwrap_or((0, 0));
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
        Codec::Aac => build_aac_track(es, track_id),
        Codec::Ac3 => build_ac3_track(es, track_id),
        Codec::Eac3 => build_eac3_track(es, track_id),
        // HEVC / DTS are recognized in the PMT but the hub IR
        // ([`CodecConfig`]) does not yet carry an HEVC video or a DTS-from-ES
        // audio config, so no track is built (issue #467: skip, do not fail).
        // Their in-band config recovery lands with the matching `CodecConfig`
        // variants (follow-up).
        Codec::Hevc | Codec::Dts => None,
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

/// Sort access-unit indices into decode order (ascending unwrapped DTS) and
/// return, per index, the unwrapped `(pts, dts)`. Preserves input order for
/// equal DTS (stable). Timestamps are unwrapped across the 33-bit 90 kHz wrap
/// using the stream (wire) order.
fn decode_order(units: &[AccessUnit]) -> Vec<(usize, i128, i128)> {
    if units.is_empty() {
        return Vec::new();
    }
    let mut unwrapped: Vec<(i128, i128)> = Vec::with_capacity(units.len());
    let (mut prev_dts_raw, mut prev_dts_uw) = (units[0].dts, units[0].dts as i128);
    let (mut prev_pts_raw, mut prev_pts_uw) = (units[0].pts, units[0].pts as i128);
    for (i, au) in units.iter().enumerate() {
        let (dts_uw, pts_uw) = if i == 0 {
            (units[0].dts as i128, units[0].pts as i128)
        } else {
            (
                unwrap_ts(prev_dts_uw, prev_dts_raw, au.dts),
                unwrap_ts(prev_pts_uw, prev_pts_raw, au.pts),
            )
        };
        prev_dts_raw = au.dts;
        prev_dts_uw = dts_uw;
        prev_pts_raw = au.pts;
        prev_pts_uw = pts_uw;
        unwrapped.push((pts_uw, dts_uw));
    }
    let mut order: Vec<usize> = (0..units.len()).collect();
    order.sort_by_key(|&i| unwrapped[i].1); // stable sort by unwrapped DTS
    order
        .into_iter()
        .map(|i| (i, unwrapped[i].0, unwrapped[i].1))
        .collect()
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
                H264_NAL_IDR => idr = true,
                _ => {}
            }
        }
        is_idr.push(idr);
    }
    let sps = sps?;
    let pps = pps?;
    if sps.len() < 4 {
        return None;
    }
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
        })
        .collect();

    Some(Track::new(
        TrackSpec {
            track_id,
            timescale: VIDEO_TIMESCALE,
            config: CodecConfig::Avc {
                config,
                width: 0,
                height: 0,
            },
        },
        samples,
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
    let mut samples: Vec<Sample> = Vec::new();
    for au in &es.access_units {
        for frame in split_adts_frames(&au.data) {
            if frame.len() > ADTS_HEADER_SIZE {
                samples.push(Sample::from_raw(
                    frame[ADTS_HEADER_SIZE..].to_vec(),
                    AAC_SAMPLES_PER_FRAME,
                ));
            }
        }
    }
    if samples.is_empty() {
        return None;
    }

    Some(Track::new(
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
    ))
}

/// Recover AC-3 config (syncframe BSI → `dac3`) + one raw sample per PES AU.
fn build_ac3_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    let info = es
        .access_units
        .iter()
        .find_map(|au| Ac3SyncframeInfo::from_es(&au.data).ok())?;
    let sample_rate = info.sample_rate;
    let channel_count = info.channel_count() as u16;
    let config = info.into_dac3();
    let samples: Vec<Sample> = es
        .access_units
        .iter()
        .map(|au| Sample::from_raw(au.data.clone(), 0))
        .collect();
    Some(Track::new(
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
    ))
}

/// Recover E-AC-3 config (syncframe BSI → `dec3`) + one raw sample per PES AU.
fn build_eac3_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    let info = es
        .access_units
        .iter()
        .find_map(|au| Ec3SyncframeInfo::from_es(&au.data).ok())?;
    let sample_rate = info.sample_rate;
    let channel_count = info.channel_count() as u16;
    let config = info.into_dec3();
    let samples: Vec<Sample> = es
        .access_units
        .iter()
        .map(|au| Sample::from_raw(au.data.clone(), 0))
        .collect();
    Some(Track::new(
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
