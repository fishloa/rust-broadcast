//! MPEG-2 Transport Stream muxer (the output TS spoke): [`TsMuxStage`].
//!
//! [`TsMuxStage`] is the **output** side of the any-to-any container hub. It
//! speaks the muxer stage vocabulary ([`MuxInput`]/[`MuxOutput`]): feed it the
//! tracks to carry (announced, or updated mid-stream) and their coded samples
//! in decode order, and it emits whole 188-byte TS packets as
//! [`MuxOutput::Bytes`]. A caller that still holds collected tracks feeds
//! it through the same decode-order interleave. It is the byte-level inverse
//! of [`StreamingTsDemux`](crate::StreamingTsDemux): a `TsMuxStage → StreamingTsDemux` round-trip recovers
//! an equivalent IR (same tracks, codec configs, coded NAL payloads, frame
//! counts, and per-sample timing).
//!
//! Pipeline: announce the tracks → assign a PID per elementary stream and a
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
//!   (`private/specs-verified/mpeg/transmux/codec/ts-demux-13818-1.md`) — 4-byte header, `adaptation_field()`
//!   carrying `PCR` (§2.4.3.5) and stuffing (§2.4.3.4).
//! - **PES header**: ISO/IEC 13818-1 §2.4.3.6 / §2.4.3.7 — `packet_start_code`
//!   `00 00 01`, `stream_id`, `PES_packet_length`, PTS/DTS (33-bit @ 90 kHz).
//! - **PAT / PMT program-specific information**: ISO/IEC 13818-1 §2.4.4.4 /
//!   §2.4.4.9 — long-form sections with a trailing `CRC_32`
//!   ([`broadcast_common::crc32_mpeg2`]).
//! - **stream_type → codec**: ISO/IEC 13818-1 Table 2-34 + ETSI TS 101 154 §G
//!   (AC-3 / E-AC-3 / DTS user-private assignments) — mirrors [`StreamingTsDemux`](crate::StreamingTsDemux).
//! - **Timeline discontinuity** ([`MuxInput::Discontinuity`]): the adaptation
//!   field's `discontinuity_indicator` (§2.4.3.5, flags-byte bit 7 read on the
//!   rendered page in
//!   `private/specs-verified/dvb/dvb-si/tables/iso_13818_1/adaptation-field-semantics.md`)
//!   rides the next packet of each affected elementary PID — the packet
//!   carrying the new time base's first `PCR` on the PCR PID — and is added by
//!   giving that packet an adaptation field when it has none. PSI packets are
//!   never marked, and the continuity counters keep counting (§2.4.3.3): the
//!   indicator is what tells a decoder the break is intended.
//!
//! # ES_info descriptor passthrough policy (issue #775)
//!
//! Every track's inherited PMT `ES_info` descriptor-loop bytes
//! ([`TrackSpec::es_info_descriptors`]; a [`CodecConfig::Data`] track's come
//! from its [`DataIdentity::Ts`] identity, so a mapped identity's descriptors
//! reach the PMT)
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
//!   (§2.4.4.9) — a truncated descriptor loop is a malformed PMT.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use broadcast_core::error::InvalidValue;
use core::time::Duration;
use mpeg_ts::pid::Pid;

#[cfg(test)]
use broadcast_common::crc32_mpeg2;
use broadcast_common::stage::{Stage, Timestamp};
use broadcast_core::Serialize;
use broadcast_core::timescale::{Rounding, Timescale, rescale_i64, rescale_u64};
use mpeg_pes::StreamId;
use mpeg_ts::mux::SectionPacketiser;
#[cfg(feature = "hls")]
use mpeg_ts::ts::PID_MASK_HI;
use mpeg_ts::ts::{
    AdaptationField, AdaptationFieldControl, CC_MASK, ContinuityCounter, Pcr, ScramblingControl,
    TS_PACKET_SIZE, TsHeader,
};

use crate::TrackId;
use crate::aac_asc::{ADTS_HEADER_SIZE, AudioSpecificConfig};
use crate::annexb::{NAL_LENGTH_SIZE, iter_length_prefixed_nals, length_prefixed_to_annexb};
use crate::codec_config_ext::EsdsBoxCodecInfo;
#[cfg(test)]
use crate::collect::Collected as Media;
use crate::error::{Error, Result};
use crate::mux_stage::{MuxInput, MuxOutput};
use crate::nal::{AVC_NAL_SPS, NAL_TYPE_MASK, NalCodec, nal_unit_type};
use crate::pipeline::{CodecConfig, DataCarriage, DataIdentity, Sample, TrackSpec};
use crate::ts_demux::DESCRIPTOR_HEADER_LEN;
use isobmff::mp4esds::{EsdsBox, OTI_MPEG1_AUDIO, OTI_MPEG2_AUDIO};
use isobmff::nalu_types::HEVC_SPS_NUT;

// ── PID / PSI constants (ISO/IEC 13818-1 §2.4.4) ────────────────────────────

/// PID carrying the Program Association Table (§2.4.4.4).
const PAT_PID: Pid = Pid::from_const(0x0000);
/// PID chosen for the single Program Map Table this muxer emits.
const PMT_PID: Pid = Pid::from_const(0x1000);
/// First elementary-stream PID; each subsequent ES gets the next value.
const ES_PID_BASE: Pid = Pid::from_const(0x0100);
/// `program_number` assigned to the single program.
const PROGRAM_NUMBER: u16 = 1;
/// `table_id` of a PAT section (§2.4.4.4, Table 2-31).
#[cfg(test)]
const TABLE_ID_PAT: u8 = 0x00;
/// `table_id` of a PMT section (§2.4.4.9, Table 2-31).
#[cfg(test)]
const TABLE_ID_PMT: u8 = 0x02;
/// Mask for the low 4 bits of the 12-bit `section_length` high byte.
#[cfg(test)]
const SECTION_LENGTH_HI_MASK: u8 = 0x0F;
/// Maximum `section_length` this crate accepts (§2.4.4.9): the 12-bit wire
/// field can encode up to 4095, but §2.4.4.4 additionally bounds every
/// section other than the private/DSM-CC forms to 1021 bytes, so a PMT past
/// this — about 20 audio tracks with language/AC-3/DVB descriptors — is
/// rejected rather than emitting an out-of-spec section or, past 4095,
/// silently wrapping the length field.
const MAX_SECTION_LENGTH: usize = 0x3FD;
/// `version_number` is a 5-bit field (§2.4.4.9), so it increments modulo 32.
const SECTION_VERSION_MODULUS: u8 = 32;
// ── stream_type → codec (ISO/IEC 13818-1 Table 2-34 + ETSI TS 101 154) ──────

/// MPEG-2 video (ITU-T H.262 / ISO/IEC 13818-2) — ISO/IEC 13818-1 Table 2-34
/// (issue #627).
const STREAM_TYPE_MPEG2_VIDEO: u8 = 0x02;
/// MPEG-1 audio (ISO/IEC 11172-3) — ISO/IEC 13818-1 Table 2-34 (issue #627).
const STREAM_TYPE_MPEG1_AUDIO: u8 = 0x03;
/// MPEG-2 audio (ISO/IEC 13818-3, LSF) — ISO/IEC 13818-1 Table 2-34 (issue
/// #627).
const STREAM_TYPE_MPEG2_AUDIO: u8 = 0x04;
/// ISO/IEC 13818-7 AAC in ADTS — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_AAC_ADTS: u8 = 0x0F;
/// AVC (H.264) video — ISO/IEC 13818-1 Table 2-34.
const STREAM_TYPE_AVC: u8 = 0x1B;
/// HEVC (H.265) video — ISO/IEC 13818-1 Table 2-34 (issue #627).
const STREAM_TYPE_HEVC: u8 = 0x24;
/// MPEG-H 3D Audio main stream (MHAS) — ISO/IEC 13818-1 Table 2-34 / ETSI
/// TS 101 154 §6.8 (issue #579).
const STREAM_TYPE_MPEGH: u8 = 0x2D;
/// AC-3 (ATSC/DVB user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_AC3: u8 = 0x81;
/// DTS (canonical DVB assignment, user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_DTS: u8 = 0x82;
/// E-AC-3 (user-private) — ETSI TS 101 154 §G.
const STREAM_TYPE_EAC3: u8 = 0x87;

/// Maximum value of the 12-bit `ES_info_length` field (§2.4.4.9).
const MAX_ES_INFO_LENGTH: usize = 0x0FFF;

/// `CA_descriptor` tag (ISO/IEC 13818-1 §2.6.16) — denied from ES_info
/// passthrough (issue #775, see the module doc's policy section): it names a
/// `CA_PID` scrambling the elementary stream, which this cleartext-output
/// muxer never has, so copying it forward would falsely signal the re-muxed
/// stream as scrambled.
const DESCRIPTOR_TAG_CA: u8 = 0x09;

// ── MPEG-H_3dAudio_descriptor (ISO/IEC 13818-1 §2.6.106, Table 2-121) ───────
//
// Syntax transcribed (verified on the rendered page, H.222.0 08/2023 PDF p.156)
// in `private/specs-verified/mpeg/transmux/codec/mpegh-3daudio-descriptor-h222.md`. The wire form is
// built by `dvb-si`'s typed `MpegH3dAudio` inside an `Mpeg2ExtensionDescriptor`
// (tag 0x3F, `extension_descriptor_tag` 0x08), not by hand-assembled bytes.

/// `extension_descriptor` tag (ISO/IEC 13818-1 Table 2-45) — the umbrella
/// descriptor under which post-2013 additions, including MPEG-H signalling,
/// register a second-level `extension_descriptor_tag`.
const DESCRIPTOR_TAG_EXTENSION: u8 = 0x3F;

/// Widest value of the 6-bit `referenceChannelLayout` field (Table 2-121).
const MPEGH_REFERENCE_CHANNEL_LAYOUT_MAX: u8 = 0x3F;
/// `referenceChannelLayout` written when the `mhaC` value does not fit the
/// 6-bit field: `0`, the "unspecified/not derived" marker (also what
/// `ts_demux` records for a TS-sourced track).
const MPEGH_REFERENCE_CHANNEL_LAYOUT_FALLBACK: u8 = 0;

/// Build the `MPEG-H_3dAudio_descriptor` ES_info entry (issue #579) from the
/// `mhaC` fields transmux holds: `mpegh3daProfileLevelIndication` and
/// `referenceChannelLayout` (6 bits in Table 2-121). An `mhaC` value above
/// `0x3F` cannot be represented and is written as `0` (unspecified) rather than
/// failing the mux or dropping the descriptor, which H.222.0 §2.6.106 requires
/// for `stream_type` 0x2D.
///
/// `interactivityEnabled` is written `0` and `compatibleProfileSetsPresent` is
/// written `1` (no `CompatibleSetIndication` list): the IR carries neither, and
/// the spec defines `0` as "no user interactivity of any kind is available".
/// The `reserved` runs are all-ones (`MpegH3dAudio::new`).
fn mpegh_3daudio_descriptor(
    profile_level_indication: u8,
    reference_channel_layout: u8,
) -> Result<Vec<u8>> {
    use broadcast_core::Serialize as _;
    use dvb_si::descriptors::mpeg2_extension::{Mpeg2ExtensionBody, Mpeg2ExtensionDescriptor};
    use dvb_si::descriptors::mpegh_3d_audio::MpegH3dAudio;

    // The `mhaC` byte is 8 bits but Table 2-121 carries 6: a wider value is
    // not a valid ISO/IEC 23001-8 ChannelConfiguration code point. The
    // descriptor SHALL be present for stream_type 0x2D (H.222.0 §2.6.106), so
    // neither omitting it nor failing the whole mux is acceptable; fall back
    // to the "unspecified" marker 0 (the same one the demuxer writes) instead.
    let reference_channel_layout = if reference_channel_layout > MPEGH_REFERENCE_CHANNEL_LAYOUT_MAX
    {
        MPEGH_REFERENCE_CHANNEL_LAYOUT_FALLBACK
    } else {
        reference_channel_layout
    };
    let body = MpegH3dAudio::new(
        profile_level_indication,
        false,
        None,
        reference_channel_layout,
    );
    Mpeg2ExtensionDescriptor::new(Mpeg2ExtensionBody::MpegH3dAudio(body))
        .try_to_bytes()
        .map_err(|_| Error::InvalidInput("MPEG-H_3dAudio_descriptor does not encode"))
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
/// 4-bit prefix on a standalone PTS field (`0010`). §2.4.3.7.
const TS_PREFIX_PTS_ONLY: u8 = 0b0010;
/// 4-bit prefix on the DTS field of a PTS+DTS pair (`0001`). §2.4.3.7.
const TS_PREFIX_DTS: u8 = 0b0001;

// ── H.264 NAL constants (ISO/IEC 14496-10 Table 7-1) ────────────────────────

/// `nal_unit_type` for an Access Unit Delimiter.
const H264_NAL_AUD: u8 = 9;
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
/// The complete AVC AUD NAL unit this muxer prepends (H.264 §7.3.2.4).
const AVC_AUD_NAL: [u8; 2] = [H264_NAL_AUD_BYTE, H264_AUD_PRIMARY_PIC_TYPE];

// ── H.265/HEVC NAL constants (ITU-T H.265 Table 7-1) — issue #627 ───────────

/// H.265 `nal_unit_type` for VPS (`VPS_NUT`) — Table 7-1 (type 32).
const HEVC_NAL_VPS: u8 = 32;
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
/// The complete HEVC AUD NAL unit this muxer prepends (H.265 §7.4.2.4).
const HEVC_AUD_NAL: [u8; 3] = [HEVC_AUD_FIRST_BYTE, HEVC_AUD_SECOND_BYTE, HEVC_AUD_PIC_TYPE];

// ── TS adaptation-field constants (ISO/IEC 13818-1 §2.4.3.4/§2.4.3.5) ───────

/// `adaptation_field_control` bit: adaptation field present.
const AF_CTRL_ADAPTATION: u8 = 0x20;
/// `adaptation_field_control` bit: payload present.
const AF_CTRL_PAYLOAD: u8 = 0x10;
/// Encoded PCR occupies 6 bytes (§2.4.3.5).
const PCR_FIELD_LEN: usize = 6;
/// Stuffing byte for unused TS/PES payload bytes (§2.4.4).
const STUFFING_BYTE: u8 = 0xFF;

/// Bytes of the fixed TS packet header before the adaptation field or payload
/// (§2.4.3.2 Table 2-4).
const TS_HEADER_LEN: usize = 4;
/// Offset of the header byte carrying `adaptation_field_control` and the
/// `continuity_counter`.
const CC_OFFSET: usize = 3;
/// Offset of `adaptation_field_length`, the first adaptation-field byte.
const AF_LENGTH_OFFSET: usize = TS_HEADER_LEN;
/// Offset of the adaptation-field flags byte.
const AF_FLAGS_OFFSET: usize = TS_HEADER_LEN + 1;

/// Bytes of a PES `PTS`/`DTS` field (§2.4.3.7).
const PES_TIMESTAMP_LEN: usize = 5;
/// Bytes of a PTS+DTS pair in the optional header (§2.4.3.7).
const PES_TIMESTAMP_PAIR_LEN: usize = PES_TIMESTAMP_LEN * 2;
/// Offset of the PID's low byte in the TS header (§2.4.3.2).
#[cfg(feature = "hls")]
const PID_LO_OFFSET: usize = 2;
/// Stamps per gap when scanning consecutive PCR stamps (`windows(2)`).
#[cfg(feature = "hls")]
const PCR_STAMP_PAIR_LEN: usize = 2;

/// Bytes a PSI `section_length` does not count: `table_id` and the 2-byte
/// `section_length` field itself (§2.4.4).
const PSI_SECTION_LENGTH_PREFIX: usize = 3;

/// Media timescale of a Transport Stream / PES clock (90 kHz).
/// The 90 kHz MPEG-2 Systems clock, both as a `Timescale` and as the `u64`
/// scaling factor the rate arithmetic below uses.
const TS_CLOCK: Timescale = broadcast_core::timescale!(90_000);
const TS_CLOCK_HZ: u64 = 90_000;

/// A PAT/PMT repetition interval in 90 kHz clock ticks, rounded down; a span
/// past the 90 kHz range's whole seconds saturates.
fn psi_interval_ticks(interval: Duration) -> u64 {
    rescale_u64(
        u64::try_from(interval.as_micros()).unwrap_or(u64::MAX),
        Timescale::MICROS,
        TS_CLOCK,
        Rounding::Floor,
    )
}
/// 33-bit PTS/DTS modulus (90 kHz clock, §2.4.3.7) — alias of
/// [`broadcast_common::clock33::WRAP_33BIT`], the shared owner.
const TS_TIMESTAMP_MOD: u64 = broadcast_common::clock33::WRAP_33BIT;
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
                identity:
                    DataIdentity::Ts {
                        stream_type,
                        carriage,
                        ..
                    },
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
#[derive(Debug, Clone)]
pub(crate) struct EsPlan {
    pid: Pid,
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
    /// track's inherited descriptors
    /// ([`TrackSpec::es_info_descriptors`](crate::pipeline::TrackSpec::es_info_descriptors),
    /// or a [`CodecConfig::Data`] track's
    /// [`DataIdentity::Ts`](crate::pipeline::DataIdentity::Ts) descriptors)
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
#[derive(Debug, Clone)]
struct TaggedPacket {
    order: u64,
    /// The track's announcement index: the stable tie-break of the interleave
    /// order for packets sharing a key. A PCR-only packet takes
    /// [`TIELESS`], so it follows every real packet keyed at the same time.
    track_index: u32,
    packet: [u8; TS_PACKET_SIZE],
}

/// Tie-break index of a packet that belongs to no track (a PCR-only packet).
const TIELESS: u32 = u32::MAX;

/// Packets budgeted for the usual one-packet PAT and one-packet PMT in an emit
/// buffer; a programme map too large for one packet still grows it.
const PSI_REPEAT_PACKETS: usize = 2;

/// Sections written per PSI occurrence: the PAT and the PMT travel together.
const PSI_SECTIONS_PER_REPEAT: usize = 2;

/// The one MPEG-2 TS muxer: a [`Stage`] fed a track set and its samples,
/// emitting whole 188-byte TS packets as [`MuxOutput::Bytes`].
///
/// A single program (PAT PID `0x0000` → PMT PID `0x1000`) enumerates every
/// carriable track as an elementary stream (PID `0x0100+`); the PCR rides the
/// first video PID (or, absent video, the first continuous-audio track, else
/// the first non-section-carried ES). Each sample becomes one PES packet,
/// packetised into 188-byte TS packets, interleaved in ascending decode-time
/// order.
///
/// - **Continuity counters** (ISO/IEC 13818-1 §2.4.3.3) continue on every PID
///   for the life of the stage.
/// - **One time origin**: the first sample carrying a `dts`; every later
///   sample is stamped from its own absolute `dts`/`pts` shifted by that same
///   origin, so inter-track offsets survive. A sample decoding *before* the
///   origin clamps to it, so feed samples in (roughly) decode order.
/// - **PSI**: the PAT/PMT are written once at the start and then repeated
///   every constructor-supplied interval of media time, never inside a PES
///   packet (ETSI TR 101 290 indicators 1.3/1.5 require PAT/PMT at least every
///   0.5 s). `None` (or a zero interval) writes them once only.
/// - **A track announced or updated mid-stream** emits a new PMT with
///   `version_number` incremented modulo 32 (§2.4.4.9) at the next repetition,
///   and the stream continues.
///
/// The stage holds at most one interleave window of built packets: packets
/// may only be released once no further PCR-only packet can be inserted before
/// them, so it retains one PCR-PID interval of media time.
#[derive(Debug, Clone)]
pub struct TsMuxStage {
    psi_interval: Option<Duration>,
    psi_interval_ticks: Option<u64>,
    /// Announced tracks, in announcement order (the PID / `stream_id`
    /// assignment order).
    tracks: Vec<StageTrack>,
    /// The stream's time origin: the first sample `dts` and its timescale.
    origin: Option<(i64, Timescale)>,
    /// The PCR PID of the current track set, and the PMT sections that
    /// describe it (rebuilt whenever a track is announced or updated).
    pcr_pid: Option<Pid>,
    pat: Option<Vec<u8>>,
    pmt: Option<Vec<u8>>,
    continuity: TsContinuity,
    /// Built packets awaiting release, ascending by interleave key with equal
    /// keys in insertion order (the stable-sort order the mux describes).
    pending: Vec<TaggedPacket>,
    /// Per-sample packetisation scratch, cleared for each sample so a feed does
    /// not allocate its packet run or its PES.
    scratch: Vec<TaggedPacket>,
    pes_buf: Vec<u8>,
    /// Interleave key of the last PCR-carrying sample on [`Self::pcr_pid`],
    /// and the continuity counter its last packet carried.
    last_pcr_at: Option<u64>,
    pcr_cc: u8,
    /// Interleave key of the most recently fed sample: keys are non-decreasing
    /// in feed order, so no later sample can be keyed below it.
    last_fed_at: Option<u64>,
    /// The leading PSI has been written.
    started: bool,
    /// Media time the next PSI repetition is due, and the interleave key of
    /// the packet emitted last (a repetition lands only on a key change).
    psi_due: Option<u64>,
    prev_at: Option<u64>,
    /// Elementary PIDs whose next packet must carry the adaptation-field
    /// `discontinuity_indicator` (§2.4.3.5): one entry per PID awaiting its
    /// mark, removed as that PID's packet is built.
    discontinuity_marks: Vec<Pid>,
    out: VecDeque<MuxOutput>,
}

/// Default PAT/PMT repetition interval, measured on the media timebase (well
/// inside the 0.5 s bound of ETSI TR 101 290 indicators 1.3/1.5). A caller
/// argument, not a stage default: [`TsMuxStage::new`] takes the interval.
pub const DEFAULT_PSI_INTERVAL: Duration = Duration::from_millis(100);

/// One announced track's muxing state.
#[derive(Debug, Clone)]
struct StageTrack {
    spec: TrackSpec,
    /// The elementary stream this track is emitted as; `None` when the TS
    /// layer cannot carry its codec (such a track is skipped, never fatal).
    plan: Option<EsPlan>,
    /// Decode time of the next sample, relative to the stream origin, in this
    /// track's own timescale (the running value
    /// the shared rule derives for a sample with no `dts`).
    next: i64,
    /// Interleave order of the last packet this track produced — a track's own
    /// orders never decrease, so its packets never reorder.
    last_order: u64,
    /// A section-carried `Data` track's next `continuity_counter`, which
    /// persists across its samples (§2.4.3.3). `None` for a PES-carried ES.
    sections_cc: Option<u8>,
}

impl StageTrack {
    fn new(spec: TrackSpec) -> Self {
        Self {
            spec,
            plan: None,
            next: 0,
            last_order: 0,
            sections_cc: None,
        }
    }
}

impl TsMuxStage {
    /// A muxer repeating PAT/PMT every `psi_interval` of media time (`None`,
    /// or a zero interval, writes them once only). Callers that want the
    /// usual repetition pass [`DEFAULT_PSI_INTERVAL`].
    pub fn new(psi_interval: Option<Duration>) -> Self {
        let interval = psi_interval.filter(|d| !d.is_zero());
        Self {
            psi_interval: interval,
            psi_interval_ticks: interval.map(psi_interval_ticks),
            tracks: Vec::new(),
            origin: None,
            pcr_pid: None,
            pat: None,
            pmt: None,
            continuity: TsContinuity::new(),
            pending: Vec::new(),
            scratch: Vec::new(),
            pes_buf: Vec::new(),
            last_pcr_at: None,
            pcr_cc: 0,
            last_fed_at: None,
            started: false,
            psi_due: None,
            prev_at: None,
            discontinuity_marks: Vec::new(),
            out: VecDeque::new(),
        }
    }

    /// The configured PAT/PMT repetition interval (`None` = written once).
    pub fn psi_interval(&self) -> Option<Duration> {
        self.psi_interval
    }

    /// The configured PAT/PMT repetition interval on the 90 kHz clock
    /// (`None` = written once); the same value as [`Self::psi_interval`].
    pub fn psi_interval_ticks(&self) -> Option<u64> {
        self.psi_interval_ticks
    }

    /// Announce or update one track. A new or changed track set is written
    /// with a new PMT `version_number` (§2.4.4.9) at the next PSI repetition.
    fn announce(&mut self, spec: TrackSpec) -> Result<()> {
        let changed = match self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == spec.track_id)
        {
            Some(index) => {
                if self.tracks[index].spec == spec {
                    return Ok(());
                }
                self.tracks[index].spec = spec;
                true
            }
            None => {
                self.tracks.push(StageTrack::new(spec));
                true
            }
        };
        if changed {
            self.rebuild_plans()?;
        }
        Ok(())
    }

    /// Re-derive each announced track's elementary stream, PID assignment, the
    /// PCR PID and the PAT/PMT sections. A track whose codec the TS layer
    /// cannot carry gets no plan.
    fn rebuild_plans(&mut self) -> Result<()> {
        let specs: Vec<TrackSpec> = self.tracks.iter().map(|t| t.spec.clone()).collect();
        let (plans, planned_idx) = plan_plans(&specs)?;
        for track in &mut self.tracks {
            track.plan = None;
        }
        for (plan, &index) in plans.into_iter().zip(&planned_idx) {
            let track = &mut self.tracks[index];
            let same_section_pid = track
                .plan
                .as_ref()
                .is_some_and(|old| old.kind.is_section_carried() && old.pid == plan.pid);
            if !same_section_pid {
                track.sections_cc = plan
                    .kind
                    .is_section_carried()
                    .then(|| self.continuity.next_for(plan.pid));
            }
            track.plan = Some(plan);
        }
        let pcr_pid = Self::choose_pcr_pid(&self.tracks);
        if pcr_pid != self.pcr_pid {
            // A PCR-only gap is filled on one PID at a time: a carrier change
            // starts a fresh gap on the new PID.
            self.last_pcr_at = None;
        }
        self.pcr_pid = pcr_pid;
        self.pat = None;
        self.pmt = None;
        Ok(())
    }

    /// The PCR carrier (§2.4.3.4 / §2.4.2.2): the first video ES; else the
    /// first *continuous* audio ES (its packets arrive at the codec frame
    /// rate, within the 100 ms bound); else the first non-section-carried ES;
    /// else the first ES. A sparse, PES-carried `Data` stream (subtitles,
    /// teletext) must not carry it while any audio ES exists (TR 101 290
    /// indicator 2.3).
    fn choose_pcr_pid(tracks: &[StageTrack]) -> Option<Pid> {
        let plans: Vec<&EsPlan> = tracks.iter().filter_map(|t| t.plan.as_ref()).collect();
        let first = *plans.first()?;
        plans
            .iter()
            .copied()
            .find(|p| p.kind.is_video())
            .or_else(|| plans.iter().copied().find(|p| p.kind.is_continuous_audio()))
            .or_else(|| plans.iter().copied().find(|p| !p.kind.is_section_carried()))
            .or(Some(first))
            .map(|p| p.pid)
    }

    /// Arm the adaptation-field `discontinuity_indicator` (§2.4.3.5) on the
    /// next packet of each elementary PID `track` names — every announced one
    /// when it is `None`. A track the caller filtered out, or whose codec the
    /// TS layer cannot carry, has no packet to mark and arms nothing; a PID
    /// already waiting for its mark keeps the one it has, since two breaks
    /// before its next packet are one mark.
    fn mark_discontinuity(&mut self, track: Option<TrackId>) {
        let pids: Vec<Pid> = match track {
            None => self
                .tracks
                .iter()
                .filter_map(|t| t.plan.as_ref().map(|p| p.pid))
                .collect(),
            Some(track_id) => self
                .tracks
                .iter()
                .filter(|t| t.spec.track_id == track_id)
                .filter_map(|t| t.plan.as_ref().map(|p| p.pid))
                .collect(),
        };
        for pid in pids {
            if !self.discontinuity_marks.contains(&pid) {
                self.discontinuity_marks.push(pid);
            }
        }
    }

    /// Take `pid`'s pending discontinuity mark, if it has one.
    fn take_discontinuity_mark(&mut self, pid: Pid) -> bool {
        match self
            .discontinuity_marks
            .iter()
            .position(|&held| held == pid)
        {
            Some(index) => {
                self.discontinuity_marks.remove(index);
                true
            }
            None => false,
        }
    }

    /// Feed one coded sample.
    fn push_sample(&mut self, track_id: TrackId, sample: Sample) -> Result<()> {
        let index = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == track_id)
            .ok_or(Error::InvalidInput(
                "sample for a track the TS muxer was not told about",
            ))?;
        if self.tracks[index].plan.is_none() {
            return Ok(());
        }
        let timescale = self.tracks[index].spec.timescale;
        let dts = sample.timing.and_then(|t| t.dts());
        if let Some(dts) = dts
            && self.origin.is_none()
        {
            self.origin = Some((dts, timescale));
        }
        let base = self
            .origin
            .map(|(ticks, scale)| rescale_i64(ticks, scale, timescale, Rounding::Floor))
            .unwrap_or(0);
        let local = match dts {
            Some(dts) => dts.checked_sub(base).ok_or(Error::InvalidInput(
                "sample decode time is more than i64 ticks from the timeline origin",
            ))?,
            None => self.tracks[index].next,
        };
        let duration = i64::from(sample.timing.and_then(|t| t.duration()).unwrap_or(0));
        self.tracks[index].next = local.saturating_add(duration);

        let dts_local = local.max(0) as u64;
        let dts90 = rescale(dts_local, timescale) + PCR_LEAD_TICKS;
        let sort_key =
            rescale_for_ordering(dts_local, timescale).max(self.tracks[index].last_order);
        self.tracks[index].last_order = sort_key;
        self.last_fed_at = Some(self.last_fed_at.map_or(sort_key, |fed| fed.max(sort_key)));

        if self.tracks[index]
            .plan
            .as_ref()
            .is_some_and(|p| p.kind.is_section_carried())
        {
            let pid = self.tracks[index].plan.as_ref().expect("checked above").pid;
            let marked = self.take_discontinuity_mark(pid);
            let track = &mut self.tracks[index];
            let cc = track
                .sections_cc
                .ok_or(Error::InvalidInput("section-carried ES has no packetiser"))?;
            let mut packetiser =
                SectionPacketiser::with_continuity(pid, ContinuityCounter::from_wire_masked(cc));
            let track_index = index as u32;
            let mut packets = packetiser.packetise(&[&sample.data[..]]);
            if marked {
                mark_section_packets(&mut packets, pid, cc, sample.data.len());
            }
            for packet in packets {
                insert_packet(&mut self.pending, sort_key, track_index, packet);
            }
            track.sections_cc = Some(packetiser.continuity_counter().value());
            self.continuity
                .set_next_for(pid, packetiser.continuity_counter().value());
            return Ok(());
        }

        let pts_local = local.max(0) + i64::from(sample.composition_offset()?.unwrap_or(0));
        let pts90 = rescale_signed(pts_local, timescale) + PCR_LEAD_TICKS;
        let pid = self.tracks[index].plan.as_ref().expect("checked above").pid;
        let marked = self.take_discontinuity_mark(pid);
        let mut cc = self.continuity.next_for(pid);
        let mut tagged = core::mem::take(&mut self.scratch);
        tagged.clear();
        {
            let (tracks, pes_buf) = (&self.tracks, &mut self.pes_buf);
            let plan = tracks[index].plan.as_ref().expect("checked above");
            let es_payload = build_es_payload(plan, &sample)?;
            // One packet per payload chunk plus the PES header spill and the
            // stuffed final packet: an exact reserve keeps the per-sample
            // packetisation from doubling its way up.
            tagged.reserve(es_payload.len() / TS_PAYLOAD_CAPACITY + PACKETS_PER_SAMPLE_SLACK);
            let carry_pcr = self.pcr_pid == Some(plan.pid);
            build_pes_bytes(plan, &es_payload, pts90, dts90, pes_buf)?;
            packetise_pes(
                plan,
                pes_buf,
                dts90,
                carry_pcr,
                marked,
                &mut cc,
                sort_key,
                &mut tagged,
            )?;
        }
        self.continuity.set_next_for(pid, cc);
        if self.pcr_pid == Some(pid) {
            // A mark's sample starts a new time base, so no PCR-only packet
            // fills the gap into it: the marked packet itself carries the new
            // time base's first PCR (§2.4.3.5).
            if let Some(previous) = self.last_pcr_at
                && !marked
            {
                for key in pcr_gap_at(previous, sort_key) {
                    let mut packet = [STUFFING_BYTE; TS_PACKET_SIZE];
                    write_pcr_only_packet(
                        &mut packet,
                        pid,
                        ContinuityCounter::from_wire_masked(self.pcr_cc),
                        pcr_for(key + PCR_LEAD_TICKS),
                        false,
                    );
                    insert_packet(&mut self.pending, key, TIELESS, packet);
                }
            }
            self.last_pcr_at = Some(sort_key);
            self.pcr_cc = cc.wrapping_sub(1) & CC_MASK;
        }
        for held in &tagged {
            insert_packet(&mut self.pending, sort_key, index as u32, held.packet);
        }
        self.scratch = tagged;
        self.release_ready()
    }

    /// Release every held packet no future PCR-only insertion can precede: a
    /// packet keyed at or before `last_pcr_at + MAX_PCR_INTERVAL_TICKS` comes
    /// before the first insertion of any gap that could still open (its
    /// insertions start strictly after that bound).
    fn release_ready(&mut self) -> Result<()> {
        let (Some(fed), Some(previous)) = (self.last_fed_at, self.last_pcr_at) else {
            return Ok(());
        };
        // A key equal to the one just fed may still be shared by a sample of an
        // earlier track fed next (two tracks can round onto one 90 kHz tick),
        // and packets keyed past the first possible PCR-only insertion of a gap
        // that could still open must wait for that gap's end.
        let limit = fed
            .saturating_sub(1)
            .min(previous.saturating_add(MAX_PCR_INTERVAL_TICKS));
        let ready = self.pending.partition_point(|packet| packet.order <= limit);
        self.emit(ready)
    }

    /// Release every held packet (end of the input, or an explicit flush).
    fn release_all(&mut self) -> Result<()> {
        let ready = self.pending.len();
        self.emit(ready)
    }

    /// Emit the first `ready` held packets, with the leading and periodic PSI.
    fn emit(&mut self, ready: usize) -> Result<()> {
        if ready == 0 {
            return Ok(());
        }
        // Room for the packets plus any PSI prepended to this run, so a
        // repeat never regrows the buffer.
        let psi_room = if self.psi_interval_ticks.is_some() {
            PSI_SECTIONS_PER_REPEAT * PSI_REPEAT_PACKETS
        } else {
            PSI_REPEAT_PACKETS
        };
        let mut bytes: Vec<u8> =
            Vec::with_capacity(ready * TS_PACKET_SIZE + psi_room * TS_PACKET_SIZE);
        let mut pending = core::mem::take(&mut self.pending);
        for tagged in pending.drain(..ready) {
            let key = tagged.order;
            if !self.started {
                self.started = true;
                self.write_psi(&mut bytes)?;
                self.psi_due = self
                    .psi_interval_ticks
                    .map(|interval| key.saturating_add(interval));
            } else if let (Some(due), Some(interval)) = (self.psi_due, self.psi_interval_ticks)
                && key >= due
                && self.prev_at != Some(key)
            {
                self.write_psi(&mut bytes)?;
                self.psi_due = Some(key.saturating_add(interval));
            }
            self.prev_at = Some(key);
            bytes.extend_from_slice(&tagged.packet);
        }
        self.pending = pending;
        self.out.push_back(MuxOutput::Bytes(bytes.into()));
        Ok(())
    }

    /// Packetise the current PAT + PMT with the running per-PID continuity
    /// counters and append them to `bytes`.
    fn write_psi(&mut self, bytes: &mut Vec<u8>) -> Result<()> {
        if self.pat.is_none() || self.pmt.is_none() {
            let Some(pcr_pid) = self.pcr_pid else {
                return Ok(());
            };
            let ordered: Vec<EsPlan> = self.tracks.iter().filter_map(|t| t.plan.clone()).collect();
            let pmt_v0 = build_pmt_section(pcr_pid, &ordered, 0)?;
            let version = self.continuity.pmt_version_for(pmt_v0.clone());
            let pmt = if version == 0 {
                pmt_v0
            } else {
                build_pmt_section(pcr_pid, &ordered, version)?
            };
            self.pat = Some(build_pat_section(PMT_PID)?);
            self.pmt = Some(pmt);
        }
        let pat = self.pat.take().expect("built above");
        let mut pat_cc = self.continuity.next_for(PAT_PID);
        for packet in packetise_section(PAT_PID, &pat, pat_cc) {
            pat_cc = ((packet[CC_OFFSET] & CC_MASK) + 1) & CC_MASK;
            bytes.extend_from_slice(&packet);
        }
        self.continuity.set_next_for(PAT_PID, pat_cc);
        self.pat = Some(pat);

        let pmt = self.pmt.take().expect("built above");
        let mut pmt_cc = self.continuity.next_for(PMT_PID);
        for packet in packetise_section(PMT_PID, &pmt, pmt_cc) {
            pmt_cc = ((packet[CC_OFFSET] & CC_MASK) + 1) & CC_MASK;
            bytes.extend_from_slice(&packet);
        }
        self.continuity.set_next_for(PMT_PID, pmt_cc);
        self.pmt = Some(pmt);
        Ok(())
    }
}

impl Stage for TsMuxStage {
    type In<'a> = MuxInput;
    type Out = MuxOutput;
    type Error = Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Track(spec) => self.announce(*spec),
            MuxInput::Sample { track, sample } => self.push_sample(track, sample),
            MuxInput::Discontinuity { track } => {
                self.mark_discontinuity(track);
                Ok(())
            }
            MuxInput::Flush => self.release_all(),
        }
    }

    fn poll(&mut self) -> Option<MuxOutput> {
        self.out.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        if self.tracks.is_empty() {
            return Err(Error::InvalidInput("cannot package media with no tracks"));
        }
        if self.tracks.iter().all(|t| t.plan.is_none()) {
            let specs: Vec<TrackSpec> = self.tracks.iter().map(|t| t.spec.clone()).collect();
            plan_elementary_streams(&specs)?;
        }
        if !self.started && self.pending.is_empty() {
            // A batch mux with no samples still writes the PSI once.
            self.started = true;
            let mut bytes = Vec::new();
            self.write_psi(&mut bytes)?;
            self.out.push_back(MuxOutput::Bytes(bytes.into()));
        }
        self.release_all()
    }
}

/// Mux a whole [`Media`] through a fresh [`TsMuxStage`] — the bridge a unit
/// test that already holds the IR uses.
#[cfg(test)]
pub(crate) fn mux_media_with(media: &Media, psi_interval: Option<Duration>) -> Result<Vec<u8>> {
    let mut stage = TsMuxStage::new(psi_interval);
    let outputs = crate::collect::drive(&mut stage, media, Timestamp::ZERO)?;
    let mut bytes = Vec::new();
    for output in outputs {
        if let MuxOutput::Bytes(chunk) = output {
            bytes.extend_from_slice(&chunk);
        }
    }
    Ok(bytes)
}

/// Insert `packet` into `pending` after every packet with the same or an
/// earlier interleave key, keeping the sequence ordered by key with equal keys
/// in insertion order (the stable order the batch mux produces).
fn insert_packet(
    pending: &mut Vec<TaggedPacket>,
    sort_key: u64,
    track_index: u32,
    packet: [u8; TS_PACKET_SIZE],
) {
    let at =
        pending.partition_point(|held| (held.order, held.track_index) <= (sort_key, track_index));
    pending.insert(
        at,
        TaggedPacket {
            order: sort_key,
            track_index,
            packet,
        },
    );
}

/// The interleave orders of the PCR-only packets the mux fills the gap between
/// consecutive PCR stamps `previous` and `next` with: one every
/// [`MAX_PCR_INTERVAL_TICKS`], starting one interval after `previous`, while
/// still strictly before `next`.
fn pcr_gap_at(previous: u64, next: u64) -> PcrGap {
    PcrGap {
        next,
        pending: previous
            .checked_add(MAX_PCR_INTERVAL_TICKS)
            .filter(|&key| key < next),
        step: MAX_PCR_INTERVAL_TICKS,
    }
}

/// The ascending PCR-only insertion times of one PCR gap; see [`pcr_gap_at`].
struct PcrGap {
    next: u64,
    pending: Option<u64>,
    step: u64,
}

impl Iterator for PcrGap {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        let key = self.pending?;
        self.pending = key.checked_add(self.step).filter(|&next| next < self.next);
        Some(key)
    }
}

/// Plan the carriable elementary streams of `specs` (PID + `stream_type` +
/// per-codec framing state), failing when no track carries a codec the TS
/// layer can represent at all.
///
/// Returns the plans in track order plus the parallel indices of the planned
/// tracks within `specs` (so a caller can select the matching samples).
pub(crate) fn plan_elementary_streams(specs: &[TrackSpec]) -> Result<(Vec<EsPlan>, Vec<usize>)> {
    let planned = plan_plans(specs)?;
    if planned.0.is_empty() {
        return Err(Error::InvalidInput(concat!(
            "no track carries a TS-representable codec (AVC/HEVC/MPEG-2-video/AAC/",
            "MPEG-audio/AC-3/E-AC-3/DTS/MPEG-H/Data)"
        )));
    }
    Ok(planned)
}

/// [`plan_elementary_streams`] without the "nothing carriable" rejection, for
/// a caller that only announced tracks and has not fed a sample yet.
fn plan_plans(specs: &[TrackSpec]) -> Result<(Vec<EsPlan>, Vec<usize>)> {
    let mut plans: Vec<EsPlan> = Vec::new();
    let mut planned_idx: Vec<usize> = Vec::new();
    let mut next_pid = ES_PID_BASE.value();
    let (mut n_video, mut n_audio) = (0u8, 0u8);
    for (idx, spec) in specs.iter().enumerate() {
        // A data track carries the identity of the container it came from;
        // the TS muxer places only a TS elementary-stream identity. The other
        // identities are a named refusal, not a silent skip: an opaque track
        // the caller meant to carry must not vanish from the output.
        if let CodecConfig::Data { identity } = &spec.config
            && !matches!(identity, DataIdentity::Ts { .. })
        {
            return Err(Error::DataIdentityMismatch {
                track: spec.track_id,
                have: identity.name(),
                want: "ts",
            });
        }
        let Some(kind) = EsKind::from_config(&spec.config) else {
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
            let id = StreamId::new(STREAM_ID_VIDEO_BASE + n_video);
            n_video += 1;
            id
        } else {
            match kind {
                EsKind::Data {
                    carriage: DataCarriage::Pes,
                    ..
                } => StreamId::new(STREAM_ID_PRIVATE_1),
                EsKind::Data {
                    carriage: DataCarriage::Sections,
                    ..
                } => StreamId::new(0),
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
                    let id = StreamId::new(STREAM_ID_AUDIO_BASE + n_audio);
                    n_audio += 1;
                    id
                }
            }
        };
        let asc = match &spec.config {
            CodecConfig::Aac { esds, .. } => Some(esds.audio_specific_config()?),
            _ => None,
        };
        let avc_sps_pps = match &spec.config {
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
        let hevc_vps_sps_pps = match &spec.config {
            CodecConfig::Hevc { config, .. } => hevc_parameter_sets(&config.config),
            _ => Vec::new(),
        };
        // MPEG-H's ES_info descriptor is synthesized fresh from the typed
        // `mpegh3daProfileLevelIndication` field, rather than trusting a raw
        // byte loop preserved from demux (issue #579; ETSI TS 101 154
        // §4.1.8.31); [`merge_es_info_descriptors`] drops any inherited
        // duplicate of the same tag in its favour (issue #775).
        let synthesized: Vec<Vec<u8>> = match &spec.config {
            CodecConfig::MpegH { config, .. } => {
                alloc::vec![mpegh_3daudio_descriptor(
                    config.mpegh3da_profile_level_indication,
                    config.reference_channel_layout,
                )?]
            }
            _ => Vec::new(),
        };
        // A mapped data track's descriptor loop is on its identity, so a
        // `DataIdentityStage`-rewritten track reaches the PMT with the target
        // descriptors. A spec that carries the loop only on
        // `TrackSpec::es_info_descriptors` (the demux mirror) still works: the
        // identity's copy is preferred when it has one.
        let inherited = match &spec.config {
            CodecConfig::Data {
                identity: DataIdentity::Ts { descriptors, .. },
            } if !descriptors.is_empty() => descriptors.as_slice(),
            _ => spec.es_info_descriptors.as_slice(),
        };
        let descriptors = merge_es_info_descriptors(inherited, &synthesized)?;
        plans.push(EsPlan {
            pid: Pid::try_new(next_pid).map_err(|_| {
                Error::InvalidInput("too many tracks: elementary PID exceeds 0x1FFF")
            })?,
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
/// cap (§2.4.4.9) — if the merged loop would overflow it, rather than
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
    while off + DESCRIPTOR_HEADER_LEN <= inherited.len() {
        let tag = inherited[off];
        let len = inherited[off + 1] as usize;
        let end = (off + DESCRIPTOR_HEADER_LEN + len).min(inherited.len());
        let body = &inherited[(off + DESCRIPTOR_HEADER_LEN).min(end)..end];
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
    record: &isobmff::hevc_config::HEVCDecoderConfigurationRecord,
) -> Vec<Vec<u8>> {
    let mut vps = Vec::new();
    let mut sps = Vec::new();
    let mut pps = Vec::new();
    for array in &record.arrays {
        let bucket = match array.nal_unit_type {
            HEVC_NAL_VPS => &mut vps,
            HEVC_SPS_NUT => &mut sps,
            HEVC_NAL_PPS => &mut pps,
            _ => continue,
        };
        for nalu in &array.nalus {
            bucket.push(nalu.0.clone());
        }
    }
    vps.into_iter().chain(sps).chain(pps).collect()
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
/// crate's own [`StreamingTsDemux`](crate::StreamingTsDemux) reports `InputDegraded` on it.
#[cfg(feature = "hls")]
pub(crate) fn mux_tracks_at_continuing(
    specs: &[TrackSpec],
    samples: &[&[Sample]],
    base_dts_ticks: &[u64],
    cc: &mut TsContinuity,
) -> Result<Vec<u8>> {
    debug_assert_eq!(specs.len(), base_dts_ticks.len());
    let times: Vec<Vec<i64>> = samples
        .iter()
        .zip(base_dts_ticks)
        .map(|(ss, &base)| {
            let mut t = i64::try_from(base).unwrap_or(i64::MAX);
            ss.iter()
                .map(|s| {
                    let now = t;
                    t = t.saturating_add(i64::from(
                        s.timing.and_then(|t| t.duration()).unwrap_or(0),
                    ));
                    now
                })
                .collect()
        })
        .collect();
    let times: Vec<&[i64]> = times.iter().map(Vec::as_slice).collect();
    mux_tracks_timed_with_cc(specs, samples, &times, cc, &mut PsiState::new(None))
}

/// PAT/PMT scheduling state threaded through [`mux_tracks_timed_with_cc`]
/// calls: the repetition interval, whether the leading PSI was already
/// written, and the media time (interleave key) the next repeat is due.
#[cfg(feature = "hls")]
#[derive(Debug, Clone)]
struct PsiState {
    interval_ticks: Option<u64>,
    started: bool,
    due: Option<u64>,
}

#[cfg(feature = "hls")]
impl PsiState {
    fn new(interval_ticks: Option<u64>) -> Self {
        Self {
            interval_ticks,
            started: false,
            due: None,
        }
    }
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
    /// The last PMT written through this state, for `version_number` tracking.
    pmt: Option<PmtRecord>,
}

/// The PMT most recently emitted: its content built at version 0 (the change
/// key) and the `version_number` it was written with.
#[derive(Debug, Clone)]
struct PmtRecord {
    pmt_at_v0: Vec<u8>,
    version: u8,
}

impl TsContinuity {
    /// A fresh state: every PID's next packet carries CC 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// The CC the next payload-bearing packet on `pid` must carry.
    fn next_for(&self, pid: Pid) -> u8 {
        self.counters
            .get(usize::from(pid.value()))
            .copied()
            .flatten()
            .unwrap_or(0)
    }

    /// Record that a payload-bearing packet with counter `cc` was written on
    /// `pid`; the next one takes `cc + 1` modulo 16 (§2.4.3.3).
    #[cfg(feature = "hls")]
    fn advance(&mut self, pid: Pid, cc: u8) {
        self.set_next_for(pid, (cc + 1) & CC_MASK);
    }

    /// The PMT `version_number` for a section whose version-0 rendering is
    /// `pmt_at_v0`: unchanged content keeps the previous version, changed
    /// content increments it modulo 32 (§2.4.4.9), and the first PMT is 0.
    fn pmt_version_for(&mut self, pmt_at_v0: Vec<u8>) -> u8 {
        let version = match &self.pmt {
            None => 0,
            Some(prev) if prev.pmt_at_v0 == pmt_at_v0 => prev.version,
            Some(prev) => (prev.version + 1) % SECTION_VERSION_MODULUS,
        };
        self.pmt = Some(PmtRecord { pmt_at_v0, version });
        version
    }

    /// Set the counter the next payload-bearing packet on `pid` must carry.
    fn set_next_for(&mut self, pid: Pid, cc: u8) {
        let idx = usize::from(pid.value());
        if self.counters.len() <= idx {
            self.counters.resize(idx + 1, None);
        }
        self.counters[idx] = Some(cc & CC_MASK);
    }
}

/// The shared body of [`mux_tracks`] and [`mux_tracks_at_continuing`], threading a
/// [`TsContinuity`] across calls. `dts_ticks[i][j]` is the decode time of
/// `samples[i][j]` in `tracks[i]`'s timescale, relative to the stream's origin
/// (negative values clamp to the origin).
#[cfg(feature = "hls")]
fn mux_tracks_timed_with_cc(
    specs: &[TrackSpec],
    samples: &[&[Sample]],
    dts_ticks: &[&[i64]],
    continuity: &mut TsContinuity,
    psi: &mut PsiState,
) -> Result<Vec<u8>> {
    let psi_interval_ticks = psi.interval_ticks;
    debug_assert_eq!(specs.len(), samples.len());
    debug_assert_eq!(specs.len(), dts_ticks.len());

    // ── 1. Plan the elementary streams (PID + stream_type + framing) ──
    let (plans, planned_idx) = plan_elementary_streams(specs)?;

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
    let pmt_v0 = build_pmt_section(pcr_pid, &plans, 0)?;
    let pmt_version = continuity.pmt_version_for(pmt_v0.clone());
    let pmt = if pmt_version == 0 {
        pmt_v0
    } else {
        build_pmt_section(pcr_pid, &plans, pmt_version)?
    };
    // A continuing stream (`PsiState::started`) already wrote its leading PSI
    // in an earlier call; only the periodic repeats below apply.
    if !psi.started {
        let pat_pkts = packetise_section(PAT_PID, &pat, continuity.next_for(PAT_PID));
        let pat_last_cc = pat_pkts.last().map(|p| p[CC_OFFSET] & CC_MASK).unwrap_or(0);
        for pkt in pat_pkts {
            out.extend_from_slice(&pkt);
        }
        continuity.advance(PAT_PID, pat_last_cc);
        let pmt_pkts = packetise_section(PMT_PID, &pmt, continuity.next_for(PMT_PID));
        let pmt_last_cc = pmt_pkts.last().map(|p| p[CC_OFFSET] & CC_MASK).unwrap_or(0);
        for pkt in pmt_pkts {
            out.extend_from_slice(&pkt);
        }
        continuity.advance(PMT_PID, pmt_last_cc);
    }

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
    let mut pcr_stamps: Vec<u64> = Vec::new();
    let mut pes: Vec<u8> = Vec::new();
    for (plan, &track_idx) in plans.iter().zip(&planned_idx) {
        let ts_scale = specs[track_idx].timescale;
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
        let mut section_packetiser =
            SectionPacketiser::with_continuity(plan.pid, ContinuityCounter::from_wire_masked(cc));
        for (sample, &dts_local) in samples[track_idx].iter().zip(dts_ticks[track_idx]) {
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
                        order: sort_key,
                        track_index: track_idx as u32,
                        packet: pkt,
                    });
                }
            } else {
                // media plane step 2c: `composition_offset()` derives PTS−DTS
                // from the sample's own absolute `dts`/`pts` when both are
                // known (§0 invariant), falling back to `0` — identical to
                // the old stored field's value for every real (non-`None`)
                // sample this muxer ever sees.
                let pts_local =
                    dts_local.max(0) + i64::from(sample.composition_offset()?.unwrap_or(0));
                let pts90 = rescale_signed(pts_local, ts_scale) + PCR_LEAD_TICKS;
                let es_payload = build_es_payload(plan, sample)?;
                let carry_pcr = plan.pid == pcr_pid;
                build_pes_bytes(plan, &es_payload, pts90, dts90, &mut pes)?;
                packetise_pes(
                    plan,
                    &pes,
                    dts90,
                    carry_pcr,
                    false,
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
            section_packetiser.continuity_counter().value()
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
    tagged.sort_by_key(|t| t.order);
    // Repeat PAT + PMT every `psi_interval_ticks` of media time (the sort key is
    // the 90 kHz decode time) so the stream can be joined at any packet-aligned
    // offset (TR 101 290 indicators 1.3/1.5). A repeat lands on a key change,
    // i.e. between samples' packet runs, never inside a PES packet. The leading
    // PSI above counts as the first occurrence. Each repeat re-packetises the
    // same sections with the running per-PID continuity counter.
    let mut pat_cc = continuity.next_for(PAT_PID);
    let mut pmt_cc = continuity.next_for(PMT_PID);
    let mut psi_due = match (psi_interval_ticks, tagged.first()) {
        // A continuing stream carries its schedule across calls; a fresh one
        // counts the leading PSI as the first occurrence.
        (Some(_), Some(_)) if psi.due.is_some() => psi.due,
        (Some(iv), Some(first)) => Some(first.order.saturating_add(iv)),
        _ => None,
    };
    let mut prev_at: Option<u64> = None;
    // Room for the repeats too (a section needs its pointer_field byte, hence
    // the +1), so they never trigger a double-and-copy regrowth of `out`.
    let psi_repeats = match (psi_interval_ticks, tagged.first(), tagged.last()) {
        (Some(iv), Some(first), Some(last)) => {
            usize::try_from((last.order - first.order) / iv).unwrap_or(0) + 1
        }
        _ => 0,
    };
    let psi_repeat_bytes = (pat.len() + 1).div_ceil(TS_PAYLOAD_CAPACITY) * TS_PACKET_SIZE
        + (pmt.len() + 1).div_ceil(TS_PAYLOAD_CAPACITY) * TS_PACKET_SIZE;
    out.reserve_exact(tagged.len() * TS_PACKET_SIZE + psi_repeats * psi_repeat_bytes);
    for t in &tagged {
        if let (Some(due), Some(iv)) = (psi_due, psi_interval_ticks)
            && t.order >= due
            && prev_at != Some(t.order)
        {
            for (pid, section, next_cc) in
                [(PAT_PID, &pat, &mut pat_cc), (PMT_PID, &pmt, &mut pmt_cc)]
            {
                for pkt in packetise_section(pid, section, *next_cc) {
                    *next_cc = ((pkt[3] & 0x0F) + 1) & 0x0F;
                    out.extend_from_slice(&pkt);
                }
            }
            // Next repeat one interval after this one (catching up if a long
            // gap in the media skipped several).
            psi_due = Some(t.order.saturating_add(iv));
        }
        prev_at = Some(t.order);
        out.extend_from_slice(&t.packet);
    }
    continuity.set_next_for(PAT_PID, pat_cc);
    continuity.set_next_for(PMT_PID, pmt_cc);
    psi.started = true;
    psi.due = psi_due.or(psi.due);

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
#[cfg(feature = "hls")]
fn emit_pcr_only_packets(pcr_pid: Pid, pcr_stamps: &mut [u64], tagged: &mut Vec<TaggedPacket>) {
    if pcr_stamps.is_empty() {
        return;
    }
    pcr_stamps.sort_unstable();
    let mut insertions: Vec<u64> = Vec::new();
    for w in pcr_stamps.windows(PCR_STAMP_PAIR_LEN) {
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
        .filter(|tp| pid_of_packet(&tp.packet) == pcr_pid.value())
        .map(|tp| (tp.order, tp.packet[CC_OFFSET] & CC_MASK))
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
        write_pcr_only_packet(
            &mut pkt,
            pcr_pid,
            ContinuityCounter::from_wire_masked(cc),
            pcr_for(t + PCR_LEAD_TICKS),
            false,
        );
        tagged.push(TaggedPacket {
            order: t,
            track_index: TIELESS,
            packet: pkt,
        });
    }
}

/// Read the 13-bit PID from a TS packet's header.
#[cfg(feature = "hls")]
fn pid_of_packet(pkt: &[u8; TS_PACKET_SIZE]) -> u16 {
    (((pkt[1] & PID_MASK_HI) as u16) << u8::BITS) | pkt[PID_LO_OFFSET] as u16
}

/// Write a PCR-only TS packet: an adaptation field with the PCR and no payload
/// (`adaptation_field_control` `10`), optionally marking a time-base
/// discontinuity. §2.4.3.3 / §2.4.3.4 / §2.4.3.5.
fn write_pcr_only_packet(
    pkt: &mut [u8; TS_PACKET_SIZE],
    pid: Pid,
    cc: ContinuityCounter,
    pcr: Pcr,
    discontinuity: bool,
) {
    let hdr = TsHeader {
        tei: false,
        pusi: false,
        pid,
        scrambling: ScramblingControl::NotScrambled,
        adaptation_field_control: AdaptationFieldControl::AdaptationOnly,
        continuity_counter: cc,
    };
    hdr.serialize_into(&mut pkt[..4]).expect("4-byte TS header");
    pkt[AF_LENGTH_OFFSET] = PCR_ONLY_AF_LEN as u8;
    let mut field = AdaptationField::new();
    field.pcr = Some(pcr);
    field.discontinuity_indicator = discontinuity;
    field
        .serialize_into(&mut pkt[AF_FLAGS_OFFSET..])
        .expect("adaptation field fits the packet");
}

/// The `adaptation_field_length` of a packet whose whole remaining bytes are
/// its adaptation field (§2.4.3.4).
const AF_ONLY_LEN: usize = TS_PACKET_SIZE - TS_HEADER_LEN - 1;

/// Write an adaptation-field-only TS packet (`adaptation_field_control` `10`)
/// carrying the `discontinuity_indicator` and nothing else, for a packet that
/// cannot gain an adaptation field of its own. §2.4.3.3 / §2.4.3.5.
fn write_discontinuity_only_packet(pkt: &mut [u8; TS_PACKET_SIZE], pid: Pid, cc: u8) {
    let hdr = TsHeader {
        tei: false,
        pusi: false,
        pid,
        scrambling: ScramblingControl::NotScrambled,
        adaptation_field_control: AdaptationFieldControl::AdaptationOnly,
        continuity_counter: ContinuityCounter::from_wire_masked(cc),
    };
    hdr.serialize_into(&mut pkt[..4]).expect("4-byte TS header");
    pkt[AF_LENGTH_OFFSET] = AF_ONLY_LEN as u8;
    let mut field = AdaptationField::new();
    field.discontinuity_indicator = true;
    field
        .serialize_into(&mut pkt[AF_FLAGS_OFFSET..])
        .expect("adaptation field fits the packet");
}

/// Rescale `ticks` from a track's `timescale` to the 90 kHz TS clock, rounding to
/// nearest, and reduce modulo the 33-bit timestamp field.
fn rescale(ticks: u64, timescale: broadcast_core::timescale::Timescale) -> u64 {
    rescale_for_ordering(ticks, timescale) % TS_TIMESTAMP_MOD
}

/// Rescale a possibly-negative tick count (used for `pts = dts + composition`),
/// clamping negatives to 0, then reduce modulo the 33-bit field.
fn rescale_signed(ticks: i64, timescale: broadcast_core::timescale::Timescale) -> u64 {
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
fn rescale_for_ordering(ticks: u64, timescale: broadcast_core::timescale::Timescale) -> u64 {
    rescale_u64(ticks, timescale, TS_CLOCK, Rounding::Nearest)
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
            let au = build_annexb_au(
                &sample.data,
                sample.flags.is_sync,
                &plan.avc_sps_pps,
                |n| !n.is_empty() && (n[0] & NAL_TYPE_MASK) == AVC_NAL_SPS,
                |n| n.first().map(|b| b & NAL_TYPE_MASK) == Some(H264_NAL_AUD),
            )?;
            ensure_avc_aud(au)
        }
        EsKind::Hevc => {
            let au = build_annexb_au(
                &sample.data,
                sample.flags.is_sync,
                &plan.hevc_vps_sps_pps,
                |n| nal_unit_type(NalCodec::Hevc, n) == Some(HEVC_SPS_NUT),
                |n| nal_unit_type(NalCodec::Hevc, n) == Some(HEVC_NAL_AUD),
            )?;
            ensure_hevc_aud(au)
        }
        EsKind::Aac => {
            let asc = plan
                .asc
                .as_ref()
                .ok_or(Error::InvalidInput("AAC ES has no AudioSpecificConfig"))?;
            let frame_len_usize = sample.data.len() + ADTS_HEADER_SIZE;
            let frame_len: u16 = frame_len_usize.try_into().map_err(|_| {
                Error::InvalidValue(InvalidValue {
                    field: "frame_len",
                    value: frame_len_usize as u64,
                    reason: "exceeds ADTS 13-bit frame_length maximum (8191)",
                })
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
/// [`StreamingTsDemux`](crate::StreamingTsDemux)-sourced keyframe does — it preserves every NAL of
/// the access unit) nothing is inserted, so the length↔Annex B round-trip stays
/// byte-identical NAL-for-NAL. Inserted parameter sets are placed after a leading
/// Access Unit Delimiter (ISO/IEC 14496-10 §7.4.1.2.3 AU order), each with a
/// 4-byte start code — the canonical Annex B form the demuxer re-splits.
fn build_annexb_au<F, G>(
    length_prefixed: &[u8],
    is_sync: bool,
    param_sets: &[Vec<u8>],
    is_sps: F,
    is_aud: G,
) -> Result<Vec<u8>>
where
    F: Fn(&[u8]) -> bool,
    G: Fn(&[u8]) -> bool,
{
    let nals = iter_length_prefixed_nals(length_prefixed)?;

    let needs_params = is_sync && !param_sets.is_empty() && !nals.iter().any(|n| is_sps(n));

    if !needs_params {
        // Straight, byte-exact rewrite of the existing NAL sequence.
        return length_prefixed_to_annexb(length_prefixed);
    }

    // Insert the parameter sets after a leading AUD (if any), before the slices.
    let mut out = Vec::with_capacity(length_prefixed.len() + total_param_len(param_sets));
    let mut inserted = false;
    for nal in &nals {
        if !inserted && !is_aud(nal) {
            append_param_sets(&mut out, param_sets);
            inserted = true;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
    }
    if !inserted {
        // Access unit was only an AUD (degenerate) — still emit the params.
        append_param_sets(&mut out, param_sets);
    }
    Ok(out)
}

/// Total Annex B length the parameter sets add (4-byte start code each).
fn total_param_len(sps_pps: &[Vec<u8>]) -> usize {
    sps_pps.iter().map(|p| NAL_LENGTH_SIZE + p.len()).sum()
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
    let mut out = Vec::with_capacity(ANNEXB_START_CODE.len() + AVC_AUD_NAL.len() + au.len());
    out.extend_from_slice(&ANNEXB_START_CODE);
    out.extend_from_slice(&AVC_AUD_NAL);
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
    let mut out = Vec::with_capacity(ANNEXB_START_CODE.len() + HEVC_AUD_NAL.len() + au.len());
    out.extend_from_slice(&ANNEXB_START_CODE);
    out.extend_from_slice(&HEVC_AUD_NAL);
    out.append(&mut au);
    Ok(out)
}

/// The AVC `nal_unit_type` of the first NAL in an Annex B buffer, or `None`
/// when the buffer is empty (a zero-length sample has no NAL at all).
fn first_avc_nal_type(annexb: &[u8]) -> Option<u8> {
    crate::annexb::iter_annexb_nals(annexb)
        .next()
        .and_then(|n| n.first().map(|b| b & NAL_TYPE_MASK))
}

/// The HEVC `nal_unit_type` of the first NAL in an Annex B buffer (2-byte NAL
/// header, H.265 §7.3.1.2), or `None` when the buffer is empty.
fn first_hevc_nal_type(annexb: &[u8]) -> Option<u8> {
    crate::annexb::iter_annexb_nals(annexb)
        .next()
        .and_then(|n| nal_unit_type(NalCodec::Hevc, n))
}

/// Build a PAT section (one program → `pmt_pid`) with its trailing CRC_32.
/// ISO/IEC 13818-1 §2.4.4.4.
fn build_pat_section(pmt_pid: Pid) -> Result<Vec<u8>> {
    let pat = dvb_si::tables::pat::PatSection::new(
        1, // transport_stream_id
        0, // version_number
        true,
        0, // section_number
        0, // last_section_number
        alloc::vec![dvb_si::tables::pat::PatEntry::new(PROGRAM_NUMBER, pmt_pid)],
    );
    let len = pat.serialized_len();
    let mut buf = alloc::vec![0u8; len];
    pat.serialize_into(&mut buf)
        .map_err(|_| Error::InvalidInput("dvb-si PAT serialize"))?;
    Ok(buf)
}

/// Build a PMT section listing every planned elementary stream, with its
/// trailing CRC_32. ISO/IEC 13818-1 §2.4.4.9.
///
/// Each ES's `ES_info` descriptor loop carries its already-merged
/// [`EsPlan::descriptors`] verbatim — every track kind, not only an opaque
/// [`EsKind::Data`] one, per the module doc's ES_info passthrough policy
/// (issue #775) — so a receiver can recover e.g. a DVB subtitling/teletext
/// descriptor (issue #576) or an audio track's language (issue #775) after a
/// re-mux. `program_info` stays empty (no program-level descriptors are
/// modelled).
fn build_pmt_section(pcr_pid: Pid, plans: &[EsPlan], version: u8) -> Result<Vec<u8>> {
    let mut streams = Vec::with_capacity(plans.len());
    for p in plans {
        let es_len = p.descriptors.len().min(MAX_ES_INFO_LENGTH);
        let es_info = dvb_si::descriptors::any::DescriptorLoop::from(&p.descriptors[..es_len]);
        streams.push(dvb_si::tables::pmt::PmtStream::new(
            dvb_si::tables::pmt::StreamType::from_u8(p.kind.stream_type()),
            p.pid,
            es_info,
        ));
    }
    let program_info = dvb_si::descriptors::any::DescriptorLoop::from(&[][..]);
    let pmt = dvb_si::tables::pmt::PmtSection::new(
        PROGRAM_NUMBER,
        dvb_si::tables::SectionVersion::new(version, true, 0, 0),
        pcr_pid,
        program_info,
        streams,
    );
    // Preserve the §2.4.4.4 1021-byte section_length cap check (issue #1129).
    if pmt
        .serialized_len()
        .saturating_sub(PSI_SECTION_LENGTH_PREFIX)
        > MAX_SECTION_LENGTH
    {
        return Err(Error::BufferCapExceeded {
            what: "PSI section_length",
            cap: MAX_SECTION_LENGTH,
        });
    }
    let len = pmt.serialized_len();
    let mut buf = alloc::vec![0u8; len];
    pmt.serialize_into(&mut buf)
        .map_err(|_| Error::InvalidInput("dvb-si PMT serialize"))?;
    Ok(buf)
}

/// Packetise one complete PSI section into 188-byte TS packets on `pid`.
/// A single PUSI packet with a `pointer_field = 0` prefix, 0xFF-stuffed
/// (all this crate's sections fit one packet); multi-packet continuation is
/// handled by the generic loop for safety. ISO/IEC 13818-1 §2.4.4.
fn packetise_section(pid: Pid, section: &[u8], start_cc: u8) -> Vec<[u8; TS_PACKET_SIZE]> {
    let mut packets = Vec::new();
    let mut cc: u8 = start_cc & CC_MASK;
    let mut pos = 0usize;
    let mut first = true;
    while pos < section.len() || first {
        let mut pkt = [STUFFING_BYTE; TS_PACKET_SIZE];
        let hdr = TsHeader {
            tei: false,
            pusi: first,
            pid,
            scrambling: ScramblingControl::NotScrambled,
            adaptation_field_control: AdaptationFieldControl::PayloadOnly,
            continuity_counter: ContinuityCounter::from_wire_masked(cc),
        };
        hdr.serialize_into(&mut pkt[..4]).expect("4-byte TS header");
        cc = (cc + 1) & CC_MASK;
        let mut w = TS_HEADER_LEN;
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

/// Bytes of a packet's payload region an adaptation field costs: its
/// `adaptation_field_length` byte and the flags byte after it (§2.4.3.4).
const AF_PAYLOAD_OVERHEAD: usize = 1 + 1;

/// Give the first packet of a section-carried elementary stream's batch the
/// adaptation-field `discontinuity_indicator` (§2.4.3.5).
///
/// The section packetiser emits payload-only packets, so the mark brings an
/// adaptation field of its own and the payload moves back by the length and
/// flags bytes that costs — possible while the first packet's `pointer_field`
/// plus `section_len` bytes leave that much trailing stuffing. A section that
/// fills the packet leaves no room, and the flag then rides an
/// adaptation-field-only packet written ahead of the batch instead (§2.4.3.3:
/// a packet with no payload does not advance the continuity counter).
fn mark_section_packets(
    packets: &mut Vec<[u8; TS_PACKET_SIZE]>,
    pid: Pid,
    cc: u8,
    section_len: usize,
) {
    let Some(first) = packets.first_mut() else {
        return;
    };
    // `pointer_field` (1) + the section bytes this packet carries.
    let used = TS_HEADER_LEN + 1 + section_len.min(TS_PAYLOAD_CAPACITY - 1);
    if used + AF_PAYLOAD_OVERHEAD > TS_PACKET_SIZE {
        let mut marker = [STUFFING_BYTE; TS_PACKET_SIZE];
        write_discontinuity_only_packet(&mut marker, pid, cc);
        packets.insert(0, marker);
        return;
    }
    first.copy_within(TS_HEADER_LEN..used, AF_FLAGS_OFFSET + 1);
    first[CC_OFFSET] |= AF_CTRL_ADAPTATION;
    first[AF_LENGTH_OFFSET] = 1;
    let mut field = AdaptationField::new();
    field.discontinuity_indicator = true;
    field
        .serialize_into(&mut first[AF_FLAGS_OFFSET..])
        .expect("adaptation field fits the packet");
}

/// Packetise one PES payload (already framed as its `stream_id` payload) into
/// 188-byte TS packets on `plan.pid`, appended to `tagged` (each tagged with
/// `sort_key`, the interleave-order key — see [`rescale_for_ordering`], NOT
/// the on-wire `dts90`). The first packet sets PUSI and — when `carry_pcr` —
/// an adaptation field with the PCR; the final packet is stuffed via an
/// adaptation field so the PES ends exactly on a packet boundary. When
/// `discontinuity` is set the first packet's adaptation field also carries the
/// `discontinuity_indicator`, gaining one if it has none (§2.4.3.5).
/// ISO/IEC 13818-1 §2.4.3.
fn packetise_pes(
    plan: &EsPlan,
    pes: &[u8],
    dts90: u64,
    carry_pcr: bool,
    discontinuity: bool,
    cc: &mut u8,
    sort_key: u64,
    tagged: &mut Vec<TaggedPacket>,
) -> Result<()> {
    let mut pos = 0usize;
    let mut first = true;
    while pos < pes.len() {
        let mut pkt = [STUFFING_BYTE; TS_PACKET_SIZE];
        let remaining = pes.len() - pos;

        // PCR rides the first packet (if this PID owns the PCR), as does the
        // discontinuity indicator (if the PID has one pending). When present
        // the adaptation field carries flags(1) + PCR(6) content bytes, so the
        // payload capacity of this packet is reduced accordingly.
        let want_pcr = first && carry_pcr;
        let want_mark = first && discontinuity;
        // Minimum AF content bytes forced by the PCR (flags + PCR) or, without
        // a PCR, by the flags byte the indicator needs, else 0.
        let af_content = if want_pcr {
            1 + PCR_FIELD_LEN
        } else if want_mark {
            1
        } else {
            0
        };
        // Header bytes before the payload when only the forced AF (if any) is
        // present: 4 header + (1 af_len byte + af_content) when an AF exists.
        let forced_header = TS_HEADER_LEN + if af_content > 0 { 1 + af_content } else { 0 };
        let cap = TS_PACKET_SIZE - forced_header;

        let is_last = remaining <= cap;
        let to_copy = remaining.min(cap);
        // Bytes that must be filled by adaptation-field stuffing so the payload
        // ends exactly at byte 188 (only ever > 0 on the last packet).
        let stuff = cap - to_copy;

        if af_content > 0 {
            // AF carries the PCR and/or the indicator (+ any stuffing on the
            // last packet): af_len = af_content + stuffing.
            let af_len = af_content + stuff;
            write_af_packet(
                &mut pkt,
                plan.pid,
                first,
                *cc,
                af_len,
                want_pcr.then(|| pcr_for(dts90)),
                want_mark,
                &pes[pos..pos + to_copy],
            );
            pos += to_copy;
        } else if is_last && stuff > 0 {
            // No PCR, but the last packet underfills → an AF of pure stuffing.
            // af_len = flags(1) + stuffing; but the AF also costs its own 1-byte
            // length prefix, so total added = 2 + (stuff - 1) accounted below.
            // Choose af_len so 4 + 1 + af_len + to_copy == 188.
            let af_len = TS_PACKET_SIZE - TS_HEADER_LEN - 1 - to_copy;
            write_af_packet(
                &mut pkt,
                plan.pid,
                first,
                *cc,
                af_len,
                None,
                false,
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
                scrambling: ScramblingControl::NotScrambled,
                adaptation_field_control: AdaptationFieldControl::PayloadOnly,
                continuity_counter: ContinuityCounter::from_wire_masked(*cc),
            };
            hdr.serialize_into(&mut pkt[..4]).expect("4-byte TS header");
            pkt[4..4 + to_copy].copy_from_slice(&pes[pos..pos + to_copy]);
            pos += to_copy;
        }

        *cc = (*cc + 1) & 0x0F;
        tagged.push(TaggedPacket {
            order: sort_key,
            track_index: TIELESS,
            packet: pkt,
        });
        first = false;
    }
    Ok(())
}

/// The PCR value to stamp for a packet whose access-unit DTS is `dts90` (90 kHz):
/// place the PCR `PCR_LEAD_TICKS` behind the DTS on the 27 MHz clock.
fn pcr_for(dts90: u64) -> Pcr {
    // `PCR_base` is the 33-bit 90 kHz field (ISO/IEC 13818-1 §2.4.3.5); the
    // lead is already subtracted, so only the wire width is applied.
    let base = dts90.saturating_sub(PCR_LEAD_TICKS) & (broadcast_common::clock33::WRAP_33BIT - 1);
    Pcr::from_parts(base, 0).expect("base masked to 33 bits, extension 0")
}

/// Write a TS packet with an adaptation field into `pkt` (initialised to
/// stuffing), then copy `payload` at the byte following the adaptation field.
/// `af_len` is the `adaptation_field_length` value (bytes after the length
/// byte). A `pcr` sets `PCR_flag` and is encoded, `discontinuity` sets
/// `discontinuity_indicator`; any bytes between the encoded content and
/// `4 + 1 + af_len` stay 0xFF stuffing.
/// ISO/IEC 13818-1 §2.4.3.4 / §2.4.3.5.
fn write_af_packet(
    pkt: &mut [u8; TS_PACKET_SIZE],
    pid: Pid,
    pusi: bool,
    cc: u8,
    af_len: usize,
    pcr: Option<Pcr>,
    discontinuity: bool,
    payload: &[u8],
) {
    let hdr = TsHeader {
        tei: false,
        pusi,
        pid,
        scrambling: ScramblingControl::NotScrambled,
        adaptation_field_control: AdaptationFieldControl::AdaptationAndPayload,
        continuity_counter: ContinuityCounter::from_wire_masked(cc),
    };
    // serialize_into sets both AF + payload control bits from the booleans above.
    hdr.serialize_into(&mut pkt[..4]).expect("4-byte TS header");
    // Ensure the control bits reflect adaptation+payload (bits already set by the
    // header serializer via has_adaptation/has_payload).
    debug_assert_eq!(
        pkt[CC_OFFSET] & (AF_CTRL_ADAPTATION | AF_CTRL_PAYLOAD),
        AF_CTRL_ADAPTATION | AF_CTRL_PAYLOAD
    );
    pkt[AF_LENGTH_OFFSET] = af_len as u8;
    // An af_len of 0 is a valid single-stuffing-byte adaptation field with no
    // flags byte (§2.4.3.4); anything larger carries the flags byte.
    if af_len >= 1 {
        let mut field = AdaptationField::new();
        field.pcr = pcr;
        field.discontinuity_indicator = discontinuity;
        field
            .serialize_into(&mut pkt[AF_FLAGS_OFFSET..])
            .expect("adaptation field fits the packet");
    }
    // Remaining AF bytes (up to 5 + af_len) stay 0xFF stuffing (already set).
    let payload_start = TS_HEADER_LEN + 1 + af_len;
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
/// [`mpeg_pes::PesPacket::parse`], which the [`StreamingTsDemux`](crate::StreamingTsDemux) uses.
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
fn build_pes_bytes(
    plan: &EsPlan,
    es_payload: &[u8],
    pts90: u64,
    dts90: u64,
    out: &mut Vec<u8>,
) -> Result<()> {
    let include_dts = dts90 != pts90;
    // PES optional-header content length after the 3 fixed bytes: PTS (5) always,
    // + DTS (5) when present.
    let opt_content = if include_dts {
        PES_TIMESTAMP_PAIR_LEN
    } else {
        PES_TIMESTAMP_LEN
    };
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

    out.clear();
    out.reserve(MIN_LEN + HEADER_FIXED + opt_content + es_payload.len());
    out.extend_from_slice(&mpeg_pes::PACKET_START_CODE_PREFIX);
    out.push(plan.stream_id.get());
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
        out.extend_from_slice(&encode_timestamp(pts90, TS_PREFIX_PTS_ONLY));
    }
    out.extend_from_slice(es_payload);
    Ok(())
}

/// Encode a 33-bit timestamp into the 5-byte PTS/DTS field with the given 4-bit
/// `prefix`, interleaving the mandatory `marker_bit`s. ISO/IEC 13818-1 §2.4.3.7.
fn encode_timestamp(ts: u64, prefix: u8) -> [u8; PES_TIMESTAMP_LEN] {
    broadcast_common::clock33::pack_marked(ts, prefix)
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
    fn pmt_version_wraps_modulo_32_across_changes() {
        let mut c = TsContinuity::new();
        let key = |n: u8| vec![n];
        assert_eq!(c.pmt_version_for(key(0)), 0, "first PMT is version 0");
        assert_eq!(c.pmt_version_for(key(0)), 0, "unchanged content keeps it");
        for n in 1..=SECTION_VERSION_MODULUS {
            assert_eq!(
                c.pmt_version_for(key(n)),
                n % SECTION_VERSION_MODULUS,
                "change {n}"
            );
        }
        assert_eq!(c.pmt_version_for(key(SECTION_VERSION_MODULUS)), 0);
    }

    #[test]
    fn pmt_section_crc_is_valid() {
        let plans = alloc::vec![EsPlan {
            pid: ES_PID_BASE,
            stream_id: StreamId::new(STREAM_ID_VIDEO_BASE),
            kind: EsKind::Avc,
            asc: None,
            avc_sps_pps: Vec::new(),
            hevc_vps_sps_pps: Vec::new(),
            descriptors: Vec::new(),
        }];
        let pmt = build_pmt_section(ES_PID_BASE, &plans, 0).unwrap();
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
            stream_id: StreamId::new(0),
            kind: EsKind::Data {
                stream_type: 0x06,
                carriage: DataCarriage::Pes,
            },
            asc: None,
            avc_sps_pps: Vec::new(),
            hevc_vps_sps_pps: Vec::new(),
            descriptors: descriptors.clone(),
        }];
        let pmt = build_pmt_section(ES_PID_BASE, &plans, 0).unwrap();
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

    /// The 184-byte PSI payload (§2.4.4.2) minus the PUSI packet's
    /// `pointer_field`: a section of exactly this many bytes fills one packet.
    const ONE_PACKET_SECTION: usize = 183;

    /// A section of `len` bytes whose every byte is its own index, so a
    /// mis-split shows up as a moved byte rather than as a same-value overwrite.
    fn numbered_section(len: usize) -> alloc::vec::Vec<u8> {
        (0..len).map(|i| i as u8).collect()
    }

    #[test]
    fn packetise_section_fits_183_bytes_in_one_packet() {
        let section = numbered_section(ONE_PACKET_SECTION);
        let pkts = packetise_section(PAT_PID, &section, 0);
        assert_eq!(pkts.len(), 1, "183 section bytes + pointer_field = 184");
        assert_eq!(pkts[0][4], 0, "pointer_field is 0");
        assert_eq!(&pkts[0][5..], &section[..], "the section follows whole");
        assert_eq!(pkts[0][3] & 0x0F, 0, "first packet carries CC 0");
    }

    #[test]
    fn packetise_section_needs_two_packets_at_184_bytes() {
        let section = numbered_section(ONE_PACKET_SECTION + 1);
        let pkts = packetise_section(PAT_PID, &section, 0);
        assert_eq!(pkts.len(), 2, "one byte past the first packet's capacity");
        assert_ne!(pkts[0][1] & 0x40, 0, "PUSI on the first packet");
        assert_eq!(pkts[1][1] & 0x40, 0, "no PUSI on the continuation");
        assert_eq!(&pkts[0][5..], &section[..ONE_PACKET_SECTION]);
        // A continuation packet has no pointer_field, so its payload starts at
        // the four-byte header.
        assert_eq!(pkts[1][4], section[ONE_PACKET_SECTION]);
        assert_eq!(
            pkts[1][5..].iter().filter(|&&b| b == 0xFF).count(),
            TS_PACKET_SIZE - 5,
            "every byte after the last section byte is 0xFF stuffing"
        );
        assert_eq!(
            [pkts[0][3] & 0x0F, pkts[1][3] & 0x0F],
            [0, 1],
            "the continuity counter increments (§2.4.3.3)"
        );
    }

    #[test]
    fn packetise_section_splits_at_184_byte_continuations() {
        // 183 + 184 + 184 = 551 fills three packets exactly; one more byte
        // needs a fourth, whose payload is a single section byte.
        let full = numbered_section(ONE_PACKET_SECTION + 2 * (TS_PACKET_SIZE - 4));
        let pkts = packetise_section(PAT_PID, &full, 0);
        assert_eq!(pkts.len(), 3, "551 bytes is three packets");
        assert_eq!(
            &pkts[1][4..],
            &full[ONE_PACKET_SECTION..ONE_PACKET_SECTION + 184]
        );
        assert_eq!(
            pkts[2][4],
            full[ONE_PACKET_SECTION + 184],
            "the third packet starts at the 368th section byte"
        );

        let past = numbered_section(ONE_PACKET_SECTION + 2 * (TS_PACKET_SIZE - 4) + 1);
        let pkts = packetise_section(PAT_PID, &past, 0);
        assert_eq!(pkts.len(), 4, "one byte past three full packets");
        assert_eq!(
            [
                pkts[0][3] & 0x0F,
                pkts[1][3] & 0x0F,
                pkts[2][3] & 0x0F,
                pkts[3][3] & 0x0F
            ],
            [0, 1, 2, 3],
            "continuity_counter runs 0,1,2,3"
        );
    }

    #[test]
    fn packetise_section_starts_from_the_given_counter() {
        let section = numbered_section(ONE_PACKET_SECTION + 1);
        let pkts = packetise_section(PAT_PID, &section, 15);
        assert_eq!(
            [pkts[0][3] & 0x0F, pkts[1][3] & 0x0F],
            [15, 0],
            "the counter wraps at 16 (§2.4.3.3)"
        );
    }

    /// `extension_descriptor_tag` of `MPEG-H_3dAudio_descriptor` (H.222.0
    /// Table 2-110), as it appears on the wire in these tests.
    const MPEGH_EXT_TAG_FOR_TEST: u8 = 0x08;

    #[test]
    fn mpegh_3daudio_descriptor_matches_real_fixture_bytes() {
        // Real Fraunhofer MPEG-H-in-TS fixture ES_info
        // (`private/fixtures/audio/mpegh/mpegh-cicp-01-baseline-32kbps-single.ts`): `3F 04 08 10 7F C1`.
        // Table 2-121 bit layout of the 3 body bytes after the 0x08 ext tag:
        //   0x10                       mpegh3daProfileLevelIndication
        //   0x7F = 0 1 111111          interactivityEnabled=0,
        //                              compatibleProfileSetsPresent=1, reserved(6 of 8)
        //   0xC1 = 11 000001           reserved(last 2 of 8), referenceChannelLayout=1
        let bytes = mpegh_3daudio_descriptor(0x10, 0x01).expect("encodes");
        assert_eq!(bytes, alloc::vec![0x3F, 0x04, 0x08, 0x10, 0x7F, 0xC1]);
    }

    #[test]
    fn mpegh_3daudio_descriptor_fields_each_change_the_bytes() {
        // Hand-computed: profile 0x0B, layout 0x06 -> 3F 04 08 0B 7F C6.
        let bytes = mpegh_3daudio_descriptor(0x0B, 0x06).expect("encodes");
        assert_eq!(bytes, alloc::vec![0x3F, 0x04, 0x08, 0x0B, 0x7F, 0xC6]);
        assert_ne!(bytes, mpegh_3daudio_descriptor(0x10, 0x06).unwrap());
        assert_ne!(bytes, mpegh_3daudio_descriptor(0x0B, 0x01).unwrap());
    }

    #[test]
    fn mpegh_3daudio_descriptor_layout_wider_than_six_bits_falls_back_to_zero() {
        // 0x3F is the widest representable value and is kept verbatim ...
        assert_eq!(
            mpegh_3daudio_descriptor(0x10, 0x3F).unwrap(),
            alloc::vec![0x3F, 0x04, 0x08, 0x10, 0x7F, 0xFF]
        );
        // ... 0x40 and 0xFF do not fit: the mux still succeeds, writing 0
        // (and never leaking the excess bits into the reserved run).
        for wide in [0x40u8, 0x81, 0xFF] {
            assert_eq!(
                mpegh_3daudio_descriptor(0x10, wide).unwrap(),
                alloc::vec![0x3F, 0x04, 0x08, 0x10, 0x7F, 0xC0],
                "layout {wide:#04x}"
            );
        }
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
            0x02, // body length
            MPEGH_EXT_TAG_FOR_TEST,
            0xFF, // stale/wrong profile-level
        ];
        let after = alloc::vec![0x89u8, 0x01, 0x02];
        let mut inherited = before.clone();
        inherited.extend_from_slice(&stale_mpegh);
        inherited.extend_from_slice(&after);

        let synthesized = alloc::vec![mpegh_3daudio_descriptor(0x0B, 0x01).unwrap()];
        let merged = merge_es_info_descriptors(&inherited, &synthesized).expect("no overflow");

        // Exactly one occurrence of the extension tag, holding the
        // synthesized (grounded-in-CodecConfig) bytes, not the stale
        // inherited ones; the two unrelated siblings both survive in order,
        // and the synthesized descriptor is appended after them.
        let mut expected = before;
        expected.extend_from_slice(&after);
        expected.extend_from_slice(&mpegh_3daudio_descriptor(0x0B, 0x01).unwrap());
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
        // (0x08, `MPEGH_EXT_TAG_FOR_TEST`). Keying dedup on 0x3F alone
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

        let synthesized = alloc::vec![mpegh_3daudio_descriptor(0x0B, 0x01).unwrap()];
        let merged = merge_es_info_descriptors(&inherited, &synthesized).expect("no overflow");

        let mut expected = other_extension;
        expected.extend_from_slice(&mpegh_3daudio_descriptor(0x0B, 0x01).unwrap());
        assert_eq!(
            merged, expected,
            "two extension_descriptor entries with distinct extension_descriptor_tag values \
             are not duplicates and must both survive"
        );
    }

    #[test]
    fn merge_es_info_descriptors_accepts_exactly_the_cap_and_refuses_more() {
        // A 3-byte descriptor (tag 0x80, length 1, one body byte): 1365 of them
        // are exactly the 4095-byte ES_info_length cap (§2.4.4.9), which a PMT
        // may state but not exceed.
        let descriptor = [0x80u8, 0x01, 0xAA];
        let at_cap: alloc::vec::Vec<u8> =
            core::iter::repeat_n(descriptor, 1365).flatten().collect();
        assert_eq!(at_cap.len(), MAX_ES_INFO_LENGTH);
        assert_eq!(
            merge_es_info_descriptors(&at_cap, &[]).expect("exactly at the cap is legal"),
            at_cap,
            "a loop of exactly the cap is neither truncated nor refused"
        );

        let mut over = at_cap;
        over.extend_from_slice(&descriptor);
        assert!(
            matches!(
                merge_es_info_descriptors(&over, &[]),
                Err(Error::BufferCapExceeded { .. })
            ),
            "one descriptor past the cap is an error, not a silent truncation"
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
            stream_id: StreamId::new(0),
            kind: EsKind::Data {
                stream_type: 0x06,
                carriage: DataCarriage::Pes,
            },
            asc: None,
            avc_sps_pps: Vec::new(),
            hevc_vps_sps_pps: Vec::new(),
            descriptors: alloc::vec![0u8; 1004],
        }];
        let err = build_pmt_section(ES_PID_BASE, &plans, 0).unwrap_err();
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
            stream_id: StreamId::new(0),
            kind: EsKind::Data {
                stream_type: 0x06,
                carriage: DataCarriage::Pes,
            },
            asc: None,
            avc_sps_pps: Vec::new(),
            hevc_vps_sps_pps: Vec::new(),
            descriptors: alloc::vec![0u8; 1003],
        }];
        let pmt = build_pmt_section(ES_PID_BASE, &plans, 0).unwrap();
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
            stream_id: StreamId::new(STREAM_ID_AUDIO_BASE),
            kind: EsKind::Ac3,
            asc: None,
            avc_sps_pps: Vec::new(),
            hevc_vps_sps_pps: Vec::new(),
            descriptors: Vec::new(),
        };
        // Largest payload that still fits: 65535 − (3 fixed + 5 PTS) = 65527.
        let fits = alloc::vec![0u8; u16::MAX as usize - HEADER_FIXED - 5];
        let mut ok = Vec::new();
        build_pes_bytes(&plan, &fits, 0, 0, &mut ok).expect("the maximum legal PES must build");
        assert_eq!(
            u16::from_be_bytes([ok[4], ok[5]]),
            u16::MAX,
            "PES_packet_length = 65535 at the boundary"
        );

        let over = alloc::vec![0u8; fits.len() + 1];
        let mut out = Vec::new();
        let err = build_pes_bytes(&plan, &over, 0, 0, &mut out).unwrap_err();
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
            stream_id: StreamId::new(STREAM_ID_VIDEO_BASE),
            kind: EsKind::Avc,
            asc: None,
            avc_sps_pps: Vec::new(),
            hevc_vps_sps_pps: Vec::new(),
            descriptors: Vec::new(),
        };
        let big = alloc::vec![0u8; u16::MAX as usize + 1];
        let mut pes = Vec::new();
        build_pes_bytes(&plan, &big, 0, 0, &mut pes).expect("video may be unbounded");
        assert_eq!(
            u16::from_be_bytes([pes[4], pes[5]]),
            0,
            "video PES_packet_length is the unbounded 0 form"
        );
    }

    /// A timeline break reaches the wire as the adaptation-field
    /// `discontinuity_indicator` (§2.4.3.5) on the first packet of each
    /// elementary PID after it — exactly one packet per marked PID, never a
    /// PSI packet, with the continuity counters still counting (§2.4.3.3).
    #[test]
    fn discontinuity_marks_the_next_packet_of_each_elementary_pid() {
        use crate::collect::{CollectedTrack, media_from_ts};
        use mpeg_ts::ts::TsPacket;

        const SAMPLES_BEFORE_BREAK: usize = 2;
        const SAMPLES_AFTER_BREAK: usize = 2;
        const SAMPLES: usize = SAMPLES_BEFORE_BREAK + SAMPLES_AFTER_BREAK;

        /// The facts one emitted packet carries for this test.
        struct Seen {
            pid: u16,
            pusi: bool,
            cc: u8,
            discontinuity: bool,
        }

        /// The video and audio tracks of one real capture, and their ids.
        fn pair() -> (Vec<CollectedTrack>, Vec<TrackId>) {
            let media = media_from_ts(&test_fixtures::read("ts/h264-aac.ts")).expect("demux");
            let wanted: Vec<CollectedTrack> = [true, false]
                .into_iter()
                .map(|video| {
                    media
                        .tracks
                        .iter()
                        .find(|t| t.spec.config.is_video() == video)
                        .cloned()
                        .unwrap_or_else(|| panic!("the capture must carry a {video} video track"))
                })
                .collect();
            for track in &wanted {
                assert!(
                    track.samples.len() >= SAMPLES,
                    "the capture must carry at least {SAMPLES} samples per track"
                );
            }
            let ids = wanted.iter().map(|t| t.spec.track_id).collect();
            (wanted, ids)
        }

        /// Two samples of each track, a break on `break_on`, then two more;
        /// the packets of the resulting stream.
        fn play(break_on: Option<TrackId>) -> Vec<Seen> {
            let (tracks, _) = pair();
            let mut stage = TsMuxStage::new(None);
            for track in &tracks {
                stage
                    .feed(
                        MuxInput::Track(Box::new(track.spec.clone())),
                        Timestamp::ZERO,
                    )
                    .expect("announce");
            }
            let mut bytes = Vec::new();
            for index in 0..SAMPLES {
                if index == SAMPLES_BEFORE_BREAK {
                    stage
                        .feed(MuxInput::Discontinuity { track: break_on }, Timestamp::ZERO)
                        .expect("mark the break");
                }
                for track in &tracks {
                    stage
                        .feed(
                            MuxInput::Sample {
                                track: track.spec.track_id,
                                sample: track.samples[index].clone(),
                            },
                            Timestamp::ZERO,
                        )
                        .expect("sample");
                }
                while let Some(output) = stage.poll() {
                    if let MuxOutput::Bytes(chunk) = output {
                        bytes.extend_from_slice(&chunk);
                    }
                }
            }
            stage.finish().expect("finish");
            while let Some(output) = stage.poll() {
                if let MuxOutput::Bytes(chunk) = output {
                    bytes.extend_from_slice(&chunk);
                }
            }
            bytes
                .chunks_exact(TS_PACKET_SIZE)
                .map(|raw| {
                    let packet = TsPacket::parse(raw).expect("a whole TS packet");
                    let field = packet
                        .adaptation_field()
                        .map(|field| field.expect("the adaptation field parses"));
                    Seen {
                        pid: packet.header.pid.value(),
                        pusi: packet.header.pusi,
                        cc: packet.header.continuity_counter.value(),
                        discontinuity: field.is_some_and(|field| field.discontinuity_indicator),
                    }
                })
                .collect()
        }

        fn packets_on(seen: &[Seen], pid: u16) -> Vec<&Seen> {
            seen.iter().filter(|packet| packet.pid == pid).collect()
        }

        fn flagged_pids(seen: &[Seen]) -> Vec<u16> {
            let mut pids: Vec<u16> = seen
                .iter()
                .filter(|packet| packet.discontinuity)
                .map(|packet| packet.pid)
                .collect();
            pids.sort_unstable();
            pids.dedup();
            pids
        }

        /// The PID's third sample packet is its first after the break: it and
        /// only it carries the indicator, and its continuity counter carries on
        /// from the packet before it rather than restarting.
        fn assert_first_after_break(seen: &[Seen], pid: u16) {
            let run = packets_on(seen, pid);
            let starts: Vec<usize> = run
                .iter()
                .enumerate()
                .filter(|(_, packet)| packet.pusi)
                .map(|(index, _)| index)
                .collect();
            assert!(
                starts.len() >= SAMPLES,
                "{pid:#06x}: one PUSI packet per sample, got {starts:?}"
            );
            let marked = starts[SAMPLES_BEFORE_BREAK];
            let flagged: Vec<usize> = run
                .iter()
                .enumerate()
                .filter(|(_, packet)| packet.discontinuity)
                .map(|(index, _)| index)
                .collect();
            assert_eq!(
                flagged,
                alloc::vec![marked],
                "{pid:#06x}: only the first post-break packet is marked"
            );
            assert_eq!(
                run[marked].cc,
                (run[marked - 1].cc + 1) & CC_MASK,
                "{pid:#06x}: the continuity counter keeps counting across the break"
            );
        }

        let (_, ids) = pair();
        let seen = play(None);
        let es: Vec<u16> = {
            let mut pids: Vec<u16> = seen
                .iter()
                .map(|packet| packet.pid)
                .filter(|&pid| pid != PAT_PID.value() && pid != PMT_PID.value())
                .collect();
            pids.sort_unstable();
            pids.dedup();
            pids
        };
        assert_eq!(
            es.len(),
            2,
            "one video and one audio elementary PID: {es:?}"
        );
        for &pid in &es {
            assert_first_after_break(&seen, pid);
        }
        for pid in [PAT_PID.value(), PMT_PID.value()] {
            assert!(
                packets_on(&seen, pid)
                    .iter()
                    .all(|packet| !packet.discontinuity),
                "{pid:#06x}: a PSI packet is never marked"
            );
        }
        assert_eq!(flagged_pids(&seen), es, "the break marks every track");

        let video_only = play(Some(ids[0]));
        let audio_only = play(Some(ids[1]));
        let video_pid = flagged_pids(&video_only);
        let audio_pid = flagged_pids(&audio_only);
        assert_eq!(video_pid.len(), 1, "the video break marks one PID");
        assert_eq!(audio_pid.len(), 1, "the audio break marks one PID");
        assert_ne!(video_pid[0], audio_pid[0], "each break marks its own PID");
        for (pid, seen) in [(video_pid[0], &video_only), (audio_pid[0], &audio_only)] {
            assert_first_after_break(seen, pid);
            for &other in es.iter().filter(|&&other| other != pid) {
                assert!(
                    packets_on(seen, other)
                        .iter()
                        .all(|packet| !packet.discontinuity),
                    "{other:#06x}: a per-track break leaves the other PID unmarked"
                );
            }
        }
    }
}
