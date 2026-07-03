//! Hub [`Media`] IR → MPEG-2 Transport Stream muxer (the output TS spoke).
//!
//! `TsMux` is the **output** side of the any-to-any container hub: it consumes
//! the neutral [`Media`] IR (one [`Track`] per elementary
//! stream, coded samples in decode order) and produces a whole-packet MPEG-2 TS
//! byte stream, implementing the abstract [`broadcast_common::Package`] trait so
//! `{any} → IR → {TS}` composes with the existing
//! [`Fmp4Demux`](crate::media::Fmp4Demux) / [`TsDemux`](crate::TsDemux)
//! depackagers. It is the byte-level inverse of [`TsDemux`](crate::TsDemux):
//! a `TsMux → TsDemux` round-trip recovers an equivalent IR (same tracks, codec
//! configs, coded NAL payloads, frame counts, and per-sample timing).
//!
//! Pipeline: enumerate the IR tracks → assign a PID per elementary stream and a
//! `stream_type` per codec → emit a PAT (PID 0) + one PMT → for each sample,
//! build a PES packet (PTS always; DTS when it differs) whose payload is the
//! access unit (video: length-prefixed NAL → Annex B, prepending in-band
//! SPS/PPS/AUD only when the sample lacks them; audio: the raw frame re-wrapped
//! in ADTS) → packetize the PES into 188-byte TS packets, carrying the PCR on
//! the video (first) PID via the adaptation field. Packets are interleaved by
//! DTS across streams.
//!
//! # Spec
//!
//! - **TS packet + adaptation field**: ITU-T H.222.0 (= ISO/IEC 13818-1) §2.4.3
//!   (`docs/codec/ts-demux-13818-1.md`) — 4-byte header, `adaptation_field()`
//!   carrying `PCR` (§2.4.3.5) and stuffing (§2.4.3.4).
//! - **PES header**: ISO/IEC 13818-1 §2.4.3.6 / §2.4.3.7 — `packet_start_code`
//!   `00 00 01`, `stream_id`, `PES_packet_length`, PTS/DTS (33-bit @ 90 kHz).
//! - **PAT / PMT program-specific information**: ISO/IEC 13818-1 §2.4.4.3 /
//!   §2.4.4.8 — long-form sections with a trailing `CRC_32`
//!   ([`broadcast_common::crc32_mpeg2`]).
//! - **stream_type → codec**: ISO/IEC 13818-1 Table 2-34 + ETSI TS 101 154 §G
//!   (AC-3 / E-AC-3 / DTS user-private assignments) — mirrors [`TsDemux`](crate::TsDemux).

use alloc::vec::Vec;

use broadcast_common::{Package, Parse, crc32_mpeg2};
use mpeg_pes::{Pts as PesPts, StreamId};
use mpeg_ts::mux::SectionPacketizer;
use mpeg_ts::ts::{Pcr, TS_PACKET_SIZE, TsHeader};

use crate::aac_asc::AudioSpecificConfig;
use crate::annexb::{iter_length_prefixed_nals, length_prefixed_to_annexb};
use crate::error::{Error, Result};
use crate::media::{Media, Track};
use crate::pipeline::{CodecConfig, DataCarriage, Sample};

// ── PID / PSI constants (ISO/IEC 13818-1 §2.4.4) ────────────────────────────

/// PID carrying the Program Association Table (§2.4.4.3).
const PAT_PID: u16 = 0x0000;
/// PID chosen for the single Program Map Table this muxer emits.
const PMT_PID: u16 = 0x1000;
/// First elementary-stream PID; each subsequent ES gets the next value.
const ES_PID_BASE: u16 = 0x0100;
/// `program_number` assigned to the single program.
const PROGRAM_NUMBER: u16 = 1;
/// `table_id` of a PAT section (§2.4.4.3, Table 2-31).
const TABLE_ID_PAT: u8 = 0x00;
/// `table_id` of a PMT section (§2.4.4.8, Table 2-31).
const TABLE_ID_PMT: u8 = 0x02;
/// Trailing `CRC_32` length on every long-form PSI section (§2.4.4.1).
const CRC32_LEN: usize = 4;
/// `section_syntax_indicator`(1)=1 | private(1)=0 | reserved(2)=11 → 0xB0,
/// combined into the high byte of the 2-byte flags/`section_length` field.
const SECTION_SYNTAX_FLAGS_HI: u8 = 0xB0;
/// Mask for the low 4 bits of the 12-bit `section_length` high byte.
const SECTION_LENGTH_HI_MASK: u8 = 0x0F;
/// `version_number`(5)=0 | `current_next_indicator`(1)=1, with the two leading
/// reserved bits set to 1 (`11` per the spec reserved convention) → 0xC1.
const VERSION_CURRENT_NEXT: u8 = 0xC1;
/// Reserved 3-bit prefix (all 1s) on the 13-bit `network_PID` / `program_map_PID`
/// / `PCR_PID` / `elementary_PID` fields (§2.4.4.3 / §2.4.4.8).
const PID_RESERVED_HI: u8 = 0xE0;
/// Reserved 4-bit prefix (all 1s) on the 12-bit `program_info_length` /
/// `ES_info_length` fields (§2.4.4.8) — combined into their high byte.
const INFO_RESERVED_HI: u8 = 0xF0;

// ── stream_type → codec (ISO/IEC 13818-1 Table 2-34 + ETSI TS 101 154) ──────

/// AVC (H.264) video — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_AVC: u8 = 0x1B;
/// ISO/IEC 13818-7 AAC in ADTS — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_AAC_ADTS: u8 = 0x0F;
/// AC-3 (ATSC/DVB user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_AC3: u8 = 0x81;
/// E-AC-3 (user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_EAC3: u8 = 0x87;
/// DTS (canonical DVB assignment, user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_DTS: u8 = 0x82;

/// Maximum value of the 12-bit `ES_info_length` field (§2.4.4.8).
const MAX_ES_INFO_LENGTH: usize = 0x0FFF;

// ── PES / stream_id constants (ISO/IEC 13818-1 §2.4.3.6, Table 2-22) ────────

/// Base `stream_id` for video elementary streams (`1110 xxxx`, 0xE0–0xEF).
const STREAM_ID_VIDEO_BASE: u8 = 0xE0;
/// Base `stream_id` for audio elementary streams (`110x xxxx`, 0xC0–0xDF).
const STREAM_ID_AUDIO_BASE: u8 = 0xC0;
/// `private_stream_1` — the default `stream_id` for a PES-carried opaque
/// [`CodecConfig::Data`] elementary stream (issue #576), Table 2-22.
const STREAM_ID_PRIVATE_1: u8 = 0xBD;
/// PES `packet_start_code_prefix` (§2.4.3.6).
const PES_START_CODE: [u8; 3] = [0x00, 0x00, 0x01];
/// Fixed bytes preceding the PES optional-header payload: 3 (marker/flags(1) +
/// PTS_DTS flags(1) + PES_header_data_length(1)). ISO/IEC 13818-1 §2.4.3.7.
const HEADER_FIXED: usize = 3;
/// Bytes before the optional header: start code(3) + stream_id(1) + length(2).
const MIN_LEN: usize = 6;
/// PES optional-header first byte: `10` marker in bits `[7:6]`, all other flag
/// bits (scrambling/priority/alignment/copyright/original) 0 → 0x80.
const PES_OPTIONAL_MARKER: u8 = 0x80;
/// PTS_DTS_flags byte with `PTS_DTS_flags == 10` (PTS only) in bits `[7:6]`.
const PTS_DTS_FLAGS_PTS_ONLY: u8 = 0x80;
/// PTS_DTS_flags byte with `PTS_DTS_flags == 11` (PTS + DTS) in bits `[7:6]`.
const PTS_DTS_FLAGS_BOTH: u8 = 0xC0;
/// 4-bit prefix on the PTS field of a PTS+DTS pair (`0011`). §2.4.3.7.
const TS_PREFIX_PTS_WITH_DTS: u8 = 0b0011;
/// 4-bit prefix on the DTS field of a PTS+DTS pair (`0001`). §2.4.3.7.
const TS_PREFIX_DTS: u8 = 0b0001;
/// 33-bit mask for a PTS/DTS value.
const TS_VALUE_MASK: u64 = TS_TIMESTAMP_MOD - 1;

// ── H.264 NAL constants (ISO/IEC 14496-10 Table 7-1) ────────────────────────

/// Mask for the 5-bit `nal_unit_type` in the NAL header byte.
const H264_NAL_TYPE_MASK: u8 = 0x1F;
/// `nal_unit_type` for an Access Unit Delimiter.
const H264_NAL_AUD: u8 = 9;
/// `nal_unit_type` for a Sequence Parameter Set.
const H264_NAL_SPS: u8 = 7;

// ── TS adaptation-field constants (ISO/IEC 13818-1 §2.4.3.4/§2.4.3.5) ───────

/// `adaptation_field_control` bit: adaptation field present.
const AF_CTRL_ADAPTATION: u8 = 0x20;
/// `adaptation_field_control` bit: payload present.
const AF_CTRL_PAYLOAD: u8 = 0x10;
/// Adaptation-field flag: `PCR_flag`.
const AF_PCR_FLAG: u8 = 0x10;
/// Encoded PCR occupies 6 bytes (§2.4.3.5).
const PCR_FIELD_LEN: usize = 6;
/// Stuffing byte for unused TS/PES payload bytes (§2.4.4).
const STUFFING_BYTE: u8 = 0xFF;

/// Media timescale of a Transport Stream / PES clock (90 kHz).
const TS_CLOCK_HZ: u64 = 90_000;
/// 33-bit PTS/DTS modulus (90 kHz clock, §2.4.3.7).
const TS_TIMESTAMP_MOD: u64 = 1 << 33;
/// PCR lead time ahead of the first DTS, ~100 ms of 90 kHz ticks — keeps the PCR
/// slightly ahead of the earliest presentation so a decoder's STC is primed.
const PCR_LEAD_TICKS: u64 = 9_000;

/// Elementary-stream class recovered from a track's [`CodecConfig`], selecting
/// the `stream_type`, PES `stream_id` family, and per-sample payload framing.
/// Data-carrying dispatch discriminant (not a spec label enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EsKind {
    /// H.264/AVC video.
    Avc,
    /// AAC audio (re-wrapped in ADTS).
    Aac,
    /// AC-3 audio.
    Ac3,
    /// E-AC-3 audio.
    Eac3,
    /// DTS audio (core substream, passed through verbatim).
    Dts,
    /// Opaque data (issue #576): the preserved PMT `stream_type` +
    /// [`DataCarriage`] of a [`CodecConfig::Data`] track.
    Data {
        /// PMT `stream_type` (ISO/IEC 13818-1 Table 2-34), carried verbatim.
        stream_type: u8,
        /// PES- or section-carried — selects how samples are re-emitted.
        carriage: DataCarriage,
    },
}

impl EsKind {
    /// The PMT `stream_type` for this elementary stream.
    fn stream_type(self) -> u8 {
        match self {
            EsKind::Avc => STREAM_TYPE_AVC,
            EsKind::Aac => STREAM_TYPE_AAC_ADTS,
            EsKind::Ac3 => STREAM_TYPE_AC3,
            EsKind::Eac3 => STREAM_TYPE_EAC3,
            EsKind::Dts => STREAM_TYPE_DTS,
            EsKind::Data { stream_type, .. } => stream_type,
        }
    }

    /// Whether this is a video stream (drives PES `stream_id` family + PCR PID).
    fn is_video(self) -> bool {
        matches!(self, EsKind::Avc)
    }

    /// Whether this elementary stream is re-emitted as PSI/private sections
    /// rather than PES (issue #576) — see [`DataCarriage::Sections`].
    fn is_section_carried(self) -> bool {
        matches!(
            self,
            EsKind::Data {
                carriage: DataCarriage::Sections,
                ..
            }
        )
    }

    /// Classify a track's [`CodecConfig`] into the elementary-stream kind the
    /// TS muxer emits it as. Every [`CodecConfig`] this crate knows is
    /// carriable in a TS: decoded codecs get their canonical `stream_type`,
    /// and [`CodecConfig::Data`] carries its preserved `stream_type` +
    /// `carriage` straight through (issue #576) — only a config this muxer
    /// has never heard of (there is none today) would return `None`.
    fn from_config(config: &CodecConfig) -> Option<Self> {
        match config {
            CodecConfig::Avc { .. } => Some(EsKind::Avc),
            CodecConfig::Aac { .. } => Some(EsKind::Aac),
            CodecConfig::Ac3 { .. } => Some(EsKind::Ac3),
            CodecConfig::Eac3 { .. } => Some(EsKind::Eac3),
            CodecConfig::Dts { .. } => Some(EsKind::Dts),
            CodecConfig::Data {
                stream_type,
                carriage,
                ..
            } => Some(EsKind::Data {
                stream_type: *stream_type,
                carriage: *carriage,
            }),
            _ => None,
        }
    }
}

/// One elementary stream to emit: its PID, `stream_id`, kind, and codec-derived
/// framing state (AAC ADTS template / AVC parameter sets).
pub(crate) struct EsPlan {
    pid: u16,
    stream_id: StreamId,
    kind: EsKind,
    /// AAC AudioSpecificConfig (for re-wrapping raw frames in ADTS), else `None`.
    asc: Option<AudioSpecificConfig>,
    /// AVC SPS + PPS NALs (from `avcC`), prepended to a keyframe access unit that
    /// lacks them so every TS video AU is independently decodable. Empty for
    /// non-AVC streams.
    avc_sps_pps: Vec<Vec<u8>>,
    /// Preserved PMT ES_info descriptor-loop bytes for this stream (issue
    /// #576) — non-empty only for an [`EsKind::Data`] track sourced from a
    /// [`CodecConfig::Data`] (the IR carries no descriptors for decoded
    /// codecs today, so their ES_info loop stays empty).
    descriptors: Vec<u8>,
}

/// A single TS packet queued for output, tagged with a monotonic (never
/// 33-bit-wrapped — see [`rescale_for_ordering`]) decode-order key so the
/// muxer can interleave elementary streams by decode time.
struct TaggedPacket {
    sort_key: u64,
    packet: [u8; TS_PACKET_SIZE],
}

/// Mux a hub [`Media`] IR into an MPEG-2 Transport Stream byte stream.
///
/// A single program (PAT PID `0x0000` → PMT PID `0x1000`) enumerates every
/// carriable track as an elementary stream (PID `0x0100+`); the PCR rides the
/// first video PID (or, absent video, the first track). Each sample becomes one
/// PES packet, packetized into 188-byte TS packets, and all packets are
/// interleaved in ascending decode-time order.
///
/// Construct with [`TsMux::new`] or [`TsMux::default`].
#[derive(Debug, Default, Clone)]
pub struct TsMux {
    _private: (),
}

impl TsMux {
    /// Create a new TS muxer.
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Package for TsMux {
    type Media = Media;
    type Output = Vec<u8>;
    type Error = Error;

    fn package(&mut self, media: &Media) -> Result<Vec<u8>> {
        if media.tracks.is_empty() {
            return Err(Error::InvalidInput("cannot package a Media with no tracks"));
        }
        // Mux every track over its full sample list — one PAT/PMT then the
        // DTS-interleaved PES for all samples.
        let samples: Vec<&[Sample]> = media.tracks.iter().map(|t| t.samples.as_slice()).collect();
        mux_tracks(&media.tracks, &samples)
    }
}

/// Plan the carriable elementary streams of `tracks` (PID + `stream_type` +
/// per-codec framing state), skipping tracks whose codec the TS layer cannot
/// carry. Shared by [`TsMux`] and the classic-HLS segmenter
/// ([`crate::ts_hls::TsHlsPackager`]) so both assign identical PIDs / PSI.
///
/// Returns the plans in track order plus the parallel indices of the planned
/// tracks within `tracks` (so a caller can select the matching sample slices).
pub(crate) fn plan_elementary_streams(tracks: &[Track]) -> Result<(Vec<EsPlan>, Vec<usize>)> {
    let mut plans: Vec<EsPlan> = Vec::new();
    let mut planned_idx: Vec<usize> = Vec::new();
    let mut next_pid = ES_PID_BASE;
    let (mut n_video, mut n_audio) = (0u8, 0u8);
    for (idx, track) in tracks.iter().enumerate() {
        let Some(kind) = EsKind::from_config(&track.spec.config) else {
            continue; // uncarriable codec: skip, never fatal.
        };
        // `stream_id` families (ISO/IEC 13818-1 Table 2-22): video and audio
        // get the next sequential ID in their `0xEx`/`0xCx` family; an opaque
        // PES-carried Data stream always gets the fixed `private_stream_1`
        // (issue #576); a section-carried Data stream emits no PES at all, so
        // its `stream_id` is never serialized (value irrelevant).
        let stream_id = match kind {
            EsKind::Avc => {
                let id = StreamId(STREAM_ID_VIDEO_BASE + n_video);
                n_video += 1;
                id
            }
            EsKind::Data {
                carriage: DataCarriage::Pes,
                ..
            } => StreamId(STREAM_ID_PRIVATE_1),
            EsKind::Data {
                carriage: DataCarriage::Sections,
                ..
            } => StreamId(0),
            _ => {
                let id = StreamId(STREAM_ID_AUDIO_BASE + n_audio);
                n_audio += 1;
                id
            }
        };
        let asc = match &track.spec.config {
            CodecConfig::Aac { esds, .. } => Some(asc_from_esds(esds)?),
            _ => None,
        };
        let avc_sps_pps = match &track.spec.config {
            CodecConfig::Avc { config, .. } => {
                let r = &config.config;
                let mut sets = Vec::new();
                for sps in &r.sps {
                    sets.push(sps.0.clone());
                }
                for pps in &r.pps {
                    sets.push(pps.0.clone());
                }
                sets
            }
            _ => Vec::new(),
        };
        let descriptors = match &track.spec.config {
            CodecConfig::Data { descriptors, .. } => descriptors.clone(),
            _ => Vec::new(),
        };
        plans.push(EsPlan {
            pid: next_pid,
            stream_id,
            kind,
            asc,
            avc_sps_pps,
            descriptors,
        });
        planned_idx.push(idx);
        next_pid += 1;
    }
    if plans.is_empty() {
        return Err(Error::InvalidInput(
            "no track carries a TS-representable codec (AVC/AAC/AC-3/E-AC-3/DTS/Data)",
        ));
    }
    Ok((plans, planned_idx))
}

/// Mux `tracks` into one self-contained MPEG-2 TS byte stream: a leading
/// PAT (PID `0x0000`) + PMT, then the DTS-interleaved PES packets for the
/// per-track `samples` (`samples[i]` is the sample slice for `tracks[i]`).
///
/// The PSI is (re)emitted at the start of every call, so each invocation yields
/// an independently decodable stream — this is what lets the classic-HLS
/// segmenter build one call per `.ts` segment, each opening with PAT/PMT
/// (ISO/IEC 13818-1 §2.4.4 PSI repetition). `TsMux` calls it once over the whole
/// input with a zero base DTS.
pub(crate) fn mux_tracks(tracks: &[Track], samples: &[&[Sample]]) -> Result<Vec<u8>> {
    let zero = alloc::vec![0u64; tracks.len()];
    mux_tracks_at(tracks, samples, &zero)
}

/// Like [`mux_tracks`], but each track's first sample is stamped at decode time
/// `base_dts_ticks[track_idx]` (in that track's own timescale) instead of 0.
///
/// The classic-HLS segmenter uses this so each segment's PES timestamps continue
/// the previous segment's timeline: concatenating the segments then yields one
/// monotonically increasing DTS/PTS timeline, so a demuxer recovers each sample's
/// original duration (DTS delta) — including across segment boundaries — instead
/// of seeing the clock reset to 0 at each segment.
pub(crate) fn mux_tracks_at(
    tracks: &[Track],
    samples: &[&[Sample]],
    base_dts_ticks: &[u64],
) -> Result<Vec<u8>> {
    debug_assert_eq!(tracks.len(), samples.len());
    debug_assert_eq!(tracks.len(), base_dts_ticks.len());

    // ── 1. Plan the elementary streams (PID + stream_type + framing) ──
    let (plans, planned_idx) = plan_elementary_streams(tracks)?;

    // PCR PID: the first video ES, else the first non-section-carried ES (a
    // section-carried Data stream is packetized without an adaptation field
    // at all — issue #576 — so it can never itself carry the PCR), else
    // (only if every ES is section-carried) the first ES regardless.
    let pcr_pid = plans
        .iter()
        .find(|p| p.kind.is_video())
        .or_else(|| plans.iter().find(|p| !p.kind.is_section_carried()))
        .map(|p| p.pid)
        .unwrap_or(plans[0].pid);

    // ── 2. Build the PSI (PAT + PMT) and packetize it first (PUSI order) ──
    let mut out: Vec<u8> = Vec::new();
    let pat = build_pat_section(PMT_PID);
    for pkt in packetize_section(PAT_PID, &pat) {
        out.extend_from_slice(&pkt);
    }
    let pmt = build_pmt_section(pcr_pid, &plans);
    for pkt in packetize_section(PMT_PID, &pmt) {
        out.extend_from_slice(&pkt);
    }

    // ── 3. Elementary-stream PES → TS packets, tagged by DTS ──
    // Base DTS = PCR_LEAD_TICKS so the first PCR (DTS − lead) is non-negative.
    let mut tagged: Vec<TaggedPacket> = Vec::new();
    for (plan, &track_idx) in plans.iter().zip(&planned_idx) {
        let track = &tracks[track_idx];
        let ts_scale = track.spec.timescale.max(1) as u64;
        // in the track's own timescale, seeded from the caller's base DTS.
        let mut dts_ticks_local: u64 = base_dts_ticks[track_idx];
        let mut cc: u8 = 0;
        // Section-carried Data samples are already whole PSI/private
        // sections (issue #576) — packetized directly, never PES-wrapped.
        // The packetizer's own continuity_counter (independent of `cc`
        // above, which only tracks the PES path) persists across samples.
        let mut section_packetizer = SectionPacketizer::new(plan.pid);
        for sample in samples[track_idx] {
            // Rescale the sample's decode/composition time to the 90 kHz TS
            // clock. composition_offset is (pts − dts) in the track scale.
            let dts90 = rescale(dts_ticks_local, ts_scale) + PCR_LEAD_TICKS;
            // The interleave key: monotonic by construction (`dts_ticks_local`
            // only ever grows), UNLIKE `dts90` — which wraps at the 33-bit
            // field per §2.4.3.7, so using it to order packets would reorder
            // a single track's own packets against each other once its
            // cumulative decode time (however implausible the recovered
            // per-sample durations) crosses that wrap point (issue #576: an
            // opaque Data(Pes) track's recovered durations are exactly the
            // kind of untrusted input that can do this).
            let sort_key = rescale_for_ordering(dts_ticks_local, ts_scale);

            if plan.kind.is_section_carried() {
                for pkt in section_packetizer.packetize(&[sample.data.as_slice()]) {
                    tagged.push(TaggedPacket {
                        sort_key,
                        packet: pkt,
                    });
                }
            } else {
                let pts_local = dts_ticks_local as i64 + sample.composition_offset as i64;
                let pts90 = rescale_signed(pts_local, ts_scale) + PCR_LEAD_TICKS;
                let es_payload = build_es_payload(plan, sample)?;
                let carry_pcr = plan.pid == pcr_pid;
                packetize_pes(
                    plan,
                    &es_payload,
                    pts90,
                    dts90,
                    carry_pcr,
                    &mut cc,
                    sort_key,
                    &mut tagged,
                );
            }

            dts_ticks_local += sample.duration as u64;
        }
    }

    // ── 4. Interleave ES packets by decode order (stable) and append ──
    tagged.sort_by_key(|t| t.sort_key);
    for t in &tagged {
        out.extend_from_slice(&t.packet);
    }

    debug_assert_eq!(out.len() % TS_PACKET_SIZE, 0);
    Ok(out)
}

/// Rescale `ticks` from a track's `timescale` to the 90 kHz TS clock, rounding to
/// nearest, and reduce modulo the 33-bit timestamp field.
fn rescale(ticks: u64, timescale: u64) -> u64 {
    let scaled = (ticks * TS_CLOCK_HZ + timescale / 2) / timescale;
    scaled % TS_TIMESTAMP_MOD
}

/// Rescale a possibly-negative tick count (used for `pts = dts + composition`),
/// clamping negatives to 0, then reduce modulo the 33-bit field.
fn rescale_signed(ticks: i64, timescale: u64) -> u64 {
    if ticks <= 0 {
        return 0;
    }
    rescale(ticks as u64, timescale)
}

/// Rescale `ticks` from a track's `timescale` to the 90 kHz TS clock,
/// rounding to nearest, **without** reducing modulo the 33-bit timestamp
/// field — used only as [`TaggedPacket`]'s interleave-order key, never
/// written to the wire (that is [`rescale`]'s job, which must wrap at 2^33
/// per §2.4.3.7). `ticks` (a track's own cumulative decode time, summed only
/// from non-negative sample durations) is monotonically non-decreasing by
/// construction; this function preserves that so the global interleave sort
/// never reorders one track's own packets against each other, even once its
/// cumulative decode time would cross the 33-bit wrap point (issue #576: an
/// opaque `CodecConfig::Data` track's recovered durations are untrusted
/// input that can otherwise do exactly that).
fn rescale_for_ordering(ticks: u64, timescale: u64) -> u64 {
    let scaled = (ticks as u128 * TS_CLOCK_HZ as u128 + timescale as u128 / 2) / timescale as u128;
    scaled.min(u64::MAX as u128) as u64
}

/// Recover the [`AudioSpecificConfig`] from an AAC `esds` box's
/// DecoderSpecificInfo (the inverse of the [`TsDemux`](crate::TsDemux) build).
fn asc_from_esds(esds: &crate::mp4esds::EsdsBox) -> Result<AudioSpecificConfig> {
    let dsi = esds
        .es_descriptor
        .decoder_config
        .as_ref()
        .and_then(|dc| dc.decoder_specific_info.as_ref())
        .ok_or(Error::InvalidInput(
            "AAC esds carries no DecoderSpecificInfo (AudioSpecificConfig)",
        ))?;
    AudioSpecificConfig::parse(&dsi.data)
}

/// Build the elementary-stream PES payload for one sample:
/// video → length-prefixed NAL back to Annex B (prepending SPS/PPS/AUD only when
/// absent so the stream stays self-decodable); AAC → the raw frame re-wrapped in
/// an ADTS header; other audio, and a PES-carried opaque [`CodecConfig::Data`]
/// sample (issue #576) → the raw frame/payload verbatim. Never called for a
/// section-carried `EsKind::Data` — those samples are whole PSI sections,
/// packetized directly by the caller instead ([`SectionPacketizer`]).
fn build_es_payload(plan: &EsPlan, sample: &Sample) -> Result<Vec<u8>> {
    match plan.kind {
        EsKind::Avc => build_annexb_au(&sample.data, sample.is_sync, &plan.avc_sps_pps),
        EsKind::Aac => {
            let asc = plan
                .asc
                .as_ref()
                .ok_or(Error::InvalidInput("AAC ES has no AudioSpecificConfig"))?;
            let frame_len = (sample.data.len() + 7) as u16; // 7-byte ADTS header
            let header = asc.to_adts_header(frame_len)?;
            let mut out = Vec::with_capacity(header.len() + sample.data.len());
            out.extend_from_slice(&header);
            out.extend_from_slice(&sample.data);
            Ok(out)
        }
        EsKind::Ac3 | EsKind::Eac3 | EsKind::Dts | EsKind::Data { .. } => Ok(sample.data.clone()),
    }
}

/// Convert a length-prefixed video sample to an Annex B access unit, prepending
/// the parameter sets `sps_pps` to a `is_sync` (keyframe) access unit that does
/// not already carry an SPS so the TS video AU is independently decodable.
///
/// When the IR sample already carries its SPS/PPS in-band (as a
/// [`TsDemux`](crate::TsDemux)-sourced keyframe does — it preserves every NAL of
/// the access unit) nothing is inserted, so the length↔Annex B round-trip stays
/// byte-identical NAL-for-NAL. Inserted parameter sets are placed after a leading
/// Access Unit Delimiter (ISO/IEC 14496-10 §7.4.1.2.3 AU order), each with a
/// 4-byte start code — the canonical Annex B form the demuxer re-splits.
fn build_annexb_au(length_prefixed: &[u8], is_sync: bool, sps_pps: &[Vec<u8>]) -> Result<Vec<u8>> {
    let nals = iter_length_prefixed_nals(length_prefixed)?;

    let needs_params = is_sync
        && !sps_pps.is_empty()
        && !nals
            .iter()
            .any(|n| !n.is_empty() && (n[0] & H264_NAL_TYPE_MASK) == H264_NAL_SPS);

    if !needs_params {
        // Straight, byte-exact rewrite of the existing NAL sequence.
        return length_prefixed_to_annexb(length_prefixed);
    }

    // Insert the parameter sets after a leading AUD (if any), before the slices.
    let mut out = Vec::with_capacity(length_prefixed.len() + total_param_len(sps_pps));
    let mut inserted = false;
    for nal in &nals {
        let nal_type = nal.first().map(|b| b & H264_NAL_TYPE_MASK);
        // Emit the parameter sets right before the first non-AUD NAL.
        if !inserted && nal_type != Some(H264_NAL_AUD) {
            append_param_sets(&mut out, sps_pps);
            inserted = true;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
    }
    if !inserted {
        // Access unit was only an AUD (degenerate) — still emit the params.
        append_param_sets(&mut out, sps_pps);
    }
    Ok(out)
}

/// Total Annex B length the parameter sets add (4-byte start code each).
fn total_param_len(sps_pps: &[Vec<u8>]) -> usize {
    sps_pps.iter().map(|p| 4 + p.len()).sum()
}

/// Append each parameter set as a 4-byte-start-code Annex B NAL.
fn append_param_sets(out: &mut Vec<u8>, sps_pps: &[Vec<u8>]) {
    for p in sps_pps {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(p);
    }
}

/// Build a PAT section (one program → `pmt_pid`) with its trailing CRC_32.
/// ISO/IEC 13818-1 §2.4.4.3.
fn build_pat_section(pmt_pid: u16) -> Vec<u8> {
    // table_body: transport_stream_id(2) + version/cni(1) + section_number(1) +
    // last_section_number(1) + one program-loop entry (program_number(2) +
    // reserved/program_map_PID(2)).
    let mut body = Vec::new();
    body.extend_from_slice(&1u16.to_be_bytes()); // transport_stream_id = 1
    body.push(VERSION_CURRENT_NEXT);
    body.push(0); // section_number
    body.push(0); // last_section_number
    body.extend_from_slice(&PROGRAM_NUMBER.to_be_bytes());
    body.push(PID_RESERVED_HI | ((pmt_pid >> 8) as u8 & !PID_RESERVED_HI));
    body.push((pmt_pid & 0xFF) as u8);
    finish_section(TABLE_ID_PAT, body)
}

/// Build a PMT section listing every planned elementary stream, with its
/// trailing CRC_32. ISO/IEC 13818-1 §2.4.4.8.
///
/// Each ES's `ES_info` descriptor loop carries its [`EsPlan::descriptors`]
/// verbatim (issue #576) — non-empty for an [`EsKind::Data`] track sourced
/// from a [`CodecConfig::Data`] (so a receiver can identify a carried opaque
/// stream, e.g. its DVB subtitling/teletext descriptor); every decoded codec
/// carries none in the IR today, so its loop stays empty. `program_info`
/// stays empty (no program-level descriptors are modelled).
fn build_pmt_section(pcr_pid: u16, plans: &[EsPlan]) -> Vec<u8> {
    let mut body = Vec::new();
    // table_id_extension = program_number, then version/cni + section numbers.
    body.extend_from_slice(&PROGRAM_NUMBER.to_be_bytes());
    body.push(VERSION_CURRENT_NEXT);
    body.push(0); // section_number
    body.push(0); // last_section_number
    // reserved(3) + PCR_PID(13).
    body.push(PID_RESERVED_HI | ((pcr_pid >> 8) as u8 & !PID_RESERVED_HI));
    body.push((pcr_pid & 0xFF) as u8);
    // reserved(4) + program_info_length(12) = 0 (no program descriptors).
    body.push(INFO_RESERVED_HI);
    body.push(0);
    // Elementary-stream loop: stream_type(1) + reserved/elementary_PID(2) +
    // reserved/ES_info_length(2) + descriptor()×ES_info_length.
    for p in plans {
        body.push(p.kind.stream_type());
        body.push(PID_RESERVED_HI | ((p.pid >> 8) as u8 & !PID_RESERVED_HI));
        body.push((p.pid & 0xFF) as u8);
        let es_info_length = p.descriptors.len().min(MAX_ES_INFO_LENGTH);
        body.push(INFO_RESERVED_HI | ((es_info_length >> 8) as u8 & !INFO_RESERVED_HI));
        body.push((es_info_length & 0xFF) as u8);
        body.extend_from_slice(&p.descriptors[..es_info_length]);
    }
    finish_section(TABLE_ID_PMT, body)
}

/// Prepend the long-form section header (`table_id` + `section_length`) to a
/// table body and append the trailing CRC_32, yielding a complete PSI section.
/// ISO/IEC 13818-1 §2.4.4.1.
fn finish_section(table_id: u8, body: Vec<u8>) -> Vec<u8> {
    // section_length counts everything after the 3-byte prefix, i.e. the body
    // (which already includes table_id_extension etc.) plus the 4-byte CRC.
    let section_length = body.len() + CRC32_LEN;
    let mut section = Vec::with_capacity(3 + section_length);
    section.push(table_id);
    section.push(SECTION_SYNTAX_FLAGS_HI | ((section_length >> 8) as u8 & SECTION_LENGTH_HI_MASK));
    section.push((section_length & 0xFF) as u8);
    section.extend_from_slice(&body);
    let crc = crc32_mpeg2::compute(&section);
    section.extend_from_slice(&crc.to_be_bytes());
    section
}

/// Packetize one complete PSI section into 188-byte TS packets on `pid`.
/// A single PUSI packet with a `pointer_field = 0` prefix, 0xFF-stuffed
/// (all this crate's sections fit one packet); multi-packet continuation is
/// handled by the generic loop for safety. ISO/IEC 13818-1 §2.4.4.
fn packetize_section(pid: u16, section: &[u8]) -> Vec<[u8; TS_PACKET_SIZE]> {
    let mut packets = Vec::new();
    let mut cc: u8 = 0;
    let mut pos = 0usize;
    let mut first = true;
    while pos < section.len() || first {
        let mut pkt = [STUFFING_BYTE; TS_PACKET_SIZE];
        let hdr = TsHeader {
            tei: false,
            pusi: first,
            pid,
            scrambling: 0,
            has_adaptation: false,
            has_payload: true,
            continuity_counter: cc,
        };
        hdr.serialize_into(&mut pkt[..4]).expect("4-byte TS header");
        cc = (cc + 1) & 0x0F;
        let mut w = 4usize;
        let cap = if first {
            pkt[w] = 0; // pointer_field
            w += 1;
            TS_PACKET_SIZE - w
        } else {
            TS_PACKET_SIZE - w
        };
        let take = (section.len() - pos).min(cap);
        pkt[w..w + take].copy_from_slice(&section[pos..pos + take]);
        pos += take;
        packets.push(pkt);
        first = false;
    }
    packets
}

/// Packetize one PES payload (already framed as its `stream_id` payload) into
/// 188-byte TS packets on `plan.pid`, appended to `tagged` (each tagged with
/// `sort_key`, the interleave-order key — see [`rescale_for_ordering`], NOT
/// the on-wire `dts90`). The first packet sets PUSI and — when `carry_pcr` —
/// an adaptation field with the PCR; the final packet is stuffed via an
/// adaptation field so the PES ends exactly on a packet boundary.
/// ISO/IEC 13818-1 §2.4.3.
#[allow(clippy::too_many_arguments)]
fn packetize_pes(
    plan: &EsPlan,
    es_payload: &[u8],
    pts90: u64,
    dts90: u64,
    carry_pcr: bool,
    cc: &mut u8,
    sort_key: u64,
    tagged: &mut Vec<TaggedPacket>,
) {
    let pes = build_pes_bytes(plan, es_payload, pts90, dts90);

    let mut pos = 0usize;
    let mut first = true;
    while pos < pes.len() {
        let mut pkt = [STUFFING_BYTE; TS_PACKET_SIZE];
        let remaining = pes.len() - pos;

        // PCR rides the first packet (if this PID owns the PCR). When present the
        // adaptation field carries flags(1) + PCR(6) = 7 content bytes, so the
        // payload capacity of this packet is reduced accordingly.
        let want_pcr = first && carry_pcr;
        // Minimum AF content bytes forced by the PCR (flags + PCR), else 0.
        let pcr_af_content = if want_pcr { 1 + PCR_FIELD_LEN } else { 0 };
        // Header bytes before the payload when only the forced AF (if any) is
        // present: 4 header + (1 af_len byte + pcr_af_content) when an AF exists.
        let forced_header = 4 + if want_pcr { 1 + pcr_af_content } else { 0 };
        let cap = TS_PACKET_SIZE - forced_header;

        let is_last = remaining <= cap;
        let to_copy = remaining.min(cap);
        // Bytes that must be filled by adaptation-field stuffing so the payload
        // ends exactly at byte 188 (only ever > 0 on the last packet).
        let stuff = cap - to_copy;

        if want_pcr {
            // AF carries the PCR (+ any stuffing on the last packet).
            // af_len = flags(1) + PCR(6) + stuffing.
            let af_len = pcr_af_content + stuff;
            write_af_packet(
                &mut pkt,
                plan.pid,
                first,
                *cc,
                af_len,
                true,
                Some(pcr_for(dts90)),
                &pes[pos..pos + to_copy],
            );
            pos += to_copy;
        } else if is_last && stuff > 0 {
            // No PCR, but the last packet underfills → an AF of pure stuffing.
            // af_len = flags(1) + stuffing; but the AF also costs its own 1-byte
            // length prefix, so total added = 2 + (stuff - 1) accounted below.
            // Choose af_len so 4 + 1 + af_len + to_copy == 188.
            let af_len = TS_PACKET_SIZE - 4 - 1 - to_copy;
            write_af_packet(
                &mut pkt,
                plan.pid,
                first,
                *cc,
                af_len,
                false,
                None,
                &pes[pos..pos + to_copy],
            );
            pos += to_copy;
        } else {
            // Plain payload-only packet (fills the whole 184-byte payload region,
            // or is an interior packet).
            let hdr = TsHeader {
                tei: false,
                pusi: first,
                pid: plan.pid,
                scrambling: 0,
                has_adaptation: false,
                has_payload: true,
                continuity_counter: *cc,
            };
            hdr.serialize_into(&mut pkt[..4]).expect("4-byte TS header");
            pkt[4..4 + to_copy].copy_from_slice(&pes[pos..pos + to_copy]);
            pos += to_copy;
        }

        *cc = (*cc + 1) & 0x0F;
        tagged.push(TaggedPacket {
            sort_key,
            packet: pkt,
        });
        first = false;
    }
}

/// The PCR value to stamp for a packet whose access-unit DTS is `dts90` (90 kHz):
/// place the PCR `PCR_LEAD_TICKS` behind the DTS on the 27 MHz clock.
fn pcr_for(dts90: u64) -> Pcr {
    let base = dts90.saturating_sub(PCR_LEAD_TICKS);
    Pcr::from_27mhz(base * 300)
}

/// Write a TS packet with an adaptation field into `pkt` (initialised to
/// stuffing), then copy `payload` at the byte following the adaptation field.
/// `af_len` is the `adaptation_field_length` value (bytes after the length
/// byte). When `has_pcr` the flags byte sets `PCR_flag` and `pcr` is encoded;
/// any bytes between the encoded content and `4 + 1 + af_len` stay 0xFF stuffing.
/// ISO/IEC 13818-1 §2.4.3.4 / §2.4.3.5.
#[allow(clippy::too_many_arguments)]
fn write_af_packet(
    pkt: &mut [u8; TS_PACKET_SIZE],
    pid: u16,
    pusi: bool,
    cc: u8,
    af_len: usize,
    has_pcr: bool,
    pcr: Option<Pcr>,
    payload: &[u8],
) {
    let hdr = TsHeader {
        tei: false,
        pusi,
        pid,
        scrambling: 0,
        has_adaptation: true,
        has_payload: true,
        continuity_counter: cc,
    };
    // serialize_into sets both AF + payload control bits from the booleans above.
    hdr.serialize_into(&mut pkt[..4]).expect("4-byte TS header");
    // Ensure the control bits reflect adaptation+payload (bits already set by the
    // header serializer via has_adaptation/has_payload).
    debug_assert_eq!(pkt[3] & (AF_CTRL_ADAPTATION | AF_CTRL_PAYLOAD), 0x30);
    pkt[4] = af_len as u8;
    // An af_len of 0 is a valid single-stuffing-byte adaptation field with no
    // flags byte (§2.4.3.4); anything larger carries the 1-byte flags field.
    if af_len >= 1 {
        // Flags byte (byte 5); the rest of the AF stays 0xFF stuffing from init.
        pkt[5] = if has_pcr { AF_PCR_FLAG } else { 0 };
        if has_pcr {
            if let Some(p) = pcr {
                pkt[6..6 + PCR_FIELD_LEN].copy_from_slice(&p.to_field_bytes());
            }
        }
    }
    // Remaining AF bytes (up to 5 + af_len) stay 0xFF stuffing (already set).
    let payload_start = 5 + af_len;
    pkt[payload_start..payload_start + payload.len()].copy_from_slice(payload);
}

/// Build the raw PES packet bytes for one access unit: `00 00 01` +
/// `stream_id` + `PES_packet_length` + optional header (PTS always, DTS when it
/// differs) + the elementary-stream payload. Video uses `PES_packet_length = 0`
/// (unbounded, as broadcast encoders do for video); audio sets the exact length.
///
/// The PES optional header is hand-built per ISO/IEC 13818-1 §2.4.3.7 (mpeg-pes
/// exposes only a parser + a `#[non_exhaustive]` [`mpeg_pes::PesHeader`], so it
/// cannot be constructed externally); the emitted bytes round-trip through
/// [`mpeg_pes::PesPacket::parse`], which the [`TsDemux`](crate::TsDemux) uses.
fn build_pes_bytes(plan: &EsPlan, es_payload: &[u8], pts90: u64, dts90: u64) -> Vec<u8> {
    let include_dts = dts90 != pts90;
    // PES optional-header content length after the 3 fixed bytes: PTS (5) always,
    // + DTS (5) when present.
    let opt_content = if include_dts { 10 } else { 5 };
    // PES_packet_length counts everything after the 16-bit length field: the
    // 3 fixed optional-header bytes + optional content + payload. Video uses 0
    // (unbounded) so an access unit may exceed 65535 bytes; audio sets it exactly.
    let after_len = HEADER_FIXED + opt_content + es_payload.len();
    let pes_packet_length = if plan.kind.is_video() {
        0u16
    } else {
        after_len.min(u16::MAX as usize) as u16
    };

    let mut out = Vec::with_capacity(MIN_LEN + HEADER_FIXED + opt_content + es_payload.len());
    out.extend_from_slice(&PES_START_CODE);
    out.push(plan.stream_id.0);
    out.extend_from_slice(&pes_packet_length.to_be_bytes());
    // Fixed optional-header bytes (§2.4.3.7): '10' marker + flags, then PTS_DTS
    // flags byte, then PES_header_data_length.
    out.push(PES_OPTIONAL_MARKER); // '10' marker, all other flags 0
    out.push(if include_dts {
        PTS_DTS_FLAGS_BOTH
    } else {
        PTS_DTS_FLAGS_PTS_ONLY
    });
    out.push(opt_content as u8); // PES_header_data_length
    if include_dts {
        // PTS carries prefix '0011' when a DTS follows; DTS carries '0001'.
        out.extend_from_slice(&encode_timestamp(pts90, TS_PREFIX_PTS_WITH_DTS));
        out.extend_from_slice(&encode_timestamp(dts90, TS_PREFIX_DTS));
    } else {
        // PTS-only field carries prefix '0010'.
        out.extend_from_slice(&PesPts(pts90).to_field_bytes());
    }
    out.extend_from_slice(es_payload);
    out
}

/// Encode a 33-bit timestamp into the 5-byte PTS/DTS field with the given 4-bit
/// `prefix`, interleaving the mandatory `marker_bit`s. ISO/IEC 13818-1 §2.4.3.7.
fn encode_timestamp(ts: u64, prefix: u8) -> [u8; 5] {
    let ts = ts & TS_VALUE_MASK;
    [
        (prefix << 4) | ((((ts >> 30) & 0x07) as u8) << 1) | 0x01,
        ((ts >> 22) & 0xFF) as u8,
        ((((ts >> 15) & 0x7F) as u8) << 1) | 0x01,
        ((ts >> 7) & 0xFF) as u8,
        (((ts & 0x7F) as u8) << 1) | 0x01,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn es_kind_stream_types_mirror_demux() {
        assert_eq!(EsKind::Avc.stream_type(), 0x1B);
        assert_eq!(EsKind::Aac.stream_type(), 0x0F);
        assert_eq!(EsKind::Ac3.stream_type(), 0x81);
        assert_eq!(EsKind::Eac3.stream_type(), 0x87);
        assert_eq!(EsKind::Dts.stream_type(), 0x82);
        // Opaque Data (issue #576): the preserved stream_type round-trips
        // verbatim regardless of carriage.
        assert_eq!(
            EsKind::Data {
                stream_type: 0x06,
                carriage: DataCarriage::Pes,
            }
            .stream_type(),
            0x06
        );
        assert_eq!(
            EsKind::Data {
                stream_type: 0x86,
                carriage: DataCarriage::Sections,
            }
            .stream_type(),
            0x86
        );
        assert!(
            EsKind::Data {
                stream_type: 0x86,
                carriage: DataCarriage::Sections,
            }
            .is_section_carried()
        );
        assert!(
            !EsKind::Data {
                stream_type: 0x06,
                carriage: DataCarriage::Pes,
            }
            .is_section_carried()
        );
    }

    #[test]
    fn pat_section_crc_is_valid() {
        let pat = build_pat_section(PMT_PID);
        // CRC over the whole section (incl. its own trailing CRC) must be 0 for a
        // valid MPEG-2 section (crc32_mpeg2 residue property).
        assert_eq!(crc32_mpeg2::compute(&pat), 0);
        assert_eq!(pat[0], TABLE_ID_PAT);
    }

    #[test]
    fn pmt_section_crc_is_valid() {
        let plans = alloc::vec![EsPlan {
            pid: ES_PID_BASE,
            stream_id: StreamId(STREAM_ID_VIDEO_BASE),
            kind: EsKind::Avc,
            asc: None,
            avc_sps_pps: Vec::new(),
            descriptors: Vec::new(),
        }];
        let pmt = build_pmt_section(ES_PID_BASE, &plans);
        assert_eq!(crc32_mpeg2::compute(&pmt), 0);
        assert_eq!(pmt[0], TABLE_ID_PMT);
    }

    #[test]
    fn pmt_section_carries_es_info_descriptors() {
        // A Data ES's preserved descriptors must appear verbatim in the
        // PMT's ES_info loop (issue #576), and the CRC must still be valid.
        let descriptors = alloc::vec![0x59, 0x02, 0xAA, 0xBB]; // fake tag+len+body
        let plans = alloc::vec![EsPlan {
            pid: ES_PID_BASE,
            stream_id: StreamId(0),
            kind: EsKind::Data {
                stream_type: 0x06,
                carriage: DataCarriage::Pes,
            },
            asc: None,
            avc_sps_pps: Vec::new(),
            descriptors: descriptors.clone(),
        }];
        let pmt = build_pmt_section(ES_PID_BASE, &plans);
        assert_eq!(crc32_mpeg2::compute(&pmt), 0);
        // Locate the ES_info bytes: body starts at offset 8 (section header),
        // program_info_length is 0, so the ES loop starts right after the
        // 4-byte PCR_PID + program_info_length prefix.
        let es_loop_start = 8 + 4;
        assert_eq!(pmt[es_loop_start], 0x06, "stream_type");
        let es_info_length =
            (((pmt[es_loop_start + 3] & 0x0F) as usize) << 8) | pmt[es_loop_start + 4] as usize;
        assert_eq!(es_info_length, descriptors.len());
        let desc_start = es_loop_start + 5;
        assert_eq!(
            &pmt[desc_start..desc_start + es_info_length],
            &descriptors[..]
        );
    }

    #[test]
    fn section_packets_are_whole_and_pusi() {
        let pat = build_pat_section(PMT_PID);
        let pkts = packetize_section(PAT_PID, &pat);
        assert_eq!(pkts.len(), 1);
        // sync byte + PUSI bit.
        assert_eq!(pkts[0][0], 0x47);
        assert_ne!(pkts[0][1] & 0x40, 0, "PUSI must be set on the first packet");
    }
}
