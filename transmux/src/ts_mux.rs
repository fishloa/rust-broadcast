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
//! in ADTS) → packetise the PES into 188-byte TS packets, carrying the PCR on
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
//!
//! # ES_info descriptor passthrough policy (issue #775)
//!
//! Every track's inherited PMT `ES_info` descriptor-loop bytes
//! ([`TrackSpec::es_info_descriptors`](crate::pipeline::TrackSpec::es_info_descriptors))
//! are carried into the re-muxed PMT, not only an opaque
//! [`CodecConfig::Data`] track's — a track loses no information just because
//! this crate understood its codec. The policy is a **deny-list, not an
//! allow-list**: an allow-list would silently drop an unknown-but-valid
//! descriptor (a broadcaster's private or newly-registered tag), which is
//! exactly the bug this fixes.
//!
//! - **Denied: `CA_descriptor`** (ISO/IEC 13818-1 §2.6.16, tag `0x09`).
//!   `CA_descriptor` signals that the elementary stream is scrambled and
//!   names the `CA_PID`/`CA_system_ID` carrying its ECMs. This muxer never
//!   encrypts its output; copying an inherited `CA_descriptor` forward would
//!   falsely advertise the cleartext re-mux as scrambled, pointing at a
//!   `CA_PID` that does not exist in the new PMT — a decoder or DVB
//!   conformance probe reading it would wrongly conclude the stream needs a
//!   CA module to decrypt.
//! - **Everything else passes through**, preserving the inherited
//!   descriptors' source order.
//! - **De-duplicated against this muxer's own synthesised descriptors** (e.g.
//!   the `MPEG-H_3dAudio_descriptor` built from the typed
//!   `mpegh3daProfileLevelIndication`, issue #579): if an inherited
//!   descriptor's tag matches one this muxer already synthesises, the
//!   inherited copy is dropped and the synthesised one (grounded in the
//!   current [`CodecConfig`]) is kept — emitting both would yield a
//!   malformed `ES_info` loop with contradictory signalling under one tag.
//! - The merged loop is rejected with a typed error
//!   ([`Error::BufferCapExceeded`]) rather than silently truncated if it
//!   would exceed the 12-bit `ES_info_length` field's 4095-byte maximum
//!   (§2.4.4.8) — a truncated descriptor loop is a malformed PMT.

use alloc::vec::Vec;

use broadcast_common::{Package, crc32_mpeg2};
use mpeg_pes::{Pts as PesPts, StreamId};
use mpeg_ts::mux::SectionPacketiser;
use mpeg_ts::ts::{Pcr, TS_PACKET_SIZE, TsHeader};

use crate::aac_asc::AudioSpecificConfig;
use crate::annexb::{iter_length_prefixed_nals, length_prefixed_to_annexb};
use crate::error::{Error, Result};
use crate::media::{Media, TimelineOrigin, Track, relative_decode_times};
use crate::mp4esds::EsdsBox;
use crate::nal::{NalCodec, nal_unit_type};
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
/// Maximum `section_length` this crate accepts (§2.4.4.8): the 12-bit wire
/// field can encode up to 4095, but §2.4.4.4 additionally bounds every
/// section other than the private/DSM-CC forms to 1021 bytes, so a PMT past
/// this — about 20 audio tracks with language/AC-3/DVB descriptors — is
/// rejected rather than emitting an out-of-spec section or, past 4095,
/// silently wrapping the length field.
const MAX_SECTION_LENGTH: usize = 0x3FD;
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
/// HEVC (H.265) video — ISO/IEC 13818-1 Table 2-34 (issue #627).
const STREAM_TYPE_HEVC: u8 = 0x24;
/// MPEG-2 video (ITU-T H.262 / ISO/IEC 13818-2) — ISO/IEC 13818-1 Table 2-34
/// (issue #627).
const STREAM_TYPE_MPEG2_VIDEO: u8 = 0x02;
/// ISO/IEC 13818-7 AAC in ADTS — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_AAC_ADTS: u8 = 0x0F;
/// MPEG-1 audio (ISO/IEC 11172-3) — ISO/IEC 13818-1 Table 2-34 (issue #627).
const STREAM_TYPE_MPEG1_AUDIO: u8 = 0x03;
/// MPEG-2 audio (ISO/IEC 13818-3, LSF) — ISO/IEC 13818-1 Table 2-34 (issue
/// #627).
const STREAM_TYPE_MPEG2_AUDIO: u8 = 0x04;
/// AC-3 (ATSC/DVB user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_AC3: u8 = 0x81;
/// E-AC-3 (user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_EAC3: u8 = 0x87;
/// DTS (canonical DVB assignment, user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_DTS: u8 = 0x82;
/// MPEG-H 3D Audio main stream (MHAS) — ISO/IEC 13818-1 Table 2-34 / ETSI
/// TS 101 154 §6.8 (issue #579).
const STREAM_TYPE_MPEGH: u8 = 0x2D;

/// `esds` `objectTypeIndication` for MPEG-1 Audio (ISO/IEC 14496-1 Table 5) —
/// selects TS `stream_type` 0x03 vs 0x04 for a re-muxed
/// [`CodecConfig::MpegAudio`] track (issue #627).
const OTI_MPEG1_AUDIO: u8 = 0x6B;
/// `esds` `objectTypeIndication` for MPEG-2 Audio (ISO/IEC 14496-1 Table 5).
const OTI_MPEG2_AUDIO: u8 = 0x69;

/// Maximum value of the 12-bit `ES_info_length` field (§2.4.4.8).
const MAX_ES_INFO_LENGTH: usize = 0x0FFF;

/// `CA_descriptor` tag (ISO/IEC 13818-1 §2.6.16) — denied from ES_info
/// passthrough (issue #775, see the module doc's policy section): it names a
/// `CA_PID` scrambling the elementary stream, which this cleartext-output
/// muxer never has, so copying it forward would falsely signal the re-muxed
/// stream as scrambled.
const DESCRIPTOR_TAG_CA: u8 = 0x09;

// ── MPEG-H_3dAudio_descriptor (ISO/IEC 13818-1 §2.6.106, ETSI TS 101 154
// §4.1.8.31) ─────────────────────────────────────────────────────────────

/// `extension_descriptor` tag (ISO/IEC 13818-1 Table 2-45) — the umbrella
/// descriptor under which post-2013 additions, including MPEG-H signalling,
/// register a second-level `extension_descriptor_tag`.
const DESCRIPTOR_TAG_EXTENSION: u8 = 0x3F;
/// `MPEGH_3dAudio_descriptor`'s `extension_descriptor_tag`. ISO/IEC 13818-1
/// §2.6.106 (which registers this value) is paid and not vendored; the value
/// `0x08` is taken from the real Fraunhofer MPEG-H-in-TS fixture's ES_info
/// bytes (`3F 04 08 10 7F C1` — see `transmux/docs/codec/mpegh-ts-101154.md`),
/// real DVB-conformant broadcast content from the format's own originator.
const MPEGH_3DAUDIO_EXTENSION_TAG: u8 = 0x08;
/// Body length (bytes after `descriptor_length`) of the
/// `MPEGH_3dAudio_descriptor` this muxer emits: the sum of one
/// `extension_descriptor_tag` byte and one `mpegh3daProfileLevelIndication`
/// byte. ETSI TS 101 154 §4.1.8.31 documents `mpegh3daProfileLevelIndication`
/// as the field that "shall be signalled" — the only part of the
/// descriptor's body this crate has spec grounding for. The real fixture's
/// descriptor carries 2 further bytes (`7F C1`) this crate cannot ground
/// (the full syntax is in the paid ISO/IEC 13818-1 §2.6.106) and therefore
/// does not reproduce; a shorter, correctly-length-prefixed descriptor is
/// spec-legal (§2.6.10 defines the generic `descriptor_length`-bounded
/// extension mechanism precisely so a decoder skips exactly that many
/// bytes).
const MPEGH_3DAUDIO_DESCRIPTOR_BODY_LEN: u8 = 2;

/// Build the `MPEG-H_3dAudio_descriptor` ES_info entry carrying
/// `mpegh3daProfileLevelIndication` (issue #579).
fn mpegh_3daudio_descriptor(profile_level_indication: u8) -> Vec<u8> {
    alloc::vec![
        DESCRIPTOR_TAG_EXTENSION,
        MPEGH_3DAUDIO_DESCRIPTOR_BODY_LEN,
        MPEGH_3DAUDIO_EXTENSION_TAG,
        profile_level_indication,
    ]
}

// ── PES / stream_id constants (ISO/IEC 13818-1 §2.4.3.6, Table 2-22) ────────

/// Base `stream_id` for video elementary streams (`1110 xxxx`, 0xE0–0xEF).
const STREAM_ID_VIDEO_BASE: u8 = 0xE0;
/// Exclusive upper bound of the video `stream_id` family: `0xEF + 1`
/// (Table 2-22), so at most [`MAX_VIDEO_STREAMS`] video ES can be numbered.
const STREAM_ID_VIDEO_LIMIT: u8 = 0xF0;
/// Base `stream_id` for audio elementary streams (`110x xxxx`, 0xC0–0xDF).
const STREAM_ID_AUDIO_BASE: u8 = 0xC0;
/// Exclusive upper bound of the audio `stream_id` family: `0xDF + 1`
/// (Table 2-22), so at most [`MAX_AUDIO_STREAMS`] audio ES can be numbered.
const STREAM_ID_AUDIO_LIMIT: u8 = 0xE0;
/// Most video elementary streams one program may carry (`0xE0..0xEF`).
const MAX_VIDEO_STREAMS: u8 = STREAM_ID_VIDEO_LIMIT - STREAM_ID_VIDEO_BASE;
/// Most audio elementary streams one program may carry (`0xC0..0xDF`).
const MAX_AUDIO_STREAMS: u8 = STREAM_ID_AUDIO_LIMIT - STREAM_ID_AUDIO_BASE;
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
/// AVC AUD NAL header byte: `forbidden_zero_bit`(1)=0 + `nal_ref_idc`(2)=0 +
/// `nal_unit_type`(5)=9 (H.264 §7.3.2.4 / Table 7-1 — Table 7-1 gives the AUD
/// the *category* 6, not `nal_ref_idc` 6: `nal_ref_idc` is a 2-bit field, and an
/// access unit delimiter is `non-VCL`, so it is 0).
const H264_NAL_AUD_BYTE: u8 = 0x09;
/// AVC AUD `primary_pic_type` = 7, which Table 7-5 ("Meaning of
/// `primary_pic_type`") defines as "slice_type values 0..9 may be present" —
/// i.e. a picture of any type. The byte is `primary_pic_type`(3 bits, `111`)
/// followed by `rbsp_trailing_bits` (`1` then zero-padding).
const H264_AUD_PRIMARY_PIC_TYPE: u8 = 0xF0;

// ── H.265/HEVC NAL constants (ITU-T H.265 Table 7-1) — issue #627 ───────────

/// H.265 `nal_unit_type` for VPS (`VPS_NUT`) — Table 7-1 (type 32).
const HEVC_NAL_VPS: u8 = 32;
/// H.265 `nal_unit_type` for SPS (`SPS_NUT`) — Table 7-1 (type 33).
const HEVC_NAL_SPS: u8 = 33;
/// H.265 `nal_unit_type` for PPS (`PPS_NUT`) — Table 7-1 (type 34).
const HEVC_NAL_PPS: u8 = 34;
/// H.265 `nal_unit_type` for an Access Unit Delimiter (`AUD_NUT`) — Table 7-1
/// (type 35).
const HEVC_NAL_AUD: u8 = 35;
/// HEVC AUD NAL header bytes 1-2: `forbidden_zero_bit`(1)=0,
/// `nal_unit_type`(6)=35, `nuh_layer_id`(6)=0, `nuh_temporal_id_plus1`(3)=1,
/// i.e. `0x46 0x01` (H.265 §7.3.1.2 / Table 7-1).
const HEVC_AUD_FIRST_BYTE: u8 = 0x46;
/// See [`HEVC_AUD_FIRST_BYTE`].
const HEVC_AUD_SECOND_BYTE: u8 = 0x01;
/// HEVC AUD `pic_type` = 2, which Table 7-2 ("Interpretation of `pic_type`")
/// defines as "B, P, I may be present" — every slice type, so the delimiter is
/// valid for any access unit. Followed by `rbsp_trailing_bits` (`1` then
/// zero-padding).
const HEVC_AUD_PIC_TYPE: u8 = 0x50;

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
    /// H.265/HEVC video (issue #627).
    Hevc,
    /// MPEG-2 video / H.262 (issue #627).
    Mpeg2Video,
    /// AAC audio (re-wrapped in ADTS).
    Aac,
    /// MPEG-1/2 audio, Layers I/II/III, passed through verbatim (issue #627).
    MpegAudio {
        /// Whether the source `esds` carries the MPEG-2 (LSF) `objectTypeIndication`
        /// (`0x69`) rather than MPEG-1 (`0x6B`) — selects `stream_type` 0x04 vs
        /// 0x03 (ISO/IEC 13818-1 Table 2-34).
        is_mpeg2: bool,
    },
    /// AC-3 audio.
    Ac3,
    /// E-AC-3 audio.
    Eac3,
    /// DTS audio (core substream, passed through verbatim).
    Dts,
    /// MPEG-H 3D Audio (MHAS, passed through verbatim) — issue #579.
    MpegH,
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
            EsKind::Hevc => STREAM_TYPE_HEVC,
            EsKind::Mpeg2Video => STREAM_TYPE_MPEG2_VIDEO,
            EsKind::Aac => STREAM_TYPE_AAC_ADTS,
            EsKind::MpegAudio { is_mpeg2: true } => STREAM_TYPE_MPEG2_AUDIO,
            EsKind::MpegAudio { is_mpeg2: false } => STREAM_TYPE_MPEG1_AUDIO,
            EsKind::Ac3 => STREAM_TYPE_AC3,
            EsKind::Eac3 => STREAM_TYPE_EAC3,
            EsKind::Dts => STREAM_TYPE_DTS,
            EsKind::MpegH => STREAM_TYPE_MPEGH,
            EsKind::Data { stream_type, .. } => stream_type,
        }
    }

    /// Whether this is a video stream (drives PES `stream_id` family + PCR PID).
    fn is_video(self) -> bool {
        matches!(self, EsKind::Avc | EsKind::Hevc | EsKind::Mpeg2Video)
    }

    /// Whether this is a continuous, self-timed audio stream: one whose PES
    /// packets arrive densely enough to anchor the PCR (ISO/IEC 13818-1
    /// §2.4.2.2 bounds the PCR interval at 100 ms). A PES-carried opaque
    /// `Data` stream (DVB subtitles / teletext, `stream_id` `0xBD`) is *not*
    /// one — its PES packets appear only when a subtitle event does, seconds
    /// apart, so a PCR anchored to it violates the repetition bound (TR 101 290
    /// indicator 2.3).
    fn is_continuous_audio(self) -> bool {
        matches!(
            self,
            EsKind::Aac
                | EsKind::MpegAudio { .. }
                | EsKind::Ac3
                | EsKind::Eac3
                | EsKind::Dts
                | EsKind::MpegH
        )
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
    /// TS muxer emits it as, or `None` if the TS layer has no `stream_type`
    /// mapping for that codec (issue #627: the track is silently dropped by
    /// [`plan_elementary_streams`], not an error). Decoded codecs get their
    /// canonical `stream_type`; [`CodecConfig::Data`] carries its preserved
    /// `stream_type` + `carriage` straight through (issue #576).
    ///
    /// Not TS-carriable today: [`CodecConfig::Vvc`]/[`CodecConfig::Av1`]/
    /// [`CodecConfig::Vp9`] (no allocated/implemented TS `stream_type` mapping
    /// in this crate yet), [`CodecConfig::Opus`]/[`CodecConfig::Flac`]/
    /// [`CodecConfig::Ac4`] (no ES framing implemented for TS),
    /// [`CodecConfig::Subtitle`] (an fMP4/CMAF-sourced `stpp`/`wvtt` track has
    /// no TS `stream_type` mapping in this crate; a DVB-subtitle/teletext PES
    /// stream is still recovered as [`CodecConfig::Data`] on the TS demux
    /// path), and the WebM-native [`CodecConfig::Vp8`]/[`CodecConfig::Vorbis`]
    /// (out of scope for any ISOBMFF/TS mux path — see their doc comments).
    fn from_config(config: &CodecConfig) -> Option<Self> {
        match config {
            CodecConfig::Avc { .. } => Some(EsKind::Avc),
            CodecConfig::Hevc { .. } => Some(EsKind::Hevc),
            CodecConfig::Mpeg2Video { .. } => Some(EsKind::Mpeg2Video),
            CodecConfig::Aac { .. } => Some(EsKind::Aac),
            CodecConfig::MpegAudio { esds, .. } => Some(EsKind::MpegAudio {
                is_mpeg2: mpeg_audio_is_mpeg2(esds),
            }),
            CodecConfig::Ac3 { .. } => Some(EsKind::Ac3),
            CodecConfig::Eac3 { .. } => Some(EsKind::Eac3),
            CodecConfig::Dts { .. } => Some(EsKind::Dts),
            CodecConfig::MpegH { .. } => Some(EsKind::MpegH),
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

/// Whether an MPEG-1/2 audio `esds` was recovered from an MPEG-2 (LSF) rather
/// than MPEG-1 elementary stream, from its `objectTypeIndication` — selects TS
/// `stream_type` 0x04 vs 0x03 (ISO/IEC 13818-1 Table 2-34; issue #627).
/// Defaults to MPEG-1 if the decoder-config descriptor is absent (this crate's
/// demuxers always populate it for `CodecConfig::MpegAudio`).
fn mpeg_audio_is_mpeg2(esds: &EsdsBox) -> bool {
    let oti = esds
        .es_descriptor
        .decoder_config
        .as_ref()
        .map(|dc| dc.object_type_indication.0)
        .unwrap_or(OTI_MPEG1_AUDIO);
    oti == OTI_MPEG2_AUDIO
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
    /// HEVC VPS + SPS + PPS NALs (from `hvcC`, in that AU order — ITU-T H.265
    /// §7.4.2.1), prepended to a keyframe access unit that lacks an SPS so
    /// every TS video AU is independently decodable (issue #627). Empty for
    /// non-HEVC streams.
    hevc_vps_sps_pps: Vec<Vec<u8>>,
    /// PMT ES_info descriptor-loop bytes to emit for this stream: the
    /// track's inherited [`TrackSpec::es_info_descriptors`](crate::pipeline::TrackSpec::es_info_descriptors)
    /// merged with this muxer's own synthesised descriptors (issue #576,
    /// #579), per the module doc's ES_info passthrough policy (issue #775) —
    /// `CA_descriptor` denied, everything else passed through de-duplicated
    /// against the synthesised set, source order preserved. Populated for
    /// every track kind, not only [`EsKind::Data`] — the IR carries
    /// ES_info descriptors for a recognised codec's track too.
    descriptors: Vec<u8>,
}

/// TS packet payload capacity: the 188-byte packet less its 4-byte header
/// (ISO/IEC 13818-1 §2.4.3.2).
const TS_PAYLOAD_CAPACITY: usize = TS_PACKET_SIZE - 4;
/// Extra packets budgeted per sample in the packet-count pre-size hint: the PES
/// header spill plus the stuffed final packet.
const PACKETS_PER_SAMPLE_SLACK: usize = 2;

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
/// PES packet, packetised into 188-byte TS packets, and all packets are
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
        let stream_id = if kind.is_video() {
            // `0xE0..=0xEF` (Table 2-22) is the whole video family; a 17th video
            // ES would take `0xF0` (`ECM_stream`) and a 33rd would wrap past
            // `0xFF` — so the count is bounded rather than narrowed (r05-W22).
            if n_video >= MAX_VIDEO_STREAMS {
                return Err(Error::TooManyElementaryStreams {
                    family: "video",
                    max: MAX_VIDEO_STREAMS,
                });
            }
            let id = StreamId(STREAM_ID_VIDEO_BASE + n_video);
            n_video += 1;
            id
        } else {
            match kind {
                EsKind::Data {
                    carriage: DataCarriage::Pes,
                    ..
                } => StreamId(STREAM_ID_PRIVATE_1),
                EsKind::Data {
                    carriage: DataCarriage::Sections,
                    ..
                } => StreamId(0),
                _ => {
                    // `0xC0..=0xDF` is the audio family; `0xC0 + 32 = 0xE0` would
                    // collide with the video range (`0xE0..`) and the demuxer
                    // would classify it as video (r05-W22).
                    if n_audio >= MAX_AUDIO_STREAMS {
                        return Err(Error::TooManyElementaryStreams {
                            family: "audio",
                            max: MAX_AUDIO_STREAMS,
                        });
                    }
                    let id = StreamId(STREAM_ID_AUDIO_BASE + n_audio);
                    n_audio += 1;
                    id
                }
            }
        };
        let asc = match &track.spec.config {
            CodecConfig::Aac { esds, .. } => Some(esds.audio_specific_config()?),
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
        let hevc_vps_sps_pps = match &track.spec.config {
            CodecConfig::Hevc { config, .. } => hevc_parameter_sets(&config.config),
            _ => Vec::new(),
        };
        // MPEG-H's ES_info descriptor is synthesized fresh from the typed
        // `mpegh3daProfileLevelIndication` field, rather than trusting a raw
        // byte loop preserved from demux (issue #579; ETSI TS 101 154
        // §4.1.8.31); [`merge_es_info_descriptors`] drops any inherited
        // duplicate of the same tag in its favour (issue #775).
        let synthesized: Vec<Vec<u8>> = match &track.spec.config {
            CodecConfig::MpegH { config, .. } => {
                alloc::vec![mpegh_3daudio_descriptor(
                    config.mpegh3da_profile_level_indication
                )]
            }
            _ => Vec::new(),
        };
        let descriptors = merge_es_info_descriptors(&track.spec.es_info_descriptors, &synthesized)?;
        plans.push(EsPlan {
            pid: next_pid,
            stream_id,
            kind,
            asc,
            avc_sps_pps,
            hevc_vps_sps_pps,
            descriptors,
        });
        planned_idx.push(idx);
        next_pid += 1;
    }
    if plans.is_empty() {
        return Err(Error::InvalidInput(
            "no track carries a TS-representable codec (AVC/HEVC/MPEG-2-video/AAC/\
             MPEG-audio/AC-3/E-AC-3/DTS/MPEG-H/Data)",
        ));
    }
    Ok((plans, planned_idx))
}

/// Merge a track's inherited PMT `ES_info` descriptor-loop bytes with this
/// muxer's own `synthesized` descriptors, per the module doc's ES_info
/// passthrough policy (issue #775):
///
/// - `CA_descriptor` (tag [`DESCRIPTOR_TAG_CA`]) is denied — dropped
///   unconditionally.
/// - Any other inherited descriptor whose **dedup key** ([`descriptor_dedup_key`])
///   matches one of `synthesized`'s is dropped too (dedup: the synthesized
///   copy — grounded in the current [`CodecConfig`] — is kept instead, once,
///   further down). For every tag except [`DESCRIPTOR_TAG_EXTENSION`] (`0x3F`)
///   the key is the tag alone; for `0x3F` it is `(0x3F, extension_descriptor_tag)`
///   (issue #775 follow-up, R4) — `extension_descriptor` is an umbrella tag
///   under which unrelated post-2013 registrations (MPEG-H's own
///   `MPEGH_3dAudio_descriptor` among them) each pick their own second-level
///   `extension_descriptor_tag` (ISO/IEC 13818-1 Table 2-45), so two `0x3F`
///   descriptors sharing only the outer tag are not duplicates and keying on
///   `0x3F` alone would wrongly collapse a second, unrelated synthesized (or
///   inherited) extension onto this one.
/// - Every other inherited descriptor passes through, in source order.
/// - `synthesized` is then appended.
///
/// A malformed inherited loop (a truncated trailing `tag`/`length` pair) is
/// walked only up to its last complete descriptor, mirroring the demux
/// side's own defensive TLV walk (`ts_demux::Codec::refine_with_descriptors`).
///
/// Returns [`Error::BufferCapExceeded`] — naming the 4095-byte `ES_info_length`
/// cap (§2.4.4.8) — if the merged loop would overflow it, rather than
/// silently truncating into a malformed PMT.
fn merge_es_info_descriptors(inherited: &[u8], synthesized: &[Vec<u8>]) -> Result<Vec<u8>> {
    let synthesized_keys: Vec<(u8, Option<u8>)> = synthesized
        .iter()
        .filter_map(|d| {
            let tag = *d.first()?;
            // A descriptor's body starts after `tag`(1) + `length`(1).
            Some(descriptor_dedup_key(tag, d.get(2..).unwrap_or(&[])))
        })
        .collect();

    let mut out = Vec::with_capacity(inherited.len() + total_param_len(synthesized));
    let mut off = 0usize;
    while off + 2 <= inherited.len() {
        let tag = inherited[off];
        let len = inherited[off + 1] as usize;
        let end = (off + 2 + len).min(inherited.len());
        let body = &inherited[(off + 2).min(end)..end];
        let key = descriptor_dedup_key(tag, body);
        if tag != DESCRIPTOR_TAG_CA && !synthesized_keys.contains(&key) {
            out.extend_from_slice(&inherited[off..end]);
        }
        off = end;
    }
    for d in synthesized {
        out.extend_from_slice(d);
    }

    if out.len() > MAX_ES_INFO_LENGTH {
        return Err(Error::BufferCapExceeded {
            what: "PMT ES_info descriptor loop",
            cap: MAX_ES_INFO_LENGTH,
        });
    }
    Ok(out)
}

/// The dedup key [`merge_es_info_descriptors`] compares an inherited
/// descriptor's tag against a synthesized one's (issue #775 follow-up, R4).
///
/// For every tag except [`DESCRIPTOR_TAG_EXTENSION`] (`extension_descriptor`,
/// `0x3F`, ISO/IEC 13818-1 Table 2-45) the tag alone identifies what the
/// descriptor is, so the key is `(tag, None)`. `0x3F` is different: it is an
/// umbrella under which independent post-2013 registrations each choose their
/// own second-level `extension_descriptor_tag` — the first body byte — so two
/// `0x3F` descriptors are the same registration only if that second-level tag
/// also matches; the key is `(0x3F, Some(extension_descriptor_tag))`. A `0x3F`
/// descriptor with an empty body (malformed: no `extension_descriptor_tag` on
/// the wire) keys as `(0x3F, None)`, matching only another equally-malformed
/// `0x3F` entry, never a well-formed one — this dedup is a same-registration
/// check, not a byte-for-byte one, and an absent tag cannot be shown to be the
/// same registration as a present one.
fn descriptor_dedup_key(tag: u8, body: &[u8]) -> (u8, Option<u8>) {
    if tag == DESCRIPTOR_TAG_EXTENSION {
        (tag, body.first().copied())
    } else {
        (tag, None)
    }
}

/// Collect a HEVC `hvcC` record's parameter-set NALs in AU order — VPS, then
/// SPS, then PPS (ITU-T H.265 §7.4.2.1) — flattening [`HEVCDecoderConfigurationRecord::arrays`]
/// (which may list the three types in any order, one array per type). Used to
/// prepend missing parameter sets to a keyframe TS access unit (issue #627).
fn hevc_parameter_sets(
    record: &crate::hevc_config::HEVCDecoderConfigurationRecord,
) -> Vec<Vec<u8>> {
    let mut vps = Vec::new();
    let mut sps = Vec::new();
    let mut pps = Vec::new();
    for array in &record.arrays {
        let bucket = match array.nal_unit_type {
            HEVC_NAL_VPS => &mut vps,
            HEVC_NAL_SPS => &mut sps,
            HEVC_NAL_PPS => &mut pps,
            _ => continue,
        };
        for nalu in &array.nalus {
            bucket.push(nalu.0.clone());
        }
    }
    vps.into_iter().chain(sps).chain(pps).collect()
}

/// Mux `tracks` into one self-contained MPEG-2 TS byte stream: a leading
/// PAT (PID `0x0000`) + PMT, then the DTS-interleaved PES packets for the
/// per-track `samples` (`samples[i]` is the sample slice for `tracks[i]`).
///
/// Each sample is stamped from its own absolute [`Sample::dts`]/[`Sample::pts`],
/// shifted by one origin common to every track (the earliest first-sample
/// DTS), so inter-track offsets and gaps survive (issue #1020). `TsMux` calls
/// it once over the whole input.
pub(crate) fn mux_tracks(tracks: &[Track], samples: &[&[Sample]]) -> Result<Vec<u8>> {
    let origin = TimelineOrigin::of(tracks);
    let times: Vec<Vec<i64>> = tracks
        .iter()
        .map(|t| relative_decode_times(t, &origin))
        .collect();
    let times: Vec<&[i64]> = times.iter().map(Vec::as_slice).collect();
    mux_tracks_timed(tracks, samples, &times)
}

/// Mux one more segment of a continuing stream: each track's first sample is
/// stamped at decode time `base_dts_ticks[track_idx]` (in that track's own
/// timescale) and every later one at the running sum of the previous samples'
/// durations, while the transport-stream continuity counters
/// (`continuity_counter`, ISO/IEC 13818-1 §2.4.3.3) continue from `cc` and its
/// updated state is left in `cc` for the next call.
///
/// The classic-HLS segmenter uses this so each segment's PES timestamps continue
/// the previous segment's timeline: concatenating the segments then yields one
/// monotonically increasing DTS/PTS timeline, so a demuxer recovers each sample's
/// original duration (DTS delta) — including across segment boundaries — instead
/// of seeing the clock reset to 0 at each segment. The shared [`TsContinuity`]
/// does the same for the `continuity_counter`, which would otherwise restart at
/// 0 on every PID at each boundary (TR 101 290 indicator 1.4).
///
/// A segmenter that emits one TS per segment — the classic-HLS path
/// ([`crate::ts_hls`]) — must pass the *same* [`TsContinuity`] to every segment:
/// HLS media segments without an `#EXT-X-DISCONTINUITY` between them form one
/// continuous transport stream, so a client concatenating them sees a CC jump
/// on every PID at each boundary otherwise (TR 101 290 indicator 1.4), and this
/// crate's own [`TsDemux`](crate::TsDemux) reports `InputDegraded` on it.
pub(crate) fn mux_tracks_at_continuing(
    tracks: &[Track],
    samples: &[&[Sample]],
    base_dts_ticks: &[u64],
    cc: &mut TsContinuity,
) -> Result<Vec<u8>> {
    debug_assert_eq!(tracks.len(), base_dts_ticks.len());
    let times: Vec<Vec<i64>> = samples
        .iter()
        .zip(base_dts_ticks)
        .map(|(ss, &base)| {
            let mut t = i64::try_from(base).unwrap_or(i64::MAX);
            ss.iter()
                .map(|s| {
                    let now = t;
                    t = t.saturating_add(i64::from(s.duration.unwrap_or(0)));
                    now
                })
                .collect()
        })
        .collect();
    let times: Vec<&[i64]> = times.iter().map(Vec::as_slice).collect();
    mux_tracks_timed_with_cc(tracks, samples, &times, cc)
}

/// Per-PID transport-stream continuity-counter state
/// (`continuity_counter`, ISO/IEC 13818-1 §2.4.3.3), carried across successive
/// `mux_tracks_at_continuing` calls so a multi-segment output is one
/// continuous transport stream.
///
/// A PID is identified by its 13-bit value; the counter is the value the *next*
/// payload-bearing packet on that PID must carry. PSI PIDs (`PAT_PID` /
/// `PMT_PID`) are included.
///
/// Any change to the set of elementary streams (a different track list) between
/// calls is fine: a PID that did not exist before starts at 0, and one that is
/// gone is simply never used again.
#[derive(Debug, Default, Clone)]
pub struct TsContinuity {
    /// Indexed by PID (`0..=0x1FFF`); `None` until the PID's first payload.
    counters: Vec<Option<u8>>,
}

impl TsContinuity {
    /// A fresh state: every PID's next packet carries CC 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// The CC the next payload-bearing packet on `pid` must carry.
    fn next_for(&self, pid: u16) -> u8 {
        self.counters
            .get(pid as usize)
            .copied()
            .flatten()
            .unwrap_or(0)
    }

    /// Record that a payload-bearing packet with counter `cc` was written on
    /// `pid`; the next one takes `cc + 1` modulo 16 (§2.4.3.3).
    fn advance(&mut self, pid: u16, cc: u8) {
        self.set_next_for(pid, (cc + 1) & 0x0F);
    }

    /// Set the counter the next payload-bearing packet on `pid` must carry.
    fn set_next_for(&mut self, pid: u16, cc: u8) {
        let idx = pid as usize;
        if self.counters.len() <= idx {
            self.counters.resize(idx + 1, None);
        }
        self.counters[idx] = Some(cc & 0x0F);
    }
}

/// The whole-`Media` body of [`mux_tracks`]: `dts_ticks[i][j]` is the decode
/// time of `samples[i][j]` in `tracks[i]`'s timescale, relative to the stream's
/// origin (negative values clamp to the origin).
fn mux_tracks_timed(
    tracks: &[Track],
    samples: &[&[Sample]],
    dts_ticks: &[&[i64]],
) -> Result<Vec<u8>> {
    let mut cc = TsContinuity::default();
    mux_tracks_timed_with_cc(tracks, samples, dts_ticks, &mut cc)
}

/// The shared body of [`mux_tracks`] and [`mux_tracks_at_continuing`], threading a
/// [`TsContinuity`] across calls. `dts_ticks[i][j]` is the decode time of
/// `samples[i][j]` in `tracks[i]`'s timescale, relative to the stream's origin
/// (negative values clamp to the origin).
fn mux_tracks_timed_with_cc(
    tracks: &[Track],
    samples: &[&[Sample]],
    dts_ticks: &[&[i64]],
    continuity: &mut TsContinuity,
) -> Result<Vec<u8>> {
    debug_assert_eq!(tracks.len(), samples.len());
    debug_assert_eq!(tracks.len(), dts_ticks.len());

    // ── 1. Plan the elementary streams (PID + stream_type + framing) ──
    let (plans, planned_idx) = plan_elementary_streams(tracks)?;

    // PCR PID (§2.4.3.4 / §2.4.2.2): the first video ES; else the first
    // *continuous* audio ES (its PES packets arrive at the codec frame rate —
    // ≤ 100 ms apart — so a PCR anchored to it meets the repetition bound);
    // else the first non-section-carried ES (a section-carried Data stream is
    // packetised without an adaptation field at all — issue #576 — so it can
    // never itself carry the PCR); else (only if every ES is section-carried)
    // the first ES regardless.
    //
    // An opaque PES-carried `Data` stream (DVB subtitles / teletext) must not be
    // chosen while any audio ES exists: its PES packets appear only when a
    // subtitle event does, seconds apart, far beyond the §2.4.2.2 100 ms bound
    // (TR 101 290 indicator 2.3), and decoders lose clock lock (audit r05-W20).
    let pcr_pid = plans
        .iter()
        .find(|p| p.kind.is_video())
        .or_else(|| plans.iter().find(|p| p.kind.is_continuous_audio()))
        .or_else(|| plans.iter().find(|p| !p.kind.is_section_carried()))
        .map(|p| p.pid)
        .unwrap_or(plans[0].pid);

    // ── 2. Build the PSI (PAT + PMT) and packetise it first (PUSI order) ──
    let mut out: Vec<u8> = Vec::new();
    let pat = build_pat_section(PMT_PID)?;
    let pat_pkts = packetise_section(PAT_PID, &pat, continuity.next_for(PAT_PID));
    let pat_last_cc = pat_pkts.last().map(|p| p[3] & 0x0F).unwrap_or(0);
    for pkt in pat_pkts {
        out.extend_from_slice(&pkt);
    }
    continuity.advance(PAT_PID, pat_last_cc);
    let pmt = build_pmt_section(pcr_pid, &plans)?;
    let pmt_pkts = packetise_section(PMT_PID, &pmt, continuity.next_for(PMT_PID));
    let pmt_last_cc = pmt_pkts.last().map(|p| p[3] & 0x0F).unwrap_or(0);
    for pkt in pmt_pkts {
        out.extend_from_slice(&pkt);
    }
    continuity.advance(PMT_PID, pmt_last_cc);

    // ── 3. Elementary-stream PES → TS packets, tagged by DTS ──
    // Base DTS = PCR_LEAD_TICKS so the first PCR (DTS − lead) is non-negative.
    // Sized from the sample bytes (a PES packet adds a header, and adaptation
    // stuffing fills the last packet, hence the per-sample slack) so the vector
    // does not repeatedly double-and-copy its way up to the whole output
    // (audit r05-O4). A hint only: an under-estimate just grows as before.
    let packet_hint: usize = planned_idx
        .iter()
        .map(|&i| {
            samples[i]
                .iter()
                .map(|s| s.data.len() / TS_PAYLOAD_CAPACITY + PACKETS_PER_SAMPLE_SLACK)
                .sum::<usize>()
        })
        .sum();
    let mut tagged: Vec<TaggedPacket> = Vec::with_capacity(packet_hint);
    // Sort keys of the packets that already carry a PCR, ascending.
    let mut pcr_stamps: Vec<u64> = Vec::new();
    for (plan, &track_idx) in plans.iter().zip(&planned_idx) {
        let track = &tracks[track_idx];
        let ts_scale = track.spec.timescale.max(1) as u64;
        // Interleave keys only ever grow within one track, so the global sort
        // never reorders a track's own packets (issue #576).
        let mut last_key: u64 = 0;
        // The CC this PID's next packet carries, read from (and, after each
        // sample, written back to) the cross-call continuity state (§2.4.3.3).
        let mut cc: u8 = continuity.next_for(plan.pid);
        // Section-carried Data samples are already whole PSI/private
        // sections (issue #576) — packetised directly, never PES-wrapped.
        // The packetiser's own continuity_counter (independent of `cc`
        // above, which only tracks the PES path) persists across samples.
        let mut section_packetiser = SectionPacketiser::with_continuity(plan.pid, cc);
        for (sample, &dts_local) in samples[track_idx].iter().zip(dts_ticks[track_idx]) {
            // Rescale the sample's decode/composition time to the 90 kHz TS
            // clock. composition_offset is (pts − dts) in the track scale.
            let dts_ticks_local = dts_local.max(0) as u64;
            let dts90 = rescale(dts_ticks_local, ts_scale) + PCR_LEAD_TICKS;
            // The interleave key is NOT wrapped at the 33-bit field (§2.4.3.7)
            // the way `dts90` is, so a long track's own packets never reorder
            // against each other past the wrap point.
            let sort_key = rescale_for_ordering(dts_ticks_local, ts_scale).max(last_key);
            last_key = sort_key;

            if plan.kind.is_section_carried() {
                for pkt in section_packetiser.packetise(&[&sample.data[..]]) {
                    tagged.push(TaggedPacket {
                        sort_key,
                        packet: pkt,
                    });
                }
            } else {
                // media plane step 2c: `composition_offset()` derives PTS−DTS
                // from the sample's own absolute `dts`/`pts` when both are
                // known (§0 invariant), falling back to `0` — identical to
                // the old stored field's value for every real (non-`None`)
                // sample this muxer ever sees.
                let pts_local = dts_local.max(0) + sample.composition_offset() as i64;
                let pts90 = rescale_signed(pts_local, ts_scale) + PCR_LEAD_TICKS;
                let es_payload = build_es_payload(plan, sample)?;
                let carry_pcr = plan.pid == pcr_pid;
                packetise_pes(
                    plan,
                    &es_payload,
                    pts90,
                    dts90,
                    carry_pcr,
                    &mut cc,
                    sort_key,
                    &mut tagged,
                )?;
                if carry_pcr {
                    pcr_stamps.push(sort_key);
                }
            }
        }
        // Hand the PID's next CC to the cross-call state (§2.4.3.3). A
        // section-carried PID's packets come from the packetiser's own counter;
        // a PES PID's from `cc`, which `packetise_pes` advanced.
        let next_cc = if plan.kind.is_section_carried() {
            section_packetiser.continuity_counter()
        } else {
            cc
        };
        continuity.set_next_for(plan.pid, next_cc);
    }

    // ── 4. Fill PCR gaps on the PCR PID with PCR-only packets ──
    // §2.4.2.2 bounds the interval between two consecutive PCRs to 100 ms
    // (TR 101 290 indicator 2.3 flags a repeat beyond 40 ms as an error, so that
    // is the interval this muxer targets). A sparse PCR PID — a low-bitrate
    // audio ES, or a stream whose only audio is a few frames apart — would
    // otherwise leave decoders without a clock reference between its PES
    // packets. A PCR-only packet carries an adaptation field with the PCR and
    // *no* payload, so it adds no data to the elementary stream.
    emit_pcr_only_packets(pcr_pid, &mut pcr_stamps, &mut tagged);

    // ── 5. Interleave ES packets by decode order (stable) and append ──
    tagged.sort_by_key(|t| t.sort_key);
    out.reserve_exact(tagged.len() * TS_PACKET_SIZE);
    for t in &tagged {
        out.extend_from_slice(&t.packet);
    }

    debug_assert_eq!(out.len() % TS_PACKET_SIZE, 0);
    Ok(out)
}

/// The PCR-only adaptation field's content length, in bytes after the
/// `adaptation_field_length` byte: the 1-byte flags field + the 6-byte PCR
/// (§2.4.3.4 / §2.4.3.5). No payload follows, so the packet carries
/// `adaptation_field_control` `10`.
const PCR_ONLY_AF_LEN: usize = 1 + PCR_FIELD_LEN;

/// Maximum interval between two consecutive PCRs of one PID, in 90 kHz ticks.
/// ISO/IEC 13818-1 §2.4.2.2 allows up to 100 ms; TR 101 290 indicator 2.3
/// flags a repeat beyond 40 ms as an error, so this muxer targets the stricter
/// bound. Used only to decide where a PCR-only packet is needed.
const MAX_PCR_INTERVAL_TICKS: u64 = PCR_INTERVAL_MS * TS_CLOCK_HZ / 1000;

/// See [`MAX_PCR_INTERVAL_TICKS`].
const PCR_INTERVAL_MS: u64 = 40;

/// Insert PCR-only TS packets on `pcr_pid` wherever the gap between two
/// consecutive PCRs exceeds [`MAX_PCR_INTERVAL_TICKS`], so the §2.4.2.2
/// repetition bound holds even when the PCR PID's own PES packets are sparse.
///
/// `pcr_stamps` holds, ascending, the interleave sort keys of the packets that
/// already carry a PCR. Each inserted packet is a TS packet with an adaptation
/// field carrying the PCR and `payload_flag = 0` (§2.4.3.3: such a packet does
/// **not** increment the continuity counter, so the CC state of `pcr_pid` is
/// unchanged), placed at the gap's midpoint key so it interleaves in order.
fn emit_pcr_only_packets(pcr_pid: u16, pcr_stamps: &mut [u64], tagged: &mut Vec<TaggedPacket>) {
    if pcr_stamps.is_empty() {
        return;
    }
    pcr_stamps.sort_unstable();
    let mut insertions: Vec<u64> = Vec::new();
    for w in pcr_stamps.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b - a <= MAX_PCR_INTERVAL_TICKS {
            continue;
        }
        // One PCR-only packet every MAX_PCR_INTERVAL_TICKS, starting one interval
        // after `a` so no gap in the result exceeds the bound. A packet's
        // interleave key and its PCR are the same 90 kHz decode time here (the
        // real packets' PCR is their sample's rescaled DTS, which is exactly
        // their sort key), so both use `t`.
        let mut t = a + MAX_PCR_INTERVAL_TICKS;
        while t < b {
            insertions.push(t);
            t += MAX_PCR_INTERVAL_TICKS;
        }
    }
    if insertions.is_empty() {
        return;
    }
    // A PCR-only packet carries no payload, so §2.4.3.3 says it does not
    // increment the PID's continuity counter: it must repeat the CC of the last
    // payload-bearing packet on that PID *before it*. Collect the (sort key, CC)
    // of this PID's existing packets, ascending, so each insertion can look its
    // own predecessor up.
    let mut cc_at: Vec<(u64, u8)> = tagged
        .iter()
        .filter(|tp| pid_of_packet(&tp.packet) == pcr_pid)
        .map(|tp| (tp.sort_key, tp.packet[3] & 0x0F))
        .collect();
    cc_at.sort_by_key(|&(k, _)| k);

    for t in insertions {
        let cc = cc_at
            .iter()
            .rfind(|&&(k, _)| k <= t)
            .map(|&(_, cc)| cc)
            .unwrap_or(0);
        let mut pkt = [STUFFING_BYTE; TS_PACKET_SIZE];
        // A real packet stamps the PCR `PCR_LEAD_TICKS` behind its DTS, so pass
        // `t + PCR_LEAD_TICKS` to `pcr_for` to land the PCR exactly on `t`.
        write_pcr_only_packet(&mut pkt, pcr_pid, cc, pcr_for(t + PCR_LEAD_TICKS));
        tagged.push(TaggedPacket {
            sort_key: t,
            packet: pkt,
        });
    }
}

/// Read the 13-bit PID from a TS packet's header.
fn pid_of_packet(pkt: &[u8; TS_PACKET_SIZE]) -> u16 {
    (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16
}

/// Write a PCR-only TS packet: an adaptation field with the PCR and no payload
/// (`adaptation_field_control` `10`). §2.4.3.4 / §2.4.3.5.
fn write_pcr_only_packet(pkt: &mut [u8; TS_PACKET_SIZE], pid: u16, cc: u8, pcr: Pcr) {
    let hdr = TsHeader {
        tei: false,
        pusi: false,
        pid,
        scrambling: 0,
        has_adaptation: true,
        has_payload: false,
        continuity_counter: cc,
    };
    hdr.serialize_into(&mut pkt[..4]).expect("4-byte TS header");
    pkt[4] = PCR_ONLY_AF_LEN as u8;
    pkt[5] = AF_PCR_FLAG;
    pkt[6..6 + PCR_FIELD_LEN].copy_from_slice(&pcr.to_field_bytes());
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

/// Build the elementary-stream PES payload for one sample:
/// AVC/HEVC → length-prefixed NAL back to Annex B (prepending parameter
/// sets/AUD only when absent so the stream stays self-decodable); AAC → the
/// raw frame re-wrapped in an ADTS header; MPEG-2 video, MPEG-1/2 audio, other
/// audio, and a PES-carried opaque [`CodecConfig::Data`] sample (issue #576)
/// → the raw frame/payload verbatim (already self-framed byte streams — no
/// NAL length-prefixing to undo). Never called for a section-carried
/// `EsKind::Data` — those samples are whole PSI sections, packetised directly
/// by the caller instead ([`SectionPacketiser`]).
fn build_es_payload(plan: &EsPlan, sample: &Sample) -> Result<Vec<u8>> {
    match plan.kind {
        EsKind::Avc => {
            let au = build_annexb_au(&sample.data, sample.flags.is_sync, &plan.avc_sps_pps)?;
            ensure_avc_aud(au)
        }
        EsKind::Hevc => {
            let au =
                build_hevc_annexb_au(&sample.data, sample.flags.is_sync, &plan.hevc_vps_sps_pps)?;
            ensure_hevc_aud(au)
        }
        EsKind::Aac => {
            let asc = plan
                .asc
                .as_ref()
                .ok_or(Error::InvalidInput("AAC ES has no AudioSpecificConfig"))?;
            let frame_len_usize = sample.data.len() + 7; // 7-byte ADTS header
            let frame_len: u16 = frame_len_usize
                .try_into()
                .map_err(|_| Error::InvalidValue {
                    field: "frame_len",
                    value: frame_len_usize as u64,
                    reason: "exceeds ADTS 13-bit frame_length maximum (8191)",
                })?;
            let header = asc.to_adts_header(frame_len)?;
            let mut out = Vec::with_capacity(header.len() + sample.data.len());
            out.extend_from_slice(&header);
            out.extend_from_slice(&sample.data);
            Ok(out)
        }
        EsKind::Mpeg2Video
        | EsKind::MpegAudio { .. }
        | EsKind::Ac3
        | EsKind::Eac3
        | EsKind::Dts
        | EsKind::MpegH
        | EsKind::Data { .. } => Ok(sample.data.to_vec()),
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

/// Convert a length-prefixed HEVC video sample to an Annex B access unit,
/// prepending the parameter sets `vps_sps_pps` (VPS + SPS + PPS, in that AU
/// order — ITU-T H.265 §7.4.2.1) to an `is_sync` (IRAP keyframe) access unit
/// that does not already carry an SPS, so the TS video AU is independently
/// decodable (issue #627).
///
/// Mirrors [`build_annexb_au`] for HEVC's 2-byte NAL header (ITU-T H.265
/// §7.3.1.2) and `nal_unit_type` classification instead of AVC's 1-byte
/// header: when the IR sample already carries its parameter sets in-band, the
/// length↔Annex B round-trip stays byte-identical NAL-for-NAL.
fn build_hevc_annexb_au(
    length_prefixed: &[u8],
    is_sync: bool,
    vps_sps_pps: &[Vec<u8>],
) -> Result<Vec<u8>> {
    let nals = iter_length_prefixed_nals(length_prefixed)?;

    let needs_params = is_sync
        && !vps_sps_pps.is_empty()
        && !nals
            .iter()
            .any(|n| nal_unit_type(NalCodec::Hevc, n) == Some(HEVC_NAL_SPS));

    if !needs_params {
        // Straight, byte-exact rewrite of the existing NAL sequence.
        return length_prefixed_to_annexb(length_prefixed);
    }

    // Insert the parameter sets after a leading AUD (if any), before the slices.
    let mut out = Vec::with_capacity(length_prefixed.len() + total_param_len(vps_sps_pps));
    let mut inserted = false;
    for nal in &nals {
        let nal_type = nal_unit_type(NalCodec::Hevc, nal);
        // Emit the parameter sets right before the first non-AUD NAL.
        if !inserted && nal_type != Some(HEVC_NAL_AUD) {
            append_param_sets(&mut out, vps_sps_pps);
            inserted = true;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
    }
    if !inserted {
        // Access unit was only an AUD (degenerate) — still emit the params.
        append_param_sets(&mut out, vps_sps_pps);
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

/// 4-byte Annex B start code.
const ANNEXB_START_CODE: [u8; 4] = [0, 0, 0, 1];

/// Ensure the first Annex B NAL of `au` is an AVC Access Unit Delimiter,
/// prepending the canonical `00 00 00 01 09 F0` when it is not.
///
/// H.222.0 §2.14.1 constrains an AVC stream carried in a transport stream: "Each
/// AVC access unit shall contain an access unit delimiter NAL Unit", and H.264
/// §7.4.1.2.3 requires that AUD, when present, is the *first* NAL of the access
/// unit. An IR sample sourced from fMP4/MKV/FLV/RTMP never carries one, and a
/// `TsDemux`-sourced sample carries it only if the source stream did, so the
/// delimiter is synthesised here rather than assumed.
///
/// `primary_pic_type` is 7 (Table 7-5): slice types 0..9 may be present, i.e.
/// a picture of any type — the safe choice when the sample's slice types are
/// not inspected. The byte is `primary_pic_type`(3) + `rbsp_trailing_bits`.
fn ensure_avc_aud(mut au: Vec<u8>) -> Result<Vec<u8>> {
    if first_avc_nal_type(&au) == Some(H264_NAL_AUD) {
        return Ok(au);
    }
    let mut out = Vec::with_capacity(ANNEXB_START_CODE.len() + 2 + au.len());
    out.extend_from_slice(&ANNEXB_START_CODE);
    out.extend_from_slice(&[H264_NAL_AUD_BYTE, H264_AUD_PRIMARY_PIC_TYPE]);
    out.append(&mut au);
    Ok(out)
}

/// Ensure the first Annex B NAL of `au` is an HEVC Access Unit Delimiter,
/// prepending the canonical `00 00 00 01 46 01 50` when it is not.
///
/// H.222.0 §2.17.1: "Each HEVC access unit shall contain an access unit
/// delimiter NAL unit", which H.265 §7.4.2.4 requires to be the first NAL of
/// the access unit. `pic_type` is 2 ("I only", H.265 Table 7-4) — any picture of
/// an HEVC IR sample is an IRAP or a P/B slice, and `2` is the conservative
/// value a decoder accepts for all of them.
fn ensure_hevc_aud(mut au: Vec<u8>) -> Result<Vec<u8>> {
    if first_hevc_nal_type(&au) == Some(HEVC_NAL_AUD) {
        return Ok(au);
    }
    let mut out = Vec::with_capacity(ANNEXB_START_CODE.len() + 3 + au.len());
    out.extend_from_slice(&ANNEXB_START_CODE);
    out.extend_from_slice(&[HEVC_AUD_FIRST_BYTE, HEVC_AUD_SECOND_BYTE, HEVC_AUD_PIC_TYPE]);
    out.append(&mut au);
    Ok(out)
}

/// The AVC `nal_unit_type` of the first NAL in an Annex B buffer, or `None`
/// when the buffer is empty (a zero-length sample has no NAL at all).
fn first_avc_nal_type(annexb: &[u8]) -> Option<u8> {
    annexb_first_nal(annexb).and_then(|n| n.first().map(|b| b & H264_NAL_TYPE_MASK))
}

/// The HEVC `nal_unit_type` of the first NAL in an Annex B buffer (2-byte NAL
/// header, H.265 §7.3.1.2), or `None` when the buffer is empty.
fn first_hevc_nal_type(annexb: &[u8]) -> Option<u8> {
    annexb_first_nal(annexb).and_then(|n| nal_unit_type(NalCodec::Hevc, n))
}

/// Slice the first Annex B NAL (after its start code) out of `annexb`.
/// Accepts both the 4-byte and the 3-byte start code a source stream may use.
fn annexb_first_nal(annexb: &[u8]) -> Option<&[u8]> {
    let skip = if annexb.starts_with(&ANNEXB_START_CODE) {
        ANNEXB_START_CODE.len()
    } else if annexb.starts_with(&[0, 0, 1]) {
        3
    } else {
        0
    };
    let rest = annexb.get(skip..)?;
    let end = rest
        .windows(3)
        .position(|w| w == [0, 0, 1])
        .unwrap_or(rest.len());
    // A 4-byte start code's trailing zero is left at the end of the slice.
    let end = if rest.get(end.wrapping_sub(1)) == Some(&0) && end > 0 {
        end - 1
    } else {
        end
    };
    rest.get(..end)
}

/// Build a PAT section (one program → `pmt_pid`) with its trailing CRC_32.
/// ISO/IEC 13818-1 §2.4.4.3.
fn build_pat_section(pmt_pid: u16) -> Result<Vec<u8>> {
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
/// Each ES's `ES_info` descriptor loop carries its already-merged
/// [`EsPlan::descriptors`] verbatim — every track kind, not only an opaque
/// [`EsKind::Data`] one, per the module doc's ES_info passthrough policy
/// (issue #775) — so a receiver can recover e.g. a DVB subtitling/teletext
/// descriptor (issue #576) or an audio track's language (issue #775) after a
/// re-mux. `program_info` stays empty (no program-level descriptors are
/// modelled).
fn build_pmt_section(pcr_pid: u16, plans: &[EsPlan]) -> Result<Vec<u8>> {
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
///
/// # Errors
///
/// Returns [`Error::BufferCapExceeded`] if `body.len() + CRC32_LEN` exceeds
/// [`MAX_SECTION_LENGTH`] — checked before narrowing, so a PMT this large
/// never silently wraps the 12-bit `section_length` field (past 4095) or
/// emits an out-of-spec section (1022..=4095, §2.4.4.4).
fn finish_section(table_id: u8, body: Vec<u8>) -> Result<Vec<u8>> {
    // section_length counts everything after the 3-byte prefix, i.e. the body
    // (which already includes table_id_extension etc.) plus the 4-byte CRC.
    let section_length = body.len() + CRC32_LEN;
    if section_length > MAX_SECTION_LENGTH {
        return Err(Error::BufferCapExceeded {
            what: "PSI section_length",
            cap: MAX_SECTION_LENGTH,
        });
    }
    let mut section = Vec::with_capacity(3 + section_length);
    section.push(table_id);
    section.push(SECTION_SYNTAX_FLAGS_HI | ((section_length >> 8) as u8 & SECTION_LENGTH_HI_MASK));
    section.push((section_length & 0xFF) as u8);
    section.extend_from_slice(&body);
    let crc = crc32_mpeg2::compute(&section);
    section.extend_from_slice(&crc.to_be_bytes());
    Ok(section)
}

/// Packetise one complete PSI section into 188-byte TS packets on `pid`.
/// A single PUSI packet with a `pointer_field = 0` prefix, 0xFF-stuffed
/// (all this crate's sections fit one packet); multi-packet continuation is
/// handled by the generic loop for safety. ISO/IEC 13818-1 §2.4.4.
fn packetise_section(pid: u16, section: &[u8], start_cc: u8) -> Vec<[u8; TS_PACKET_SIZE]> {
    let mut packets = Vec::new();
    let mut cc: u8 = start_cc & 0x0F;
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

/// Packetise one PES payload (already framed as its `stream_id` payload) into
/// 188-byte TS packets on `plan.pid`, appended to `tagged` (each tagged with
/// `sort_key`, the interleave-order key — see [`rescale_for_ordering`], NOT
/// the on-wire `dts90`). The first packet sets PUSI and — when `carry_pcr` —
/// an adaptation field with the PCR; the final packet is stuffed via an
/// adaptation field so the PES ends exactly on a packet boundary.
/// ISO/IEC 13818-1 §2.4.3.
#[allow(clippy::too_many_arguments)]
fn packetise_pes(
    plan: &EsPlan,
    es_payload: &[u8],
    pts90: u64,
    dts90: u64,
    carry_pcr: bool,
    cc: &mut u8,
    sort_key: u64,
    tagged: &mut Vec<TaggedPacket>,
) -> Result<()> {
    let pes = build_pes_bytes(plan, es_payload, pts90, dts90)?;

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
    Ok(())
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
        if has_pcr && let Some(p) = pcr {
            pkt[6..6 + PCR_FIELD_LEN].copy_from_slice(&p.to_field_bytes());
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
///
/// # Errors
///
/// Returns [`Error::BufferCapExceeded`] for a non-video payload whose
/// `PES_packet_length` would exceed 65535 (§2.4.3.7). `PES_packet_length = 0`
/// ("unbounded") is defined only for video, so a longer audio/data access unit
/// cannot be framed at all — clamping the field would emit a packet that
/// claims to end 64 KiB short of its payload, and every demuxer would truncate
/// the PES and read the remainder as garbage. A caller that needs to carry a
/// larger unit must split it into several PES packets itself (e.g. one per
/// audio frame), which this function cannot do because it does not know the
/// unit's internal framing.
fn build_pes_bytes(plan: &EsPlan, es_payload: &[u8], pts90: u64, dts90: u64) -> Result<Vec<u8>> {
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
        u16::try_from(after_len).map_err(|_| Error::BufferCapExceeded {
            what: "PES_packet_length",
            cap: u16::MAX as usize,
        })?
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
    Ok(out)
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
        assert_eq!(EsKind::Hevc.stream_type(), 0x24);
        assert_eq!(EsKind::Mpeg2Video.stream_type(), 0x02);
        assert_eq!(EsKind::Aac.stream_type(), 0x0F);
        assert_eq!(
            EsKind::MpegAudio { is_mpeg2: false }.stream_type(),
            0x03,
            "MPEG-1 audio"
        );
        assert_eq!(
            EsKind::MpegAudio { is_mpeg2: true }.stream_type(),
            0x04,
            "MPEG-2 audio"
        );
        assert_eq!(EsKind::Ac3.stream_type(), 0x81);
        assert_eq!(EsKind::Eac3.stream_type(), 0x87);
        assert_eq!(EsKind::Dts.stream_type(), 0x82);
        assert_eq!(EsKind::MpegH.stream_type(), 0x2D);
        assert!(EsKind::Avc.is_video());
        assert!(EsKind::Hevc.is_video());
        assert!(EsKind::Mpeg2Video.is_video());
        assert!(!EsKind::Aac.is_video());
        assert!(!EsKind::MpegAudio { is_mpeg2: false }.is_video());
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
        let pat = build_pat_section(PMT_PID).unwrap();
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
            hevc_vps_sps_pps: Vec::new(),
            descriptors: Vec::new(),
        }];
        let pmt = build_pmt_section(ES_PID_BASE, &plans).unwrap();
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
            hevc_vps_sps_pps: Vec::new(),
            descriptors: descriptors.clone(),
        }];
        let pmt = build_pmt_section(ES_PID_BASE, &plans).unwrap();
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
        let pat = build_pat_section(PMT_PID).unwrap();
        let pkts = packetise_section(PAT_PID, &pat, 0);
        assert_eq!(pkts.len(), 1);
        // sync byte + PUSI bit.
        assert_eq!(pkts[0][0], 0x47);
        assert_ne!(pkts[0][1] & 0x40, 0, "PUSI must be set on the first packet");
    }

    #[test]
    fn mpegh_3daudio_descriptor_carries_profile_level() {
        let bytes = mpegh_3daudio_descriptor(0x0B);
        assert_eq!(
            bytes,
            alloc::vec![
                DESCRIPTOR_TAG_EXTENSION,
                MPEGH_3DAUDIO_DESCRIPTOR_BODY_LEN,
                MPEGH_3DAUDIO_EXTENSION_TAG,
                0x0B,
            ]
        );
        // Mutating the profile-level must change the descriptor bytes (not a
        // fixed/cached template).
        assert_ne!(bytes, mpegh_3daudio_descriptor(0x10));
    }

    // ── ES_info descriptor passthrough policy (issue #775) ──────────────────

    #[test]
    fn merge_es_info_descriptors_denies_ca_and_preserves_order() {
        // ISO_639_language_descriptor (tag 0x0A), then a CA_descriptor (tag
        // 0x09) sandwiched in the middle, then a private descriptor (tag
        // 0x88) — proves the CA tag is excised out of the middle of the
        // loop (not just truncated off the end) and the survivors keep
        // their original relative order.
        let lang = alloc::vec![
            DESCRIPTOR_TAG_ISO_639_LANGUAGE_FOR_TEST,
            0x04,
            b'e',
            b'n',
            b'g',
            0x00
        ];
        let ca = alloc::vec![DESCRIPTOR_TAG_CA, 0x04, 0x00, 0x01, 0x00, 0x82];
        let private = alloc::vec![0x88u8, 0x02, 0xAA, 0xBB];
        let mut inherited = lang.clone();
        inherited.extend_from_slice(&ca);
        inherited.extend_from_slice(&private);

        let merged = merge_es_info_descriptors(&inherited, &[]).expect("no overflow");

        let mut expected = lang;
        expected.extend_from_slice(&private);
        assert_eq!(
            merged, expected,
            "CA_descriptor must be dropped; siblings survive in source order"
        );
    }

    /// Local alias so the test above doesn't depend on a demux-side constant
    /// this module doesn't otherwise need — ISO_639_language_descriptor's
    /// tag (ETSI EN 300 468 §6.2.28) is `0x0A`.
    const DESCRIPTOR_TAG_ISO_639_LANGUAGE_FOR_TEST: u8 = 0x0A;

    #[test]
    fn merge_es_info_descriptors_dedups_synthesized_tag_favouring_synthesized_bytes() {
        // A stale inherited MPEG-H_3dAudio_descriptor-shaped entry (tag 0x3F,
        // same extension_descriptor_tag 0x08, but a different — wrong —
        // profile-level byte), sandwiched between two unrelated survivors.
        let before = alloc::vec![0x88u8, 0x01, 0x01];
        let stale_mpegh = alloc::vec![
            DESCRIPTOR_TAG_EXTENSION,
            MPEGH_3DAUDIO_DESCRIPTOR_BODY_LEN,
            MPEGH_3DAUDIO_EXTENSION_TAG,
            0xFF, // stale/wrong profile-level
        ];
        let after = alloc::vec![0x89u8, 0x01, 0x02];
        let mut inherited = before.clone();
        inherited.extend_from_slice(&stale_mpegh);
        inherited.extend_from_slice(&after);

        let synthesized = alloc::vec![mpegh_3daudio_descriptor(0x0B)];
        let merged = merge_es_info_descriptors(&inherited, &synthesized).expect("no overflow");

        // Exactly one occurrence of the extension tag, holding the
        // synthesized (grounded-in-CodecConfig) bytes, not the stale
        // inherited ones; the two unrelated siblings both survive in order,
        // and the synthesized descriptor is appended after them.
        let mut expected = before;
        expected.extend_from_slice(&after);
        expected.extend_from_slice(&mpegh_3daudio_descriptor(0x0B));
        assert_eq!(
            merged, expected,
            "inherited duplicate of a synthesized tag must be dropped, keeping only the \
             synthesized copy, appended after the surviving unrelated siblings"
        );
        assert_eq!(
            merged
                .iter()
                .enumerate()
                .filter(|&(i, &b)| b == DESCRIPTOR_TAG_EXTENSION && i + 1 < merged.len())
                .count(),
            1,
            "extension_descriptor tag (0x3F) must appear exactly once in the merged loop"
        );
    }

    #[test]
    fn merge_es_info_descriptors_keeps_distinct_extension_descriptor_tag_siblings() {
        // Two `extension_descriptor` (0x3F) entries that share only the
        // outer tag: the inherited one carries a *different* second-level
        // extension_descriptor_tag (0x15) than the synthesized MPEG-H one
        // (0x08, `MPEGH_3DAUDIO_EXTENSION_TAG`). Keying dedup on 0x3F alone
        // (the pre-R4 behaviour) would wrongly collapse these two unrelated
        // registrations onto one; keyed on `(tag, extension_descriptor_tag)`
        // both must survive.
        const OTHER_EXTENSION_TAG_FOR_TEST: u8 = 0x15;
        let other_extension = alloc::vec![
            DESCRIPTOR_TAG_EXTENSION,
            0x02, // body length
            OTHER_EXTENSION_TAG_FOR_TEST,
            0xAB, // arbitrary payload byte
        ];
        let inherited = other_extension.clone();

        let synthesized = alloc::vec![mpegh_3daudio_descriptor(0x0B)];
        let merged = merge_es_info_descriptors(&inherited, &synthesized).expect("no overflow");

        let mut expected = other_extension;
        expected.extend_from_slice(&mpegh_3daudio_descriptor(0x0B));
        assert_eq!(
            merged, expected,
            "two extension_descriptor entries with distinct extension_descriptor_tag values \
             are not duplicates and must both survive"
        );
    }

    #[test]
    fn merge_es_info_descriptors_overflow_returns_typed_error_not_truncated() {
        // 17 descriptors of 255 bytes each (tag + 0xFD length + 253-byte
        // body) = 17 * 255 = 4335 bytes, comfortably over the 4095-byte
        // ES_info_length cap — using a tag that is neither denied nor
        // synthesized, so every one of them would otherwise pass through.
        const BODY_LEN: u8 = 0xFD; // 253
        const N: usize = 17;
        let mut inherited = Vec::new();
        for _ in 0..N {
            inherited.push(0x80u8); // arbitrary private descriptor tag
            inherited.push(BODY_LEN);
            inherited.extend(core::iter::repeat_n(0xAAu8, BODY_LEN as usize));
        }
        assert!(
            inherited.len() > MAX_ES_INFO_LENGTH,
            "test setup must actually overflow"
        );

        let result = merge_es_info_descriptors(&inherited, &[]);
        match result {
            Err(Error::BufferCapExceeded { what, cap }) => {
                assert_eq!(what, "PMT ES_info descriptor loop");
                assert_eq!(cap, MAX_ES_INFO_LENGTH);
            }
            other => panic!(
                "expected Err(Error::BufferCapExceeded {{ .. }}) for an over-cap ES_info loop, \
                 got {other:?} — a truncated loop would be a malformed PMT"
            ),
        }
    }

    /// One ES entry with a large-but-in-cap `ES_info` loop pushes the PMT's
    /// overall `section_length` (fixed 9 bytes + this entry's 5-byte header +
    /// descriptors + 4-byte trailing CRC) to 1022 — one past the §2.4.4.4 cap
    /// of 1021 (#1129). Unfixed, `finish_section` masked the 12-bit field
    /// (`& SECTION_LENGTH_HI_MASK`) and returned `Ok` with an out-of-spec (and,
    /// past 4095, wrapped) section.
    #[test]
    fn pmt_section_length_past_1021_errors() {
        let plans = alloc::vec![EsPlan {
            pid: ES_PID_BASE,
            stream_id: StreamId(0),
            kind: EsKind::Data {
                stream_type: 0x06,
                carriage: DataCarriage::Pes,
            },
            asc: None,
            avc_sps_pps: Vec::new(),
            hevc_vps_sps_pps: Vec::new(),
            descriptors: alloc::vec![0u8; 1004],
        }];
        let err = build_pmt_section(ES_PID_BASE, &plans).unwrap_err();
        assert!(
            matches!(
                err,
                Error::BufferCapExceeded {
                    what: "PSI section_length",
                    cap: MAX_SECTION_LENGTH,
                }
            ),
            "expected BufferCapExceeded for PSI section_length, got {err:?}"
        );
    }

    /// The boundary: a PMT whose `section_length` is exactly 1021 (the
    /// §2.4.4.4 maximum) still builds and CRCs cleanly.
    #[test]
    fn pmt_section_length_at_1021_succeeds() {
        let plans = alloc::vec![EsPlan {
            pid: ES_PID_BASE,
            stream_id: StreamId(0),
            kind: EsKind::Data {
                stream_type: 0x06,
                carriage: DataCarriage::Pes,
            },
            asc: None,
            avc_sps_pps: Vec::new(),
            hevc_vps_sps_pps: Vec::new(),
            descriptors: alloc::vec![0u8; 1003],
        }];
        let pmt = build_pmt_section(ES_PID_BASE, &plans).unwrap();
        let section_length = (((pmt[1] & SECTION_LENGTH_HI_MASK) as usize) << 8) | pmt[2] as usize;
        assert_eq!(section_length, MAX_SECTION_LENGTH);
        assert_eq!(crc32_mpeg2::compute(&pmt), 0);
    }

    /// An audio PES whose `PES_packet_length` would exceed 65535 must be
    /// rejected, never clamped (audit r05-W19, §2.4.3.7): `0` ("unbounded") is
    /// defined only for video.
    #[test]
    fn audio_pes_packet_length_past_65535_errors() {
        let plan = EsPlan {
            pid: ES_PID_BASE,
            stream_id: StreamId(STREAM_ID_AUDIO_BASE),
            kind: EsKind::Ac3,
            asc: None,
            avc_sps_pps: Vec::new(),
            hevc_vps_sps_pps: Vec::new(),
            descriptors: Vec::new(),
        };
        // Largest payload that still fits: 65535 − (3 fixed + 5 PTS) = 65527.
        let fits = alloc::vec![0u8; u16::MAX as usize - HEADER_FIXED - 5];
        let ok = build_pes_bytes(&plan, &fits, 0, 0).expect("the maximum legal PES must build");
        assert_eq!(
            u16::from_be_bytes([ok[4], ok[5]]),
            u16::MAX,
            "PES_packet_length = 65535 at the boundary"
        );

        let over = alloc::vec![0u8; fits.len() + 1];
        let err = build_pes_bytes(&plan, &over, 0, 0).unwrap_err();
        assert!(
            matches!(
                err,
                Error::BufferCapExceeded {
                    what: "PES_packet_length",
                    cap: 65535,
                }
            ),
            "one byte past the cap must be an error, got {err:?}"
        );
    }

    /// Video is exempt from the 16-bit `PES_packet_length`: §2.4.3.7 allows the
    /// unbounded `0` form, so an access unit past 65535 bytes is legal.
    #[test]
    fn video_pes_packet_length_above_65535_is_unbounded() {
        let plan = EsPlan {
            pid: ES_PID_BASE,
            stream_id: StreamId(STREAM_ID_VIDEO_BASE),
            kind: EsKind::Avc,
            asc: None,
            avc_sps_pps: Vec::new(),
            hevc_vps_sps_pps: Vec::new(),
            descriptors: Vec::new(),
        };
        let big = alloc::vec![0u8; u16::MAX as usize + 1];
        let pes = build_pes_bytes(&plan, &big, 0, 0).expect("video may be unbounded");
        assert_eq!(
            u16::from_be_bytes([pes[4], pes[5]]),
            0,
            "video PES_packet_length is the unbounded 0 form"
        );
    }
}
