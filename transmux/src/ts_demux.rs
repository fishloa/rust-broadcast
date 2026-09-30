//! MPEG-2 Transport Stream demuxer → hub [`Media`] IR.
//!
//! [`StreamingTsDemux`] (issue #555) is the **one** demux core: an
//! event-driven, incremental engine that consumes TS bytes of any size or
//! alignment and emits [`DemuxEvent`]s (`TrackAdded`/`Sample`/
//! `ClockReference`/`Discontinuity`/`TracksResolved`) as soon as they are
//! known.
//! `TracksResolved` (issue #624) additionally tells a consumer when every
//! currently-known PMT-declared PID has resolved — the "safe to build a
//! multi-track segmenter now" signal. [`TsDemux`] — the
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
//! `dac3`/`dec3`, DTS core-frame header → `ddts`) → length-prefixed video /
//! raw audio samples.
//!
//! Config recovery happens incrementally, access unit by access unit: a
//! track's `DemuxEvent::TrackAdded` fires as soon as its config is known —
//! with an opaque [`CodecConfig::Data`] track (issue #557) firing on its very
//! first access unit, since its config needs no in-band header at all.
//!
//! The config is **not** frozen at that point. A stream may change its
//! parameter sets mid-flight (an SD↔HD ad break, a re-encode, a multiplex
//! reconfiguration), and a track whose in-band AVC/HEVC parameter sets or AAC
//! `ADTS` header actually change re-probes on that access unit and emits
//! `DemuxEvent::TrackUpdated` with the new config on the *same* `track_id`
//! (r04-W51) — mirroring `flv_stream`'s handling of a re-sent sequence
//! header. An unchanging repeat, which encoders send routinely, emits
//! nothing.
//!
//! HEVC (H.265) elementary streams are carried into the IR: the in-band
//! VPS/SPS/PPS NAL units are gathered from the Annex-B access units, decoded
//! into an `hvcC` [`HEVCConfigurationBox`], and emitted as a `hvc1`
//! [`CodecConfig::Hevc`] track — identical to the config `Fmp4Demux` recovers
//! from an fMP4 `hvcC` (issue #467). DTS elementary streams (stream_type
//! `0x82`/`0x85`/`0x8A`) are carried: the core-substream frame header
//! (`0x7FFE8001` sync) is parsed into a core-only `ddts` [`CodecConfig::Dts`]
//! track, mirroring the AC-3/E-AC-3 recovery path (issue #560, see
//! [`crate::dts`]).
//!
//! Every video and audio sample additionally carries **absolute** `dts`/`pts`
//! (media plane step 2c) recovered from the PES clock (issue #556): the
//! 33-bit wire PTS/DTS is unwrapped incrementally, once, right here at the
//! demux edge (by this module's internal `WrapState`, matching
//! `timed_metadata::Timeline`'s
//! semantics) — nothing downstream re-derives it. Video/AAC/MPEG-audio
//! samples get the unwrapped PTS/DTS of the access unit they were decoded
//! from (with per-frame interpolation when a PES payload splits into several
//! frames); AC-3/E-AC-3/DTS elementary streams are additionally split into
//! individual syncframes/core frames (rather than one zero-duration `Sample`
//! per PES access unit — see [`crate::ac3`] / [`crate::dts`]) so real
//! durations and exact PES-boundary timestamps survive into the IR.
//! Video/data-track sample durations are resolved **one access unit
//! behind**: the timestamp delta to the *next* access unit (unwrapped DTS
//! for video, PTS for data — ISO/IEC 13818-1 §2.4.3.7) finalizes the
//! *previous* sample's duration, with the final sample of a finished stream
//! reusing the previous duration ([`finish`](StreamingTsDemux::finish)).
//!
//! Any PMT `stream_type` that is not a decoded codec is carried losslessly as
//! an opaque [`CodecConfig::Data`] track (issues #557/#576) rather than
//! silently dropped — `stream_type` 0x06 (PES private data — DVB
//! subtitles/teletext/SMPTE 2038/AC-3/E-AC-3/DTS/etc.) and 0x15 (metadata in
//! PES) were the first examples; every other unrecognised `stream_type`
//! follows the same path. A `0x06`/`0x15` stream carrying an AC-3 (`0x6A`),
//! enhanced AC-3 (`0x7A`), or DTS (`0x7B`) ES_info descriptor is instead
//! reclassified to that audio codec (issue #641: DVB's standard descriptor-
//! disambiguated Dolby/DTS carriage), reaching the same syncframe-recovery
//! path as the native `0x81`/`0x87`/`0x8*` stream_types. `descriptors`
//! preserves the raw PMT ES_info descriptor loop for the caller to classify.
//! ISO/IEC 13818-1 §2.4.4.8 / Table 2-34 splits
//! `stream_type` into two carriage families, and the two are reassembled
//! completely differently (PES-reassembling a section stream, or vice versa,
//! silently yields nothing): most `stream_type`s (including every
//! unrecognised one) are PES-packetised and each `Sample` is one verbatim PES
//! payload; a fixed set (`0x05` private_sections, `0x0A`-`0x0D` DSM-CC, `0x14`
//! DSM-CC synchronized download, `0x86` SCTE-35/ANSI-scoped) carry PSI/private
//! *sections* directly on the PID (§2.4.4) — each reassembled via
//! [`mpeg_ts::ts::SectionReassembler`] instead of a PES assembler, and each
//! complete section becomes one `Sample` with no timestamp at all
//! (`dts: None, pts: None` — never fabricated).
//! [`CodecConfig::Data`]'s `carriage` field ([`DataCarriage`]) records which
//! family a track uses. The demuxer also collects every PCR observation from
//! the TS adaptation fields, both into [`Media`]'s `pcr` field (batch) and as
//! [`DemuxEvent::ClockReference`] (streaming).
//!
//! [`CodecConfig`]: crate::pipeline::CodecConfig
//! [`DataCarriage`]: crate::pipeline::DataCarriage
//!
//! # Spec
//!
//! - **PAT / PMT section syntax**: ITU-T H.222.0 (= ISO/IEC 13818-1) §2.4.4.3 /
//!   §2.4.4.8 — see `docs/codec/ts-demux-13818-1.md`.
//! - **stream_type → codec / carriage**: ISO/IEC 13818-1 §2.4.4.8, Table 2-34
//!   (PES- vs section-carried `stream_type`s) + ETSI TS 101 154 §G (DVB
//!   user-private AC-3/E-AC-3/DTS assignments).
//! - **PES-over-TS reassembly + PTS/DTS**: ISO/IEC 13818-1 §2.4.3.6 / §2.4.3.7
//!   (via [`mpeg_pes`], 33-bit @ 90 kHz).
//! - **PSI/private section reassembly**: ISO/IEC 13818-1 §2.4.4, via
//!   [`mpeg_ts::ts::SectionReassembler`].
//! - **PCR**: ISO/IEC 13818-1 §2.4.3.4 (adaptation field) / §2.4.3.5 (PCR encoding).
//! - **Byte-stream resynchronisation**: ISO/IEC 13818-1 §2.4.3.2, via
//!   [`mpeg_ts::resync::TsResync`] (also strips 204-byte Reed-Solomon FEC).

use alloc::collections::btree_map::Entry;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Demand, Serialize, Stage, Timestamp, Unpackage};
use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::resync::TsResync;
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};

use crate::aac_asc::{AdtsHeader, AudioSpecificConfig, parse_adts_header};
use crate::ac3::{
    AC3_SAMPLES_PER_SYNCFRAME, Ac3SyncframeInfo, Ec3SpecificBox, Ec3SyncframeInfo,
    split_ac3_syncframes, split_eac3_syncframes,
};
use crate::annexb::{annexb_to_length_prefixed, iter_annexb_nals};
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::dts::{DtsCoreFrameInfo, split_dts_core_frames};
use crate::error::{Error, Result};
use crate::hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
use crate::media::{Media, PcrSample, Track};
use crate::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor,
};
use crate::mpeg_legacy::{Mpeg2SeqHeader, MpegAudioFrameHeader};
use crate::mpegh::{MHADecoderConfigurationRecord, find_mpegh3da_config};
use crate::nal::{NalCodec, access_unit_is_rap, is_keyframe_nal, nal_unit_type};
use crate::nalu_types::{AvcPps, AvcSps, HevcNalArray, HevcNalUnit};
use crate::pipeline::{CodecConfig, DataCarriage, Provenance, Sample, SampleFlags, TrackSpec};

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
/// Mask for the 5-bit `version_number` within a long-form section's byte 5
/// (§2.4.4.1: `reserved`(2) + `version_number`(5) + `current_next_indicator`(1)),
/// after shifting right by 1 to drop the `current_next_indicator` bit.
const VERSION_NUMBER_MASK: u8 = 0x1F;
/// Bit for `current_next_indicator` within a long-form section's byte 5
/// (§2.4.4.1) — `1` means the table is applicable now, `0` means it is a
/// not-yet-applicable "next" table (parsed, never acted on).
const CURRENT_NEXT_INDICATOR_BIT: u8 = 0x01;
/// Trailing `CRC_32` on every long-form PSI section (§2.4.4.1).
const CRC32_LEN: usize = 4;
/// `section_syntax_indicator` bit within a section's byte 1 (§2.4.4.1). `1`
/// marks the long form — a `table_id_extension`/`version_number` header **and**
/// a trailing [`CRC32_LEN`]-byte `CRC_32`. A PAT (§2.4.4.5 Table 2-30) and a
/// PMT (§2.4.4.9 Table 2-33) both fix it at `1`, so a PAT/PMT section that
/// clears it is malformed and carries no CRC to check.
const SECTION_SYNTAX_INDICATOR_BIT: u8 = 0x80;
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
/// Largest possible TS payload (no adaptation field at all, ISO/IEC
/// 13818-1 §2.4.3.2): [`TS_PACKET_SIZE`] minus the 4-byte fixed header.
/// [`Stage::demand`](broadcast_common::Stage::demand)'s saturation check uses
/// this as the "one more worst-case packet" margin against
/// [`MAX_UNATTRIBUTED_BYTES`] (see that impl's doc comment).
const TS_MAX_PAYLOAD_BYTES: usize = TS_PACKET_SIZE - 4;
/// Offset just past `PES_packet_length` within a PES packet — the first 6
/// bytes are `packet_start_code_prefix`(3) + `stream_id`(1) +
/// `PES_packet_length`(2) (ISO/IEC 13818-1 §2.4.3.7, Table 2-21).
const PES_LENGTH_FIELD_END: usize = 6;
/// Hard cap on one PID's in-progress PES buffer (issue #663 P5.2,
/// audit-ingest's "bounded reassembly" recommendation applied to TS). A PES
/// runs from one `payload_unit_start_indicator` to the next
/// ([`mpeg_pes::PesAssembler`]'s doc); the unbounded-video case
/// (`PES_packet_length = 0`) means there is no length field to bound it
/// in-band, so a PUSI that never recurs — a wedged/lossy capture, or a
/// hostile stream — would otherwise grow that PID's buffer for the life of
/// the stream. Comfortably above any real elementary-stream PES payload (a
/// 4K IDR frame is typically well under a megabyte), but far below what a
/// malformed input could accumulate unbounded. On overflow the in-progress
/// PES is dropped (never emitted) and a [`DemuxEvent::Discontinuity`] is
/// raised for the PID — reassembly resyncs at the next PUSI. Note:
/// PSI/private-section buffering (`Carrier::Section`) needs no equivalent
/// cap — [`mpeg_ts::ts::SectionReassembler`] is already inherently bounded by
/// `section_length`'s 12-bit field (`MAX_SECTION_SIZE`, 4098 bytes).
const MAX_PES_BUFFER_BYTES: usize = 4 * 1024 * 1024;
/// Hard cap on one PID's accumulated [`TrackState::Probing`]/
/// [`TrackState::Parked`] backlog (issue B8, media plane step 2 fix wave 3).
/// A PMT-listed codec PID whose parameter sets never arrive (a broken
/// encoder, not malice — e.g. an H.264 ES that never carries SPS/PPS) leaves
/// that PID `Probing` forever, growing `backlog` without bound; worse,
/// [`StreamingTsDemux::try_promote_ready`] `break`s at the first `Probing`
/// PID it finds, so a later-ranked PID that *has* resolved (`Parked`)
/// accumulates its own backlog as collateral for as long as the earlier PID
/// never resolves. Tracked incrementally in
/// [`StreamState::backlog_bytes`] (never re-walked per push, matching
/// [`MAX_UNATTRIBUTED_BYTES`]'s own running-total convention). On overflow —
/// whether `Probing` or `Parked` — [`advance_track`] abandons the PID
/// ([`TrackState::Abandoned`]: permanently resolved without ever promoting to
/// `Live`, backlog dropped to free the memory), the same conclusion
/// [`StreamingTsDemux::finish`] already reaches for a probe that never
/// resolves, just reached early via the byte cap instead of end-of-input; a
/// [`DemuxEvent::Discontinuity`] is raised so the loss is visible, and
/// `try_promote_ready` continues past it, unblocking any later-ranked PID.
const MAX_PROBE_BACKLOG_BYTES: usize = 4 * 1024 * 1024;

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
/// MPEG-H 3D Audio main stream (MHAS, ISO/IEC 23008-3) — ISO/IEC 13818-1
/// Table 2-34 / ETSI TS 101 154 §6.8 (issue #579). §6.8 additionally allows
/// `0x2E` for an auxiliary (non-main) multi-stream MPEG-H component
/// (§6.8.7) — out of scope here; only the single/main-stream `0x2D` is
/// recognised.
const STREAM_TYPE_MPEGH: u8 = 0x2D;
/// PES private data (ISO/IEC 13818-1 Table 2-34) — DVB's standard carriage
/// for AC-3/E-AC-3/DTS audio, subtitles, teletext, SMPTE 2038, etc., all
/// disambiguated by the ES_info descriptor loop, not the `stream_type` byte
/// itself (issue #641).
const STREAM_TYPE_PES_PRIVATE: u8 = 0x06;
/// Metadata in PES packets (ISO/IEC 13818-1 Table 2-34) — the other
/// descriptor-disambiguated `stream_type`, per [`STREAM_TYPE_PES_PRIVATE`].
const STREAM_TYPE_METADATA_PES: u8 = 0x15;
/// AC-3 descriptor tag (ETSI EN 300 468 Annex D, issue #641).
const DESC_TAG_AC3: u8 = 0x6A;
/// Enhanced AC-3 (E-AC-3) descriptor tag (ETSI EN 300 468 Annex D).
const DESC_TAG_ENHANCED_AC3: u8 = 0x7A;
/// DTS descriptor tag (ETSI EN 300 468 Annex G, Table G.1).
const DESC_TAG_DTS: u8 = 0x7B;
// ── Section-carried stream_types (ISO/IEC 13818-1 Table 2-34) — issue #576 ──
//
// These stream_types carry PSI/private *sections* directly on their PID, not
// PES packets: PES-reassembling them silently yields nothing (no PES start
// code is ever present), so `data_carriage` routes them to a
// [`mpeg_ts::ts::SectionReassembler`] instead.

/// ISO/IEC 13818-1 `private_sections` carried directly (not in PES packets).
const STREAM_TYPE_PRIVATE_SECTIONS: u8 = 0x05;
/// ISO/IEC 13818-6 DSM-CC Type A (Multiprotocol Encapsulation), sectioned.
const STREAM_TYPE_DSMCC_TYPE_A: u8 = 0x0A;
/// ISO/IEC 13818-6 DSM-CC Type B (Type B), sectioned.
const STREAM_TYPE_DSMCC_TYPE_B: u8 = 0x0B;
/// ISO/IEC 13818-6 DSM-CC Type C (data or object carousel), sectioned.
const STREAM_TYPE_DSMCC_TYPE_C: u8 = 0x0C;
/// ISO/IEC 13818-6 DSM-CC Type D, sectioned.
const STREAM_TYPE_DSMCC_TYPE_D: u8 = 0x0D;
/// ISO/IEC 13818-6 DSM-CC synchronized download protocol, sectioned.
const STREAM_TYPE_DSMCC_SYNC_DOWNLOAD: u8 = 0x14;
/// SCTE-35 / ANSI-scoped applications (splice information table), sectioned.
const STREAM_TYPE_SCTE35: u8 = 0x86;

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
/// `pub(crate)`: also used by `ps_demux` (C6, #1009) to build the same `esds`.
pub(crate) const OTI_MPEG2_VIDEO_MAIN: u8 = 0x61;
/// `esds` `objectTypeIndication` for MPEG-1 Audio, ISO/IEC 11172-3 (Table 5).
const OTI_MPEG1_AUDIO: u8 = 0x6B;
/// `esds` `objectTypeIndication` for MPEG-2 Audio, ISO/IEC 13818-3 (Table 5).
const OTI_MPEG2_AUDIO: u8 = 0x69;
/// `esds` `streamType` for an AudioStream (ISO/IEC 14496-1 Table 6).
const STREAM_TYPE_AUDIO: u8 = 0x05;
/// `esds` `streamType` for a VisualStream (ISO/IEC 14496-1 Table 6).
/// `pub(crate)`: also used by `ps_demux` (C6, #1009).
pub(crate) const STREAM_TYPE_VISUAL: u8 = 0x04;
/// `esds` `ES_ID` assigned to the single audio elementary stream.
const ESDS_ES_ID: u16 = 1;
/// `esds` `ES_ID` assigned to the single video elementary stream.
/// `pub(crate)`: also used by `ps_demux` (C6, #1009).
pub(crate) const ESDS_VIDEO_ES_ID: u16 = 2;
/// `SLConfigDescriptor` predefined body for MP4 file SL packaging
/// (ISO/IEC 14496-1 §7.3.2.3 — `predefined = 0x02`).
/// `pub(crate)`: also used by `ps_demux` (C6, #1009).
/// Audio sample size in bits carried in the sample entry (PCM-equivalent; 16).
const AUDIO_SAMPLE_SIZE_BITS: u16 = 16;

/// `MHADecoderConfigurationRecord.reference_channel_layout` placeholder for
/// TS carriage: the real CICP `ChannelConfiguration` is a field *inside* the
/// opaque `mpegh3daConfig()` bitstream (ISO/IEC 23008-3 §5, paid) that this
/// crate does not decode (config passthrough only — issue #579 scope), and
/// MPEG-2 TS carries no equivalent systems-layer field for it (unlike the
/// ISOBMFF `mhaC` box, whose `referenceChannelLayout` byte is authored
/// out-of-band by the muxer). `0` marks "not derived", mirroring this file's
/// existing `HVCC_*_UNSPEC` placeholders for fields it likewise cannot
/// recover from the elementary stream alone.
const MPEGH_REFERENCE_CHANNEL_LAYOUT_UNSPECIFIED: u8 = 0;
/// `CodecConfig::MpegH.channel_count` placeholder — same rationale as
/// [`MPEGH_REFERENCE_CHANNEL_LAYOUT_UNSPECIFIED`]: MPEG-2 TS carriage (PMT
/// `stream_type`/`MPEG-H_3dAudio_descriptor`) signals no channel count.
const MPEGH_CHANNEL_COUNT_UNSPECIFIED: u16 = 0;
/// `CodecConfig::MpegH.sample_rate` placeholder — same rationale. Samples
/// are still timed correctly: [`LiveKind::MpegH`] anchors durations on the
/// 90 kHz TS clock ([`VIDEO_TIMESCALE`]) rather than an audio sample count,
/// so an unknown `sample_rate` here never affects timing.
const MPEGH_SAMPLE_RATE_UNSPECIFIED: u32 = 0;
/// Video media timescale (90 kHz — the TS/PES timestamp clock).
const VIDEO_TIMESCALE: u32 = 90_000;
/// Samples per AAC access unit (ISO/IEC 14496-3 — one frame = 1024 samples).
const AAC_SAMPLES_PER_FRAME: u32 = 1024;
/// ADTS fixed header length (bytes) — `crate::aac_asc` `ADTS_HEADER_SIZE`.
const ADTS_HEADER_SIZE: usize = 7;

/// MPEG-2 video `picture_start_code` (0x00000100) — ISO/IEC 13818-2 §6.2.3.
/// `pub(crate)`: also used by `ps_demux` (C6, #1009) to split/flag MPEG-2
/// pictures the same way this module does.
pub(crate) const MPEG2_PICTURE_START_CODE: u8 = 0x00;
/// `picture_coding_type` value for an intra-coded (I) picture — §6.3.9 Table 6-12.
pub(crate) const MPEG2_PICTURE_CODING_TYPE_I: u8 = 0x01;

/// 33-bit PTS/DTS modulus, for wrap-around unrolling (§2.4.3.7, 90 kHz clock).
/// Alias for [`broadcast_common::clock33::WRAP_33BIT`] — the actual
/// wrap-correction math (below, [`WrapState`]) is delegated there so a fix
/// reaches every 33-bit clock consumer in the workspace, not just this one.
const TS_WRAP: u64 = broadcast_common::clock33::WRAP_33BIT;

/// Mask of the 33-bit PTS/DTS field — [`TS_WRAP`] minus one, for reducing a
/// modular difference back onto the wire clock (§2.4.3.7).
const TS_WRAP_MASK: u64 = TS_WRAP - 1;

/// Largest observed inter-access-unit step (90 kHz ticks) still believed to be
/// a frame period when interpolating an unstamped PES (§2.4.2.6, r04-W48) and
/// when lifting a decode timeline at a signalled discontinuity.
///
/// One second on the 90 kHz clock. The earlier value was ten seconds, which is
/// far too loose: an 8-second forward splice is not this stream's cadence, yet
/// it sat inside the window and became *the* frame period for every unstamped
/// access unit that followed (r04-W48/W50 review). No real frame rate is below
/// 1 fps, so a second is a generous ceiling that still excludes every gap,
/// splice and clock jump this is meant to reject.
const MAX_FRAME_PERIOD_TICKS: u64 = VIDEO_TIMESCALE as u64;

/// How many recent inter-access-unit steps
/// [`StreamState::recent_steps`] keeps for the median estimate. Five is the
/// smallest window in which a single outlier (one late access unit) cannot
/// dominate the middle value.
const FRAME_PERIOD_WINDOW: usize = 5;

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
    /// MPEG-H 3D Audio main stream, MHAS-formatted (issue #579) — see
    /// [`crate::mpegh`].
    MpegH,
    /// Opaque data stream (issue #557/#576): any `stream_type` this demuxer
    /// does not decode to a typed codec — carried losslessly instead of
    /// dropped. The field is the PMT `stream_type` itself, carried through
    /// into [`CodecConfig::Data`]; [`data_carriage`] classifies it as PES- or
    /// section-carried.
    Data(u8),
}

impl Codec {
    /// Map a PMT `stream_type` to a [`Codec`] — a decoded codec when this
    /// demuxer understands it, else an opaque [`Codec::Data`] carrying the
    /// `stream_type` verbatim (issue #576: every PMT-listed elementary stream
    /// gets a track, never silently dropped). ISO/IEC 13818-1 Table 2-34.
    fn from_stream_type(stream_type: u8) -> Self {
        match stream_type {
            STREAM_TYPE_MPEG2_VIDEO => Codec::Mpeg2Video,
            STREAM_TYPE_MPEG1_AUDIO => Codec::MpegAudio(false),
            STREAM_TYPE_MPEG2_AUDIO => Codec::MpegAudio(true),
            STREAM_TYPE_AVC => Codec::H264,
            STREAM_TYPE_HEVC => Codec::Hevc,
            STREAM_TYPE_AAC_ADTS => Codec::Aac,
            STREAM_TYPE_AC3 => Codec::Ac3,
            STREAM_TYPE_EAC3 => Codec::Eac3,
            STREAM_TYPE_DTS_82 | STREAM_TYPE_DTS_85 | STREAM_TYPE_DTS_8A => Codec::Dts,
            STREAM_TYPE_MPEGH => Codec::MpegH,
            _ => Codec::Data(stream_type),
        }
    }

    /// Refine a [`Codec::Data`] classification for `stream_type` `0x06`/`0x15`
    /// (DVB's descriptor-disambiguated PES private data / metadata carriage)
    /// by consulting the ES_info descriptor loop, per ETSI EN 300 468: an
    /// AC-3 (`0x6A`), enhanced AC-3 (`0x7A`), or DTS (`0x7B`) descriptor
    /// reclassifies the stream to the matching audio codec instead of opaque
    /// data (issue #641). Any other `stream_type`, or a `0x06`/`0x15` stream
    /// with none of those descriptors (e.g. DVB subtitles/teletext), is
    /// returned unchanged.
    fn refine_with_descriptors(self, stream_type: u8, descriptors: &[u8]) -> Self {
        if !matches!(
            stream_type,
            STREAM_TYPE_PES_PRIVATE | STREAM_TYPE_METADATA_PES
        ) {
            return self;
        }
        let mut off = 0usize;
        while off + 2 <= descriptors.len() {
            let tag = descriptors[off];
            let len = descriptors[off + 1] as usize;
            match tag {
                DESC_TAG_AC3 => return Codec::Ac3,
                DESC_TAG_ENHANCED_AC3 => return Codec::Eac3,
                DESC_TAG_DTS => return Codec::Dts,
                _ => {}
            }
            off += 2 + len;
        }
        self
    }
}

/// Classify a [`Codec::Data`] `stream_type` as PES- or section-carried
/// (ISO/IEC 13818-1 §2.4.4.8 / Table 2-34) — see [`DataCarriage`]. A fixed set
/// of `stream_type`s carry PSI/private sections directly; every other
/// `stream_type` (the historical 0x06/0x15 carriage, plus any unrecognised
/// value) is PES-packetised.
fn data_carriage(stream_type: u8) -> DataCarriage {
    match stream_type {
        STREAM_TYPE_PRIVATE_SECTIONS
        | STREAM_TYPE_DSMCC_TYPE_A
        | STREAM_TYPE_DSMCC_TYPE_B
        | STREAM_TYPE_DSMCC_TYPE_C
        | STREAM_TYPE_DSMCC_TYPE_D
        | STREAM_TYPE_DSMCC_SYNC_DOWNLOAD
        | STREAM_TYPE_SCTE35 => DataCarriage::Sections,
        _ => DataCarriage::Pes,
    }
}

/// Extend a running unwrapped timestamp by the delta to the next raw 33-bit
/// value, correcting for a single 90 kHz wrap in either direction (§2.4.3.7).
///
/// Thin alias for [`broadcast_common::clock33::unwrap_delta`] — see that
/// function's doc comment for why the wrap correction must be bidirectional
/// (a forward-only epoch counter gets a backward-reorder-across-origin case
/// wrong). Kept as a local `fn` (rather than calling the shared function
/// directly at each call site) only so [`WrapState::push`] below reads the
/// same as it always has.
fn unwrap_ts(prev_unwrapped: i128, prev_raw: u64, raw: u64) -> i128 {
    broadcast_common::clock33::unwrap_delta(prev_unwrapped, prev_raw, raw)
}

/// Rescale an unwrapped 90 kHz PES-clock timestamp (ISO/IEC 13818-1 §2.4.3.7)
/// into a track's own media timescale, floored (i128 math so a full 33-bit
/// anchor cannot overflow).
///
/// A **negative** unwrapped anchor is preserved, not clamped to zero. It is a
/// legitimate value: reordering (or a capture that starts mid-GOP) across the
/// 2^33 wrap boundary unwraps to a small negative absolute time, and every
/// other track kind already carries that through to `Sample::dts` verbatim —
/// clamping it only for audio (as this used to, via `.max(0) as u128`)
/// fabricated `dts = 0` for the audio track alone and desynced it from the
/// video it was muxed against.
///
/// The PES clock is always 90 kHz, but an audio track's IR timescale is its
/// **sample rate** (`TrackSpec::timescale`), and since media plane step 2c
/// `Sample::dts`/`Sample::pts` are defined to be in that track timescale — the
/// same unit as `Sample::duration`. Storing the raw 90 kHz value for an audio
/// track would make `dts` deltas (e.g. 2089) disagree with `duration` (1024
/// AAC samples), which is exactly the quantity every downstream consumer
/// (RTP packetisation, segmentation, `tfdt`) reads. For a 90 kHz track (video,
/// opaque `Data`) this is the identity.
fn rescale_to_track(anchor_90k: i128, timescale: u32) -> i64 {
    let ts = timescale.max(1) as i128;
    let scaled = if ts == VIDEO_TIMESCALE as i128 {
        anchor_90k
    } else {
        // `div_euclid` (not `/`, which truncates toward zero) keeps the
        // documented floor semantics on both sides of zero.
        (anchor_90k * ts).div_euclid(VIDEO_TIMESCALE as i128)
    };
    to_ticks(scaled)
}

/// Whether an MPEG-2 video access unit is a random-access point: it carries a
/// `sequence_header()` (0x000001B3) or its `picture_header()` codes an I-frame
/// (`picture_coding_type == 1`) — ISO/IEC 13818-2 §6.2.2.1 / §6.3.9.
pub(crate) fn mpeg2_is_sync(au: &[u8]) -> bool {
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

/// Counts one byte-offset probe of a codec-config sync scanner
/// ([`find_mpeg_audio_sync`] / [`find_adts_sync`]).
///
/// Test-only instrumentation for the r04-W47 complexity bound: the codec probes
/// must scan the *newest* access unit only, so the total probes a demux spends
/// on a never-resolving PID must stay proportional to the input length, not to
/// its square. The counter is read by
/// `probe_sync_scan_work_is_linear_in_the_input_not_quadratic`, which asserts
/// the bound, and is incremented at the actual scan site (inside the sync loop)
/// rather than re-derived by the test, so the number it measures is the work
/// the demux really did. Per-thread because the crate's own test suite runs in
/// parallel: a process-wide counter would be summed across unrelated tests, so
/// the bound would measure other tests' demux work instead of this one's.
///
/// Compiled only for the test build — the crate is `no_std`, where
/// `thread_local!` is not available, and nothing outside a test reads it.
#[cfg(test)]
mod probe_counter {
    thread_local! {
        static SYNC_PROBES: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
    }

    pub(super) fn record() {
        SYNC_PROBES.with(|c| c.set(c.get().wrapping_add(1)));
    }

    pub(super) fn read() -> u64 {
        SYNC_PROBES.with(|c| c.get())
    }
}

/// Record one scan-site byte probe (see [`probe_counter`]).
#[cfg(test)]
fn record_sync_probe() {
    probe_counter::record();
}

/// Record one pass of a codec-config probe over an access unit.
///
/// The H.264/HEVC probes have no byte-by-byte *sync* scan — they walk the
/// access unit's NAL units with [`iter_annexb_nals`], which itself scans for
/// start codes — so `record_sync_probe` does not see them. This counter does,
/// making the "scan only the newest access unit" bound measurable for every
/// probe kind (r04-W47 review).
#[cfg(test)]
fn record_probe_pass() {
    probe_counter::record();
}

/// Non-test build: nothing to record.
#[cfg(not(test))]
#[inline(always)]
fn record_probe_pass() {}

/// Non-test build: nothing to record.
#[cfg(not(test))]
#[inline(always)]
fn record_sync_probe() {}

/// Read this thread's probe total (see [`record_sync_probe`]).
#[cfg(test)]
fn sync_probes() -> u64 {
    probe_counter::read()
}

/// Scan forward from the start of `data` for the first byte offset carrying a
/// valid MPEG audio frame header, returning that offset and the parsed
/// header. A broadcast MP2-in-PES payload is not guaranteed to start on a
/// frame boundary (issue #638: a real DVB-S multiplexer routinely splits PES
/// payloads without regard to the ~1253/1254-byte frame length) -- this
/// resyncs instead of requiring the syncword at offset 0. Bytes before the
/// returned offset are a partial frame tail from the previous payload and are
/// discarded (no cross-PES carry).
fn find_mpeg_audio_sync(data: &[u8]) -> Option<(usize, MpegAudioFrameHeader)> {
    let mut off = 0usize;
    while off + 4 <= data.len() {
        record_sync_probe();
        if let Ok(hdr) = MpegAudioFrameHeader::parse(&data[off..]) {
            return Some((off, hdr));
        }
        off += 1;
    }
    None
}

/// Split a concatenated MPEG audio payload into individual frames using the
/// frame-header length field (ISO/IEC 11172-3 §2.4.1.3). Resyncs to the next
/// frame boundary on a bad sync (see [`find_mpeg_audio_sync`]); stops once no
/// further sync is found or a frame would run past the end of `payload`, so a
/// partial tail does not lose earlier frames.
fn split_mpeg_audio_frames(payload: &[u8]) -> Vec<&[u8]> {
    let mut frames = Vec::new();
    let mut off = 0usize;
    while off + 4 <= payload.len() {
        let Some((sync_off, hdr)) = find_mpeg_audio_sync(&payload[off..]) else {
            break;
        };
        off += sync_off;
        let flen = hdr.frame_length;
        if flen < 4 || off + flen > payload.len() {
            break;
        }
        frames.push(&payload[off..off + flen]);
        off += flen;
    }
    frames
}

/// Scan forward from the start of `data` for the first byte offset carrying a
/// valid ADTS frame header, returning that offset and the parsed header --
/// see [`find_mpeg_audio_sync`] for why a broadcast PES payload isn't
/// guaranteed to start on a frame boundary (issue #638). Bytes before the
/// returned offset are a partial frame tail from the previous payload and are
/// discarded (no cross-PES carry).
fn find_adts_sync(data: &[u8]) -> Option<(usize, AdtsHeader)> {
    let mut off = 0usize;
    while off + ADTS_HEADER_SIZE <= data.len() {
        record_sync_probe();
        if let Ok(hdr) = parse_adts_header(&data[off..]) {
            return Some((off, hdr));
        }
        off += 1;
    }
    None
}

/// Split a concatenated ADTS payload into individual frames (whole frame
/// bytes — fixed+variable header, the CRC when present, and the raw data
/// block(s) — plus that frame's parsed [`AdtsHeader`]). Resyncs to the next
/// frame boundary on a bad sync (see [`find_adts_sync`]); stops once no
/// further sync is found or a frame would run past the end of `payload`, so
/// a partial tail does not lose earlier frames. `frame_length` (ISO/IEC
/// 13818-7 §6.2) already spans the whole frame including any CRC, so this
/// boundary-finding needs no change for C11 (#1012) — only the caller's
/// fixed 7-byte strip did.
fn split_adts_frames(payload: &[u8]) -> Vec<(&[u8], AdtsHeader)> {
    let mut frames = Vec::new();
    let mut off = 0usize;
    while off + ADTS_HEADER_SIZE <= payload.len() {
        let Some((sync_off, hdr)) = find_adts_sync(&payload[off..]) else {
            break;
        };
        off += sync_off;
        let frame_len = hdr.frame_length as usize;
        if frame_len < ADTS_HEADER_SIZE || off + frame_len > payload.len() {
            break;
        }
        frames.push((&payload[off..off + frame_len], hdr));
        off += frame_len;
    }
    frames
}

/// `adts_error_check()`'s `crc_check` (ISO/IEC 13818-7 §6.2): a 16-bit CRC
/// immediately after the fixed+variable header when `protection_absent ==
/// 0`.
const ADTS_CRC_SIZE: usize = 2;

/// Bytes to strip from the front of a whole ADTS frame to reach its raw
/// audio payload: the header, plus the 2-byte CRC when present (C11,
/// #1012). For `number_of_raw_data_blocks_in_frame > 0` with a CRC present,
/// the spec also inserts one CRC per raw data block between them
/// (`adts_header_error_check()` / `adts_raw_data_block_error_check()`) —
/// those are not removed; only the single header-level CRC is, matching the
/// documented, intentionally-partial fix for the rare multi-block+CRC case.
fn adts_header_len(hdr: &AdtsHeader) -> usize {
    if hdr.protection_absent {
        ADTS_HEADER_SIZE
    } else {
        ADTS_HEADER_SIZE + ADTS_CRC_SIZE
    }
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

/// Parse a PAT section, returning every `(program_number, program_map_PID)`
/// pair it lists (network entries — `program_number == 0` — are skipped).
/// ISO/IEC 13818-1 §2.4.4.3. The `program_number` is kept (not just the PID)
/// so a PMT section can be cross-checked against the program it was learned
/// under (issue #774).
fn parse_pat(section: &[u8]) -> Result<Vec<(u16, u16)>> {
    if section.first().copied() != Some(TABLE_ID_PAT) {
        return Ok(Vec::new());
    }
    let body = section_body(section, "PAT")?;
    let mut programs = Vec::new();
    let mut off = 0usize;
    while off + PAT_ENTRY_LEN <= body.len() {
        let program_number = u16::from_be_bytes([body[off], body[off + 1]]);
        let pid = (((body[off + 2] & PID_HI_MASK) as u16) << 8) | body[off + 3] as u16;
        if program_number != NETWORK_PROGRAM_NUMBER {
            programs.push((program_number, pid));
        }
        off += PAT_ENTRY_LEN;
    }
    Ok(programs)
}

/// A PMT section's header fields beyond `table_id` (ISO/IEC 13818-1 §2.4.4.8 /
/// Table 2-33), read directly from `section[]` (the whole section including
/// its 8-byte header) — the version-diffing prerequisite for issue #774.
struct PmtSectionHeader {
    /// `table_id_extension`, which for a PMT is `program_number` (`section[3..5]`).
    program_number: u16,
    /// `version_number` (`section[5]`, bits `[5:1]`).
    version: u8,
    /// `current_next_indicator` (`section[5]`, bit 0). `false` means this
    /// table is not yet applicable — parsed, never diffed/acted on.
    current_next: bool,
    /// `section_number` (`section[6]`).
    section_number: u8,
    /// `last_section_number` (`section[7]`). A PMT is always single-section,
    /// so a genuine PMT always has `section_number == last_section_number == 0`.
    last_section_number: u8,
    /// `PCR_PID` (`section[8..10]`, 13 bits — §2.4.4.8 Table 2-33): the PID
    /// whose adaptation fields carry this program's `PCR`, and therefore the
    /// PID on which a system time-base discontinuity is signalled
    /// (§2.4.3.5). `0x1FFF` means "no PCR for this program".
    pcr_pid: u16,
}

/// Parse a PMT section's header fields (§2.4.4.8) — everything needed to
/// decide whether a newly-reassembled section should be *applied*
/// (`current_next_indicator == 1` and `version_number` differs from the last
/// **applied** version), before paying for the ES-loop walk in [`parse_pmt`].
fn parse_pmt_section_header(section: &[u8]) -> Result<PmtSectionHeader> {
    if section.first().copied() != Some(TABLE_ID_PMT) {
        return Err(Error::InvalidValue {
            field: "table_id",
            value: section.first().copied().unwrap_or(0) as u64,
            reason: "not a PMT section",
        });
    }
    if section.len() < SECTION_HEADER_LEN {
        return Err(Error::BufferTooShort {
            need: SECTION_HEADER_LEN,
            have: section.len(),
            what: "PMT section header",
        });
    }
    Ok(PmtSectionHeader {
        program_number: u16::from_be_bytes([section[3], section[4]]),
        version: (section[5] >> 1) & VERSION_NUMBER_MASK,
        current_next: section[5] & CURRENT_NEXT_INDICATOR_BIT != 0,
        section_number: section[6],
        last_section_number: section[7],
        // PCR_PID sits at the start of the section body, which is
        // `section[8..]` (the 8-byte header ends at index 8): reserved(3) +
        // PCR_PID(13).
        pcr_pid: (((section[8] & PID_HI_MASK) as u16) << 8) | section[9] as u16,
    })
}

/// Parse a PMT section, returning `(elementary_PID, codec, ES_info
/// descriptors)` for every elementary stream listed (issue #576: every
/// PMT-listed ES becomes a track — typed when the `stream_type` maps to a
/// decoded codec, else opaque [`Codec::Data`]). ISO/IEC 13818-1 §2.4.4.8.
/// `descriptors` is the raw ES_info descriptor-loop bytes for that stream
/// (empty when `ES_info_length` is 0); consumers that don't need it (every
/// codec but [`Codec::Data`]) simply ignore it.
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
        let descriptors = body[desc_start..desc_end].to_vec();
        let codec =
            Codec::from_stream_type(stream_type).refine_with_descriptors(stream_type, &descriptors);
        out.push((es_pid, codec, descriptors));
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

/// Validate a long-form PSI section's trailing `CRC_32` (ISO/IEC 13818-1
/// §2.4.4.1) — the gate every PAT/PMT section must clear *before* anything
/// acts on it.
///
/// PMT application is **destructive** (issue #774 turned it into a track-set
/// diff that tears a live track down and reassigns its `track_id`), and a PAT
/// entry binds a PID to a `program_number` that every later PMT on that PID is
/// cross-checked against — so a single flipped bit in a version byte or an ES
/// loop must never be believed. Both tables fix `section_syntax_indicator` at
/// `1` (§2.4.4.5 Table 2-30 / §2.4.4.9 Table 2-33), i.e. both always carry the
/// CRC this checks; a PAT/PMT section that clears the bit is malformed and
/// carries no checkable trailer, so it is rejected here rather than acted on
/// unverified.
///
/// A rejected section is **dropped silently**: DEMUX is lenient — a corrupt
/// section is a discarded section, not a stream error — and, critically, it
/// must not disturb any already-applied state (no `TrackRemoved`, no
/// `last_applied_version` bump, no PMT-PID rebinding).
///
/// The CRC itself comes from [`broadcast_common::crc32_mpeg2`] (the shared
/// CRC-32/MPEG-2 every PSI trailer in this workspace uses — never hand-rolled
/// here), computed over `table_id` through the last table byte and compared
/// against the big-endian trailer.
fn psi_section_crc_ok(section: &[u8]) -> bool {
    if section.len() < SECTION_HEADER_LEN + CRC32_LEN {
        return false;
    }
    if section[1] & SECTION_SYNTAX_INDICATOR_BIT == 0 {
        return false;
    }
    // `SectionReassembler` hands out exactly `3 + section_length` bytes, so
    // the declared length and the slice length already agree; re-deriving it
    // keeps this correct for any other caller and bounds the slice either way.
    let section_length =
        (((section[1] & SECTION_LENGTH_HI_MASK) as usize) << 8) | section[2] as usize;
    let total = 3 + section_length;
    if total > section.len() || total < SECTION_HEADER_LEN + CRC32_LEN {
        return false;
    }
    let (covered, trailer) = section[..total].split_at(total - CRC32_LEN);
    let declared = u32::from_be_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
    broadcast_common::crc32_mpeg2::compute(covered) == declared
}

/// A long-form PSI section's `current_next_indicator` (§2.4.4.1, byte 5 bit 0):
/// `true` when the table is applicable now, `false` for a not-yet-applicable
/// "next" table. Only ever called on a section that already cleared
/// [`psi_section_crc_ok`], which guarantees byte 5 exists.
fn section_current_next(section: &[u8]) -> bool {
    section
        .get(5)
        .is_some_and(|b| b & CURRENT_NEXT_INDICATOR_BIT != 0)
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
/// config. The moment enough header data is seen, [`finalize_probe`] returns
/// the finished [`CodecConfig`] and the owning [`TrackState`] moves to
/// `Parked` (backlog carried over as-is, still accumulating — see
/// [`TrackState`]). A live track whose in-band config header later changes is
/// re-probed through the same function (r04-W51).
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
    /// DTS core substream (issue #560): resolves from the first frame whose
    /// header parses — see [`crate::dts::DtsCoreFrameInfo`].
    Dts,
    /// MPEG-H 3D Audio (issue #579): resolves from the first access unit
    /// whose MHAS packets contain a `PACTYP_MPEGH3DACFG` — see
    /// [`crate::mpegh::find_mpegh3da_config`].
    MpegH,
    /// Opaque PES data (#557): the config (`stream_type` + descriptors) is
    /// already fully known from the PMT, so this probe finalizes on the very
    /// first access unit — no header scan needed.
    Data,
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
        Codec::Dts => ConfigProbe::Dts,
        Codec::MpegH => ConfigProbe::MpegH,
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
    Dts,
    MpegAudio { samples_per_frame: u32 },
}

/// Frame-exact dts/pts accumulator for a live [`LiveKind::Audio`] track
/// (issue B5, media plane step-2 fix wave 1).
///
/// An AAC/AC-3/E-AC-3/DTS/MPEG-audio frame's duration is *exact* in the
/// track's own timescale (e.g. always 1024 samples for an AAC frame), but the
/// 90 kHz PES clock every access unit is stamped with (ISO/IEC 13818-1
/// §2.4.3.7) is a lossy representation of that same instant — 90000 does not
/// evenly divide a typical audio sample rate — so re-deriving the track-tick
/// anchor from the wire clock on *every* access unit (via
/// [`rescale_to_track`]) injects up to ±1 track tick of jitter at every PES
/// boundary, even though the intrinsic per-frame durations within one access
/// unit are exact. The fix: anchor once from the first access unit, then
/// advance the running cursor purely by the accumulated intrinsic durations;
/// only re-anchor — and only then, signal a [`DemuxEvent::Discontinuity`] —
/// when the wire clock drifts from the predicted position by more than
/// [`audio_discontinuity_threshold_90k`], a genuine gap (splice, encoder
/// restart), never the sub-tick rounding noise the old per-AU rescale
/// mistook for one.
#[derive(Default)]
struct AudioAnchor {
    seed: Option<AudioAnchorSeed>,
}

#[derive(Clone, Copy)]
struct AudioAnchorSeed {
    /// Track-tick cursor for the *next* frame's dts.
    next_dts: i64,
    /// Track-tick cursor for the *next* frame's pts.
    next_pts: i64,
    /// The unwrapped 90 kHz dts this anchor was last (re-)established from —
    /// used only to predict where the wire clock should land next (drift
    /// detection), never to re-derive a per-frame dts/pts.
    anchor_dts_uw: i128,
    /// The unwrapped 90 kHz pts this anchor was last (re-)established from.
    anchor_pts_uw: i128,
    /// Track ticks advanced since `anchor_dts_uw`/`anchor_pts_uw` were set.
    ticks_since_anchor: i64,
}

/// Audio dts/pts re-anchor threshold, in milliseconds of 90 kHz clock —
/// see [`audio_discontinuity_threshold_90k`] for the derivation.
const AUDIO_REANCHOR_THRESHOLD_MS: i128 = 20;

/// Discontinuity threshold for the audio dts/pts anchor (issue B5): a wire PES
/// timestamp further than this from where the frame-exact accumulator predicts
/// it should be is a genuine gap (splice, encoder restart, PID reuse); anything
/// closer is muxer noise the anchor absorbs silently.
///
/// # Derivation
///
/// The original threshold was **one intrinsic sample period** (`ceil(90000 /
/// sample_rate)`, i.e. 3 ticks at 44.1 kHz). That is below what real muxers
/// actually produce, so it fired constantly on clean streams. An AAC frame at
/// 44.1 kHz is `1024 / 44100 s = 2089.795…` ticks of 90 kHz; a muxer that
/// stamps each PES with a constant *integer* increment (2090 is what the
/// common `1024 * 90000 / 44100` rounding yields) therefore accrues
/// `+0.204…` ticks per frame **by construction, on a perfectly continuous
/// stream** — crossing a 3-tick threshold after ~15 frames and every ~15
/// frames thereafter. Non-frame-aligned MP2 PES (issue #638) crosses it on
/// essentially every access unit. So the B5 anchor was inert and
/// [`DiscontinuityKind::TimelineReanchored`] was pure noise.
///
/// The bound instead comes from what a drift of this size *means*: audio that
/// is out of step with the media timeline by less than roughly 15–20 ms is
/// below the lip-sync detectability floor the broadcast recommendations work
/// to (ITU-R BT.1359-1's subjective detectability limits; ATSC A/85's ±15 ms
/// production tolerance), and re-anchoring inside that band trades a real,
/// visible `Discontinuity` event for an inaudible correction. Above it, the
/// wire clock has genuinely moved and the accumulator must follow it.
/// [`AUDIO_REANCHOR_THRESHOLD_MS`] = 20 ms = **1800 ticks** of 90 kHz, which
/// the constant-rounding muxer above reaches only after ~8800 frames (~3.4
/// minutes) — at which point the accumulated error really is 20 ms and
/// re-anchoring is the correct call, not a false positive.
///
/// Floored at two intrinsic sample periods so a degenerate/absurd sample rate
/// (below ~100 Hz, where one frame period exceeds the millisecond bound) still
/// gets a threshold wider than its own quantisation noise.
///
/// `pub(crate)`: also used by [`crate::ps_demux::build_ac3_track`], which has
/// the identical 90 kHz-PES-stamp-vs-sample_rate-track-clock re-anchoring
/// problem (found via the FIX C invariant test, media plane step-2 fix
/// wave 1).
pub(crate) fn audio_discontinuity_threshold_90k(sample_rate: u32) -> i128 {
    let one_sample_period = (VIDEO_TIMESCALE as u128).div_ceil(sample_rate.max(1) as u128) as i128;
    let ms_bound = (VIDEO_TIMESCALE as i128 * AUDIO_REANCHOR_THRESHOLD_MS) / 1000;
    ms_bound.max(one_sample_period * 2)
}

/// A completed-but-not-yet-durationed sample, held until the *next* access
/// unit resolves its duration (video: DTS delta; data: PTS delta — mirrors
/// the old batch demuxer's "duration = delta to the next access unit, last
/// sample reuses the previous duration" rule).
struct PendingOneBehind {
    data: Vec<u8>,
    is_sync: bool,
    pts_uw: i128,
    dts_uw: i128,
}

/// Clamp an unwrapped 33-bit-derived `i128` timestamp into the `i64` range
/// [`Sample::dts`]/[`Sample::pts`] carry. `i128` is only used internally for
/// wrap arithmetic headroom; every real value here is a small non-negative
/// multiple of the 33-bit range and fits `i64` with room to spare for
/// centuries of continuous 90 kHz runtime, so this never actually clamps in
/// practice — it exists to make the conversion a checked one, not a silent
/// truncation.
fn to_ticks(uw: i128) -> i64 {
    uw.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// Debug-only [`Provenance`] for a 33-bit-wrapped TS/PES clock: the raw wire
/// value is exactly the absolute unwrapped value modulo the wrap (unwrap only
/// ever adds whole multiples of it), so it is recovered losslessly from the
/// already-unwrapped `dts`/`pts` with no extra state threaded through the
/// demux (issue #556 successor — media plane step 2c).
fn ts_provenance(dts: i64, pts: i64) -> Provenance {
    Provenance {
        wire_dts: Some((dts as u64) % TS_WRAP),
        wire_pts: Some((pts as u64) % TS_WRAP),
    }
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
    Audio {
        sample_rate: u32,
        kind: AudioKind,
        /// Frame-exact dts/pts accumulator (issue B5, media plane step-2 fix
        /// wave 1) — see [`AudioAnchor`].
        anchor: AudioAnchor,
    },
    /// Opaque PES data (#557): one `Sample` per access unit.
    Data {
        pending: Option<PendingOneBehind>,
        last_duration: u32,
    },
    /// MPEG-H 3D Audio (issue #579): one opaque `Sample` per MHAS access
    /// unit — no MHAS bitstream decode, so (like [`LiveKind::Data`]) there
    /// is no intrinsic per-sample duration to split on; duration is the
    /// one-behind PTS delta. `is_sync` is set from whether the access unit's
    /// MHAS packets contain a `PACTYP_MPEGH3DACFG` (a random-access point,
    /// ETSI TS 101 154 §6.8.4.1), not hardcoded `true`.
    MpegH {
        pending: Option<PendingOneBehind>,
        last_duration: u32,
    },
    /// Opaque section data (#576): each reassembled PSI/private section is
    /// emitted immediately as one `Sample` — sections carry no PTS/DTS, so
    /// there is no one-behind duration lookahead (every duration is `0`).
    Section,
}

struct LiveTrack {
    track_id: u32,
    kind: LiveKind,
    /// This track's current codec config, retained so a later PMT metadata
    /// change (issue #774) and a mid-stream config change (r04-W51) can
    /// rebuild a full [`TrackSpec`] for [`DemuxEvent::TrackUpdated`].
    config: CodecConfig,
    /// This track's media timescale, for the same [`DemuxEvent::TrackUpdated`]
    /// reconstruction.
    timescale: u32,
    /// The AVC/HEVC parameter sets this track has seen, tracked as each appears
    /// so a re-probe can tell a real change from a repeat and is not blind to a
    /// parameter set that arrived in an earlier access unit (r04-W51).
    parameter_sets: ParameterSets,
    /// The in-band codec-config header bytes this track's current config was
    /// built from, so a *changed* header can be told apart from a repeat
    /// (r04-W51).
    ///
    /// A broadcast stream changes its parameter sets mid-flight routinely —
    /// an SD↔HD ad break, a re-encode, a multiplex reconfiguration — and a
    /// mid-stream SPS/PPS (or AAC config) otherwise left the track labelled
    /// with the original `avcC`/`esds` for the rest of the stream, so the init
    /// segment described a stream that had stopped being sent. The parameters
    /// are compared byte-for-byte because encoders also repeat them
    /// unchanged, often on every keyframe; an identical repeat must not
    /// trigger a `TrackUpdated` (which tells a consumer to rebuild its init
    /// segment).
    config_header: Option<Vec<u8>>,
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
    /// [`MAX_PROBE_BACKLOG_BYTES`] overflowed while `Probing` or `Parked`
    /// (issue B8): permanently resolved without ever promoting to `Live` —
    /// every further access unit for this PID is silently discarded (no
    /// further growth). Matches [`StreamingTsDemux::finish`]'s own
    /// "never recoverable, skip" conclusion for a probe that never resolves,
    /// just reached early via the byte cap instead of end-of-input.
    Abandoned,
}

/// What [`WrapState::rebase_at_discontinuity`] concluded about a stamp that
/// arrived after a signalled time-base discontinuity (r04-W50 review).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscontinuityVerdict {
    /// This stamp reads as a forward step — the old time base's own cadence, or
    /// the new base already starting ahead — so the offset was left alone and
    /// the caller keeps waiting.
    Forward,
    /// This stamp moved backwards, so it is the new time base: the offset was
    /// lifted to make it continue from the last stamp emitted.
    Lifted,
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
    /// Ticks added to every unwrapped timestamp from the most recent
    /// signalled time-base discontinuity onwards (r04-W50).
    ///
    /// `discontinuity_indicator` (ISO/IEC 13818-1 §2.4.3.5) marks "a sample of
    /// a new system time clock": the new time base may start *below* the old
    /// one — the ordinary splice case — and the 33-bit unroll alone cannot see
    /// that (a backward step within half the range is read as a real backward
    /// jump, not a wrap). Every sample after it then carries a `dts` earlier
    /// than the sample before it, which violates the IR's "samples in decode
    /// order with a non-decreasing absolute dts" invariant and makes every
    /// downstream muxer emit negative deltas.
    ///
    /// The demux rebases instead of leaving it to consumers: the offset is the
    /// smallest non-negative constant that keeps the new time base continuing
    /// from the old one, so the decode timeline stays monotonic across the
    /// discontinuity while the *intervals* inside each time base are
    /// untouched.
    discontinuity_offset: i128,
}

impl WrapState {
    /// Feed the next access unit's raw 33-bit `(pts, dts)`, returning the
    /// unwrapped `(pts, dts)` with any accumulated discontinuity offset
    /// applied.
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
        // Remember the *un-offset* unwrapped values: the next access unit's
        // wrap arithmetic is a property of the wire clock, which the offset
        // does not move.
        self.prev_dts_raw = raw_dts;
        self.prev_dts_uw = dts_uw;
        self.prev_pts_raw = raw_pts;
        self.prev_pts_uw = pts_uw;
        (
            pts_uw.saturating_add(self.discontinuity_offset),
            dts_uw.saturating_add(self.discontinuity_offset),
        )
    }

    /// Fold in a signalled time-base discontinuity (r04-W50): decide whether
    /// this stamp is the first of the new time base, and if so lift the offset
    /// so the decode timeline continues rather than jumping backwards.
    ///
    /// Returns [`DiscontinuityVerdict::Lifted`] when the offset moved (this
    /// stamp really was the new base) and
    /// [`DiscontinuityVerdict::Forward`] when it reads as a forward step — which
    /// the *old* base also produces, so the caller keeps `pending_rebase` set
    /// and tests the next access unit the same way.
    ///
    /// The rules, each fixing a defect of the earlier attempt:
    ///
    /// * **Lift only, never subtract.** The offset moves *up* only when the
    ///   incoming stamp would land at or before the last one emitted. A forward
    ///   step is a continuation, not a shrinkable jump: the earlier code
    ///   computed `wanted - next` and added a *negative* shortfall, pulling a
    ///   legitimate forward splice back to a single frame period — while its
    ///   doc claimed the lift was "the smallest non-negative constant".
    /// * **Decide by a backward move, never by exact equality.** The earlier
    ///   attempt treated `shortfall == 0` as "still the old base", which cadence
    ///   jitter breaks: at 23.976 fps the frame period alternates 3754/3753
    ///   ticks and at 44.1 kHz 2351/2352, so the old base's last access unit
    ///   lands a tick early or late and the flag is consumed by the wrong unit.
    ///   A backward step is something the old base cannot produce, so that — not
    ///   an exact match — is the signal.
    /// * **Only the new base consumes the flag.** An old-base access unit
    ///   completing after the indicator reads as a forward step, so
    ///   `pending_rebase` stays set for the next access unit.
    fn rebase_at_discontinuity(&mut self, raw_dts: u64, step: i128) -> DiscontinuityVerdict {
        if !self.initialized {
            return DiscontinuityVerdict::Forward;
        }
        // Unrolled position of this stamp *before* the current offset is
        // applied, and the value the caller last received (which already
        // includes any earlier lift).
        let next_uw = if self.dts_seen_real {
            unwrap_ts(self.prev_dts_uw, self.prev_dts_raw, raw_dts)
        } else {
            raw_dts as i128
        };
        let last_emitted = self.prev_dts_uw.saturating_add(self.discontinuity_offset);
        let next_emitted = next_uw.saturating_add(self.discontinuity_offset);
        // `step` bounds the jump the *old* base could still produce: a decoder
        // polled with a nominal frame period may hand out one access unit of
        // slack either way. A move further back than that is the new base.
        let slack = step.max(1);
        if next_emitted + slack > last_emitted {
            // Forward, or level within one nominal period: the old base's own
            // cadence. Never a lift, and never a reduction.
            return DiscontinuityVerdict::Forward;
        }
        // Lift so the new base's first stamp lands *one nominal period past*
        // the last one emitted, not merely level with it: a stamp equal to its
        // predecessor gives the IR a zero step, and every writer in this crate
        // rejects a zero-duration sample. `step` is the track's own last
        // measured frame period, so a steady stream keeps its cadence across
        // the seam.
        let lift = last_emitted
            .saturating_sub(next_emitted)
            .saturating_add(slack);
        self.discontinuity_offset = self.discontinuity_offset.saturating_add(lift);
        self.prev_dts_raw = raw_dts;
        self.prev_dts_uw = next_uw;
        self.prev_pts_raw = raw_dts;
        self.prev_pts_uw = next_uw;
        self.dts_seen_real = true;
        self.pts_seen_real = true;
        DiscontinuityVerdict::Lifted
    }
}

/// A PID's reassembly engine: PES access units, or PSI/private sections
/// (issue #576) — chosen once at PID discovery from [`data_carriage`] (a
/// decoded [`Codec`] or a PES-carried [`Codec::Data`] always gets
/// [`Carrier::Pes`]).
enum Carrier {
    Pes(PesAssembler),
    Section(SectionReassembler),
}

/// The reassembly engine a newly-discovered `codec` should use.
fn initial_carrier(codec: Codec) -> Carrier {
    match codec {
        Codec::Data(stream_type) if data_carriage(stream_type) == DataCarriage::Sections => {
            Carrier::Section(SectionReassembler::default())
        }
        _ => Carrier::Pes(PesAssembler::new()),
    }
}

/// Per-PID (elementary stream) engine state.
struct StreamState {
    codec: Codec,
    descriptors: Vec<u8>,
    carrier: Carrier,
    /// Bytes accumulated in `carrier`'s `Carrier::Pes` assembler since the
    /// last `payload_unit_start` — enforces [`MAX_PES_BUFFER_BYTES`]. Always
    /// `0` and unused for `Carrier::Section` streams.
    pes_bytes: usize,
    /// Previous access unit's resolved `(pts, dts)` — the base a PES carrying
    /// none of its own is interpolated *forward* from (see
    /// [`StreamState::frame_period`]).
    fallback: (u64, u64),
    /// Estimated per-access-unit frame period (90 kHz ticks), measured from
    /// the last two *stamped* PES access units on this PID.
    ///
    /// A PES packet may legally omit both PTS and DTS (ISO/IEC 13818-1
    /// §2.4.3.7: `PTS_DTS_flags == '00'`), and the 2.7.4 interval constraint
    /// only requires them periodically. The demux used to reuse the previous
    /// access unit's stamps verbatim for such a packet, so consecutive video
    /// access units received *identical* `dts` — the one-behind duration rule
    /// then gave the earlier one `duration = 0` and the next stamped access
    /// unit absorbed the whole gap, producing zero-duration samples plus one
    /// long one (r04-W48). The T-STD instead derives them: "Decoding times
    /// tdn(j + 1), tdn(j + 2),... of access units without encoded DTS or PTS
    /// fields which directly follow access unit j may be derived from
    /// information in the elementary stream" (§2.4.2.6). The closest
    /// container-level estimate of that information is the observed spacing
    /// between stamped access units, which is what this holds.
    ///
    /// `None` until two stamped access units have been seen — a lone
    /// unstamped packet has nothing to interpolate from and keeps the old
    /// verbatim-fallback behaviour rather than inventing a period.
    frame_period: Option<u64>,
    /// The DTS of the most recent *stamped* access unit, which is what a new
    /// stamped access unit measures its period against. Kept separate from
    /// [`StreamState::fallback`] because an unstamped access unit advances
    /// `fallback` (that is the interpolation) without being a clock
    /// observation to measure against.
    last_stamped_dts: Option<u64>,
    /// Access units seen since that stamped one, so the span it covers is
    /// divided by the right count when the next stamp arrives.
    units_since_stamped: u64,
    has_any: bool,
    wrap: WrapState,
    /// Always `Some` except transiently inside [`advance_track`].
    track: Option<TrackState>,
    /// This PID's most recent observed inter-access-unit spacing (90 kHz
    /// ticks), used as the step when a signalled discontinuity rebases the
    /// timeline (r04-W50). `0` until a plausible step has been seen, in which
    /// case the rebase falls back to a single tick — monotonic, which is the
    /// invariant that matters.
    last_frame_period: i128,
    /// The last [`FRAME_PERIOD_WINDOW`] plausible inter-access-unit steps, from
    /// which the median — and so both the interpolation period and the rebase
    /// step — is taken (r04-W48/W50 review).
    recent_steps: Vec<u64>,
    /// A signalled time-base discontinuity was observed on this program and
    /// this PID has not yet seen the first access unit of the new base: the
    /// next `(pts, dts)` it resolves is fed to
    /// [`WrapState::rebase_at_discontinuity`] so its decode timeline continues
    /// from where the old base left off instead of jumping backwards
    /// (r04-W50).
    pending_rebase: bool,
    /// Whether the PES the assembler is currently building is whole: `false`
    /// once a continuity-counter gap — or a signalled discontinuity — lost one
    /// of its 184-byte payloads. It is dropped on completion rather than
    /// delivered truncated (r04-W49). Set once and never cleared by a later
    /// packet in the same unit; only the next `payload_unit_start` starts a
    /// fresh, whole unit. Unused for a non-PES carrier.
    current_pes_intact: bool,
    /// The `PES_packet_length` this PID's in-progress PES declared, once its
    /// header bytes have been received; `0` while unknown or for an unbounded
    /// PES (ISO/IEC 13818-1 §2.4.3.7 — `0` means unbounded, the ordinary
    /// video case). Compared against [`StreamState::pes_received`] at
    /// completion to decide whether a gap on the *next* packet actually lost
    /// anything (r04-W49).
    pes_declared_len: u16,
    /// Bytes of the in-progress PES received so far (header + payload), for
    /// the [`StreamState::pes_declared_len`] comparison. Reset on every
    /// `payload_unit_start`.
    pes_received: usize,
    /// Running total of bytes held in `track`'s `Probing`/`Parked` backlog —
    /// enforces [`MAX_PROBE_BACKLOG_BYTES`] (issue B8). Kept in sync on every
    /// [`advance_track`] push (never re-walked from the `Vec`), and reset to
    /// `0` when the backlog is abandoned (see [`abandon_backlog`]); `0` and
    /// unused once `track` is `Live` or `Abandoned`.
    backlog_bytes: usize,
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
        let dts = to_ticks(prev.dts_uw);
        let pts = to_ticks(prev.pts_uw);
        events.push_back(DemuxEvent::Sample {
            track_id,
            sample: Sample {
                data: prev.data.into(),
                dts: Some(dts),
                pts: Some(pts),
                duration: Some(duration),
                flags: SampleFlags::new(prev.is_sync),
                provenance: Some(ts_provenance(dts, pts)),
            },
        });
    }
    *pending = Some(PendingOneBehind {
        data,
        is_sync,
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
        let dts = to_ticks(p.dts_uw);
        let pts = to_ticks(p.pts_uw);
        events.push_back(DemuxEvent::Sample {
            track_id,
            sample: Sample {
                data: p.data.into(),
                dts: Some(dts),
                pts: Some(pts),
                duration: Some(last_duration),
                flags: SampleFlags::new(p.is_sync),
                provenance: Some(ts_provenance(dts, pts)),
            },
        });
    }
}

/// Build a video sample's coded bytes + sync flag from one Annex B (or raw
/// MPEG-2) access unit.
fn video_sample_bytes(codec: VideoCodec, au_data: &[u8]) -> (Vec<u8>, bool) {
    match codec {
        VideoCodec::H264 => {
            // Random-access anchor: IDR OR an open-GOP RAP signal (a
            // recovery-point SEI, or pragmatically an SPS in the AU) —
            // issue #595. Broadcast H.264 is frequently open-GOP and never
            // codes an IDR at all, so IDR-only detection would never anchor
            // a segment.
            let is_rap = access_unit_is_rap(NalCodec::Avc, au_data, false);
            (annexb_to_length_prefixed(au_data), is_rap)
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
///
/// `anchor` carries the frame-exact running dts/pts cursor across access
/// units (issue B5, media plane step-2 fix wave 1): this access unit's base
/// track-tick position (`dts0`/`pts0`) is either that running cursor (the
/// steady state — no dependency on the lossy 90 kHz wire stamp at all) or a
/// fresh rescale of `dts_uw`/`pts_uw` on the very first access unit or a
/// genuine discontinuity (see [`AudioAnchor`]); every frame split out of
/// this AU then advances from that base by its own `elapsed` intrinsic
/// samples, exactly as before.
#[allow(clippy::too_many_arguments)]
fn emit_audio_au(
    kind: &AudioKind,
    sample_rate: u32,
    anchor: &mut AudioAnchor,
    au_data: &[u8],
    pts_uw: i128,
    dts_uw: i128,
    track_id: u32,
    events: &mut VecDeque<DemuxEvent>,
) {
    // Resolve this AU's track-tick base: reuse the running frame-exact
    // cursor in the steady state, or (re-)anchor from the wire clock when
    // there is no cursor yet or it has drifted beyond the discontinuity
    // threshold — a genuine gap, not the ±1-tick rounding noise the old
    // per-AU rescale mistook for one.
    // Snapshot the (all-`Copy`) seed up front, so every later "is there a
    // cursor?" decision reads the same value without needing a second
    // borrow-and-`expect` of `anchor.seed` to restate an invariant the
    // compiler cannot see.
    let seed = anchor.seed;
    let fresh_anchor = match &seed {
        None => true,
        Some(seed) => {
            let expected_dts_uw = seed.anchor_dts_uw
                + (seed.ticks_since_anchor as i128 * VIDEO_TIMESCALE as i128)
                    / sample_rate.max(1) as i128;
            (dts_uw - expected_dts_uw).abs() > audio_discontinuity_threshold_90k(sample_rate)
        }
    };
    // Only signal a discontinuity when re-anchoring an ALREADY-seeded track
    // (the very first access unit establishes the anchor, it doesn't
    // "discontinue" from anything).
    if fresh_anchor && seed.is_some() {
        events.push_back(DemuxEvent::Discontinuity {
            track: Some(track_id),
            kind: DiscontinuityKind::TimelineReanchored,
            provenance: EventProvenance::default(),
        });
    }
    let (dts0, pts0) = match (fresh_anchor, &seed) {
        (false, Some(seed)) => (seed.next_dts, seed.next_pts),
        _ => (
            rescale_to_track(dts_uw, sample_rate),
            rescale_to_track(pts_uw, sample_rate),
        ),
    };

    let mut elapsed = 0u64;
    // Every frame split out of this access unit came from the same PES packet,
    // so they share that PES header's raw 90 kHz wire stamps — that, not the
    // rescaled per-frame value, is what `Provenance` means (media plane step
    // 2c: the source container's original stamps, pre-unwrap).
    let au_provenance = ts_provenance(to_ticks(dts_uw), to_ticks(pts_uw));
    // Build one audio Sample at `elapsed` samples into this access unit:
    // `dts0`/`pts0` (the AU's resolved track-tick base) plus the per-frame
    // `elapsed` intrinsic samples (issue #556 semantics preserved exactly —
    // media plane step 2c stores them directly instead of discarding them
    // into a write-only `SourceTiming`; issue B5: `dts0`/`pts0` are now
    // frame-exact rather than re-derived from the lossy wire clock per AU).
    let audio_sample = |data: Vec<u8>, duration: u32, elapsed: u64| -> Sample {
        let dts = dts0 + elapsed as i64;
        let pts = pts0 + elapsed as i64;
        Sample::from_raw(data, Some(dts), Some(pts), Some(duration)).with_provenance(au_provenance)
    };
    match kind {
        AudioKind::Aac => {
            for (frame, hdr) in split_adts_frames(au_data) {
                // C11 (#1012): strip the CRC (if present) along with the
                // header, and scale duration by the number of raw data
                // blocks this one ADTS frame actually carries — a frame
                // with `number_of_raw_data_blocks_in_frame == n` codes
                // `n + 1` blocks of `AAC_SAMPLES_PER_FRAME` samples each,
                // not one.
                let header_len = adts_header_len(&hdr);
                let duration = AAC_SAMPLES_PER_FRAME * (hdr.num_raw_data_blocks as u32 + 1);
                if frame.len() > header_len {
                    events.push_back(DemuxEvent::Sample {
                        track_id,
                        sample: audio_sample(frame[header_len..].to_vec(), duration, elapsed),
                    });
                }
                elapsed += duration as u64;
            }
        }
        AudioKind::Ac3 => {
            for frame in split_ac3_syncframes(au_data) {
                events.push_back(DemuxEvent::Sample {
                    track_id,
                    sample: audio_sample(frame.to_vec(), AC3_SAMPLES_PER_SYNCFRAME, elapsed),
                });
                elapsed += AC3_SAMPLES_PER_SYNCFRAME as u64;
            }
        }
        AudioKind::Eac3 => {
            for split in split_eac3_syncframes(au_data) {
                let duration = split.info.samples_per_frame();
                events.push_back(DemuxEvent::Sample {
                    track_id,
                    sample: audio_sample(split.data, duration, elapsed),
                });
                elapsed += duration as u64;
            }
        }
        AudioKind::Dts => {
            for frame in split_dts_core_frames(au_data) {
                events.push_back(DemuxEvent::Sample {
                    track_id,
                    sample: audio_sample(frame.data.to_vec(), frame.samples, elapsed),
                });
                elapsed += frame.samples as u64;
            }
        }
        AudioKind::MpegAudio { samples_per_frame } => {
            for frame in split_mpeg_audio_frames(au_data) {
                events.push_back(DemuxEvent::Sample {
                    track_id,
                    sample: audio_sample(frame.to_vec(), *samples_per_frame, elapsed),
                });
                elapsed += *samples_per_frame as u64;
            }
        }
    }

    // Advance the persistent anchor by this AU's total intrinsic duration so
    // the *next* AU continues the frame-exact cursor instead of re-deriving
    // it from the wire clock (issue B5). `anchor_dts_uw`/`anchor_pts_uw`/
    // `ticks_since_anchor` stay fixed at the point they were last
    // (re-)established (this AU's own values, on a fresh anchor; carried
    // forward otherwise) — they exist purely to predict the *next* AU's
    // expected wire position for drift detection, never to derive a dts/pts.
    let (anchor_dts_uw, anchor_pts_uw, ticks_since_anchor) = match (fresh_anchor, &seed) {
        (false, Some(seed)) => (
            seed.anchor_dts_uw,
            seed.anchor_pts_uw,
            seed.ticks_since_anchor,
        ),
        _ => (dts_uw, pts_uw, 0i64),
    };
    anchor.seed = Some(AudioAnchorSeed {
        next_dts: dts0 + elapsed as i64,
        next_pts: pts0 + elapsed as i64,
        anchor_dts_uw,
        anchor_pts_uw,
        ticks_since_anchor: ticks_since_anchor + elapsed as i64,
    });
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
            advance_one_behind(
                pending,
                last_duration,
                bytes,
                is_sync,
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
                pts_uw,
                dts_uw,
                true,
                track_id,
                events,
            );
        }
        LiveKind::Audio {
            sample_rate,
            kind,
            anchor,
        } => {
            emit_audio_au(
                kind,
                *sample_rate,
                anchor,
                data,
                pts_uw,
                dts_uw,
                track_id,
                events,
            );
        }
        LiveKind::MpegH {
            pending,
            last_duration,
        } => {
            let is_sync = find_mpegh3da_config(data).is_some();
            advance_one_behind(
                pending,
                last_duration,
                data.to_vec(),
                is_sync,
                pts_uw,
                dts_uw,
                true,
                track_id,
                events,
            );
        }
        LiveKind::Section => {
            // Sections carry no timestamp at all (`pts_uw`/`dts_uw` are dummy
            // zeros from `on_completed_section`, never read here) — emit
            // immediately, no lookahead, and never fabricate a dts/pts/duration.
            events.push_back(DemuxEvent::Sample {
                track_id,
                sample: Sample::from_raw(data.to_vec(), None, None, None),
            });
        }
    }
}

/// Track a live track's in-band codec-configuration **content**, access unit by
/// access unit, returning the new config when that content actually changes
/// (r04-W51).
///
/// "The content a decoder configuration would contain" — never the framing, and
/// never a single access unit's view of it:
///
/// * **AVC/HEVC**: the parameter sets themselves. A probe over *one* access
///   unit cannot see an SPS that arrived in an earlier one, and encoders
///   routinely split SPS and PPS across access units (and put both in the
///   first). Each set is therefore remembered as it appears — VPS, SPS and PPS
///   tracked separately — and the event fires only when one of them actually
///   changes value, or one goes from absent to present. That also makes an
///   identical repeat (the common case: the same SPS+PPS in every keyframe)
///   emit nothing.
/// * **AAC**: the `AudioSpecificConfig` the ADTS header implies — its
///   `audioObjectType`, `samplingFrequencyIndex` and `channelConfiguration`
///   only (ISO/IEC 14496-3 §1.6.2.1). The *raw* ADTS header must never be
///   compared: it also carries the 13-bit `aac_frame_length`, which differs on
///   every single frame, so comparing it reported a config change for
///   practically every AAC access unit.
/// * **Anything else** (no in-band config): `None` — MPEG-2 video's config is
///   only geometry and opaque data's comes from the PMT, so a change cannot be
///   seen here. Those keep their original config, exactly as documented.
fn reprobe_if_config_changed(
    stream: &mut StreamState,
    live: &mut LiveTrack,
    data: &[u8],
) -> Option<(CodecConfig, u32)> {
    let changed = match stream.codec {
        Codec::H264 | Codec::Hevc => {
            observe_parameter_sets(stream.codec, data, &mut live.parameter_sets)
        }
        Codec::Aac => {
            let (_, hdr) = find_adts_sync(data)?;
            let asc = AudioSpecificConfig::from_adts_header(&hdr).to_bytes();
            if live.config_header.as_deref() == Some(asc.as_slice()) {
                false
            } else {
                live.config_header = Some(asc);
                true
            }
        }
        _ => return None,
    };
    if !changed {
        return None;
    }
    // A fresh probe recovers the new config with the same code path the first
    // one used. For AVC/HEVC the probe needs the parameter sets, which may be
    // spread over several access units, so it is fed the accumulated set: the
    // probe's own accumulation is seeded from the track's, and this access
    // unit's copies are appended.
    let descriptors = stream.descriptors.clone();
    let mut probe = initial_probe(stream.codec);
    seed_probe_parameter_sets(&mut probe, &live.parameter_sets);
    let recovered = finalize_probe(stream.codec, &descriptors, &mut probe, data);
    let (config, timescale, _kind) = recovered?;
    Some((config, timescale))
}

/// The parameter sets a track has seen, each remembered separately so an
/// SPS/PPS split across access units still compares as a whole (r04-W51).
#[derive(Default, Clone)]
struct ParameterSets {
    vps: Option<Vec<u8>>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
}

impl ParameterSets {
    /// Fold in `data`'s parameter sets, reporting whether any of them
    /// **changed**: a set that was absent and is now present, or present with
    /// different bytes. A set that was already present identically — which is
    /// what an encoder repeating its headers on every keyframe produces — is not
    /// a change.
    fn observe(&mut self, codec: Codec, data: &[u8]) -> bool {
        let mut changed = false;
        let mut record = |slot: &mut Option<Vec<u8>>, bytes: &[u8]| {
            if slot.as_deref() != Some(bytes) {
                *slot = Some(bytes.to_vec());
                changed = true;
            }
        };
        match codec {
            Codec::H264 => {
                for nal in iter_annexb_nals(data) {
                    match nal[0] & H264_NAL_TYPE_MASK {
                        H264_NAL_SPS => record(&mut self.sps, nal),
                        H264_NAL_PPS => record(&mut self.pps, nal),
                        _ => {}
                    }
                }
            }
            Codec::Hevc => {
                for nal in iter_annexb_nals(data) {
                    match nal_unit_type(NalCodec::Hevc, nal) {
                        Some(H265_NAL_VPS) => record(&mut self.vps, nal),
                        Some(H265_NAL_SPS) => record(&mut self.sps, nal),
                        Some(H265_NAL_PPS) => record(&mut self.pps, nal),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        changed
    }
}

/// Record `data`'s parameter sets on `sets`, returning whether any changed.
fn observe_parameter_sets(codec: Codec, data: &[u8], sets: &mut ParameterSets) -> bool {
    sets.observe(codec, data)
}

/// Seed a fresh [`ConfigProbe`] with parameter sets already known for the
/// track, so a re-probe of a single access unit is not blind to an SPS that
/// arrived in an earlier one (r04-W51).
fn seed_probe_parameter_sets(probe: &mut ConfigProbe, sets: &ParameterSets) {
    match probe {
        ConfigProbe::H264 { sps, pps } => {
            sps.clone_from(&sets.sps);
            pps.clone_from(&sets.pps);
        }
        ConfigProbe::Hevc { vps, sps, pps } => {
            vps.clone_from(&sets.vps);
            sps.clone_from(&sets.sps);
            pps.clone_from(&sets.pps);
        }
        _ => {}
    }
}

/// Feed the newest access unit into a probing [`ConfigProbe`], returning the
/// finished config the moment it becomes recoverable. The caller owns
/// transferring the PID's backlog into [`TrackState::Parked`].
///
/// **Only `latest` is ever scanned.** Every probe below used to walk the whole
/// backlog from the start (`backlog.iter().find_map(..)`) on *every* incoming
/// access unit, with byte-by-byte sync scans inside (r04-W47): a PID whose
/// config never resolves — garbage payload, or an ADTS `sampling_frequency_
/// index` of 13/14 that [`sfi_to_hz`] rejects — grew toward
/// [`MAX_PROBE_BACKLOG_BYTES`] while re-scanning it each access unit, costing
/// O(n²) byte probes per PID (≈4 × 10⁹ on a 2 000-PES PID). Scanning only the
/// newest access unit is both what the H.264/HEVC arms already did and
/// sufficient: a codec's config header is repeated in-band often enough that
/// one access unit is representative (a parameter set / frame header / MHAS
/// config arrives within the first access unit or two in any real stream).
fn finalize_probe(
    codec: Codec,
    descriptors: &[u8],
    probe: &mut ConfigProbe,
    latest: &[u8],
) -> Option<(CodecConfig, u32, LiveKind)> {
    // One pass over the newest access unit — the thing the r04-W47 bound is
    // about. Counted here so the H.264/HEVC arms, whose NAL walk has no
    // byte-by-byte sync scanner to count, are covered too.
    record_probe_pass();
    match probe {
        ConfigProbe::Data => {
            let Codec::Data(stream_type) = codec else {
                // Probe/codec mismatch. Unreachable by construction today: a
                // PMT version change that reclassifies a PID's `stream_type`
                // tears the PID down and re-registers it
                // (`StreamingTsDemux::apply_pmt_diff`), which rebuilds the
                // `ConfigProbe` — and the `Carrier` — for the *new* codec,
                // rather than writing `stream.codec` in place under a probe
                // built for the old one.
                //
                // It is not asserted, though. This is a
                // `#![forbid(unsafe_code)]` library parsing untrusted remote
                // broadcast input, so a broken invariant must degrade, never
                // abort the host process: returning `None` simply leaves the
                // PID unresolved, and the existing abandonment paths conclude
                // it — `MAX_PROBE_BACKLOG_BYTES` while running, or `finish()`'s
                // `TrackAbandoned { reason: AbandonReason::ConfigUnrecoverable }`
                // at end of input.
                return None;
            };
            let carriage = data_carriage(stream_type);
            let kind = match carriage {
                DataCarriage::Pes => LiveKind::Data {
                    pending: None,
                    last_duration: 0,
                },
                DataCarriage::Sections => LiveKind::Section,
            };
            Some((
                CodecConfig::Data {
                    stream_type,
                    descriptors: descriptors.to_vec(),
                    carriage,
                },
                VIDEO_TIMESCALE,
                kind,
            ))
        }
        ConfigProbe::H264 { sps, pps } => {
            // Accumulate across access units: a parameter set is often split
            // over one access unit per NAL (an encoder emits SPS and PPS in
            // separate PES packets), and since r04-W47 only the newest access
            // unit is scanned — so the field that held the SPS two access
            // units ago is what carries it forward.
            for nal in iter_annexb_nals(latest) {
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
            // Coded dimensions + high-profile chroma/bit-depth from the SPS
            // (ISO/IEC 14496-10 §7.3.2.1.1) — the TS in-band parameter set
            // carries them (0/None if undecodable).
            let info = crate::sps::decode_avc_sps(sps_bytes).ok();
            let (width, height) = info
                .as_ref()
                .map(|i| {
                    (
                        i.width.min(u16::MAX as u32) as u16,
                        i.height.min(u16::MAX as u32) as u16,
                    )
                })
                .unwrap_or((0, 0));
            // The avcC high-profile extension (chroma_format_idc + bit depths)
            // exists only for the High-family profiles that carry it
            // (ISO/IEC 14496-15 §5.3.3.1). Populate it from the SPS for those —
            // previously hardcoded None, so a High 10/4:2:2/4:4:4 TS lost its
            // chroma/bit-depth in the recovered avcC (#563 flagged; #582 owns
            // this file). Gate matches the serializer's emission set via the
            // shared `sps::is_high_profile` source of truth.
            let ext = info
                .as_ref()
                .filter(|i| crate::sps::is_high_profile(i.profile_idc));
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
                chroma_format: ext.map(|i| i.chroma_format_idc),
                bit_depth_luma_minus8: ext.map(|i| i.bit_depth_luma.saturating_sub(8)),
                bit_depth_chroma_minus8: ext.map(|i| i.bit_depth_chroma.saturating_sub(8)),
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
            // Accumulated across access units, exactly like the H.264 arm
            // above (r04-W47: only the newest access unit is scanned).
            for nal in iter_annexb_nals(latest) {
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
            let seq = Mpeg2SeqHeader::find(latest).ok()?;
            let esds = EsdsBox::new(ESDescriptor::new(
                ESDS_VIDEO_ES_ID,
                0,
                Some(DecoderConfigDescriptor::new(
                    OTI_MPEG2_VIDEO_MAIN,
                    STREAM_TYPE_VISUAL,
                    false,
                    0,
                    0,
                    0,
                    None,
                )),
                Some(SLConfigDescriptor::predefined_two()),
            ));
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
            // Resync within the newest buffered PES payload (issue #638) -- a
            // real broadcast payload is not guaranteed to start on a frame
            // sync.
            let (_, first) = find_mpeg_audio_sync(latest)?;
            let sample_rate = first.sample_rate;
            let channel_count = first.channels;
            let samples_per_frame = first.samples_per_frame;
            let oti = if *is_mpeg2 {
                OTI_MPEG2_AUDIO
            } else {
                OTI_MPEG1_AUDIO
            };
            let esds = EsdsBox::new(ESDescriptor::new(
                ESDS_ES_ID,
                0,
                Some(DecoderConfigDescriptor::new(
                    oti,
                    STREAM_TYPE_AUDIO,
                    false,
                    0,
                    0,
                    0,
                    None,
                )),
                Some(SLConfigDescriptor::predefined_two()),
            ));
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
                    anchor: AudioAnchor::default(),
                },
            ))
        }
        ConfigProbe::Aac => {
            // Resync within the newest buffered PES payload (issue #638) -- a
            // real broadcast payload is not guaranteed to start on a frame
            // sync.
            let (_, first_hdr) = find_adts_sync(latest)?;
            let asc = AudioSpecificConfig::from_adts_header(&first_hdr);
            let sample_rate = sfi_to_hz(first_hdr.sampling_frequency_index)?;
            // ISO/IEC 14496-3 Table 1.19: `channel_configuration` is a
            // configuration *index*, not a count — 7 means eight channels
            // (7.1), and 0 means the mapping is carried in-band by a
            // `program_config_element` in the raw data stream, so no count is
            // derivable from the header at all. Using the raw field as the
            // count reported 7.1 as 7 channels and a PCE-signalled stream as
            // zero (r04-W51, extending W16).
            let channel_count = crate::flv::aac_channel_count(&asc);
            let esds = EsdsBox::new(ESDescriptor::new(
                ESDS_ES_ID,
                0,
                Some(DecoderConfigDescriptor::new(
                    OTI_MPEG4_AUDIO,
                    STREAM_TYPE_AUDIO,
                    false,
                    0,
                    0,
                    0,
                    Some(DecoderSpecificInfo::new(asc.to_bytes())),
                )),
                Some(SLConfigDescriptor::predefined_two()),
            ));
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
                    anchor: AudioAnchor::default(),
                },
            ))
        }
        ConfigProbe::Ac3 => {
            let info = Ac3SyncframeInfo::from_es(latest).ok()?;
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
                    anchor: AudioAnchor::default(),
                },
            ))
        }
        ConfigProbe::Eac3 => {
            // The `dec3` describes the bitstream's substream layout and is
            // built from the newest access unit's *first* AU: the dependent
            // substreams that make a programme 7.1 follow that AU's
            // independent frame. Scanning the whole backlog would emit one
            // substream per repeated access unit (and re-scan it on every
            // access unit — see the O(n²) note above `finalize_probe`).
            let frames: alloc::vec::Vec<Ec3SyncframeInfo> =
                Ec3SyncframeInfo::from_es_first_au(latest);
            let info = *frames.first()?;
            let sample_rate = info.sample_rate;
            let channel_count = info.channel_count() as u16;
            let config = Ec3SpecificBox::from_syncframes(&frames).ok()?;
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
                    anchor: AudioAnchor::default(),
                },
            ))
        }
        ConfigProbe::Dts => {
            let info = DtsCoreFrameInfo::from_es(latest).ok()?;
            let sample_rate = info.sample_rate;
            let channel_count = info.channels as u16;
            let config = info.into_ddts();
            Some((
                CodecConfig::Dts {
                    config,
                    codec_fourcc: crate::dts::DTSC_FOURCC,
                    channel_count,
                    sample_rate,
                    sample_size: AUDIO_SAMPLE_SIZE_BITS,
                },
                sample_rate,
                LiveKind::Audio {
                    sample_rate,
                    kind: AudioKind::Dts,
                    anchor: AudioAnchor::default(),
                },
            ))
        }
        ConfigProbe::MpegH => {
            // Scan the newest access unit's MHAS packets for a
            // PACTYP_MPEGH3DACFG (issue #579) — mirrors the Ac3/Eac3/Dts
            // header scans above, just over MHAS packets instead of a
            // sync-frame header.
            let config_bytes = find_mpegh3da_config(latest)?;
            // ATSC A/342-3 §5.2.2.1 / ISO/IEC 23008-3 §5.3.2: the
            // `mpegh3daConfig()` bitstream's leading byte *is*
            // `mpegh3daProfileLevelIndication` — the same value the
            // `MHADecoderConfigurationRecord` duplicates as its own field.
            let profile_level_indication = *config_bytes.first()?;
            let config = MHADecoderConfigurationRecord::new(
                profile_level_indication,
                MPEGH_REFERENCE_CHANNEL_LAYOUT_UNSPECIFIED,
                config_bytes.to_vec(),
            );
            Some((
                CodecConfig::MpegH {
                    config,
                    channel_count: MPEGH_CHANNEL_COUNT_UNSPECIFIED,
                    sample_rate: MPEGH_SAMPLE_RATE_UNSPECIFIED,
                    sample_size: AUDIO_SAMPLE_SIZE_BITS,
                },
                VIDEO_TIMESCALE,
                LiveKind::MpegH {
                    pending: None,
                    last_duration: 0,
                },
            ))
        }
    }
}

/// [`MAX_PROBE_BACKLOG_BYTES`] tripped for `pid` (issue B8): free the
/// backlog, signal the loss, and permanently abandon this PID's probe —
/// [`StreamingTsDemux::try_promote_ready`] treats [`TrackState::Abandoned`]
/// as resolved-without-promotion on its next pass, exactly like
/// [`StreamingTsDemux::finish`]'s own end-of-input conclusion. Emits
/// [`DemuxEvent::TrackAbandoned`] with [`AbandonReason::BudgetExceeded`]
/// (issue #774) — this PID never reached `Live`, so no `track_id` exists to
/// report; this replaces the mis-typed [`DemuxEvent::Discontinuity`] this
/// path used to emit (a budget overflow is an abandonment, not a
/// discontinuity — no track survives it to "continue" from).
fn abandon_backlog(
    stream: &mut StreamState,
    pid: u16,
    events: &mut VecDeque<DemuxEvent>,
) -> TrackState {
    stream.backlog_bytes = 0;
    events.push_back(DemuxEvent::TrackAbandoned {
        track_id: None,
        reason: AbandonReason::BudgetExceeded,
        provenance: EventProvenance {
            pid: Some(pid),
            packet_index: None,
        },
    });
    TrackState::Abandoned
}

/// Advance a [`StreamState`]'s track lifecycle by one access unit: apply it
/// directly if already live, append it to the backlog if parked, or feed the
/// probe (transitioning `Probing` → `Parked` the moment config becomes
/// recoverable) otherwise. Never assigns a track ID or emits
/// [`DemuxEvent::TrackAdded`] itself — that is
/// [`StreamingTsDemux::try_promote_ready`]'s job, since a `Parked` track must
/// still wait for its PMT-declaration-order turn.
///
/// Every push to a `Probing`/`Parked` backlog counts against
/// [`MAX_PROBE_BACKLOG_BYTES`] (issue B8); an access unit for an already
/// [`TrackState::Abandoned`] PID is silently discarded (no further growth,
/// no re-abandonment).
fn advance_track(
    stream: &mut StreamState,
    pid: u16,
    data: Vec<u8>,
    pts_uw: i128,
    dts_uw: i128,
    events: &mut VecDeque<DemuxEvent>,
) {
    // `StreamState.track` is `None` only transiently, inside this function and
    // `try_promote_ready` — never on entry. Degrade (drop this access unit)
    // instead of panicking if that ever stops holding: this crate is
    // `#![forbid(unsafe_code)]` and must not abort on remote input.
    let Some(track) = stream.track.take() else {
        return;
    };
    let new_track = match track {
        TrackState::Live(mut live) => {
            // A changed in-band codec-config header re-probes the track and
            // reports the new config (r04-W51). The config used to be
            // single-shot for the life of the stream, so a mid-stream SPS/PPS
            // or AAC-config change (SD↔HD ad break, re-encode, multiplex
            // reconfiguration) left the track labelled with the original
            // `avcC`/`hvcC`/`esds` and the init segment describing a stream
            // that was no longer being sent. An *unchanged* repeat emits
            // nothing — encoders re-send their headers routinely.
            if let Some((config, timescale)) = reprobe_if_config_changed(stream, &mut live, &data) {
                live.config = config;
                live.timescale = timescale;
                events.push_back(DemuxEvent::TrackUpdated(
                    TrackSpec::new(live.track_id, live.timescale, live.config.clone())
                        .with_source(pid, stream.descriptors.clone()),
                ));
            }
            push_live_au(&mut live, &data, pts_uw, dts_uw, events);
            TrackState::Live(live)
        }
        TrackState::Abandoned => TrackState::Abandoned,
        TrackState::Parked {
            config,
            timescale,
            kind,
            mut backlog,
        } => {
            stream.backlog_bytes = stream.backlog_bytes.saturating_add(data.len());
            backlog.push(BufferedAu {
                data,
                pts_uw,
                dts_uw,
            });
            if stream.backlog_bytes > MAX_PROBE_BACKLOG_BYTES {
                abandon_backlog(stream, pid, events)
            } else {
                TrackState::Parked {
                    config,
                    timescale,
                    kind,
                    backlog,
                }
            }
        }
        TrackState::Probing {
            mut probe,
            mut backlog,
        } => {
            stream.backlog_bytes = stream.backlog_bytes.saturating_add(data.len());
            backlog.push(BufferedAu {
                data,
                pts_uw,
                dts_uw,
            });
            // `backlog` is never empty here: the newest access unit was just
            // pushed above. Degrade to "not resolvable yet" rather than
            // panicking if that ever stops holding.
            let Some(latest) = backlog.last() else {
                return;
            };
            match finalize_probe(stream.codec, &stream.descriptors, &mut probe, &latest.data) {
                Some((config, timescale, kind)) => TrackState::Parked {
                    config,
                    timescale,
                    kind,
                    backlog,
                },
                None if stream.backlog_bytes > MAX_PROBE_BACKLOG_BYTES => {
                    abandon_backlog(stream, pid, events)
                }
                None => TrackState::Probing { probe, backlog },
            }
        }
    };
    stream.track = Some(new_track);
}

/// Feeds one TS payload to a PID's `Carrier::Pes` assembler with
/// [`MAX_PES_BUFFER_BYTES`] enforced (issue #663 P5.2). Mirrors
/// [`mpeg_pes::PesAssembler::feed`]'s own bookkeeping (reset the running
/// total on `payload_unit_start`, else add) so the cap can be checked
/// without a private accessor into the assembler's buffer. On overflow the
/// in-progress PES is dropped (`assembler.flush()`'s return discarded) and a
/// [`DemuxEvent::Discontinuity`] is raised for `pid` — any PES that had
/// *already* completed at this same call (a `payload_unit_start` whose
/// previous buffer was ready) is still returned normally; only the
/// newly-started, now-oversized buffer is affected.
fn feed_pes_bounded(
    stream: &mut StreamState,
    pid: u16,
    pusi: bool,
    payload: &[u8],
    events: &mut VecDeque<DemuxEvent>,
) -> Option<Vec<u8>> {
    let Carrier::Pes(assembler) = &mut stream.carrier else {
        return None;
    };
    // Track how much of the in-progress PES has arrived and what it declared,
    // so a later CC gap can tell "the unit was already complete" from "the unit
    // lost bytes" (r04-W49). The declared length sits at bytes 4..6 of the PES
    // (start code prefix + stream_id + `PES_packet_length`), which the first
    // packet of a unit always carries.
    if pusi {
        stream.pes_received = payload.len();
        stream.pes_declared_len = if payload.len() >= PES_LENGTH_FIELD_END {
            u16::from_be_bytes([payload[4], payload[5]])
        } else {
            0
        };
    } else {
        stream.pes_received = stream.pes_received.saturating_add(payload.len());
    }
    if pusi {
        stream.pes_bytes = payload.len();
    } else if stream.pes_bytes > 0 {
        // Only accumulate once a real `payload_unit_start` has been seen for
        // this PID — mirrors `PesAssembler::feed`'s own "ignore a
        // continuation before the first start" rule (relevant for the
        // `unattributed`-replay path, whose buffered payloads can begin
        // mid-PES), so this counter never diverges from what the assembler
        // is actually buffering.
        stream.pes_bytes = stream.pes_bytes.saturating_add(payload.len());
    }
    let completed = assembler.feed(pusi, payload);
    if stream.pes_bytes > MAX_PES_BUFFER_BYTES {
        let dropped_bytes = stream.pes_bytes as u64;
        let _ = assembler.flush();
        stream.pes_bytes = 0;
        let track = match stream.track.as_ref() {
            Some(TrackState::Live(live)) => Some(live.track_id),
            _ => None,
        };
        events.push_back(DemuxEvent::Discontinuity {
            track,
            kind: DiscontinuityKind::BudgetExceeded {
                bytes: dropped_bytes,
            },
            provenance: EventProvenance {
                pid: Some(pid),
                packet_index: None,
            },
        });
    }
    completed
}

/// Resolve a completed PES packet's `(pts, dts)` and drive it through
/// [`advance_track`] (parked/probing) or [`push_live_au`] (already live).
///
/// A PES packet may legally carry neither PTS nor DTS (`PTS_DTS_flags ==
/// '00'`, ISO/IEC 13818-1 §2.4.3.7) — the 2.7.4 constraint only requires them
/// periodically. Such an access unit is interpolated *forward* from the
/// previous one by [`StreamState::frame_period`] (the measured spacing
/// between the last two stamped access units), rather than being handed the
/// previous stamps verbatim: an identical `dts` pair gave the earlier of the
/// two access units `duration = 0` and made the next stamped one absorb the
/// whole gap (r04-W48). The T-STD permits exactly this derivation —
/// "Decoding times tdn(j + 1), tdn(j + 2),... of access units without encoded
/// DTS or PTS fields which directly follow access unit j may be derived from
/// information in the elementary stream" (§2.4.2.6).
///
/// With no measured period yet (fewer than two stamped access units seen) the
/// previous stamps are reused, exactly as before — there is nothing to
/// interpolate from, and inventing a frame rate would fabricate a timeline.
fn on_completed_pes(
    stream: &mut StreamState,
    pid: u16,
    pes_bytes: &[u8],
    events: &mut VecDeque<DemuxEvent>,
) {
    let Ok(pes) = PesPacket::parse(pes_bytes) else {
        return;
    };
    if pes.payload.is_empty() {
        return;
    }
    // `(pts, dts)` this access unit resolved, and whether either came off the
    // wire — the only values that may update the measured frame period.
    let (pts, dts, stamped) = match pes.header.as_ref() {
        Some(h) => {
            let hp = h.pts.map(|p| p.0);
            let hd = h.dts.map(|d| d.0);
            match (hp, hd) {
                // Both present (PTS_DTS_flags '11').
                (Some(p), Some(d)) => (p, d, true),
                // PTS only ('10'): DTS defaults to PTS, and the pair is a
                // real observation of the clock.
                (Some(p), None) => (p, p, true),
                // DTS only is not a legal wire combination, but a PES parsed
                // from a stream that set only `DTS` (or an in-band PES the
                // assembler synthesised) still yields one — treat it as
                // stamped, with PTS defaulting to DTS.
                (None, Some(d)) => (d, d, true),
                // Neither: interpolate (§2.4.2.6). Never a clock observation.
                (None, None) => (0, 0, false),
            }
        }
        None => (0, 0, false),
    };
    let (pts, dts) = if stamped {
        // The span to the previous stamped access unit covers one period per
        // access unit between them (`units_since_stamped` counts the *other*
        // units, plus one for this one) — what §2.4.3.7's periodic-only
        // requirement makes the raw difference over-count by.
        //
        // The first stamp after a rebase spans the discontinuity itself, so its
        // "period" is meaningless: the estimate is left alone in that case.
        // Both the interpolation period and the rebase step come from this one
        // measurement, behind one validity gate — only a small forward step is
        // evidence of the stream's own cadence, so a gap or a clock jump cannot
        // become either.
        if let Some(prev) = stream.last_stamped_dts
            && !stream.pending_rebase
        {
            let span = dts.wrapping_sub(prev) & TS_WRAP_MASK;
            let periods = stream.units_since_stamped.saturating_add(1);
            let period = span / periods;
            // Feed the window with every *plausible* step and take the median:
            // a single late access unit (or one short gap inside the window)
            // then cannot become "the frame period", which is what a
            // last-observation estimate did. An implausible step is not
            // evidence of cadence at all, so it is not recorded.
            if period > 0 && period <= MAX_FRAME_PERIOD_TICKS {
                if stream.recent_steps.len() == FRAME_PERIOD_WINDOW {
                    stream.recent_steps.remove(0);
                }
                stream.recent_steps.push(period);
                let mut sorted = stream.recent_steps.clone();
                sorted.sort_unstable();
                let median = sorted[sorted.len() / 2];
                stream.frame_period = Some(median);
                stream.last_frame_period = median as i128;
            }
        }
        stream.last_stamped_dts = Some(dts);
        stream.units_since_stamped = 0;
        stream.fallback = (pts, dts);
        stream.has_any = true;
        (pts, dts)
    } else if stream.has_any {
        // Unstamped: continue the timeline one measured frame period past the
        // previous access unit. Until a period has been measured (the first
        // stamped pair may not have arrived yet) the previous stamps are
        // reused, exactly as before this fix — there is nothing to
        // interpolate from, and inventing a frame rate would fabricate a
        // timeline.
        let step = stream.frame_period.unwrap_or(0);
        stream.units_since_stamped = stream.units_since_stamped.saturating_add(1);
        let pts = stream.fallback.0.wrapping_add(step) & TS_WRAP_MASK;
        let dts = stream.fallback.1.wrapping_add(step) & TS_WRAP_MASK;
        stream.fallback = (pts, dts);
        (pts, dts)
    } else {
        // First access unit on this PID carries no timing at all: nothing to
        // anchor to. Keep the zero anchor (a real 90 kHz timestamp on tick 0 is
        // not distinguishable, and `WrapState` already treats a leading zero as
        // "no genuine value yet") — but do **not** let it seed the timeline,
        // because a `(0, 0)` anchor would make the first *stamped* access unit
        // look like a jump of however far the real clock is from zero, and that
        // inflated span would become both the interpolation period and the
        // rebase step. `has_any` stays false, so the next stamped access unit
        // anchors from the wire clock, and `units_since_stamped` still counts
        // this one so a later measurement divides by the right count.
        stream.units_since_stamped = stream.units_since_stamped.saturating_add(1);
        (0, 0)
    };
    if stream.pending_rebase {
        // The first stamp of the new time base lifts the whole timeline by
        // whatever the jump backwards would have been, so decode order stays
        // monotonic across the discontinuity (r04-W50). The indicator may
        // arrive while the *old* base's last unit is still completing, in which
        // case this call is a no-op and the wait continues.
        stream.pending_rebase = stream
            .wrap
            .rebase_at_discontinuity(dts, stream.last_frame_period)
            == DiscontinuityVerdict::Forward;
    }
    if std::env::var("TSDBGA").is_ok() && pid == 0x0101 {
        std::eprintln!(
            "AUDIO PES pid={pid:#06x} bytes={} payload={} declared={}",
            pes_bytes.len(),
            pes.payload.len(),
            pes.pes_packet_length
        );
    }
    let (pts_uw, dts_uw) = stream.wrap.push(pts, dts);
    advance_track(stream, pid, pes.payload.to_vec(), pts_uw, dts_uw, events);
}

/// Drive one reassembled PSI/private section through [`advance_track`]
/// (issue #576) — sections carry no PTS/DTS at all, so `pts_uw`/`dts_uw` are
/// dummy zeros (never read by [`LiveKind::Section`]'s immediate-emit push).
fn on_completed_section(
    stream: &mut StreamState,
    pid: u16,
    section: &[u8],
    events: &mut VecDeque<DemuxEvent>,
) {
    if section.is_empty() {
        return;
    }
    advance_track(stream, pid, section.to_vec(), 0, 0, events);
}

/// [`DemuxEvent`] moved to `crate::ir::event` (media plane step 2e: it is
/// not TS-only — [`crate::flv_stream::StreamingFlvDemux`] emits it too).
/// Re-exported from this path so `transmux::ts_demux::DemuxEvent` keeps
/// resolving unchanged.
pub use crate::ir::{
    AbandonReason, DemuxEvent, DiscontinuityKind, EventProvenance, InputDegradation,
};

/// `program_clock_reference`'s native clock rate (ISO/IEC 13818-1 §2.4.3.5) —
/// the `clock_hz` [`DemuxEvent::ClockReference`] carries for every PCR this
/// demuxer emits.
const PCR_CLOCK_HZ: u32 = 27_000_000;

/// Per-PID continuity-counter state for [`StreamingTsDemux`] (issue #778).
///
/// Tracks the last-seen CC and its payload bytes for duplicate detection
/// (ISO/IEC 13818-1 §2.4.3.3: a legal duplicate is a re-transmitted packet
/// with same CC + identical payload). Skipping non-payload-bearing packets
/// (AFC `00`/`10`) on this PID, as they do not advance the counter.
#[derive(Debug, Clone)]
struct CcState {
    /// Whether we've seen the first payload-bearing packet for this PID.
    initialized: bool,
    /// Last continuity counter value on this PID (payload-bearing only).
    last_cc: u8,
    /// The whole last packet, for the §2.4.3.3 legal-duplicate comparison —
    /// which is defined over every byte with only the PCR field excepted, so
    /// the payload slice alone is not enough (r04-W49).
    last_packet: Option<[u8; TS_PACKET_SIZE]>,
    /// Whether `last_packet` was itself already accepted as the one legal
    /// repeat of its predecessor: §2.4.3.3 permits "two, and only two
    /// consecutive" packets, so a third identical repeat is a fault.
    dup_used_for_last: bool,
}

/// What [`StreamingTsDemux::check_cc`] concluded about a payload-bearing
/// packet (r04-W49).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CcVerdict {
    /// Feed the packet to the reassembler as normal.
    Deliver,
    /// A legal §2.4.3.3 duplicate: the decoder shall discard it, so its
    /// payload must not reach the reassembler (it would duplicate 184 bytes
    /// inside the access unit under construction).
    Duplicate,
    /// A continuity-counter gap: at least one packet on this PID was lost, so
    /// the access unit currently being reassembled is missing bytes and must
    /// be dropped rather than delivered truncated. The packet itself is still
    /// fed, since a `payload_unit_start` begins a *new* access unit that is
    /// intact from its first byte.
    Gap,
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
    pmt_reasm: BTreeMap<u16, PmtState>,
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
    /// ES PIDs whose declaration was withdrawn by an applied PMT diff, and
    /// which no PMT has declared since. Payload arriving on such a PID is
    /// dropped outright rather than buffered into `unattributed`: that buffer
    /// is strictly a *pre*-registration replay window, and replaying
    /// post-removal orphan traffic into a later re-registration would deliver
    /// stale bytes as the re-added track's first samples and anchor its
    /// `start_decode_time` in the past. Cleared per PID by
    /// [`Self::register_new_es`]. Bounded by the 13-bit PID space.
    removed_pids: BTreeSet<u16>,
    /// Which PMT PIDs currently declare each elementary PID — the refcount
    /// behind PMT-diff removal. `streams`/`es_seen` are global but `applied_es`
    /// is per-PMT, so without this a PID declared by two programs is torn down
    /// the moment *either* program's PMT stops listing it.
    es_declarers: BTreeMap<u16, BTreeSet<u16>>,
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
    /// Monotonic track-set generation (issue #774): bumped exactly once per
    /// *applied* PMT diff (add/update/remove), never per PID count. This is
    /// the [`DemuxEvent::TracksResolved`] de-dup key — a PID count is not
    /// reliable (a removal immediately followed by an addition can return the
    /// count to a previously-seen value, which a count-keyed de-dup would
    /// wrongly treat as "already signalled").
    generation: u32,
    /// The [`generation`](Self::generation) value at which
    /// [`DemuxEvent::TracksResolved`] last fired, if ever — re-arms whenever
    /// `generation` advances past this value (issue #624 original mechanism;
    /// re-keyed off `generation` instead of a PID count by issue #774).
    tracks_resolved_signalled_at: Option<u32>,
    /// Per-PID continuity-counter state for `InputDegraded::ContinuityGap`
    /// detection (issue #778). Only populated for payload-bearing, non-null
    /// packets; non-payload-bearing packets (AFC `00`/`10`) are skipped and do
    /// not create entries or advance the counter. Null PID (0x1FFF) is always
    /// excluded.
    cc_states: BTreeMap<u16, CcState>,
}

/// Per-PMT-PID reassembly + version-diffing state (issue #774): the
/// `program_number` this PID was learned under from the PAT (a defensive
/// cross-check against the PMT section's own `program_number`), the last
/// **applied** `version_number` (so a carousel-repeated identical-version
/// section — PMTs repeat several times a second on a real broadcast — is
/// parsed but never re-diffed), and the ES PID set this PMT last applied (the
/// diff baseline: only PIDs *this* PMT declared can be removed by it, never
/// another program's).
struct PmtState {
    reasm: SectionReassembler,
    program_number: u16,
    /// The program's `PCR_PID` (§2.4.4.8 Table 2-33), or `None` until a PMT
    /// has been applied. A system time-base discontinuity is signalled on this
    /// PID (§2.4.3.5), and only this program's elementary streams are rebased
    /// by it (r04-W50 review).
    pcr_pid: Option<u16>,
    last_applied_version: Option<u8>,
    applied_es: BTreeSet<u16>,
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
            removed_pids: BTreeSet::new(),
            es_declarers: BTreeMap::new(),
            unattributed_order: VecDeque::new(),
            unattributed_bytes: 0,
            codec_order: Vec::new(),
            data_order: Vec::new(),
            resolved: BTreeSet::new(),
            next_track_id: 1,
            events: VecDeque::new(),
            generation: 0,
            tracks_resolved_signalled_at: None,
            cc_states: BTreeMap::new(),
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

    /// Best-effort resolved track ID for `pid`, when it has already been
    /// promoted to [`TrackState::Live`] — used to populate
    /// [`DemuxEvent::Discontinuity`]'s `track` field. `None` (never
    /// fabricated) when the PID is not yet known, still `Probing`/`Parked`,
    /// or has no [`StreamState`] at all (e.g. a discontinuity observed before
    /// this PID's PMT entry has been seen).
    fn live_track_id(&self, pid: u16) -> Option<u32> {
        match self.streams.get(&pid)?.track.as_ref()? {
            TrackState::Live(live) => Some(live.track_id),
            _ => None,
        }
    }

    fn process_packet(&mut self, raw: &[u8; TS_PACKET_SIZE]) {
        let idx = self.packet_index;
        self.packet_index += 1;
        let Ok(pkt) = TsPacket::parse(raw) else {
            return;
        };

        // TEI degradation (issue #778) — the demodulator-set flag on an
        // uncorrectable packet error, independent of PID classification or
        // adaptation field handling.
        if pkt.header.tei {
            let provenance = EventProvenance {
                pid: Some(pkt.header.pid),
                packet_index: Some(idx),
            };
            self.events.push_back(DemuxEvent::InputDegraded {
                track: self.live_track_id(pkt.header.pid),
                kind: InputDegradation::TransportError,
                provenance,
            });
        }

        // CC check (issue #778) — payload-bearing, non-null packets only,
        // excluding signalled discontinuities (the adaptation-field check
        // that follows).
        let discontinuity_signalled = pkt.header.has_adaptation
            && raw
                .get(4)
                .is_some_and(|af_len| *af_len > 0 && raw.get(5).is_some_and(|b| b & 0x80 != 0));

        // PCR / discontinuity — independent of PID classification, matches
        // every packet's adaptation field regardless of payload routing.
        if let Some(Ok(af)) = pkt.adaptation_field() {
            let provenance = EventProvenance {
                pid: Some(pkt.header.pid),
                packet_index: Some(idx),
            };
            if af.discontinuity_indicator {
                // §2.4.3.5: a system time-base discontinuity is signalled by
                // the `discontinuity_indicator` in a packet of the program's
                // **PCR_PID** — "When the discontinuity state is true for a
                // transport stream packet of a PID designated as a PCR_PID, the
                // next PCR in a transport stream packet with that same PID
                // represents a sample of a new system time clock for the
                // associated program." So the rebase is scoped twice over:
                // only the packet's own PID can carry it, and only that PID's
                // *program* is rebased. A multi-program multiplex therefore
                // leaves the other programs' timelines untouched (r04-W50
                // review), where the first attempt marked every stream in the
                // demux.
                let pid = pkt.header.pid;
                let program_ok = self.pmt_reasm.values().any(|pmt| pmt.pcr_pid == Some(pid));
                if program_ok {
                    // Every elementary stream of this program: the ones whose
                    // declaring PMT is this PCR_PID's.
                    let members: Vec<u16> = self
                        .es_declarers
                        .iter()
                        .filter(|(_, pmt_pids)| {
                            pmt_pids.iter().any(|pmt_pid| {
                                self.pmt_reasm
                                    .get(pmt_pid)
                                    .is_some_and(|pmt| pmt.pcr_pid == Some(pid))
                            })
                        })
                        .map(|(&es_pid, _)| es_pid)
                        .collect();
                    for es_pid in members {
                        if let Some(stream) = self.streams.get_mut(&es_pid) {
                            stream.pending_rebase = true;
                        }
                    }
                }
                self.events.push_back(DemuxEvent::Discontinuity {
                    track: self.live_track_id(pid),
                    kind: DiscontinuityKind::Signalled,
                    provenance,
                });
            }
            if let Some(pcr) = af.pcr {
                self.events.push_back(DemuxEvent::ClockReference {
                    ticks: pcr.as_27mhz(),
                    clock_hz: PCR_CLOCK_HZ,
                    discontinuous: af.discontinuity_indicator,
                    provenance,
                });
            }
        }

        let mut cc_verdict = CcVerdict::Deliver;
        // CC gap degradation (issue #778) — payload-bearing, non-null packets
        // only. The discontinuity flag is passed in so check_cc can suppress
        // the EVENT but STILL UPDATE the per-PID state (matching both in-repo
        // reference implementations: dvb-conformance's check_cc at
        // lib.rs:979-982 and media-doctor's CcAnomalyCheck at cc_anomaly.rs:110-113
        // — both update last_cc unconditionally on every payload-bearing packet,
        // including those with discontinuity_indicator set).
        if pkt.header.has_payload && pkt.header.pid != NULL_PACKET_PID {
            cc_verdict = self.check_cc(
                pkt.header.pid,
                pkt.header.continuity_counter,
                raw,
                discontinuity_signalled,
                idx,
            );
        }

        let pid = pkt.header.pid;
        let pusi = pkt.header.pusi;
        let Some(payload) = pkt.payload else {
            return;
        };

        // r04-W49. A legal §2.4.3.3 duplicate is discarded outright — feeding
        // it on would append its 184 payload bytes a second time to the access
        // unit under construction. (`check_cc` already updated the per-PID
        // baseline, exactly as the spec's "same continuity_counter as the
        // original" requires.)
        if cc_verdict == CcVerdict::Duplicate {
            self.try_promote_ready();
            return;
        }
        // A gap left the access unit currently in the reassembler short of
        // bytes, so it must not be delivered: the IR cannot tell a complete
        // PES from a truncated one, and every downstream muxer would write the
        // corruption out. Mark it abandoned (`keep_payload = false`) rather
        // than resetting the assembler — a reset would also stop the byte
        // accounting that bounds a runaway PES. A `payload_unit_start` clears
        // the mark, because it begins a new, intact access unit from its first
        // byte. Adaptation-field-only packets never reach here.

        if pid == PAT_PID {
            self.pat_reasm.feed(payload, pusi);
            while let Some(section) = self.pat_reasm.pop_section() {
                // A corrupt PAT must never rebind a PID: an ES PID wrongly
                // landing in `pmt_reasm` shadows `streams` for the rest of the
                // stream (see `psi_section_crc_ok`).
                if !psi_section_crc_ok(&section) {
                    continue;
                }
                // A "next" PAT (§2.4.4.1) is parsed but not applied — the same
                // `current_next_indicator == 1` rule PMT application uses.
                if !section_current_next(&section) {
                    continue;
                }
                if let Ok(programs) = parse_pat(&section) {
                    for (program_number, pmt_pid) in programs {
                        self.learn_pmt_pid(pmt_pid, program_number);
                    }
                }
            }
            return;
        }

        if let Some(pmt_state) = self.pmt_reasm.get_mut(&pid) {
            pmt_state.reasm.feed(payload, pusi);
            let mut sections: Vec<Vec<u8>> = Vec::new();
            while let Some(section) = pmt_state.reasm.pop_section() {
                sections.push(section.to_vec());
            }
            let program_number = pmt_state.program_number;
            let mut to_apply: Option<Vec<(u16, Codec, Vec<u8>)>> = None;
            for section in &sections {
                // CRC first, before *anything* observable happens: PMT
                // application is destructive (it can tear a live track down
                // and reassign track_ids), and even bumping
                // `last_applied_version` off a corrupt section would suppress
                // the genuine version that follows. See `psi_section_crc_ok`.
                if !psi_section_crc_ok(section) {
                    continue;
                }
                let Ok(header) = parse_pmt_section_header(section) else {
                    continue;
                };
                if header.program_number != program_number {
                    // Defensive cross-check (issue #774): a PMT PID's
                    // program_number must match the PAT entry it was learned
                    // under. A mismatch is stream corruption or a PAT/PMT
                    // race — never act on it.
                    continue;
                }
                if header.section_number != 0 || header.last_section_number != 0 {
                    // A PMT is always single-section (§2.4.4.8) — a
                    // multi-section claim is malformed, ignore it.
                    continue;
                }
                if !header.current_next {
                    // A "next" table: parsed, never applied.
                    continue;
                }
                if pmt_state.last_applied_version == Some(header.version) {
                    // Carousel repeat (identical applied version) — dropped
                    // before the diff, never re-processed.
                    continue;
                }
                pmt_state.last_applied_version = Some(header.version);
                // Record the PCR_PID the program just declared: it is the PID
                // on which a system time-base discontinuity for this program is
                // signalled (§2.4.3.5), and the key for scoping a rebase to
                // this program's streams (r04-W50 review).
                pmt_state.pcr_pid = Some(header.pcr_pid);
                if let Ok(es_list) = parse_pmt(section) {
                    to_apply = Some(es_list);
                }
            }
            if let Some(es_list) = to_apply {
                let old_applied_es = pmt_state.applied_es.clone();
                let new_applied_es: BTreeSet<u16> = es_list.iter().map(|(p, _, _)| *p).collect();
                self.apply_pmt_diff(pid, &old_applied_es, es_list);
                if let Some(pmt_state) = self.pmt_reasm.get_mut(&pid) {
                    pmt_state.applied_es = new_applied_es;
                }
            }
            self.try_promote_ready();
            return;
        }

        if let Some(stream) = self.streams.get_mut(&pid) {
            let mut sections: Vec<Vec<u8>> = Vec::new();
            // ── r04-W49 intactness rule ────────────────────────────────────
            //
            // An access unit is delivered only when nothing was lost from it:
            //
            //  * **A gap on a continuation packet** (`cc_verdict == Gap`,
            //    `pusi == false`) means 184 bytes of the unit in progress are
            //    missing. It is marked damaged, and the mark survives to the
            //    end of the unit — no later packet may clear it.
            //  * **A gap on a `payload_unit_start` packet** is benign: a
            //    `payload_unit_start` *ends* the previous unit, so nothing
            //    after the jump belongs to it. The byte stream simply
            //    restarts — which is what every independently muxed segment
            //    does (its counter begins wherever its own muxer chose), and
            //    what a splice looks like when the source sets no
            //    `discontinuity_indicator`. Anything genuinely lost inside the
            //    completing unit shows up as a gap on one of its own
            //    continuation packets, which the first rule catches.
            //  * **A signalled discontinuity** (`discontinuity_indicator`) is
            //    **This is not, by itself, a reason to drop anything.** The
            //    indicator says the source's byte stream is discontinuous here,
            //    and when it lands on a *continuation* packet that packet's
            //    bytes are indeed missing (the gap rule above marks it). But a
            //    packet carrying the indicator which *completes* a whole unit —
            //    a normal HLS or splice seam, where the last unit of the old
            //    segment is intact and the indicator merely precedes the new
            //    one — must still be delivered. Dropping it lost the last old
            //    access unit at every seam, which is exactly the loss this
            //    rule exists to avoid. Only an actual CC gap inside the unit,
            //    or a bounded PES short of its declared length, damages it.
            //
            // A `payload_unit_start` always begins a *new*, whole unit.
            let gap_here = cc_verdict == CcVerdict::Gap;
            // An independent, length-based check: a *bounded* PES
            // (`PES_packet_length != 0`) that received fewer bytes than its own
            // header declared is truncated, whatever the continuity counter
            // says. `PES_packet_length` counts the bytes after its own field,
            // so the whole packet is `PES_LENGTH_FIELD_END + declared`; the
            // received count is TS-payload bytes, which can exceed that by the
            // last packet's `0xFF` stuffing, so the test is `<`.
            let short_of_declared = stream.pes_declared_len > 0
                && stream.pes_received < PES_LENGTH_FIELD_END + stream.pes_declared_len as usize;
            // A unit is damaged only by something that actually lost bytes from
            // it: a continuity-counter gap on one of its own packets (the mark)
            // or a bounded PES that arrived short of its declared length. A
            // `discontinuity_indicator` is *not* such evidence on its own —
            // see the rule above.
            let completed_intact = stream.current_pes_intact && !short_of_declared;
            let completed_pes = if matches!(stream.carrier, Carrier::Pes(_)) {
                feed_pes_bounded(stream, pid, pusi, payload, &mut self.events)
            } else if let Carrier::Section(reasm) = &mut stream.carrier {
                reasm.feed(payload, pusi);
                while let Some(s) = reasm.pop_section() {
                    sections.push(s.to_vec());
                }
                None
            } else {
                None
            };
            // `completed_intact` false means dropped, not delivered (r04-W49).
            if completed_intact && let Some(completed) = completed_pes {
                on_completed_pes(stream, pid, &completed, &mut self.events);
            }
            if matches!(stream.carrier, Carrier::Pes(_)) {
                if pusi {
                    // A new unit begins whole from its first byte.
                    stream.current_pes_intact = true;
                } else if gap_here {
                    // The unit in progress lost bytes. Two things are needed:
                    // the mark (so the unit is dropped, never delivered
                    // truncated), and an assembler reset — without it the
                    // bytes arriving *after* the gap would be appended to the
                    // pre-gap bytes as if contiguous, welding two unrelated
                    // fragments into one buffer that some later
                    // `payload_unit_start` would hand on. The byte accounting
                    // is deliberately left alone: `pes_bytes` continues to
                    // count what this PID is buffering, so a runaway PES is
                    // still bounded.
                    stream.current_pes_intact = false;
                    if let Carrier::Pes(assembler) = &mut stream.carrier {
                        let _ = assembler.flush();
                    }
                }
            }
            for s in sections {
                on_completed_section(stream, pid, &s, &mut self.events);
            }
        } else if pid != NULL_PACKET_PID && !self.removed_pids.contains(&pid) {
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

    /// Check the continuity counter for `pid` against the tracking state,
    /// emitting [`DemuxEvent::InputDegraded`]`(`[`InputDegradation::ContinuityGap`]`)`
    /// when a genuine gap is detected (issue #778), and returning what the
    /// caller must do with this packet's payload (r04-W49).
    ///
    /// Does **not** fire for:
    /// - Signalled discontinuities (`discontinuity_signalled`): the event is
    ///   suppressed, but `last_cc` and `last_payload` are still updated to
    ///   this packet (matching the dvb-conformance and media-doctor reference
    ///   implementations — both update the CC baseline unconditionally).
    /// - Legal duplicates: same CC + byte-identical packet with only the PCR
    ///   field excepted (ITU-T H.222.0 §2.4.3.3) — see [`CcVerdict::Duplicate`].
    ///
    /// # Arguments
    /// - `pid` — the TS PID.
    /// - `cc` — the 4-bit continuity counter from the packet header.
    /// - `raw` — the whole 188-byte packet, for the §2.4.3.3 duplicate
    ///   comparison (which the adapter must have handled: `pkt.payload`
    ///   excludes the adaptation field, so a PCR-only difference between an
    ///   original and its duplicate is invisible to it).
    /// - `discontinuity_signalled` — `true` when the adaptation field's
    ///   `discontinuity_indicator` is set. Suppresses the event but NOT the
    ///   state update.
    /// - `packet_index` — 0-based index of this packet in the stream.
    fn check_cc(
        &mut self,
        pid: u16,
        cc: u8,
        raw: &[u8; TS_PACKET_SIZE],
        discontinuity_signalled: bool,
        packet_index: u64,
    ) -> CcVerdict {
        let track = self.live_track_id(pid);
        match self.cc_states.entry(pid) {
            Entry::Occupied(mut e) => {
                let state = e.get_mut();
                if !state.initialized {
                    state.initialized = true;
                    state.last_cc = cc;
                    state.last_packet = Some(*raw);
                    return CcVerdict::Deliver;
                }
                // §2.4.3.3: a legal duplicate repeats the previous packet
                // byte-for-byte (PCR field excepted), same continuity_counter.
                // The decoder shall discard it — it carries no new data, and
                // feeding it to the reassembler duplicated 184 payload bytes
                // into the access unit under construction (r04-W49).
                //
                // `pkt.payload` alone cannot decide this: a duplicate's PCR is
                // legally re-encoded, so the comparison must cover the whole
                // packet. `check_duplicate` also folds in the "two, and only
                // two consecutive" rule — a third identical repeat is an error,
                // not a duplicate.
                if cc == state.last_cc
                    && let Some(prev) = state.last_packet
                    && broadcast_common::ts_dup::check_duplicate(
                        &prev,
                        raw,
                        state.dup_used_for_last,
                    ) == broadcast_common::ts_dup::DuplicateVerdict::Legal
                {
                    // §2.4.3.3's one legal repeat. `dup_used_for_last` records
                    // that this PID has now spent it, so a *third* identical
                    // repeat (`cc == last_cc` again) is a continuity fault
                    // rather than another duplicate — `check_duplicate` makes
                    // that call, and this branch only ever takes the `Legal`
                    // verdict.
                    state.dup_used_for_last = true;
                    state.last_packet = Some(*raw);
                    return CcVerdict::Duplicate;
                }
                let expected = (state.last_cc + 1) & 0x0F;
                let mut verdict = CcVerdict::Deliver;
                if !discontinuity_signalled && cc != expected {
                    self.events.push_back(DemuxEvent::InputDegraded {
                        track,
                        kind: InputDegradation::ContinuityGap { expected, got: cc },
                        provenance: EventProvenance {
                            pid: Some(pid),
                            packet_index: Some(packet_index),
                        },
                    });
                    // The gap lost at least one packet, so the access unit the
                    // reassembler is part-way through is missing bytes. Let it
                    // be delivered as if complete and it is corrupt — the
                    // coarse timeline repair (`rebase`) cannot tell a complete
                    // PES from a truncated one. Request the drop, matching
                    // `rtp_stream`'s own response to loss.
                    verdict = CcVerdict::Gap;
                }
                state.last_cc = cc;
                state.last_packet = Some(*raw);
                state.dup_used_for_last = false;
                verdict
            }
            Entry::Vacant(e) => {
                e.insert(CcState {
                    initialized: true,
                    last_cc: cc,
                    last_packet: Some(*raw),
                    dup_used_for_last: false,
                });
                CcVerdict::Deliver
            }
        }
    }

    /// Bind `pmt_pid` to the `program_number` a currently-applicable PAT
    /// (§2.4.4.3) just listed it under.
    ///
    /// The binding is **updatable**, not write-once. A PAT may legitimately
    /// remap a PMT PID to a different program mid-stream, and the previous
    /// `entry().or_insert_with()` froze the first `program_number` ever seen —
    /// after which the defensive `header.program_number != program_number`
    /// cross-check in [`Self::process_packet`] rejected *every* PMT on that
    /// PID forever, silently demuxing the program to zero tracks.
    ///
    /// A re-bind also clears `last_applied_version`: the version counter
    /// belongs to the program's PMT, not to the PID, so the new program's PMT
    /// may legitimately re-use a `version_number` the old program had already
    /// applied. `applied_es` is deliberately kept — it is the diff baseline of
    /// what this PID last put into `streams`, and the incoming PMT must still
    /// be diffed against it so the outgoing program's tracks are torn down.
    fn learn_pmt_pid(&mut self, pmt_pid: u16, program_number: u16) {
        match self.pmt_reasm.get_mut(&pmt_pid) {
            Some(state) if state.program_number != program_number => {
                state.program_number = program_number;
                state.last_applied_version = None;
            }
            Some(_) => {}
            None => {
                self.pmt_reasm.insert(
                    pmt_pid,
                    PmtState {
                        reasm: SectionReassembler::default(),
                        program_number,
                        pcr_pid: None,
                        last_applied_version: None,
                        applied_es: BTreeSet::new(),
                    },
                );
            }
        }
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

    /// Register a genuinely new elementary-stream PID discovered from an
    /// applied PMT (first ever registration, or a version diff's "added"
    /// side — issue #774): rank it into `codec_order`/`data_order`, build its
    /// fresh [`StreamState`] (`Probing` from scratch), and replay any
    /// `unattributed` payloads that arrived on this PID before its PMT
    /// registration completed. Never emits an event itself — `TrackAdded`
    /// fires once this PID reaches its PMT-declaration-order turn in
    /// [`Self::try_promote_ready`].
    ///
    /// Appends to the back of its destination order list — the correct slot
    /// for a PID never seen before. A codec-changed *re*-registration (see
    /// `apply_pmt_diff`) instead goes through [`Self::register_new_es_at`] to
    /// preserve the PID's original PMT-declaration-order slot.
    fn register_new_es(&mut self, es_pid: u16, codec: Codec, descriptors: Vec<u8>) {
        self.register_new_es_at(es_pid, codec, descriptors, None);
    }

    /// As [`Self::register_new_es`], but inserts the PID at `reinsert_at`
    /// within its destination order list instead of always appending
    /// (issue: re-registration losing PMT-declaration order). Used by
    /// `apply_pmt_diff`'s codec-changed path to restore the PID to the slot
    /// it occupied before `remove_track` erased it, instead of losing that
    /// slot to the back of the list — which would reorder `TrackAdded`
    /// emission and could block a later-ranked PID's promotion behind it.
    /// `None` (this PID has never been ranked, or it is crossing between the
    /// codec/data lists — see the caller) appends, exactly like
    /// `register_new_es`.
    fn register_new_es_at(
        &mut self,
        es_pid: u16,
        codec: Codec,
        descriptors: Vec<u8>,
        reinsert_at: Option<usize>,
    ) {
        // A PID declared again is no longer an orphan: lift the post-removal
        // buffering blacklist (see `remove_track`) so its fresh traffic is
        // routed normally from here on.
        self.removed_pids.remove(&es_pid);
        let order = if matches!(codec, Codec::Data(_)) {
            &mut self.data_order
        } else {
            &mut self.codec_order
        };
        match reinsert_at {
            Some(idx) if idx <= order.len() => order.insert(idx, es_pid),
            _ => order.push(es_pid),
        }
        let mut stream = StreamState {
            codec,
            descriptors,
            carrier: initial_carrier(codec),
            last_frame_period: 0,
            recent_steps: Vec::new(),
            pending_rebase: false,
            current_pes_intact: true,
            pes_declared_len: 0,
            pes_received: 0,
            pes_bytes: 0,
            fallback: (0, 0),
            frame_period: None,
            last_stamped_dts: None,
            units_since_stamped: 0,
            has_any: false,
            wrap: WrapState::default(),
            track: Some(TrackState::Probing {
                probe: initial_probe(codec),
                backlog: Vec::new(),
            }),
            backlog_bytes: 0,
        };
        // Replay any payloads that arrived on this PID before its PMT
        // registration completed (see `unattributed`'s doc).
        if let Some(buffered) = self.unattributed.remove(&es_pid) {
            for (buf_pusi, buf_payload) in buffered {
                self.unattributed_bytes = self.unattributed_bytes.saturating_sub(buf_payload.len());
                let mut sections: Vec<Vec<u8>> = Vec::new();
                let completed_pes = if matches!(stream.carrier, Carrier::Pes(_)) {
                    feed_pes_bounded(
                        &mut stream,
                        es_pid,
                        buf_pusi,
                        &buf_payload,
                        &mut self.events,
                    )
                } else if let Carrier::Section(reasm) = &mut stream.carrier {
                    reasm.feed(&buf_payload, buf_pusi);
                    while let Some(s) = reasm.pop_section() {
                        sections.push(s.to_vec());
                    }
                    None
                } else {
                    None
                };
                if let Some(completed) = completed_pes {
                    on_completed_pes(&mut stream, es_pid, &completed, &mut self.events);
                }
                for s in sections {
                    on_completed_section(&mut stream, es_pid, &s, &mut self.events);
                }
            }
        }
        self.streams.insert(es_pid, stream);
    }

    /// Drop a PID that a PMT no longer declares (issue #774): remove it from
    /// every bookkeeping set (`es_seen`/`codec_order`/`data_order`/`resolved`)
    /// and drop its [`StreamState`] entirely — any in-flight PES/backlog for
    /// it goes with it, so no [`DemuxEvent::Sample`] can ever follow the
    /// [`DemuxEvent::TrackRemoved`] this emits below. Only emits
    /// `TrackRemoved` when the PID had actually reached `Live` (a real
    /// `track_id` a consumer has seen via `TrackAdded`) — a PID removed while
    /// still `Probing`/`Parked`/`Abandoned` was never surfaced to a consumer
    /// in the first place, so there is nothing to report removing.
    ///
    /// Also purges — and then blacklists — this PID's `unattributed` backlog.
    /// That buffer exists solely to replay payloads that arrived *before* a
    /// PID's very first PMT registration; anything on a PID the declaration has
    /// since dropped is orphan traffic. Left in place it would accumulate to
    /// [`MAX_UNATTRIBUTED_BYTES`] and then be replayed as the *re-added*
    /// track's first samples, anchoring its `start_decode_time` in the past.
    /// The blacklist is lifted by [`Self::register_new_es`] the moment a PMT
    /// declares the PID again.
    fn remove_track(&mut self, pid: u16) {
        self.es_seen.remove(&pid);
        self.codec_order.retain(|&p| p != pid);
        self.data_order.retain(|&p| p != pid);
        self.resolved.remove(&pid);
        if let Some(buffered) = self.unattributed.remove(&pid) {
            for (_, payload) in &buffered {
                self.unattributed_bytes = self.unattributed_bytes.saturating_sub(payload.len());
            }
        }
        self.removed_pids.insert(pid);
        // Drop the continuity-counter baseline too: the PID's reassembly state
        // is gone with the track, so the CC of the first packet after a re-add
        // is judged against nothing, not against the pre-removal sequence
        // (which the source is under no obligation to continue — r04-W49's
        // gap handling would otherwise drop that first, perfectly good access
        // unit as truncated).
        self.cc_states.remove(&pid);
        if let Some(stream) = self.streams.remove(&pid)
            && let Some(TrackState::Live(live)) = stream.track
        {
            self.events.push_back(DemuxEvent::TrackRemoved {
                track_id: live.track_id,
                provenance: EventProvenance {
                    pid: Some(pid),
                    packet_index: None,
                },
            });
        }
    }

    /// Apply one PMT's newly-parsed, version-changed ES list against
    /// `old_applied_es` (that same PMT's previous applied ES set — issue
    /// #774): diff removed/added/kept-but-changed PIDs, then bump
    /// [`Self::generation`] once so [`Self::maybe_signal_tracks_resolved`]
    /// re-evaluates `TracksResolved`. Called only for a section that has
    /// already been confirmed `current_next_indicator == 1` with a
    /// genuinely new `version_number` — a carousel repeat never reaches here.
    fn apply_pmt_diff(
        &mut self,
        pmt_pid: u16,
        old_applied_es: &BTreeSet<u16>,
        es_list: Vec<(u16, Codec, Vec<u8>)>,
    ) {
        let new_pids: BTreeSet<u16> = es_list.iter().map(|(p, _, _)| *p).collect();

        let removed: Vec<u16> = old_applied_es
            .iter()
            .copied()
            .filter(|p| !new_pids.contains(p))
            .collect();
        for pid in removed {
            // Refcounted by declaring PMT (`es_declarers`): the same
            // elementary PID may legally appear in more than one program's
            // PMT (a shared audio/subtitle component), and `streams`/`es_seen`
            // are global while `applied_es` is per-PMT — so one program
            // dropping the PID must not tear down a stream another program
            // still declares. Only the *last* declarer's drop removes it.
            let declarers = self.es_declarers.get_mut(&pid);
            let still_declared = match declarers {
                Some(declarers) => {
                    declarers.remove(&pmt_pid);
                    let empty = declarers.is_empty();
                    if empty {
                        self.es_declarers.remove(&pid);
                    }
                    !empty
                }
                None => false,
            };
            if !still_declared {
                self.remove_track(pid);
            }
        }

        for (es_pid, codec, descriptors) in es_list {
            self.es_declarers.entry(es_pid).or_default().insert(pmt_pid);
            if self.es_seen.insert(es_pid) {
                self.register_new_es(es_pid, codec, descriptors);
                continue;
            }
            // Compare through a shared borrow first, then act — the two
            // outcomes need different (and mutually exclusive) mutable
            // borrows of `self`.
            let Some(existing) = self.streams.get(&es_pid) else {
                continue;
            };
            let old_codec = existing.codec;
            let codec_changed = old_codec != codec;
            let descriptors_changed = existing.descriptors != descriptors;
            if !codec_changed && !descriptors_changed {
                continue;
            }
            if codec_changed {
                // Refcount check (the same one the "removed" loop above
                // applies, F1): the same elementary PID may legally be
                // declared by more than one program's PMT (a shared
                // audio/subtitle component). `es_declarers[es_pid]` already
                // has `pmt_pid` inserted (the unconditional insert above), so
                // any *other* entry means some other program still declares
                // this PID — and `streams`/`codec_order`/`data_order` are
                // global, not per-program, so tearing down here would also
                // destroy that other program's still-valid track (wrong
                // `track_id` churn: a spurious `TrackRemoved` + `TrackAdded`
                // for a change only *this* program asked for).
                //
                // Two programs declaring the same PID under two different
                // codecs is a malformed multiplex — there is only one global
                // stream for a PID, so at most one classification can be
                // active. Decision (stated here, not left implicit): refuse
                // to reclassify while any other declarer remains. The
                // existing classification wins and this PMT's update is
                // dropped for this PID; last-writer-wins was rejected because
                // it would let either program's routine version bump flip the
                // shared track back and forth. Once every *other* declarer
                // has itself dropped or stopped disagreeing, `es_declarers`
                // shrinks to just this PMT and a later reclassification by it
                // proceeds normally below.
                let other_declarers_remain = self
                    .es_declarers
                    .get(&es_pid)
                    .is_some_and(|declarers| declarers.iter().any(|&p| p != pmt_pid));
                if other_declarers_remain {
                    continue;
                }

                // A reclassified `stream_type` is a **different elementary
                // stream**, not an in-place relabel. Writing `stream.codec`
                // through (as this used to) left three pieces of derived state
                // built for the OLD codec:
                //
                //  * the `ConfigProbe` — e.g. `stream_type` 0x06 gaining an
                //    AC-3 descriptor turns `Codec::Data(0x06)` into
                //    `Codec::Ac3` while `ConfigProbe::Data` remains, which
                //    used to reach an `unreachable!` in `finalize_probe`;
                //  * the `Carrier` — ISO/IEC 13818-1 Table 2-34 splits
                //    `stream_type` into PES- and section-carried families, and
                //    feeding one family's bytes to the other's reassembler
                //    (0x86 → 0x1B: H.264 PES into a `SectionReassembler`)
                //    silently yields nothing while the track claims to exist;
                //  * any buffered access units, decoded under the old
                //    framing.
                //
                // Teardown-and-re-register rebuilds all three in one move:
                // `remove_track` drops the stream (emitting `TrackRemoved` if
                // it had reached `Live`) and `register_new_es_at` rebuilds
                // `initial_carrier`/`initial_probe` for the new codec.
                //
                // F3: capture this PID's current slot in its *old*
                // classification's order list before `remove_track` erases it,
                // so re-registration can restore the same slot instead of
                // losing it to the back of the list (which would reorder
                // `TrackAdded` emission and could block a later-ranked PID's
                // promotion behind it). Only meaningful when the old and new
                // classification share the same order list (codec vs. data):
                // a PID crossing between the two has no old slot to preserve
                // in the list it's moving to, so it appends there like any
                // other first-time registration into that list.
                let old_is_data = matches!(old_codec, Codec::Data(_));
                let new_is_data = matches!(codec, Codec::Data(_));
                let order_slot = if old_is_data == new_is_data {
                    let order = if old_is_data {
                        &self.data_order
                    } else {
                        &self.codec_order
                    };
                    order.iter().position(|&p| p == es_pid)
                } else {
                    None
                };
                self.remove_track(es_pid);
                // `es_seen` is the "currently declared" set, which
                // `remove_track` clears — but this PID *is* still declared,
                // just as something else.
                self.es_seen.insert(es_pid);
                self.register_new_es_at(es_pid, codec, descriptors, order_slot);
                continue;
            }
            // Descriptors-only change: nothing derived from the codec is
            // stale, so the track keeps its identity and is updated in place.
            let Some(stream) = self.streams.get_mut(&es_pid) else {
                continue;
            };
            stream.descriptors = descriptors.clone();
            if let Some(TrackState::Live(live)) = stream.track.as_ref() {
                let spec = TrackSpec::new(live.track_id, live.timescale, live.config.clone())
                    .with_source(es_pid, descriptors);
                self.events.push_back(DemuxEvent::TrackUpdated(spec));
            }
        }

        self.generation = self.generation.wrapping_add(1);
    }

    /// Promote every `Parked` PID that has reached its PMT-declaration-order
    /// turn to `Live`: assign the next sequential track ID, emit
    /// `DemuxEvent::TrackAdded`, and replay its accumulated backlog as a
    /// burst of `DemuxEvent::Sample`s — repeating while the *next*-ranked PID
    /// is also already `Parked`. Stops at the first PID that is still
    /// `Probing` (blocked) or not yet known at all.
    fn try_promote_ready(&mut self) {
        while let Some(&next_pid) = self
            .codec_order
            .iter()
            .chain(self.data_order.iter())
            .find(|p| !self.resolved.contains(p))
        {
            let Some(stream) = self.streams.get_mut(&next_pid) else {
                break;
            };
            // See `advance_track`: `track` is `None` only transiently, inside
            // these two functions. Degrade rather than panic.
            let Some(track) = stream.track.take() else {
                break;
            };
            match track {
                TrackState::Parked {
                    config,
                    timescale,
                    kind,
                    backlog,
                } => {
                    let track_id = self.next_track_id;
                    self.next_track_id += 1;
                    // `Track::start_decode_time` is no longer carried by this
                    // event (issue #774 reshape dropped it along with
                    // `samples`/`encryption` when `TrackAdded` became
                    // `TrackSpec`-only) — every consumer that builds a `Track`
                    // derives it from `samples[0].dts` instead (media plane
                    // step 2c invariant: the two are always equal), so no
                    // anchor value needs computing here at all.
                    let spec = TrackSpec::new(track_id, timescale, config.clone())
                        .with_source(next_pid, stream.descriptors.clone());
                    // Look up the program_number for this ES PID via its
                    // declaring PMT.
                    let program_number = self
                        .es_declarers
                        .get(&next_pid)
                        .and_then(|pmt_pids| pmt_pids.iter().next())
                        .and_then(|&pmt_pid| self.pmt_reasm.get(&pmt_pid))
                        .map(|state| state.program_number);
                    let spec = if let Some(pn) = program_number {
                        spec.with_program(pn)
                    } else {
                        spec
                    };
                    self.events.push_back(DemuxEvent::TrackAdded(spec));
                    let mut live = LiveTrack {
                        track_id,
                        kind,
                        config,
                        timescale,
                        parameter_sets: ParameterSets::default(),
                        config_header: None,
                    };
                    // Seed the changed-config comparison from the access units
                    // the config was actually recovered from, so the track's
                    // first header repeat after promotion is not mistaken for a
                    // change (r04-W51). The whole backlog is walked, not just
                    // its last access unit: SPS and PPS may be split across
                    // several of them, and the sets must be complete before the
                    // next access unit is compared against them.
                    for au in &backlog {
                        let _ = observe_parameter_sets(
                            stream.codec,
                            &au.data,
                            &mut live.parameter_sets,
                        );
                    }
                    if stream.codec == Codec::Aac
                        && let Some(au) = backlog.last()
                        && let Some((_, hdr)) = find_adts_sync(&au.data)
                    {
                        live.config_header =
                            Some(AudioSpecificConfig::from_adts_header(&hdr).to_bytes());
                    }
                    for au in backlog {
                        push_live_au(&mut live, &au.data, au.pts_uw, au.dts_uw, &mut self.events);
                    }
                    stream.track = Some(TrackState::Live(live));
                    stream.backlog_bytes = 0;
                    self.resolved.insert(next_pid);
                    // loop again: the next-ranked PID may also already be parked
                }
                other @ TrackState::Probing { .. } => {
                    stream.track = Some(other);
                    break; // blocked — an earlier-ranked PID isn't ready yet
                }
                other @ TrackState::Live(_) => {
                    // Already resolved; `resolved` should already contain it,
                    // but stay consistent defensively and keep scanning.
                    stream.track = Some(other);
                    self.resolved.insert(next_pid);
                }
                TrackState::Abandoned => {
                    // [`MAX_PROBE_BACKLOG_BYTES`] overflowed for this PID
                    // (issue B8): permanently resolved without ever
                    // promoting — the same conclusion `finish()` reaches for
                    // a probe that never resolves at end-of-input, just
                    // reached early. Marking it resolved here is what lets a
                    // later-ranked `Parked` PID (blocked behind this one)
                    // proceed on the next loop iteration.
                    stream.track = Some(TrackState::Abandoned);
                    self.resolved.insert(next_pid);
                }
            }
        }
        self.maybe_signal_tracks_resolved();
    }

    /// Emit [`DemuxEvent::TracksResolved`] (issue #624) when every currently
    /// known PID (`codec_order` + `data_order`) has resolved to `Live` — i.e.
    /// [`try_promote_ready`](Self::try_promote_ready) just ran to a fixed
    /// point with no PID left `Probing` — and [`Self::generation`] differs
    /// from the generation the signal last fired at (de-dup: a PMT
    /// re-processed with no applied change, or plain sample traffic on an
    /// already-fully-resolved stream, must not re-fire the event every time
    /// this is called).
    ///
    /// De-duping on `generation` rather than the known-PID count (issue
    /// #774) fixes a real bug the count-keyed version had: once a track is
    /// removable, the count can return to a previously-seen value (a removal
    /// immediately followed by an addition), which a count-keyed de-dup would
    /// wrongly treat as "already signalled" and never re-fire for.
    fn maybe_signal_tracks_resolved(&mut self) {
        let known = self.codec_order.len() + self.data_order.len();
        if known == 0 {
            return;
        }
        if self.resolved.len() == known
            && self.tracks_resolved_signalled_at != Some(self.generation)
        {
            self.tracks_resolved_signalled_at = Some(self.generation);
            self.events.push_back(DemuxEvent::TracksResolved {
                generation: self.generation,
            });
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
        for (&pid, stream) in self.streams.iter_mut() {
            // Only a PES assembler has a trailing partial payload to flush; a
            // trailing partial (incomplete) section is genuinely undecodable
            // and is simply dropped by `SectionReassembler` itself.
            let completed = match &mut stream.carrier {
                Carrier::Pes(assembler) => assembler.flush(),
                Carrier::Section(_) => None,
            };
            if let Some(completed) = completed {
                // The same two-part intactness rule the mid-stream path uses
                // (r04-W49): the CC mark, *and* — for a bounded PES — the
                // declared length actually having arrived. A trailing PES that
                // stopped short of what its own header declared is truncated,
                // and flushing it at end of input would deliver that
                // truncation as if it were complete.
                let short_of_declared = stream.pes_declared_len > 0
                    && completed.len() < PES_LENGTH_FIELD_END + stream.pes_declared_len as usize;
                if stream.current_pes_intact && !short_of_declared {
                    on_completed_pes(stream, pid, &completed, &mut self.events);
                }
            }
        }
        self.try_promote_ready();

        while let Some(&next_pid) = self
            .codec_order
            .iter()
            .chain(self.data_order.iter())
            .find(|p| !self.resolved.contains(p))
        {
            match self.streams.get(&next_pid).and_then(|s| s.track.as_ref()) {
                Some(TrackState::Probing { .. }) => {
                    self.resolved.insert(next_pid);
                    // This PID's codec config never became recoverable before
                    // end of input — `TrackAdded` never fired for it, so no
                    // `track_id` exists to report (issue #774).
                    self.events.push_back(DemuxEvent::TrackAbandoned {
                        track_id: None,
                        reason: AbandonReason::ConfigUnrecoverable,
                        provenance: EventProvenance {
                            pid: Some(next_pid),
                            packet_index: None,
                        },
                    });
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
                    }
                    | LiveKind::MpegH {
                        pending,
                        last_duration,
                    } => {
                        flush_one_behind(pending, *last_duration, live.track_id, &mut self.events);
                    }
                    LiveKind::Audio { .. } => {}
                    LiveKind::Section => {}
                }
            }
        }
    }
}

/// [`Stage`] adoption (media plane step 2e): a thin, honest delegation to the
/// inherent [`feed`](StreamingTsDemux::feed)/[`poll_event`
/// ](StreamingTsDemux::poll_event)/[`finish`](StreamingTsDemux::finish) —
/// every existing inherent method keeps working unchanged; this trait impl is
/// an additional, uniform way to drive the same engine, not a replacement.
///
/// `StreamingTsDemux` never needs deadline-driven work: it only ever produces
/// output in reaction to `feed`/`finish`, so `next_deadline` is always `None`
/// and `on_deadline` is a no-op.
impl Stage for StreamingTsDemux {
    type In<'a> = &'a [u8];
    type Out = DemuxEvent;
    /// `feed`/`finish` are infallible here — TS resynchronises on `0x47`
    /// rather than erroring on malformed input (see the type's own docs).
    type Error = core::convert::Infallible;

    fn feed(&mut self, input: &[u8], _now: Timestamp) -> core::result::Result<(), Self::Error> {
        self.feed(input);
        Ok(())
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.poll_event()
    }

    fn finish(&mut self) -> core::result::Result<(), Self::Error> {
        self.finish();
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    /// Honest against the one bound this demuxer actually enforces
    /// end-to-end: `MAX_UNATTRIBUTED_BYTES` (the never-claimed-PID replay
    /// buffer). Both this buffer and the per-PID PES overflow
    /// (`MAX_PES_BUFFER_BYTES`, see `feed_pes_bounded`) self-correct
    /// (evict/reset) *within* the same `feed` call that trips them, so
    /// `unattributed_bytes` is never observed sitting exactly at or over the
    /// cap once `feed` returns — only, in the flooding steady state, within
    /// one TS packet's payload of it. Reporting `saturated` only once that
    /// exact byte count is reached would therefore be true in name only
    /// (unreachable in practice — see this step's report); this instead
    /// predicts it one packet ahead: `saturated` once the worst case (another
    /// full-size payload) would push the buffer past the cap, which the
    /// eviction dynamics above make an always-reachable, real signal in
    /// sustained-flood conditions, not a fabricated one.
    fn demand(&self) -> Demand {
        if self.unattributed_bytes.saturating_add(TS_MAX_PAYLOAD_BYTES) > MAX_UNATTRIBUTED_BYTES {
            Demand::saturated()
        } else {
            Demand::new(TS_PACKET_SIZE)
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
                DemuxEvent::TrackAdded(spec) => {
                    index_by_id.insert(spec.track_id, tracks.len());
                    tracks.push(Track::new(spec, Vec::new()));
                }
                DemuxEvent::TrackUpdated(spec) => {
                    // Whole-buffer batch demux: reflect the final PMT-derived
                    // metadata (issue #774) — samples already collected under
                    // the earlier spec are untouched, only the spec itself
                    // (e.g. corrected descriptors) is refreshed.
                    if let Some(&i) = index_by_id.get(&spec.track_id) {
                        tracks[i].spec = spec;
                    }
                }
                // A one-shot whole-buffer `Media` has no removal/abandonment
                // shape (its `tracks` is a flat, final list) — a track that
                // was removed or abandoned mid-file simply keeps whatever it
                // had already collected, exactly like `Discontinuity` below.
                DemuxEvent::TrackRemoved { .. } => {}
                DemuxEvent::TrackAbandoned { .. } => {}
                DemuxEvent::Sample { track_id, sample }
                    if let Some(&i) = index_by_id.get(&track_id) =>
                {
                    let track = &mut tracks[i];
                    // `Track::start_decode_time` is no longer carried by
                    // `TrackAdded` (issue #774 reshape) — it is exactly
                    // the first sample's own `dts` (media plane step 2c
                    // invariant, unconditionally true for every track
                    // kind), so derive it here instead.
                    if track.samples.is_empty()
                        && let Some(dts) = sample.dts
                    {
                        track.start_decode_time = dts as u64;
                    }
                    track.samples.push(sample);
                }
                DemuxEvent::Sample { .. } => {}
                DemuxEvent::ClockReference {
                    ticks,
                    discontinuous,
                    provenance,
                    ..
                } => pcr.push(PcrSample {
                    pcr_27mhz: ticks,
                    // This batch wrapper only ever demuxes TS, whose
                    // ClockReference always carries a PID/packet_index — the
                    // fallbacks are unreachable in practice, not a silent
                    // downgrade.
                    pid: provenance.pid.unwrap_or(0),
                    packet_index: provenance.packet_index.unwrap_or(0),
                    discontinuity: discontinuous,
                }),
                DemuxEvent::Discontinuity { .. } => {}
                DemuxEvent::InputDegraded { .. } => {}
                DemuxEvent::TracksResolved { .. } => {}
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
    use broadcast_common::Parse;

    /// Every section-carried `stream_type` (Table 2-34) classifies as
    /// [`DataCarriage::Sections`]; the historical PES-carried 0x06/0x15 and
    /// any unrecognised `stream_type` classify as [`DataCarriage::Pes`]
    /// (issue #576).
    #[test]
    fn data_carriage_classifies_every_known_stream_type() {
        assert_eq!(data_carriage(0x06), DataCarriage::Pes, "PES private data");
        assert_eq!(data_carriage(0x15), DataCarriage::Pes, "metadata in PES");
        assert_eq!(data_carriage(0x7F), DataCarriage::Pes, "unrecognised → PES");

        for &st in &[
            STREAM_TYPE_PRIVATE_SECTIONS,
            STREAM_TYPE_DSMCC_TYPE_A,
            STREAM_TYPE_DSMCC_TYPE_B,
            STREAM_TYPE_DSMCC_TYPE_C,
            STREAM_TYPE_DSMCC_TYPE_D,
            STREAM_TYPE_DSMCC_SYNC_DOWNLOAD,
            STREAM_TYPE_SCTE35,
        ] {
            assert_eq!(
                data_carriage(st),
                DataCarriage::Sections,
                "stream_type {st:#04X} must be section-carried"
            );
        }
    }

    /// Any `stream_type` not mapped to a decoded codec becomes opaque
    /// [`Codec::Data`] (never `None`/dropped — issue #576).
    #[test]
    fn from_stream_type_unknown_becomes_opaque_data() {
        assert_eq!(Codec::from_stream_type(STREAM_TYPE_AVC), Codec::H264);
        assert_eq!(Codec::from_stream_type(0x7F), Codec::Data(0x7F));
        assert_eq!(
            Codec::from_stream_type(STREAM_TYPE_SCTE35),
            Codec::Data(STREAM_TYPE_SCTE35)
        );
    }

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

    /// A PID whose PES never completes (`payload_unit_start` never recurs —
    /// a wedged encoder, a lossy capture, or a hostile stream) must not grow
    /// its `Carrier::Pes` buffer without bound: [`MAX_PES_BUFFER_BYTES`] must
    /// trip, dropping the partial PES and raising a
    /// [`DemuxEvent::Discontinuity`], and the PID must keep working normally
    /// afterward (resync proof) rather than being wedged or OOMing.
    #[test]
    fn runaway_pes_without_payload_unit_start_is_bounded_not_unbounded() {
        use crate::TsMux;
        use crate::media::{Media, Track};
        use crate::pipeline::{CodecConfig, Sample, TrackSpec};
        use crate::rtp_sdp::avc_config_from_sprop;
        use broadcast_common::Package;

        // A tiny real single-video-track TS (PAT + PMT + a few PES access
        // units), muxed by this crate's own `TsMux` — registers one H.264 ES
        // (`Carrier::Pes`) at the single-track convention PID (`ES_PID_BASE`
        // in `ts_mux.rs`, `0x0100`).
        const ES_PID: u16 = 0x0100;
        let avc = avc_config_from_sprop("Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==").unwrap();
        let spec = TrackSpec::new(
            1,
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config: avc,
                width: 0,
                height: 0,
            },
        );
        let frame_dur = VIDEO_TIMESCALE / 30;
        let samples: Vec<Sample> = (0..3u32)
            .map(|i| {
                let nal = [0x65u8, 0xAA, i as u8];
                let mut data = (nal.len() as u32).to_be_bytes().to_vec();
                data.extend_from_slice(&nal);
                let dts = i64::from(i) * i64::from(frame_dur);
                Sample::new(data, Some(dts), Some(dts), Some(frame_dur), i == 0)
            })
            .collect();
        let track = Track::new(spec, samples);
        let media = Media::new(vec![track], VIDEO_TIMESCALE);
        let ts_bytes = TsMux::default().package(&media).expect("mux to TS");

        let mut demux = StreamingTsDemux::new();
        demux.feed(&ts_bytes);
        // Drain the legitimate startup events (TrackAdded/Sample/…) — not
        // under test here, just clearing the queue for a clean assertion
        // below.
        while demux.poll_event().is_some() {}
        assert!(
            demux.streams.contains_key(&ES_PID),
            "expected the muxed video ES at PID {ES_PID:#06X} — TsMux's ES_PID_BASE convention"
        );

        // Now flood that PID with continuation-only packets (pusi = 0): the
        // PES never completes, exactly the audit-ingest scenario.
        // Each continuation packet contributes `PACKET_PAYLOAD_LEN` (184)
        // bytes; comfortably more than `MAX_PES_BUFFER_BYTES / 184` packets
        // are fed so the cap must trip well before the loop ends.
        let mut cc: u8 = 0;
        let mut hit_cap = false;
        for _ in 0..30_000u32 {
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            pkt[0] = 0x47;
            pkt[1] = ((ES_PID >> 8) as u8) & PID_HI_MASK; // pusi = 0
            pkt[2] = (ES_PID & 0xFF) as u8;
            pkt[3] = 0x10 | (cc & 0x0F);
            cc = cc.wrapping_add(1);
            demux.feed(&pkt);
            if matches!(
                demux.poll_event(),
                Some(DemuxEvent::Discontinuity { provenance, .. }) if provenance.pid == Some(ES_PID)
            ) {
                hit_cap = true;
                break;
            }
        }
        assert!(
            hit_cap,
            "expected MAX_PES_BUFFER_BYTES to trip well within 30000 continuation \
             packets (never grow unbounded)"
        );
        assert_eq!(
            demux.streams.get(&ES_PID).unwrap().pes_bytes,
            0,
            "pes_bytes must reset to 0 once the cap trips"
        );

        // Resync proof: a fresh payload_unit_start after the overflow is
        // accepted normally (the assembler was reset, not wedged).
        const PUSI_BIT: u8 = 0x40; // payload_unit_start_indicator (ISO/IEC 13818-1 §2.4.3.2)
        let mut pkt = [0xFFu8; TS_PACKET_SIZE];
        pkt[0] = 0x47;
        pkt[1] = PUSI_BIT | (((ES_PID >> 8) as u8) & PID_HI_MASK);
        pkt[2] = (ES_PID & 0xFF) as u8;
        pkt[3] = 0x10 | (cc & 0x0F);
        pkt[4] = 0; // pointer/no-op first byte for a PES (not a pointer_field — PES has none)
        demux.feed(&pkt);
        assert_eq!(
            demux.streams.get(&ES_PID).unwrap().pes_bytes,
            PACKET_PAYLOAD_LEN,
            "a fresh payload_unit_start must be accepted and start a new count"
        );
    }

    /// The B8 attack (media plane step 2 fix wave 3): a PMT declares two
    /// PIDs — PID A (rank 0, H.264) whose parameter sets never arrive (a
    /// broken encoder, not malice: every sample here is deliberately
    /// non-sync so `TsMux` never injects SPS/PPS in-band), and PID B (rank
    /// 1, opaque data) whose config resolves on its very first access unit.
    /// PID A's `ConfigProbe` never resolves, so it stays `Probing` forever;
    /// before this fix its `backlog` grew without bound, and — because
    /// `try_promote_ready` `break`s at the first still-`Probing` PID — PID
    /// B's `Parked` backlog grew as collateral for exactly as long. Both
    /// PIDs' own `backlog_bytes` must stay capped at
    /// `MAX_PROBE_BACKLOG_BYTES` regardless of which path (its own overflow,
    /// or being unblocked once the other is abandoned) it actually takes.
    #[test]
    fn probe_backlog_is_bounded_for_both_the_never_resolving_pid_and_its_collateral_pid() {
        use crate::TsMux;
        use crate::media::{Media, Track};
        use crate::pipeline::{CodecConfig, DataCarriage, Sample, TrackSpec};
        use crate::rtp_sdp::avc_config_from_sprop;
        use broadcast_common::Package;

        // Comfortably more than MAX_PROBE_BACKLOG_BYTES per track (~4.9 MiB).
        const SAMPLE_BYTES: usize = 4096;
        const SAMPLE_COUNT: u32 = 1200;
        let frame_dur = VIDEO_TIMESCALE / 30;

        // PID A (rank 0, ES_PID_BASE = 0x0100 in `ts_mux.rs`): H.264, never
        // carries SPS/PPS — every sample is deliberately non-sync, so
        // `build_annexb_au` never injects the parameter sets it otherwise
        // would on a keyframe. This probe can never resolve.
        let avc = avc_config_from_sprop("Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==").unwrap();
        let video_spec = TrackSpec::new(
            1,
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config: avc,
                width: 0,
                height: 0,
            },
        );
        let video_samples: Vec<Sample> = (0..SAMPLE_COUNT)
            .map(|i| {
                let mut nal = alloc::vec![0x41u8]; // nal_unit_type = 1 (non-IDR slice)
                nal.resize(SAMPLE_BYTES, 0xAA);
                let mut data = (nal.len() as u32).to_be_bytes().to_vec();
                data.extend_from_slice(&nal);
                let dts = i64::from(i) * i64::from(frame_dur);
                Sample::new(data, Some(dts), Some(dts), Some(frame_dur), false)
            })
            .collect();
        let video_track = Track::new(video_spec, video_samples);

        // PID B (rank 1): opaque data — `ConfigProbe::Data` resolves on its
        // very first access unit (already fully known from the PMT alone),
        // so it goes straight to `Parked` and stays there for as long as PID
        // A blocks it.
        let data_spec = TrackSpec::new(
            2,
            VIDEO_TIMESCALE,
            CodecConfig::Data {
                stream_type: 0x7F,
                descriptors: Vec::new(),
                carriage: DataCarriage::Pes,
            },
        );
        let data_samples: Vec<Sample> = (0..SAMPLE_COUNT)
            .map(|i| {
                let payload = alloc::vec![0xBBu8; SAMPLE_BYTES];
                let dts = i64::from(i) * i64::from(frame_dur);
                Sample::new(payload, Some(dts), Some(dts), Some(frame_dur), true)
            })
            .collect();
        let data_track = Track::new(data_spec, data_samples);

        let media = Media::new(vec![video_track, data_track], VIDEO_TIMESCALE);
        let ts_bytes = TsMux::default().package(&media).expect("mux to TS");

        let mut demux = StreamingTsDemux::new();
        demux.feed(&ts_bytes);
        let mut abandoned_pids: Vec<u16> = Vec::new();
        while let Some(ev) = demux.poll_event() {
            if let DemuxEvent::TrackAbandoned {
                reason: AbandonReason::BudgetExceeded,
                provenance,
                ..
            } = ev
                && let Some(pid) = provenance.pid
            {
                abandoned_pids.push(pid);
            }
        }
        assert!(
            !abandoned_pids.is_empty(),
            "expected at least one TrackAbandoned{{BudgetExceeded}} from an abandoned probe backlog \
             (issue #774: this replaced the mis-typed Discontinuity this path used to emit)"
        );

        // issue #774: this path used to emit a mis-typed `Discontinuity` for
        // exactly this condition — re-feed and confirm none appears anymore.
        let mut demux2 = StreamingTsDemux::new();
        demux2.feed(&ts_bytes);
        let saw_discontinuity_for_abandoned_pid = std::iter::from_fn(|| demux2.poll_event())
            .any(|ev| matches!(ev, DemuxEvent::Discontinuity { provenance, .. } if provenance.pid == Some(PID_A)));
        assert!(
            !saw_discontinuity_for_abandoned_pid,
            "a probe-backlog-budget abandonment (issue #774) must be a TrackAbandoned, \
             never a Discontinuity"
        );

        // The invariant this fix establishes: neither PID's own tracked
        // backlog byte total ever exceeded the cap.
        for (&pid, stream) in demux.streams.iter() {
            assert!(
                stream.backlog_bytes <= MAX_PROBE_BACKLOG_BYTES,
                "PID {pid:#06X} backlog_bytes {} exceeded cap {MAX_PROBE_BACKLOG_BYTES}",
                stream.backlog_bytes
            );
        }

        // PID A specifically must never have resolved — its parameter sets
        // never arrived, so it must be Abandoned, not Live.
        const PID_A: u16 = 0x0100; // ES_PID_BASE in `ts_mux.rs`
        let abandoned = matches!(
            demux.streams.get(&PID_A).and_then(|s| s.track.as_ref()),
            Some(TrackState::Abandoned)
        );
        assert!(abandoned, "PID A must be Abandoned, never Live");

        // PID B (rank 1, ES_PID_BASE + 1) must have made progress — either
        // promoted to Live once PID A was abandoned, or itself abandoned —
        // never left permanently wedged in Probing/Parked with an
        // ever-growing backlog.
        const PID_B: u16 = 0x0101;
        let pid_b_resolved = matches!(
            demux.streams.get(&PID_B).and_then(|s| s.track.as_ref()),
            Some(TrackState::Live(_)) | Some(TrackState::Abandoned)
        );
        assert!(
            pid_b_resolved,
            "PID B must reach a final disposition (Live or Abandoned), not stay wedged"
        );
    }

    /// `TrackAbandoned { reason: AbandonReason::ConfigUnrecoverable, .. }`
    /// (issue #774): a PMT-listed H.264 PID whose SPS/PPS never arrive stays
    /// `Probing` for the life of the input — well under
    /// `MAX_PROBE_BACKLOG_BYTES` here (the B8 byte-cap path is a *different*
    /// abandonment reason, covered above), so it only reaches a final
    /// disposition once `finish()` concludes end-of-input that the config
    /// will never resolve. No `track_id` was ever assigned (`TrackAdded`
    /// never fired), so `track_id` must be `None`.
    #[test]
    fn track_abandoned_config_unrecoverable_fires_at_finish() {
        use crate::TsMux;
        use crate::media::{Media, Track};
        use crate::pipeline::{CodecConfig, Sample, TrackSpec};
        use crate::rtp_sdp::avc_config_from_sprop;
        use broadcast_common::Package;

        const PID_A: u16 = 0x0100; // ES_PID_BASE in `ts_mux.rs`
        let frame_dur = VIDEO_TIMESCALE / 30;
        let avc = avc_config_from_sprop("Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==").unwrap();
        let video_spec = TrackSpec::new(
            1,
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config: avc,
                width: 0,
                height: 0,
            },
        );
        // A handful of small, deliberately non-sync access units — never
        // enough to trip MAX_PROBE_BACKLOG_BYTES, so the only way this PID
        // ever reaches a final disposition is `finish()`'s end-of-input
        // conclusion.
        let video_samples: Vec<Sample> = (0..5u32)
            .map(|i| {
                let nal = alloc::vec![0x41u8, 0xAA, 0xBB]; // non-IDR slice, no SPS/PPS
                let mut data = (nal.len() as u32).to_be_bytes().to_vec();
                data.extend_from_slice(&nal);
                let dts = i64::from(i) * i64::from(frame_dur);
                Sample::new(data, Some(dts), Some(dts), Some(frame_dur), false)
            })
            .collect();
        let video_track = Track::new(video_spec, video_samples);
        let media = Media::new(vec![video_track], VIDEO_TIMESCALE);
        let ts_bytes = TsMux::default().package(&media).expect("mux to TS");

        let mut demux = StreamingTsDemux::new();
        demux.feed(&ts_bytes);
        assert!(
            !matches!(
                demux.streams.get(&PID_A).and_then(|s| s.track.as_ref()),
                Some(TrackState::Abandoned)
            ),
            "sanity: PID A must still be Probing before finish() — the byte cap must not \
             have tripped (this test is about the end-of-input path, not the budget one)"
        );
        while demux.poll_event().is_some() {}

        demux.finish();
        let mut saw_config_unrecoverable = false;
        while let Some(ev) = demux.poll_event() {
            if let DemuxEvent::TrackAbandoned {
                track_id,
                reason: AbandonReason::ConfigUnrecoverable,
                provenance,
            } = ev
            {
                assert_eq!(
                    track_id, None,
                    "a track abandoned before ever resolving has no track_id to report"
                );
                assert_eq!(provenance.pid, Some(PID_A));
                saw_config_unrecoverable = true;
            }
        }
        assert!(
            saw_config_unrecoverable,
            "expected TrackAbandoned{{ConfigUnrecoverable}} once finish() concludes PID A's \
             config will never resolve"
        );
    }

    /// Bytes of long-form section header between the 3-byte
    /// `table_id`/`section_length` prefix and the table body:
    /// `table_id_extension`(2) + `version`(1) + `section_number`(1) +
    /// `last_section_number`(1) -- ISO/IEC 13818-1 §2.4.4.1.
    const SECTION_BODY_PREFIX_LEN: usize = 5;

    /// PES `stream_id` for an H.264 video elementary stream (ISO/IEC
    /// 13818-1 Table 2-22).
    const ES_STREAM_ID_VIDEO: u8 = 0xE0;

    /// Wrap `au` in a PES packet that carries **no** timing at all:
    /// `PTS_DTS_flags == '00'` and an empty `PES_header_data_length`
    /// (ISO/IEC 13818-1 §2.4.3.7).
    fn pes_packet_untimed(stream_id: u8, au: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[0x00, 0x00, 0x01, stream_id]);
        let payload_len = au.len() + 3;
        out.extend_from_slice(&(payload_len as u16).to_be_bytes());
        out.push(0x80); // '10' marker, not scrambled
        out.push(0x00); // PTS_DTS_flags = '00'
        out.push(0x00); // PES_header_data_length = 0
        out.extend_from_slice(au);
        out
    }

    /// `stream_id` for `private_stream_1` (ISO/IEC 13818-1 Table 2-22) —
    /// what a `stream_type` 0x06 PES uses.
    const ES_STREAM_ID_PRIVATE_DATA: u8 = 0xBD;

    /// One 300-byte access unit payload (fits in two TS packets).
    fn data_au(seed: usize) -> Vec<u8> {
        (0..300).map(|i| ((i + seed) & 0xFF) as u8).collect()
    }

    /// Access unit A for the CC-gap test.
    fn long_data_au() -> Vec<u8> {
        data_au(0)
    }

    /// Access unit B for the CC-gap test — different bytes, so a delivered
    /// sample is unambiguously one or the other.
    fn long_data_au_alt() -> Vec<u8> {
        data_au(0x55)
    }

    /// A PES carrying `au` with **no** timing and `PES_packet_length == 0`
    /// (unbounded — the ordinary video form, ISO/IEC 13818-1 §2.4.3.7).
    fn unbounded_pes(stream_id: u8, au: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[0x00, 0x00, 0x01, stream_id]);
        out.extend_from_slice(&0u16.to_be_bytes()); // PES_packet_length = 0
        out.push(0x80); // '10' marker, not scrambled
        out.push(0x00); // PTS_DTS_flags = '00'
        out.push(0x00); // PES_header_data_length = 0
        out.extend_from_slice(au);
        out
    }

    /// A PES carrying `au` with no timing at all (`PTS_DTS_flags == '00'`).
    fn untimed_pes(stream_id: u8, au: &[u8]) -> Vec<u8> {
        pes_packet_untimed(stream_id, au)
    }

    /// PES `stream_id` for a private_stream_1-carried audio elementary
    /// stream (ISO/IEC 13818-1 Table 2-22) — the demux only requires a
    /// well-formed PES header, not a particular stream_id.
    const ES_STREAM_ID_AUDIO: u8 = 0xBD;

    /// Encode a 33-bit PTS/DTS field (ISO/IEC 13818-1 §2.4.3.7) with the
    /// given 4-bit prefix (`0010` for PTS-only, `0011` for PTS in a
    /// both-present pair, `0001` for DTS).
    fn encode_ts_field(prefix: u8, value: i64) -> [u8; 5] {
        let v = (value as u64) & TS_WRAP_MASK;
        [
            (prefix << 4) | ((((v >> 30) & 0x07) as u8) << 1) | 0x01,
            ((v >> 22) & 0xFF) as u8,
            ((((v >> 15) & 0x7F) as u8) << 1) | 0x01,
            ((v >> 7) & 0xFF) as u8,
            (((v & 0x7F) as u8) << 1) | 0x01,
        ]
    }

    /// Wrap `au` in a PES packet with a PTS, and a DTS too when `dts` is
    /// `Some` (`PTS_DTS_flags` `10` / `11` — ISO/IEC 13818-1 §2.4.3.7).
    fn pes_packet(stream_id: u8, pts: i64, dts: Option<i64>, au: &[u8]) -> Vec<u8> {
        let header_data_len = if dts.is_some() { 10 } else { 5 };
        let mut out = Vec::new();
        out.extend_from_slice(&[0x00, 0x00, 0x01, stream_id]);
        let payload_len = au.len() + 3 + header_data_len;
        out.extend_from_slice(&(payload_len as u16).to_be_bytes());
        out.push(0x80); // '10' marker, not scrambled
        out.push(if dts.is_some() { 0xC0 } else { 0x80 });
        out.push(header_data_len as u8);
        if let Some(d) = dts {
            out.extend_from_slice(&encode_ts_field(0b0011, pts));
            out.extend_from_slice(&encode_ts_field(0b0001, d));
        } else {
            out.extend_from_slice(&encode_ts_field(0b0010, pts));
        }
        out.extend_from_slice(au);
        out
    }

    /// PID/table constants for [`probe_sync_scan_work_is_linear_in_the_input_not_quadratic`].
    const PAT_PID_UNDER_TEST: u16 = 0x0000;
    const PMT_PID_UNDER_TEST: u16 = 0x1000;
    const ES_PID_UNDER_TEST: u16 = 0x1100;
    /// PMT `program_number` for the one-program PAT below.
    const PROGRAM_NUMBER_UNDER_TEST: u16 = 1;

    /// PAT section body (ISO/IEC 13818-1 Table 2-30): one program. Returned
    /// without the 8-byte section header and without the CRC, which
    /// [`psi_section_packets`] adds.
    fn pat_body() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&PROGRAM_NUMBER_UNDER_TEST.to_be_bytes());
        body.extend_from_slice(&(0xE000 | PMT_PID_UNDER_TEST).to_be_bytes());
        body
    }

    /// PMT section body (ISO/IEC 13818-1 Table 2-33): one elementary stream.
    fn pmt_body(es_pid: u16, stream_type: u8) -> Vec<u8> {
        pmt_body_with_pcr_pid(es_pid, stream_type, 0)
    }

    /// A PMT body with **two** elementary streams, declaring `pcr_pid` as the
    /// program's `PCR_PID` (§2.4.4.8 Table 2-33) — the A/V program shape the
    /// multi-PID discontinuity tests need.
    fn pmt_body_two_es_with_pcr_pid(
        video_pid: u16,
        video_type: u8,
        audio_pid: u16,
        audio_type: u8,
        pcr_pid: u16,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&(0xE000 | pcr_pid).to_be_bytes());
        body.extend_from_slice(&0xF000u16.to_be_bytes()); // program_info_length 0
        for (pid, stream_type) in [(video_pid, video_type), (audio_pid, audio_type)] {
            body.push(stream_type);
            body.extend_from_slice(&(0xE000 | pid).to_be_bytes());
            body.extend_from_slice(&0xF000u16.to_be_bytes()); // ES_info_length 0
        }
        body
    }

    /// A PMT body declaring `pcr_pid` as the program's `PCR_PID`
    /// (§2.4.4.8 Table 2-33) — what a real multiplexer sets to the video PID
    /// (or a dedicated PCR PID).
    fn pmt_body_with_pcr_pid(es_pid: u16, stream_type: u8, pcr_pid: u16) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&(0xE000 | pcr_pid).to_be_bytes());
        body.extend_from_slice(&0xF000u16.to_be_bytes()); // reserved + program_info_length 0
        body.push(stream_type);
        body.extend_from_slice(&(0xE000 | es_pid).to_be_bytes());
        body.extend_from_slice(&0xF000u16.to_be_bytes()); // reserved + ES_info_length 0
        body
    }

    /// Wrap `body` in a long-form PSI section (header + CRC_32) and packetise
    /// it onto `pid` with `pusi` set on the first packet. The CRC_32 is
    /// computed by the workspace's own `CRC-32/MPEG-2` so the section passes
    /// [`psi_section_crc_ok`].
    fn psi_section_packets(pid: u16, cc0: u8, table_id: u8, body: &[u8]) -> Vec<u8> {
        psi_section_packets_for(pid, cc0, table_id, PROGRAM_NUMBER_UNDER_TEST, body)
    }

    /// As [`psi_section_packets`], but with an explicit `table_id_extension`
    /// — the `program_number` for a PAT's entries and for a PMT's own header —
    /// so a multi-program multiplex can be built.
    fn psi_section_packets_for(
        pid: u16,
        cc0: u8,
        table_id: u8,
        table_id_extension: u16,
        body: &[u8],
    ) -> Vec<u8> {
        let mut section = Vec::new();
        let section_length = (SECTION_BODY_PREFIX_LEN + body.len() + CRC32_LEN) as u16;
        section.push(table_id);
        section.push(SECTION_SYNTAX_INDICATOR_BIT | (section_length >> 8) as u8);
        section.push(section_length as u8);
        // table_id_extension / version / current_next / section numbers.
        section.extend_from_slice(&table_id_extension.to_be_bytes());
        section.push(0xC1); // reserved '11', version 0, current_next 1
        section.push(0x00); // section_number
        section.push(0x00); // last_section_number
        section.extend_from_slice(body);
        let crc = broadcast_common::crc32_mpeg2::compute(&section);
        section.extend_from_slice(&crc.to_be_bytes());
        // A section starts with a `pointer_field` (§2.4.3.4 Table 2-6: the
        // number of bytes before the first section start in this packet's
        // payload) — here 0, i.e. the section begins immediately.
        let mut packetised = Vec::with_capacity(section.len() + 1);
        packetised.push(0x00);
        packetised.extend_from_slice(&section);
        pes_packets(pid, cc0, &packetised)
    }

    /// Packetise `payload` like [`pes_packets`], but with an 8-byte adaptation
    /// field whose `discontinuity_indicator` is set on the **first** packet
    /// (§2.4.3.4 Table 2-6). Used to build the "source says the stream broke
    /// here" packet a real multiplexer emits.
    fn pes_packets_with_discontinuity(pid: u16, cc0: u8, payload: &[u8]) -> Vec<u8> {
        /// Adaptation-field bytes *after* the length byte: the flags byte plus
        /// stuffing.
        const AF_LEN: usize = 8;
        /// First payload byte: 4-byte TS header + the length byte + the field.
        const PAYLOAD_START: usize = 4 + 1 + AF_LEN;
        let first_take = (TS_PACKET_SIZE - PAYLOAD_START).min(payload.len());
        let mut out = Vec::new();
        let mut p = [0xFFu8; TS_PACKET_SIZE];
        p[0] = 0x47;
        p[1] = ((pid >> 8) as u8 & PID_HI_MASK) | 0x40; // pusi
        p[2] = (pid & 0xFF) as u8;
        p[3] = 0x30 | (cc0 & 0x0F); // afc = '11'
        p[4] = AF_LEN as u8;
        p[5] = 0x80; // discontinuity_indicator = 1
        p[6..PAYLOAD_START].fill(0xFF); // stuffing
        p[PAYLOAD_START..PAYLOAD_START + first_take].copy_from_slice(&payload[..first_take]);
        out.extend_from_slice(&p);
        // The remainder is a *continuation*, never a new unit start.
        out.extend_from_slice(&continuation_packets(
            pid,
            cc0.wrapping_add(1),
            &payload[first_take..],
        ));
        out
    }

    /// Packetise `payload` as **continuation** packets (`pusi` clear),
    /// `0xFF`-stuffed, counters incrementing from `cc0`.
    fn continuation_packets(pid: u16, cc0: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut off = 0usize;
        let mut cc = cc0;
        while off < payload.len() {
            let take = (payload.len() - off).min(TS_PACKET_SIZE - 4);
            let mut p = [0xFFu8; TS_PACKET_SIZE];
            p[0] = 0x47;
            p[1] = (pid >> 8) as u8 & PID_HI_MASK;
            p[2] = (pid & 0xFF) as u8;
            p[3] = 0x10 | (cc & 0x0F);
            p[4..4 + take].copy_from_slice(&payload[off..off + take]);
            out.extend_from_slice(&p);
            off += take;
            cc = cc.wrapping_add(1);
        }
        out
    }

    /// Packetise `payload` onto `pid`, `payload_unit_start` set on the first
    /// 188-byte packet only, CC incrementing from `cc0`, `0xFF` stuff.
    fn pes_packets(pid: u16, cc0: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut off = 0usize;
        let mut first = true;
        let mut cc = cc0;
        while off < payload.len() {
            let take = (payload.len() - off).min(TS_PACKET_SIZE - 4);
            let mut p = [0xFFu8; TS_PACKET_SIZE];
            p[0] = 0x47;
            p[1] = ((pid >> 8) as u8 & PID_HI_MASK) | if first { 0x40 } else { 0x00 };
            p[2] = (pid & 0xFF) as u8;
            p[3] = 0x10 | (cc & 0x0F);
            p[4..4 + take].copy_from_slice(&payload[off..off + take]);
            out.extend_from_slice(&p);
            off += take;
            first = false;
            cc = cc.wrapping_add(1);
        }
        out
    }

    /// r04-W51 review: an AAC track must NOT report a config change merely
    /// because the ADTS *frame length* varies.
    ///
    /// The first attempt compared the raw 7-byte ADTS header as the
    /// "config header", but that header carries the 13-bit
    /// `aac_frame_length` — different on every single frame — so nearly every
    /// AAC access unit raised `TrackUpdated` and told a consumer to rebuild
    /// its init segment thousands of times a stream.
    ///
    /// Real AAC bitstream, twice over. First: `fixtures/ts/aac-5_1-640k.ts`
    /// is a real 5.1 AAC-LC capture whose ADTS frame lengths vary (ffprobe:
    /// 742, 792, 763, 764, 767, 759 ...) and which must yield **zero**
    /// `TrackUpdated`. Second: its elementary stream re-headered with a
    /// different `channel_configuration` (5 instead of 6) and appended to the
    /// same PID, which must yield **exactly one** event carrying the new
    /// Table 1.19 count.
    #[test]
    fn aac_frame_length_variation_is_not_a_config_change() {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("aac-5_1-640k.ts");
        let source =
            std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));

        let count_updates = |bytes: &[u8]| -> (usize, Vec<u16>) {
            let mut demux = StreamingTsDemux::new();
            demux.feed(bytes);
            demux.finish();
            let mut n = 0usize;
            let mut channels = Vec::new();
            while let Some(event) = demux.poll_event() {
                if let DemuxEvent::TrackUpdated(spec) = event {
                    n += 1;
                    if let CodecConfig::Aac { channel_count, .. } = spec.config {
                        channels.push(channel_count);
                    }
                }
            }
            (n, channels)
        };

        let (updates, _) = count_updates(&source);
        assert_eq!(
            updates, 0,
            "a varying ADTS frame_length must not read as a config change; got {updates} TrackUpdated events on a real AAC capture"
        );

        // The fixture's own config, read from its recovered ASC rather than
        // assumed.
        let mut demux = TsDemux::new();
        let media = demux.demux(&source).expect("demux aac fixture");
        let audio = media
            .tracks
            .iter()
            .find(|t| matches!(t.config(), CodecConfig::Aac { .. }))
            .expect("the fixture has an AAC track");
        assert!(!audio.samples.is_empty(), "fixture must carry AAC frames");
        let (sfi, seeded_channels) = match audio.config() {
            CodecConfig::Aac {
                esds,
                channel_count,
                ..
            } => {
                let dsi = esds
                    .es_descriptor
                    .decoder_config
                    .as_ref()
                    .and_then(|dc| dc.decoder_specific_info.as_ref())
                    .expect("AAC esds carries a DecoderSpecificInfo");
                let asc = AudioSpecificConfig::parse(&dsi.data).expect("parse ASC");
                (asc.sampling_frequency_index.raw(), *channel_count)
            }
            _ => unreachable!(),
        };
        /// Table 1.19 configuration 6 is 5.1 (six channels) — what a real
        /// 5.1 capture must report (r04-W51).
        const SEEDED_CHANNELS: u16 = 6;
        assert_eq!(
            seeded_channels, SEEDED_CHANNELS,
            "the fixture is a real 5.1 capture, so its config is Table 1.19 configuration 6"
        );
        /// The rebuilt stream's `channel_configuration`: 5 = five channels,
        /// deliberately different from the fixture's own 5.1.
        const CONFIG_REBUILT: u8 = 5;

        const PES_PAYLOAD_BYTES: usize = 1800;
        let aac_pid = audio
            .spec
            .source_pid
            .expect("a TS-demuxed track carries its source PID");
        let mut input = source.clone();
        let mut cc = 0u8;
        let mut buf: Vec<u8> = Vec::new();
        let flush = |buf: &mut Vec<u8>, cc: &mut u8, input: &mut Vec<u8>| {
            if buf.is_empty() {
                return;
            }
            let pes = pes_packet(ES_STREAM_ID_AUDIO, 0, None, buf);
            let packets = pes_packets(aac_pid, *cc, &pes);
            *cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
            input.extend_from_slice(&packets);
            buf.clear();
        };
        // Whole frames per PES payload, as a real muxer packs them.
        for sample in &audio.samples {
            let frame_len = (ADTS_HEADER_SIZE + sample.data.len()) as u16;
            let hdr = crate::aac_asc::build_adts_header(1, sfi, CONFIG_REBUILT, frame_len);
            if buf.len() + frame_len as usize > PES_PAYLOAD_BYTES {
                flush(&mut buf, &mut cc, &mut input);
            }
            buf.extend_from_slice(&hdr);
            buf.extend_from_slice(&sample.data);
        }
        flush(&mut buf, &mut cc, &mut input);

        let (updates, channels) = count_updates(&input);
        assert_eq!(
            updates, 1,
            "a real channel_configuration change must raise exactly one TrackUpdated, got {updates} (channels: {channels:?})"
        );
        assert_eq!(
            channels,
            vec![5],
            "the event must carry the new Table 1.19 count"
        );
    }

    /// r04-W47: the codec-config probes must scan only the *newest* access
    /// unit, so the byte-probe work a never-resolving PID costs stays
    /// proportional to the input length, not to its square.
    ///
    /// A PID whose config never resolves (here: an AAC PID whose ADTS frames
    /// claim `sampling_frequency_index` 13, which [`sfi_to_hz`] rejects, so
    /// [`finalize_probe`] never returns a config) accumulates every access
    /// unit into its backlog while probing. Before the fix each new access
    /// unit re-walked the whole backlog with [`find_adts_sync`], whose
    /// byte-by-byte sync scan is counted by [`record_sync_probe`] at its own
    /// scan site -- measured, not re-derived in the test. Each access unit
    /// here is one 1400-byte ADTS frame carrying no second syncword, so a
    /// scan of one access unit stops at its very first probe, while the
    /// pre-fix shape re-probed every earlier access unit on every push.
    #[test]
    fn probe_sync_scan_work_is_linear_in_the_input_not_quadratic() {
        use crate::aac_asc::build_adts_header;

        /// `sampling_frequency_index` 13 (reserved) -- `sfi_to_hz` returns
        /// `None`, so the AAC probe can never finalize.
        const REJECTED_SFI: u8 = 13;
        /// Frame body bytes after the 7-byte ADTS header.
        const FRAME_BODY: usize = 1400;
        /// ADTS frames per PES access unit.
        const FRAMES_PER_AU: usize = 3;
        /// Access units fed -- chosen so the backlog stays under
        /// [`MAX_PROBE_BACKLOG_BYTES`] (500 × 3 × 1407 B ≈ 2.1 MiB) while the
        /// quadratic shape's total probe count still grows like
        /// `frames^2` (≈ 2.25 × 10^6), orders past the linear bound.
        const ACCESS_UNITS: usize = 500;

        let frame_len = (ADTS_HEADER_SIZE + FRAME_BODY) as u16;
        let mut frame = alloc::vec![0u8; frame_len as usize];
        frame[..ADTS_HEADER_SIZE].copy_from_slice(&build_adts_header(
            1,
            REJECTED_SFI,
            2,
            frame_len,
        ));
        let mut au = Vec::with_capacity(frame.len() * FRAMES_PER_AU);
        for _ in 0..FRAMES_PER_AU {
            au.extend_from_slice(&frame);
        }

        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_AAC_ADTS),
        ));
        // One PES packet per access unit; CC runs continuously over the
        // whole elementary stream (each `pes_packets` call increments from
        // the value it is given).
        let mut cc = 0u8;
        for i in 0..ACCESS_UNITS {
            let pes = pes_packet(ES_STREAM_ID_AUDIO, i as i64 * 1024, None, &au);
            let packets = pes_packets(ES_PID_UNDER_TEST, cc, &pes);
            cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
            input.extend_from_slice(&packets);
        }

        let before = sync_probes();
        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let probes = sync_probes() - before;

        // The PID must genuinely still be probing when the input ends, so this
        // measures a never-resolving probe rather than a resolved one.
        assert!(
            matches!(
                demux
                    .streams
                    .get(&ES_PID_UNDER_TEST)
                    .and_then(|s| s.track.as_ref()),
                Some(TrackState::Probing { .. })
            ),
            "the reserved-SFI AAC PID must never resolve its config, so it stays Probing"
        );
        // Bound: per access unit, exactly one probe pass and one sync scan —
        // and the scan returns at its very first probe, because the syncword is
        // at offset 0. The total is therefore exactly twice the number of access
        // units however long each one is. Re-derived from the fixture's own
        // shape, not from the demux's internals; a backlog rescan would instead
        // be ACCESS_UNITS^2/2 = 125 000.
        let expected = ACCESS_UNITS as u64 * 2;
        assert_eq!(
            probes, expected,
            "the AAC probe must scan only the newest access unit              ({ACCESS_UNITS} accesses => {expected} probes: one pass + one              sync scan each); {probes} recorded"
        );
    }

    /// The same linearity bound for the two probes that walk the access unit
    /// rather than scanning for a syncword — H.264 (a NAL walk) and MPEG audio
    /// (a header scan counted by [`record_sync_probe`]) — so all three probe
    /// families are covered, not just ADTS (r04-W47 review).
    ///
    /// A never-resolving PID of each kind is built from real elementary-stream
    /// bytes and flood-fed access units. The total *probe passes* must stay
    /// proportional to the number of access units, never their sum of lengths.
    #[test]
    fn video_and_mpeg_audio_probe_work_is_linear_in_the_input() {
        /// Access units fed; comfortably more than one, so a linear bound and a
        /// quadratic one differ by orders of magnitude.
        const ACCESS_UNITS: u32 = 400;
        /// Bytes per access unit — large enough that re-walking an accumulated
        /// backlog would be unmistakable.
        const AU_BYTES: usize = 1024;

        // Real H.264 parameter sets, but deliberately *not* a decodable SPS, so
        // the H.264 probe never finalizes: take a real SPS and truncate it.
        let (sps, pps, _) = real_avc_parameter_sets();
        let broken_sps = &sps[..sps.len().min(3)];

        for (stream_type, au) in [
            // H.264: SPS and PPS present but the SPS is unusable, so
            // `decode_avc_sps` never yields a geometry and the probe stays
            // unresolved forever.
            (
                STREAM_TYPE_AVC,
                annexb_au(&[broken_sps, &pps, &[0x41u8; 8]]),
            ),
            // MPEG-1 audio: bytes that never form a valid frame header (a
            // syncword followed by a reserved bit-rate index), so the probe
            // scans the whole access unit and still never resolves.
            (
                STREAM_TYPE_MPEG1_AUDIO,
                // No syncword anywhere in the access unit.
                alloc::vec![0xE0u8; AU_BYTES],
            ),
        ] {
            let mut au = au;
            au.resize(AU_BYTES, 0x33);

            let mut input = Vec::new();
            for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
                input.extend_from_slice(&null_packet());
            }
            input.extend_from_slice(&psi_section_packets(
                PAT_PID_UNDER_TEST,
                0,
                TABLE_ID_PAT,
                &pat_body(),
            ));
            input.extend_from_slice(&psi_section_packets(
                PMT_PID_UNDER_TEST,
                0,
                TABLE_ID_PMT,
                &pmt_body(ES_PID_UNDER_TEST, stream_type),
            ));
            let mut cc = 0u8;
            for i in 0..ACCESS_UNITS {
                let pes = pes_packet(ES_STREAM_ID_AUDIO, i as i64 * 1024, None, &au);
                let packets = pes_packets(ES_PID_UNDER_TEST, cc, &pes);
                cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
                input.extend_from_slice(&packets);
            }

            let before = sync_probes();
            let mut demux = StreamingTsDemux::new();
            demux.feed(&input);
            demux.finish();
            let probes = sync_probes() - before;

            assert!(
                matches!(
                    demux
                        .streams
                        .get(&ES_PID_UNDER_TEST)
                        .and_then(|s| s.track.as_ref()),
                    Some(TrackState::Probing { .. }) | Some(TrackState::Abandoned)
                ),
                "stream_type {stream_type:#04X} must never resolve its config"
            );
            // Exactly one probe pass per access unit for the H.264 probe,
            // whose NAL walk has no byte-by-byte sync scanner of its own — an
            // exact equality, which is the strongest statement the counter can
            // make and cannot be satisfied by a rescan. The MPEG-audio probe
            // *does* scan, so it costs one pass plus up to one probe per byte
            // of the single access unit it examined; its bound stays an
            // inequality (ACCESS_UNITS² / 2 would be ~80 000).
            if stream_type == STREAM_TYPE_AVC {
                assert_eq!(
                    probes, ACCESS_UNITS as u64,
                    "the H.264 probe must make exactly one pass per access                      unit, never a backlog rescan: {probes} probes over                      {ACCESS_UNITS} access units"
                );
            } else {
                let bound = ACCESS_UNITS as u64 * (1 + AU_BYTES as u64);
                assert!(
                    probes <= bound,
                    "stream_type {stream_type:#04X}: {probes} probes over                      {ACCESS_UNITS} access units of {AU_BYTES} bytes — bound is                      {bound}; a quadratic rescan is ~{}",
                    (ACCESS_UNITS as u64).pow(2) / 2
                );
            }
        }
    }

    // ── r04-W51 review: parameter-set tracking, not a one-AU snapshot ───────

    /// Build a TS carrying one AVC PID whose access units are exactly the
    /// `aus` given, each in its own PES packet, preceded by a PAT/PMT and the
    /// resync bootstrap. Returns the bytes and the PID used.
    fn avc_ts_over(aus: &[Vec<u8>]) -> Vec<u8> {
        let mut cc = 0u8;
        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_AVC),
        ));
        for (i, au) in aus.iter().enumerate() {
            let pes = pes_packet(
                ES_STREAM_ID_VIDEO,
                i as i64 * 3600,
                Some(i as i64 * 3600),
                au,
            );
            let packets = pes_packets(ES_PID_UNDER_TEST, cc, &pes);
            cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
            input.extend_from_slice(&packets);
        }
        input
    }

    /// The `TrackUpdated` width a TS produces, if any.
    fn updated_avc_width(bytes: &[u8]) -> Option<u16> {
        let mut demux = StreamingTsDemux::new();
        demux.feed(bytes);
        demux.finish();
        std::iter::from_fn(|| demux.poll_event()).find_map(|e| match e {
            DemuxEvent::TrackUpdated(spec) => match spec.config {
                CodecConfig::Avc { width, .. } => Some(width),
                _ => None,
            },
            _ => None,
        })
    }

    /// A two-program multiplex where **program A**'s PCR PID signals a
    /// time-base discontinuity. Program B's timeline must be completely
    /// untouched: §2.4.3.5 scopes the indicator to "the associated program"
    /// (the one whose PCR_PID carries it), so a rebase that reached every
    /// stream in the demux would corrupt an unrelated service (r04-W50
    /// review — the first attempt marked every stream across every program).
    #[test]
    fn signalled_discontinuity_is_scoped_to_the_signalling_program() {
        /// Program A: PMT PID, ES PIDs (the first is also its PCR PID),
        /// program_number.
        const A_PMT: u16 = 0x1000;
        const A_ES: u16 = 0x1100;
        /// Program A's second elementary stream (audio).
        const A_ES2: u16 = 0x1101;
        /// Program B: likewise.
        const B_PMT: u16 = 0x1001;
        const B_ES: u16 = 0x1200;
        /// Program numbers for the two PAT entries.
        const A_PROGRAM: u16 = 1;
        const B_PROGRAM: u16 = 2;
        /// Frame period, 90 kHz ticks (25 fps).
        const FRAME_PERIOD: i64 = 3600;
        /// Frames per base, per program.
        const FRAMES: i64 = 8;
        /// Program A's second base — well *below* its first, so it needs a lift.
        const A_BASE_B: i64 = 40_000;
        /// Both programs' first base.
        const BASE_A: i64 = 900_000;

        let annexb = real_annexb_frames();

        let mut cc = 0u8;
        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        // One PAT listing both programs.
        let mut pat = Vec::new();
        for (program, pmt_pid) in [(A_PROGRAM, A_PMT), (B_PROGRAM, B_PMT)] {
            pat.extend_from_slice(&program.to_be_bytes());
            pat.extend_from_slice(&(0xE000 | pmt_pid).to_be_bytes());
        }
        input.extend_from_slice(&psi_section_packets_for(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            0,
            &pat,
        ));
        // Program A carries *two* elementary streams (video + audio) and the
        // video PID is its PCR_PID: both must be rebased by A's discontinuity,
        // while program B's single stream is untouched (r04-W50 review).
        input.extend_from_slice(&psi_section_packets_for(
            A_PMT,
            0,
            TABLE_ID_PMT,
            A_PROGRAM,
            &pmt_body_two_es_with_pcr_pid(A_ES, STREAM_TYPE_AVC, A_ES2, STREAM_TYPE_AVC, A_ES),
        ));
        input.extend_from_slice(&psi_section_packets_for(
            B_PMT,
            0,
            TABLE_ID_PMT,
            B_PROGRAM,
            &pmt_body_with_pcr_pid(B_ES, STREAM_TYPE_AVC, B_ES),
        ));
        let push_au = |input: &mut Vec<u8>, cc: &mut u8, pid: u16, au: &[u8], dts: i64| {
            let wire = dts.rem_euclid(1i64 << 33) as u64;
            let pes = pes_packet(ES_STREAM_ID_VIDEO, wire as i64, Some(wire as i64), au);
            let packets = pes_packets(pid, *cc, &pes);
            *cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
            input.extend_from_slice(&packets);
        };

        // Program A: base 1, then a signalled discontinuity on its PCR PID,
        // then base 2 (below base 1).
        for i in 0..FRAMES {
            let au = annexb[(i as usize) % annexb.len()].clone();
            push_au(&mut input, &mut cc, A_ES, &au, BASE_A + i * FRAME_PERIOD);
        }
        {
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            pkt[0] = 0x47;
            pkt[1] = (A_ES >> 8) as u8 & PID_HI_MASK;
            pkt[2] = (A_ES & 0xFF) as u8;
            pkt[3] = 0x20 | (cc & 0x0F); // AFC=10: adaptation field only
            pkt[4] = 8;
            pkt[5] = 0x80; // discontinuity_indicator
            input.extend_from_slice(&pkt);
            cc = cc.wrapping_add(1);
        }
        for i in 0..FRAMES {
            let au = annexb[(i as usize) % annexb.len()].clone();
            push_au(&mut input, &mut cc, A_ES, &au, A_BASE_B + i * FRAME_PERIOD);
        }
        // Program A's *second* elementary stream: its own timeline with the
        // same base change, so the rebase must reach every stream of the
        // program, not just the PID that carried the indicator.
        for i in 0..FRAMES * 2 {
            let au = annexb[(i as usize) % annexb.len()].clone();
            let dts = if i < FRAMES {
                BASE_A + i * FRAME_PERIOD
            } else {
                A_BASE_B + (i - FRAMES) * FRAME_PERIOD
            };
            push_au(&mut input, &mut cc, A_ES2, &au, dts);
        }
        // Program B: one base, no discontinuity at all.
        for i in 0..FRAMES * 2 {
            let au = annexb[(i as usize) % annexb.len()].clone();
            push_au(&mut input, &mut cc, B_ES, &au, BASE_A + i * FRAME_PERIOD);
        }

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let events: Vec<DemuxEvent> = std::iter::from_fn(|| demux.poll_event()).collect();
        let ids: BTreeMap<u32, u16> = events
            .iter()
            .filter_map(|e| match e {
                DemuxEvent::TrackAdded(spec) => spec.source_pid.map(|pid| (spec.track_id, pid)),
                _ => None,
            })
            .collect();
        let mut samples_by_pid: BTreeMap<u16, Vec<i64>> = BTreeMap::new();
        for event in &events {
            if let DemuxEvent::Sample { track_id, sample } = event
                && let Some(pid) = ids.get(track_id)
                && let Some(dts) = sample.dts
            {
                samples_by_pid.entry(*pid).or_default().push(dts);
            }
        }
        let a_dts = samples_by_pid
            .get(&A_ES)
            .cloned()
            .expect("program A's video PID must deliver samples");
        let b_dts = samples_by_pid
            .get(&B_ES)
            .cloned()
            .expect("program B must deliver samples");

        assert!(
            b_dts.len() >= FRAMES as usize,
            "program B must deliver its samples, got {}",
            b_dts.len()
        );
        // Program B's timeline must be exactly what it would be without any
        // discontinuity: the declared base, advancing by one frame period.
        let expected: Vec<i64> = (0..FRAMES * 2).map(|i| BASE_A + i * FRAME_PERIOD).collect();
        let got: Vec<i64> = b_dts.iter().copied().take(expected.len()).collect();
        assert_eq!(
            got, expected,
            "program B's dts must be untouched by program A's signalled              discontinuity"
        );
        // And program A did get rebased — on *both* of its elementary streams,
        // so the test is not vacuous and the scope is not accidentally
        // narrowed to the PID that happened to carry the indicator.
        let a_steps: Vec<i64> = a_dts.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            !a_steps.is_empty() && a_steps.iter().all(|&d| d > 0),
            "program A's video timeline must be strictly increasing across its              discontinuity: {a_dts:?}"
        );
        let a2 = samples_by_pid
            .get(&A_ES2)
            .expect("program A's second elementary stream must deliver samples");
        let a2_steps: Vec<i64> = a2.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            !a2_steps.is_empty() && a2_steps.iter().all(|&d| d > 0),
            "program A's *second* PID must be rebased by the discontinuity on              its PCR_PID too: {a2:?}"
        );
    }

    /// Real Annex-B H.264 access units from the committed `h264_aac.ts`
    /// capture: the first sample carries the SPS/PPS, so a track resolves on
    /// the first one fed. Reused by every test that needs a decodable video
    /// elementary stream without hand-rolling one.
    fn real_annexb_frames() -> Vec<Vec<u8>> {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("h264_aac.ts");
        let source =
            std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
        let media = TsDemux::new().demux(&source).expect("demux h264_aac.ts");
        let video = media
            .tracks
            .iter()
            .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
            .expect("h264_aac.ts has an AVC track");
        video
            .samples
            .iter()
            .map(|sample| {
                let mut au = Vec::new();
                for nal in crate::annexb::iter_length_prefixed_nals(&sample.data)
                    .expect("TsMux writes valid NAL prefixes")
                {
                    au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                    au.extend_from_slice(nal);
                }
                au
            })
            .collect()
    }

    /// A backward discontinuity with a **jittered** cadence. The earlier
    /// attempt decided "old base vs new base" by exact equality
    /// (`shortfall == 0`), which cadence jitter breaks: at 23.976 fps the frame
    /// period alternates 3754/3753 ticks (90000/23.976 = 3753.75), so the old
    /// base's last access unit lands a tick early or late and consumes the
    /// `pending_rebase` flag — leaving the real new-base unit unrebase
    /// (r04-W50 review). A backward move is the signal instead, because the old
    /// base cannot produce one.
    ///
    /// The timeline fed in is the *real* alternating cadence, not a synthetic
    /// constant: that is the whole point, since a constant cadence cannot
    /// reproduce the defect.
    #[test]
    fn jittered_cadence_backward_discontinuity_rebases() {
        /// 23.976 fps at 90 kHz: 3753.75 ticks, emitted as 3754/3753 —
        /// exactly what a real encoder hands out.
        const PERIODS: [i64; 2] = [3754, 3753];
        /// First time base.
        const BASE_A: i64 = 900_000;
        /// Second base: **below** the first, the ordinary splice shape.
        const BASE_B: i64 = 120_000;
        /// Access units per base — enough for the jitter to bite.
        const FRAMES: i64 = 30;

        let annexb = real_annexb_frames();
        let mut cc = 0u8;
        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            // The ES PID is the program's PCR_PID, so the indicator is carried
            // where §2.4.3.5 requires.
            &pmt_body_with_pcr_pid(ES_PID_UNDER_TEST, STREAM_TYPE_AVC, ES_PID_UNDER_TEST),
        ));

        let push = |input: &mut Vec<u8>, cc: &mut u8, au: &[u8], dts: i64| {
            let wire = dts.rem_euclid(1i64 << 33);
            let pes = pes_packet(ES_STREAM_ID_VIDEO, wire, Some(wire), au);
            let packets = pes_packets(ES_PID_UNDER_TEST, *cc, &pes);
            *cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
            input.extend_from_slice(&packets);
        };

        let mut dts = BASE_A;
        for i in 0..FRAMES * 2 {
            if i == FRAMES {
                let mut pkt = [0xFFu8; TS_PACKET_SIZE];
                pkt[0] = 0x47;
                pkt[1] = (ES_PID_UNDER_TEST >> 8) as u8 & PID_HI_MASK;
                pkt[2] = (ES_PID_UNDER_TEST & 0xFF) as u8;
                pkt[3] = 0x20 | (cc & 0x0F);
                pkt[4] = 8;
                pkt[5] = 0x80; // discontinuity_indicator
                input.extend_from_slice(&pkt);
                cc = cc.wrapping_add(1);
                dts = BASE_B;
            }
            let au = annexb[(i as usize) % annexb.len()].clone();
            push(&mut input, &mut cc, &au, dts);
            dts += PERIODS[(i as usize) % PERIODS.len()];
        }

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let got: Vec<i64> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => sample.dts,
                _ => None,
            })
            .collect();
        assert!(
            got.len() >= FRAMES as usize,
            "expected the access units of both bases, got {}",
            got.len()
        );
        let steps: Vec<i64> = got.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            steps.iter().all(|&d| d > 0),
            "dts must be strictly increasing across a jittered discontinuity;              dts={got:?}"
        );
        // The seam continues the cadence: one nominal period, within the
        // jitter the input itself carries.
        let seam = steps
            .iter()
            .copied()
            .filter(|&d| d > PERIODS[1])
            .max()
            .unwrap_or(0);
        assert!(
            seam <= PERIODS[0] + 1,
            "the seam must continue the cadence (one period), not jump:              largest step {seam}, steps={steps:?}"
        );
    }

    /// An 8-second forward splice must **not** become the frame period. The
    /// first estimator accepted any step up to ten seconds, so a splice sat
    /// inside the window and was then used as the constant period for every
    /// unstamped access unit after it — stretching them all out by 8 seconds
    /// (r04-W48/W50 review). The window is now a median of the last few steps
    /// and its ceiling is one second.
    #[test]
    fn an_eight_second_splice_does_not_become_the_frame_period() {
        /// Frame period, 90 kHz ticks (25 fps).
        const FRAME_PERIOD: i64 = 3600;
        /// A splice far longer than any real frame period, and longer than the
        /// one-second ceiling.
        const SPLICE: i64 = 8 * 90_000;
        /// Stamped frames before the splice.
        const STAMPED: i64 = 8;
        /// Forced frame-rate PES packets after the splice — the frames whose
        /// duration the interpolator would stretch if the splice were adopted.
        const UNSTAMPED: usize = 4;

        let annexb = real_annexb_frames();
        let mut cc = 0u8;
        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_AVC),
        ));

        let push = |input: &mut Vec<u8>, cc: &mut u8, au: &[u8], dts: Option<i64>| {
            let pes = match dts {
                Some(d) => {
                    let wire = d.rem_euclid(1i64 << 33);
                    pes_packet(ES_STREAM_ID_VIDEO, wire, Some(wire), au)
                }
                None => pes_packet_untimed(ES_STREAM_ID_VIDEO, au),
            };
            let packets = pes_packets(ES_PID_UNDER_TEST, *cc, &pes);
            *cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
            input.extend_from_slice(&packets);
        };

        // A regular cadence, then a *run* of 8-second steps — enough of them
        // that even a median of the window is dominated by the splice, which is
        // exactly what the one-second ceiling exists to reject.
        for i in 0..STAMPED {
            let au = annexb[(i as usize) % annexb.len()].clone();
            push(&mut input, &mut cc, &au, Some(90_000 + i * FRAME_PERIOD));
        }
        let mut at = 90_000 + (STAMPED - 1) * FRAME_PERIOD + SPLICE;
        for i in 0..6usize {
            let au = annexb[i % annexb.len()].clone();
            push(&mut input, &mut cc, &au, Some(at));
            at += SPLICE;
        }
        let after_splice = at - SPLICE;
        for i in 1..=UNSTAMPED {
            let au = annexb[i % annexb.len()].clone();
            push(&mut input, &mut cc, &au, None);
        }

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let dts: Vec<i64> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => sample.dts,
                _ => None,
            })
            .collect();
        // The unstamped frames must be spaced by the *measured* frame period,
        // not by the splice.
        let tail: Vec<i64> = dts.iter().copied().filter(|&d| d > after_splice).collect();
        assert!(
            tail.len() >= UNSTAMPED - 1,
            "expected the unstamped frames to be delivered, got {dts:?}"
        );
        let steps: Vec<i64> = tail.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            steps.iter().all(|&d| d == FRAME_PERIOD),
            "the splice must not become the frame period: steps={steps:?}              (an adopted 8-second period would give {SPLICE})"
        );
    }

    /// A **bounded** PES that stops short of its declared length is dropped at
    /// `finish()` too, not just mid-stream: end of input is not evidence that
    /// the missing bytes were never needed (r04-W49 review).
    #[test]
    fn truncated_final_bounded_pes_is_dropped_at_flush() {
        /// A payload long enough to need two TS packets, so the declared
        /// length can be left unmet.
        const BODY: usize = 300;
        let au: Vec<u8> = (0..BODY).map(|i| (i & 0xFF) as u8).collect();
        let pes = untimed_pes(ES_STREAM_ID_PRIVATE_DATA, &au);

        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_PES_PRIVATE),
        ));
        // Only the first of the unit's two packets: the declared length is
        // never reached, and nothing follows to complete it.
        let packets = pes_packets(ES_PID_UNDER_TEST, 0, &pes);
        assert_eq!(
            packets.len(),
            2 * TS_PACKET_SIZE,
            "the unit spans two packets"
        );
        input.extend_from_slice(&packets[..TS_PACKET_SIZE]);

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let samples = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| matches!(e, DemuxEvent::Sample { .. }))
            .count();
        assert_eq!(
            samples, 0,
            "a bounded PES that never reached its declared length must not be              delivered by finish()"
        );
    }

    /// A discontinuity that coincides with the 33-bit wrap. Base A ends at raw
    /// `2^33 - 3*3600`; the indicator signals a new base whose first raw stamp
    /// is `1000` — just past the wrap, and *continuing* the old base once the
    /// 33-bit unroll is applied. Reading the raw number instead of the unrolled
    /// one makes that look like a jump backwards by nearly the whole modulus,
    /// so the lift is inflated by ~2³³ and the timeline runs away
    /// (r04-W50 review; the earlier round claimed this was untestable).
    ///
    /// The assertion is exact: every step stays one frame period — across the
    /// wrap and across the signalled seam alike.
    #[test]
    fn wrap_coincident_discontinuity_keeps_one_frame_period_steps() {
        /// Frame period, 90 kHz ticks.
        const FRAME_PERIOD: i64 = 3600;
        /// Frames in base A.
        const FRAMES_A: i64 = 3;
        /// The 33-bit clock modulus.
        const WRAP: i64 = 1i64 << 33;
        /// Base A's anchor: its last stamp is exactly one period short of the
        /// wrap.
        const BASE_A: i64 = WRAP - (FRAMES_A + 1) * FRAME_PERIOD;
        /// Base A's last raw (= unrolled) stamp.
        const A_LAST: i64 = BASE_A + (FRAMES_A - 1) * FRAME_PERIOD;
        /// Base B's first raw stamp. On the wire it is a small number just
        /// past zero; unrolled, `BASE_B + WRAP` is exactly one frame period
        /// after base A's last stamp, so the honest timeline is a perfectly
        /// regular cadence across the wrap.
        const BASE_B: i64 = BASE_A + FRAMES_A * FRAME_PERIOD - WRAP;

        let annexb = real_annexb_frames();
        let mut cc = 0u8;
        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body_with_pcr_pid(ES_PID_UNDER_TEST, STREAM_TYPE_AVC, ES_PID_UNDER_TEST),
        ));

        let push = |input: &mut Vec<u8>, cc: &mut u8, au: &[u8], raw: i64| {
            let wire = raw.rem_euclid(WRAP);
            let pes = pes_packet(ES_STREAM_ID_VIDEO, wire, Some(wire), au);
            let packets = pes_packets(ES_PID_UNDER_TEST, *cc, &pes);
            *cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
            input.extend_from_slice(&packets);
        };

        // Base A, ending one frame period short of the wrap.
        for i in 0..FRAMES_A {
            let au = annexb[(i as usize) % annexb.len()].clone();
            push(&mut input, &mut cc, &au, BASE_A + i * FRAME_PERIOD);
        }
        // The signalled discontinuity, at the wrap.
        {
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            pkt[0] = 0x47;
            pkt[1] = (ES_PID_UNDER_TEST >> 8) as u8 & PID_HI_MASK;
            pkt[2] = (ES_PID_UNDER_TEST & 0xFF) as u8;
            pkt[3] = 0x20 | (cc & 0x0F);
            pkt[4] = 8;
            pkt[5] = 0x80; // discontinuity_indicator
            input.extend_from_slice(&pkt);
            cc = cc.wrapping_add(1);
        }
        // Base B: raw stamps just past zero, i.e. unrolled one frame past base
        // A's last stamp.
        for i in 0..FRAMES_A + 2 {
            let au = annexb[((FRAMES_A + i) as usize) % annexb.len()].clone();
            push(&mut input, &mut cc, &au, BASE_B + i * FRAME_PERIOD);
        }

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let dts: Vec<i64> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => sample.dts,
                _ => None,
            })
            .collect();
        assert!(
            dts.len() >= (FRAMES_A + 2) as usize,
            "expected the access units of both bases, got {}",
            dts.len()
        );
        let steps: Vec<i64> = dts.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            steps.iter().all(|&d| d == FRAME_PERIOD),
            "every step must stay exactly one frame period across the wrap and              the seam; dts={dts:?} steps={steps:?}"
        );
        // The absolute values also pin the *unroll*: base B's raw stamps are
        // near zero, so a demux that did not unroll across the wrap would place
        // them ~2^33 below base A instead of continuing it.
        let first_of_b = dts[dts.len() - (FRAMES_A as usize + 2)];
        assert!(
            first_of_b > A_LAST && first_of_b - A_LAST < 2 * FRAME_PERIOD,
            "base B must continue base A across the wrap, not restart near zero:              first_of_b={first_of_b} last_of_a={A_LAST}"
        );
    }

    /// A **forward** jump at a signalled discontinuity must not be collapsed.
    /// Round 2 computed `wanted - next` and added it even when negative, so a
    /// legitimate forward splice — the new base starting well *ahead* — was
    /// pulled back to a single frame period, destroying the real gap
    /// (r04-W50 review).
    #[test]
    fn forward_jump_at_a_signalled_discontinuity_is_not_collapsed() {
        /// Frame period.
        const FRAME_PERIOD: i64 = 3600;
        /// Frames in the first base.
        const FRAMES: i64 = 6;
        /// The first base.
        const BASE_A: i64 = 900_000;
        /// The new base: a full 8 seconds **ahead** of the first's last stamp.
        const FORWARD_JUMP: i64 = 8 * 90_000;

        let annexb = real_annexb_frames();
        let mut cc = 0u8;
        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body_with_pcr_pid(ES_PID_UNDER_TEST, STREAM_TYPE_AVC, ES_PID_UNDER_TEST),
        ));

        let push = |input: &mut Vec<u8>, cc: &mut u8, au: &[u8], dts: i64| {
            let wire = dts.rem_euclid(1i64 << 33);
            let pes = pes_packet(ES_STREAM_ID_VIDEO, wire, Some(wire), au);
            let packets = pes_packets(ES_PID_UNDER_TEST, *cc, &pes);
            *cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
            input.extend_from_slice(&packets);
        };
        for i in 0..FRAMES {
            let au = annexb[(i as usize) % annexb.len()].clone();
            push(&mut input, &mut cc, &au, BASE_A + i * FRAME_PERIOD);
        }
        let base_b = BASE_A + (FRAMES - 1) * FRAME_PERIOD + FORWARD_JUMP;
        {
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            pkt[0] = 0x47;
            pkt[1] = (ES_PID_UNDER_TEST >> 8) as u8 & PID_HI_MASK;
            pkt[2] = (ES_PID_UNDER_TEST & 0xFF) as u8;
            pkt[3] = 0x20 | (cc & 0x0F);
            pkt[4] = 8;
            pkt[5] = 0x80;
            input.extend_from_slice(&pkt);
            cc = cc.wrapping_add(1);
        }
        for i in 0..FRAMES {
            let au = annexb[(i as usize) % annexb.len()].clone();
            push(&mut input, &mut cc, &au, base_b + i * FRAME_PERIOD);
        }

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let dts: Vec<i64> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => sample.dts,
                _ => None,
            })
            .collect();
        assert!(dts.len() >= 2, "expected samples from both bases");
        let steps: Vec<i64> = dts.windows(2).map(|w| w[1] - w[0]).collect();
        let seam = steps
            .iter()
            .copied()
            .find(|&d| d > FRAME_PERIOD)
            .unwrap_or_else(|| {
                panic!("the forward jump must survive as a large step, got {steps:?}")
            });
        assert!(
            seam >= FORWARD_JUMP,
            "a forward jump of {FORWARD_JUMP} ticks must not be collapsed to              {seam}; the offset must never be reduced"
        );
        assert!(
            steps.iter().all(|&d| d > 0),
            "and the timeline must still be strictly increasing: {steps:?}"
        );
    }

    /// Real SPS/PPS bytes from a committed capture: the fixture's own first
    /// access unit carries them, and they decode to a known geometry used as
    /// the oracle below.
    fn real_avc_parameter_sets() -> (Vec<u8>, Vec<u8>, u16) {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("h264_aac.ts");
        let source =
            std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
        let media = TsDemux::new().demux(&source).expect("demux h264_aac.ts");
        let video = media
            .tracks
            .iter()
            .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
            .expect("h264_aac.ts has an AVC track");
        let width = match video.config() {
            CodecConfig::Avc { width, .. } => *width,
            _ => unreachable!(),
        };
        let first = &video.samples[0].data;
        let nals = crate::annexb::iter_length_prefixed_nals(first)
            .expect("TsMux writes valid NAL prefixes");
        let sps = nals
            .iter()
            .find(|n| (n[0] & H264_NAL_TYPE_MASK) == H264_NAL_SPS)
            .expect("the first access unit carries an SPS");
        let pps = nals
            .iter()
            .find(|n| (n[0] & H264_NAL_TYPE_MASK) == H264_NAL_PPS)
            .expect("the first access unit carries a PPS");
        (sps.to_vec(), pps.to_vec(), width)
    }

    /// Annex-B access unit from the given NALs.
    fn annexb_au(nals: &[&[u8]]) -> Vec<u8> {
        let mut au = Vec::new();
        for nal in nals {
            au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            au.extend_from_slice(nal);
        }
        au
    }

    /// An access unit carrying **only** a slice NAL: no parameter sets at all,
    /// which is how the tail of a real stream looks between keyframes.
    fn slice_only_au(tag: u8) -> Vec<u8> {
        annexb_au(&[&[0x65, 0x88, 0x84, tag]])
    }

    /// The anti-spam half of item 5, first case: an encoder that repeats the
    /// identical SPS+PPS in its first access unit and then sends slice-only
    /// access units must produce exactly one `TrackUpdated` set — none.
    ///
    /// The first attempt compared the parameter sets of a *single* access unit
    /// against a snapshot seeded from the backlog's last one, so the very next
    /// slice-only access unit (no parameter sets at all) compared unequal and
    /// raised a spurious event.
    #[test]
    fn parameter_sets_in_one_access_unit_then_slices_emit_no_update() {
        let (sps, pps, width) = real_avc_parameter_sets();
        let mut aus = vec![annexb_au(&[&sps, &pps, &[0x65, 0x88, 0x84, 0x01]])];
        for i in 0..4u8 {
            aus.push(slice_only_au(i));
        }
        let bytes = avc_ts_over(&aus);

        let mut demux = StreamingTsDemux::new();
        demux.feed(&bytes);
        demux.finish();
        let added_width = std::iter::from_fn(|| demux.poll_event()).find_map(|e| match e {
            DemuxEvent::TrackAdded(spec) => match spec.config {
                CodecConfig::Avc { width, .. } => Some(width),
                _ => None,
            },
            _ => None,
        });
        assert_eq!(
            added_width,
            Some(width),
            "the track must resolve from the parameter sets in the first access unit"
        );
        assert_eq!(
            updated_avc_width(&bytes),
            None,
            "slice-only access units carry no parameter sets, so nothing changed; a per-access-unit comparison raised a spurious TrackUpdated here"
        );
    }

    /// Second case: SPS and PPS arriving in *separate* access units (which real
    /// encoders do) must not read as a change either — the sets are tracked
    /// separately and each is only "new" once.
    #[test]
    fn parameter_sets_split_across_access_units_emit_no_update() {
        let (sps, pps, width) = real_avc_parameter_sets();
        let aus = vec![
            annexb_au(&[&sps, &[0x65, 0x88, 0x84, 0x01]]),
            annexb_au(&[&pps, &[0x65, 0x88, 0x84, 0x02]]),
            slice_only_au(0x03),
        ];
        let bytes = avc_ts_over(&aus);
        assert_eq!(
            updated_avc_width(&bytes),
            None,
            "the PPS arriving one access unit after the SPS is not a change; a one-access-unit comparison reported one (and would have reported the SPS's access unit too). Track added at width {width}"
        );
    }

    /// And the real change: a *different* SPS mid-stream must raise exactly one
    /// `TrackUpdated`, carrying the new geometry.
    #[test]
    fn a_changed_sps_raises_exactly_one_update() {
        let (sps, pps, _) = real_avc_parameter_sets();
        // A second, genuinely different SPS: the same bytes with the level_idc
        // byte altered is enough to be a different parameter set, but it must
        // still decode, so take a real SPS from another committed capture.
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("tests");
        path.push("fixtures");
        path.push("ts");
        path.push("h264-two-resolutions.ts");
        let other =
            std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
        let media = TsDemux::new()
            .demux(&other)
            .expect("demux two-resolution fixture");
        let second_sps = media
            .tracks
            .iter()
            .filter(|t| matches!(t.config(), CodecConfig::Avc { .. }))
            .flat_map(|t| t.samples.iter())
            .flat_map(|s| {
                crate::annexb::iter_length_prefixed_nals(&s.data)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|n| (n[0] & H264_NAL_TYPE_MASK) == H264_NAL_SPS)
                    .map(|n| n.to_vec())
                    .collect::<Vec<_>>()
            })
            .find(|s| s.as_slice() != sps.as_slice())
            .expect("the two-resolution fixture carries a second, different SPS");
        assert_ne!(second_sps, sps, "the two SPS really differ");

        let aus = vec![
            annexb_au(&[&sps, &pps, &[0x65, 0x88, 0x84, 0x01]]),
            slice_only_au(0x02),
            annexb_au(&[&second_sps, &pps, &[0x65, 0x88, 0x84, 0x03]]),
            slice_only_au(0x04),
        ];
        let bytes = avc_ts_over(&aus);

        let mut demux = StreamingTsDemux::new();
        demux.feed(&bytes);
        demux.finish();
        let updates: Vec<(u16, u16)> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::TrackUpdated(spec) => match spec.config {
                    CodecConfig::Avc { width, height, .. } => Some((width, height)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(
            updates.len(),
            1,
            "exactly one SPS change, so exactly one TrackUpdated: {updates:?}"
        );
        assert_ne!(
            updates[0],
            (0, 0),
            "the event must carry the newly-decoded geometry, not zeros"
        );
    }

    /// The discriminating case for the one-access-unit mistake: the SPS and the
    /// PPS arrive in **separate** access units, and the per-access-unit
    /// comparison treats the first of them (which carries no PPS at all) as a
    /// change against the empty snapshot, then the second again. Tracking each
    /// set on its own fires only on the real changes.
    #[test]
    fn parameter_sets_split_across_access_units_raise_exactly_one_update() {
        let (sps, pps, _) = real_avc_parameter_sets();
        // AU0: SPS only (carries the parameter set a decoder config needs, but
        // not the PPS yet). AU1: PPS only. AU2: a slice. AU3: a *different*
        // slice-only access unit.
        let aus = vec![
            annexb_au(&[&sps, &[0x65, 0x88, 0x84, 0x01]]),
            annexb_au(&[&[0x65, 0x88, 0x84, 0x02]]),
            annexb_au(&[&pps, &[0x65, 0x88, 0x84, 0x03]]),
            slice_only_au(0x04),
            slice_only_au(0x05),
        ];
        let bytes = avc_ts_over(&aus);

        let mut demux = StreamingTsDemux::new();
        demux.feed(&bytes);
        demux.finish();
        let updates = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| matches!(e, DemuxEvent::TrackUpdated(_)))
            .count();
        assert_eq!(
            updates, 0,
            "no parameter set ever changes value here — the SPS and the PPS just arrive in different access units — so nothing may be reported"
        );
    }

    /// The spurious-event case, precisely: a stream that alternates
    /// "SPS+PPS" and "PPS only" access units (some encoders resend only the PPS
    /// on a non-IDR keyframe). A per-access-unit snapshot loses the SPS during
    /// the PPS-only units, so the next SPS+PPS unit *looks* like a change even
    /// though the configuration is byte-for-byte the same — and because that
    /// access unit does carry a complete set, the re-probe succeeds and the
    /// spurious `TrackUpdated` is really emitted.
    #[test]
    fn alternating_parameter_set_carriage_emits_no_update() {
        let (sps, pps, _) = real_avc_parameter_sets();
        let mut aus = vec![annexb_au(&[&sps, &pps, &[0x65, 0x88, 0x84, 0x01]])];
        for i in 0..4u8 {
            // PPS-only…
            aus.push(annexb_au(&[&pps, &[0x65, 0x88, 0x84, i + 2]]));
            // …then the full set again, unchanged.
            aus.push(annexb_au(&[&sps, &pps, &[0x65, 0x88, 0x84, i + 10]]));
        }
        let bytes = avc_ts_over(&aus);
        let mut demux = StreamingTsDemux::new();
        demux.feed(&bytes);
        demux.finish();
        let updates = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| matches!(e, DemuxEvent::TrackUpdated(_)))
            .count();
        assert_eq!(
            updates, 0,
            "the configuration never changes, so no TrackUpdated may be raised; a per-access-unit snapshot loses the SPS during the PPS-only units and reports each following full set as a change"
        );
    }

    /// A PPS-only change is also a real config change and must raise one.
    #[test]
    fn a_changed_pps_raises_exactly_one_update() {
        let (sps, pps, _) = real_avc_parameter_sets();
        // Same SPS, different PPS: flip a byte in the PPS payload.
        let mut other_pps = pps.clone();
        let last = other_pps.len() - 1;
        other_pps[last] ^= 0x55;
        assert_ne!(other_pps, pps, "the two PPS really differ");

        let aus = vec![
            annexb_au(&[&sps, &pps, &[0x65, 0x88, 0x84, 0x01]]),
            annexb_au(&[&sps, &other_pps, &[0x65, 0x88, 0x84, 0x02]]),
            slice_only_au(0x03),
        ];
        let bytes = avc_ts_over(&aus);
        let mut demux = StreamingTsDemux::new();
        demux.feed(&bytes);
        demux.finish();
        let updates = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| matches!(e, DemuxEvent::TrackUpdated(_)))
            .count();
        assert_eq!(
            updates, 1,
            "a changed PPS is a config change, so exactly one TrackUpdated"
        );
    }

    /// r04-W51: a broadcast stream changes its parameter sets mid-flight
    /// routinely (an SD↔HD ad break, a re-encode, a multiplex reconfiguration).
    /// Codec config recovery used to be single-shot for the life of the stream,
    /// so the track kept the *first* `avcC` and the init segment described a
    /// stream that had stopped being sent; a decoder then fails from the change
    /// onwards. The fixture is a real two-resolution capture (see
    /// `tests/fixtures/ts/README.md`), demuxed through the streaming core so the
    /// `TrackUpdated` event is observable.
    ///
    /// The second half is the anti-spam half: the same fixture shows the
    /// encoder repeating its parameter sets on every keyframe, and an
    /// *unchanged* repeat must emit nothing.
    #[test]
    fn mid_stream_config_change_emits_track_updated_once_and_only_when_it_changes() {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("tests");
        path.push("fixtures");
        path.push("ts");
        path.push("h264-two-resolutions.ts");
        let bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));

        let mut demux = StreamingTsDemux::new();
        demux.feed(&bytes);
        demux.finish();

        let mut added: Option<(u32, u16, u16)> = None;
        let mut updates: Vec<(u32, u16, u16)> = Vec::new();
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(spec) => {
                    if let CodecConfig::Avc { width, height, .. } = spec.config {
                        added = Some((spec.track_id, width, height));
                    }
                }
                DemuxEvent::TrackUpdated(spec) => {
                    if let CodecConfig::Avc { width, height, .. } = spec.config {
                        updates.push((spec.track_id, width, height));
                    }
                }
                _ => {}
            }
        }

        let (track_id, w0, h0) = added.expect("the fixture has one AVC track");
        assert_eq!(
            (w0, h0),
            (320, 240),
            "the first half of the fixture is 320x240 (ffprobe oracle)"
        );
        assert_eq!(
            updates.len(),
            1,
            "the config changes exactly once in this fixture (320x240 -> 640x480); the encoder's repeated, unchanged SPS must not raise more. Got {updates:?}"
        );
        assert_eq!(
            updates[0],
            (track_id, 640, 480),
            "TrackUpdated must carry the *new* config, on the same track id"
        );
    }

    /// The AAC half of r04-W51: `channel_configuration` is a Table 1.19
    /// *index*, not a channel count. Configuration 7 is eight channels (7.1)
    /// and 0 means the mapping is in-band (a `program_config_element` in the
    /// raw data stream), which this crate does not decode — so no count is
    /// fabricated for it.
    ///
    /// Real ADTS headers, built by this crate's own spec-correct encoder
    /// (`aac_asc::build_adts_header`), fed through the demux as a real AAC
    /// PID.
    #[test]
    fn adts_channel_configuration_is_mapped_through_table_1_19() {
        /// `sampling_frequency_index` 3 = 48000 Hz (ISO/IEC 14496-3 Table 1.16).
        const SFI_48000: u8 = 3;
        /// AAC-LC: `profile = audio_object_type - 1`.
        const ADTS_PROFILE_AAC_LC: u8 = 1;
        /// Table 1.19 configuration 7 (7.1, eight channels).
        const CONFIG_7_1: u8 = 7;
        /// Configuration 0: the mapping is carried in-band by a PCE.
        const CONFIG_IN_BAND: u8 = 0;

        for (config, expected) in [(CONFIG_7_1, 8u16), (CONFIG_IN_BAND, 0u16)] {
            let frame_len = (ADTS_HEADER_SIZE + 64) as u16;
            let mut au = crate::aac_asc::build_adts_header(
                ADTS_PROFILE_AAC_LC,
                SFI_48000,
                config,
                frame_len,
            )
            .to_vec();
            au.resize(frame_len as usize, 0x21);

            let mut input = Vec::new();
            for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
                input.extend_from_slice(&null_packet());
            }
            input.extend_from_slice(&psi_section_packets(
                PAT_PID_UNDER_TEST,
                0,
                TABLE_ID_PAT,
                &pat_body(),
            ));
            input.extend_from_slice(&psi_section_packets(
                PMT_PID_UNDER_TEST,
                0,
                TABLE_ID_PMT,
                &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_AAC_ADTS),
            ));
            let mut cc = 0u8;
            for i in 0..4 {
                let pes = pes_packet(ES_STREAM_ID_AUDIO, i * 1024, None, &au);
                let packets = pes_packets(ES_PID_UNDER_TEST, cc, &pes);
                cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
                input.extend_from_slice(&packets);
            }

            let mut demux = StreamingTsDemux::new();
            demux.feed(&input);
            demux.finish();
            let channel_count = std::iter::from_fn(|| demux.poll_event())
                .find_map(|e| match e {
                    DemuxEvent::TrackAdded(spec) => match spec.config {
                        CodecConfig::Aac { channel_count, .. } => Some(channel_count),
                        _ => None,
                    },
                    _ => None,
                })
                .expect("the AAC PID must resolve a track");
            assert_eq!(
                channel_count, expected,
                "ADTS channel_configuration {config} must map through Table 1.19"
            );
        }
    }

    // ── r04-W50: signalled discontinuity rebases the decode timeline ────────

    /// A TS adaptation-field `discontinuity_indicator` marks "a sample of a new
    /// system time clock" (ISO/IEC 13818-1 §2.4.3.5) — a splice, an encoder
    /// switch, or a remultiplex. The new time base routinely starts *below* the
    /// old one, and nothing in the 33-bit unroll can tell that from a genuine
    /// backward jump within the range: `dts` then goes non-monotonic, violating
    /// the IR's "samples in decode order with a non-decreasing absolute dts"
    /// invariant and making every downstream muxer write negative deltas.
    ///
    /// Real elementary-stream bytes from the committed `h264_aac.ts` fixture,
    /// re-stamped onto two time bases 2 s apart, with the discontinuity
    /// signalled exactly as a muxer does it: the bit set on an
    /// adaptation-field-only packet, then the new base's PES packets.
    #[test]
    fn signalled_discontinuity_keeps_dts_monotonic() {
        /// First time base's anchor, 90 kHz ticks.
        const BASE_A_DTS: i64 = 90_000;
        /// Second time base's anchor, *below* the first on the wire clock —
        /// the ordinary splice shape (new content whose first stamp is
        /// earlier). Both bases sit in the same 33-bit period, so the unroll
        /// reads the step as a small backward jump rather than a wrap.
        const BASE_B_DTS: i64 = 9_000;
        const FRAME_PERIOD: i64 = 3600;
        /// Frames per time base.
        const FRAMES: usize = 12;
        /// Unwrapped clock modulus — the wire stamps below stay inside one
        /// period, so the values are unambiguous.
        const WRAP: i64 = 1i64 << 33;

        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("h264_aac.ts");
        let source =
            std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
        let media = TsDemux::new().demux(&source).expect("demux h264_aac.ts");
        let video = media
            .tracks
            .iter()
            .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
            .expect("h264_aac.ts has an AVC track");
        assert!(
            video.samples.len() >= FRAMES * 2,
            "fixture too small for two time bases"
        );

        // Annex-B access units, and a first access unit that still carries the
        // parameter sets (the fixture's real ES has no SPS outside those).
        let annexb: Vec<Vec<u8>> = video
            .samples
            .iter()
            .map(|sample| {
                let mut au = Vec::new();
                for nal in crate::annexb::iter_length_prefixed_nals(&sample.data)
                    .expect("TsMux writes valid NAL prefixes")
                {
                    au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                    au.extend_from_slice(nal);
                }
                au
            })
            .collect();

        let mut cc = 0u8;
        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            // The ES PID is the program's PCR_PID, so the indicator is
            // carried where §2.4.3.5 requires and scopes to this program.
            &pmt_body_with_pcr_pid(ES_PID_UNDER_TEST, STREAM_TYPE_AVC, ES_PID_UNDER_TEST),
        ));

        let push_au = |input: &mut Vec<u8>, cc: &mut u8, au: &[u8], dts: i64| {
            let wire = (dts.rem_euclid(WRAP)) as u64;
            let pes = pes_packet(ES_STREAM_ID_VIDEO, wire as i64, Some(wire as i64), au);
            let packets = pes_packets(ES_PID_UNDER_TEST, *cc, &pes);
            *cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
            input.extend_from_slice(&packets);
        };

        // Time base A.
        for i in 0..FRAMES {
            let au = annexb[i % annexb.len()].clone();
            push_au(
                &mut input,
                &mut cc,
                &au,
                BASE_A_DTS + i as i64 * FRAME_PERIOD,
            );
        }

        // The signalled discontinuity: an adaptation-field-only packet with
        // `discontinuity_indicator` set (§2.4.3.4 Table 2-6, bit 7 of the
        // flags byte) and no payload.
        {
            let af = [
                0x80u8, // discontinuity_indicator=1, nothing else
                0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // stuffing
            ];
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            pkt[0] = 0x47;
            pkt[1] = (ES_PID_UNDER_TEST >> 8) as u8 & PID_HI_MASK;
            pkt[2] = (ES_PID_UNDER_TEST & 0xFF) as u8;
            pkt[3] = 0x20 | (cc & 0x0F); // AFC=10: adaptation only, no payload
            pkt[4] = af.len() as u8;
            pkt[5..5 + af.len()].copy_from_slice(&af);
            input.extend_from_slice(&pkt);
            cc = cc.wrapping_add(1);
        }

        // Time base B, starting 2 s lower on the wire clock.
        for i in 0..FRAMES {
            let au = annexb[i % annexb.len()].clone();
            push_au(
                &mut input,
                &mut cc,
                &au,
                BASE_B_DTS + i as i64 * FRAME_PERIOD,
            );
        }

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let mut saw_signalled = false;
        let samples: Vec<Sample> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Discontinuity {
                    kind: DiscontinuityKind::Signalled,
                    ..
                } => {
                    saw_signalled = true;
                    None
                }
                DemuxEvent::Sample { sample, .. } => Some(sample),
                _ => None,
            })
            .collect();
        assert!(
            saw_signalled,
            "the adaptation-field discontinuity_indicator must surface as a Signalled Discontinuity"
        );
        let dts: Vec<i64> = samples
            .iter()
            .map(|s| s.dts.expect("stamped dts"))
            .collect();
        assert!(
            dts.len() >= FRAMES,
            "expected one sample per access unit, got {}",
            dts.len()
        );
        assert!(
            dts.windows(2).all(|w| w[1] >= w[0]),
            "dts must stay monotonic across a signalled discontinuity; got {dts:?}"
        );
        // The rebase is *only* a lift: the second base's own intervals are
        // untouched, so the steps stay one frame period apart throughout.
        let steps: Vec<i64> = dts.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            steps.iter().all(|&d| d == FRAME_PERIOD),
            "every decode step must remain one frame period; got {steps:?}"
        );
    }

    /// Three signalled discontinuities in a row: each new time base starts
    /// *below* the previous one, so every lift compounds. The second attempt at
    /// this fix compared an offset-free "previous" value against an
    /// offset-bearing "next" one, so from the second discontinuity on the lift
    /// was too small and `dts` stepped backwards again.
    #[test]
    fn three_signalled_discontinuities_keep_dts_monotonic() {
        /// Frame period, 90 kHz ticks.
        const FRAME_PERIOD: i64 = 3600;
        /// Frames per time base.
        const FRAMES: i64 = 6;
        /// The bases, each starting *below* the one before.
        const BASES: [i64; 3] = [90_000, 30_000, 5_000];

        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("h264_aac.ts");
        let source =
            std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
        let media = TsDemux::new().demux(&source).expect("demux h264_aac.ts");
        let video = media
            .tracks
            .iter()
            .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
            .expect("h264_aac.ts has an AVC track");
        let annexb: Vec<Vec<u8>> = video
            .samples
            .iter()
            .map(|sample| {
                let mut au = Vec::new();
                for nal in crate::annexb::iter_length_prefixed_nals(&sample.data)
                    .expect("TsMux writes valid NAL prefixes")
                {
                    au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                    au.extend_from_slice(nal);
                }
                au
            })
            .collect();

        let mut cc = 0u8;
        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            // The ES PID is the program's PCR_PID, so a
            // `discontinuity_indicator` on it rebases this program's streams
            // (§2.4.3.5).
            &pmt_body_with_pcr_pid(ES_PID_UNDER_TEST, STREAM_TYPE_AVC, ES_PID_UNDER_TEST),
        ));

        let push_au = |input: &mut Vec<u8>, cc: &mut u8, au: &[u8], dts: i64| {
            let wire = dts.rem_euclid(1i64 << 33) as u64;
            let pes = pes_packet(ES_STREAM_ID_VIDEO, wire as i64, Some(wire as i64), au);
            let packets = pes_packets(ES_PID_UNDER_TEST, *cc, &pes);
            *cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
            input.extend_from_slice(&packets);
        };

        for base in BASES {
            for i in 0..FRAMES {
                let au = annexb[(i as usize) % annexb.len()].clone();
                push_au(&mut input, &mut cc, &au, base + i * FRAME_PERIOD);
            }
            // The signalled discontinuity packet (§2.4.3.4 Table 2-6: an
            // adaptation field, no payload, flags bit 7 set).
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            pkt[0] = 0x47;
            pkt[1] = (ES_PID_UNDER_TEST >> 8) as u8 & PID_HI_MASK;
            pkt[2] = (ES_PID_UNDER_TEST & 0xFF) as u8;
            pkt[3] = 0x20 | (cc & 0x0F); // AFC=10, adaptation only
            pkt[4] = 8;
            pkt[5] = 0x80; // discontinuity_indicator
            input.extend_from_slice(&pkt);
            cc = cc.wrapping_add(1);
        }

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let dts: Vec<i64> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => sample.dts,
                _ => None,
            })
            .collect();
        assert!(
            dts.len() >= (BASES.len() as i64 * FRAMES) as usize - 3,
            "expected the access units of all three time bases, got {} (first few: {:?})",
            dts.len(),
            &dts[..dts.len().min(6)]
        );
        let steps: Vec<i64> = dts.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            steps.iter().all(|&d| d > 0),
            "dts must be strictly increasing across every discontinuity; dts={dts:?} steps={steps:?}"
        );
    }

    /// A CC gap on a **continuation** packet misses 184 bytes of the access
    /// unit being assembled, so that unit must be dropped and its neighbours
    /// delivered unharmed. (The first W49 attempt set `current_pes_intact`
    /// back to `true` unconditionally after every packet, so a gap followed by
    /// any further continuation packet of the same unit was forgotten and the
    /// truncated unit was delivered anyway.)
    ///
    /// Units here are four TS packets long and the gap is placed in the
    /// *middle*, so there really are continuation packets after it — which is
    /// exactly what the old rule let clear the damage mark.
    #[test]
    fn cc_gap_on_a_continuation_drops_only_that_access_unit() {
        /// Each access unit spans four TS packets (a 9-byte PES header plus
        /// the body).
        const PACKETS_PER_AU: usize = 4;
        /// TS payload per packet.
        const PER_PACKET: usize = TS_PACKET_SIZE - 4;
        /// Bytes of PES header [`untimed_pes`] writes before the body.
        const PES_HEADER_BYTES: usize = 9;
        // Exactly `PACKETS_PER_AU` TS packets per PES: the header plus a body
        // sized to fill the rest precisely, so no stuffing is involved.
        let body_len = PACKETS_PER_AU * PER_PACKET - PES_HEADER_BYTES;
        let aus: Vec<Vec<u8>> = (0..3)
            .map(|i| (0..body_len).map(|b| ((b + i * 53) & 0xFF) as u8).collect())
            .collect();

        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_PES_PRIVATE),
        ));

        let mut cc = 0u8;
        let mut expected = Vec::new();
        for (i, au) in aus.iter().enumerate() {
            let pes = untimed_pes(ES_STREAM_ID_PRIVATE_DATA, au);
            let packets = pes_packets(ES_PID_UNDER_TEST, cc, &pes);
            let n = packets.len() / TS_PACKET_SIZE;
            assert_eq!(
                n, PACKETS_PER_AU,
                "AU {i} must span {PACKETS_PER_AU} packets"
            );
            if i == 1 {
                // Drop only packet 1 of this unit: packet 0 starts it and
                // packets 2.. follow the gap, so the old "clear the mark after
                // every packet" rule would forget the damage.
                input.extend_from_slice(&packets[..TS_PACKET_SIZE]);
                input.extend_from_slice(&packets[2 * TS_PACKET_SIZE..]);
            } else {
                input.extend_from_slice(&packets);
                expected.push(au.clone());
            }
            cc = cc.wrapping_add(n as u8);
        }

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let got: Vec<Vec<u8>> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => Some(sample.data.to_vec()),
                _ => None,
            })
            .collect();
        assert_eq!(
            got, expected,
            "only the unit whose middle packet was lost may be dropped"
        );
    }

    /// A CC jump exactly at a `payload_unit_start` does NOT damage the unit
    /// that just completed, when that unit's own declared length was fully
    /// received. This is the segment-concatenation case: every segment is
    /// muxed independently, so the counter jumps at the seam while the last
    /// unit of the previous segment is complete. Treating every such jump as
    /// truncation dropped one access unit per boundary.
    #[test]
    fn cc_jump_at_a_pes_start_keeps_the_previous_complete_access_unit() {
        // A *bounded* PES (PES_packet_length != 0), which is what a private
        // data stream uses — the length is what proves completeness.
        let au1 = data_au(0x00);
        let au2 = data_au(0x80);

        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_PES_PRIVATE),
        ));

        let pes1 = untimed_pes(ES_STREAM_ID_PRIVATE_DATA, &au1);
        let pes2 = untimed_pes(ES_STREAM_ID_PRIVATE_DATA, &au2);
        let pkt1 = pes_packets(ES_PID_UNDER_TEST, 0, &pes1);
        assert_eq!(pkt1.len(), 2 * TS_PACKET_SIZE, "unit 1 spans two packets");
        // Unit 1 used counters 0 and 1; unit 2 restarts at 0 (a freshly muxed
        // segment always does), so the counter *jumps backwards* at exactly
        // the second unit's `payload_unit_start`.
        let pkt2 = pes_packets(ES_PID_UNDER_TEST, 0, &pes2);
        input.extend_from_slice(&pkt1);
        input.extend_from_slice(&pkt2);

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let got: Vec<Vec<u8>> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => Some(sample.data.to_vec()),
                _ => None,
            })
            .collect();
        assert_eq!(
            got,
            vec![au1.clone(), au2.clone()],
            "a counter jump at a `payload_unit_start` must not drop the previous unit when its declared length was fully received"
        );
    }

    /// An **unbounded** unit (`PES_packet_length == 0`, what every video PES
    /// uses) whose own packets are all present survives a counter restart at
    /// the next `payload_unit_start`. This is the segment-concatenation and
    /// doubled-file seam: each piece is muxed independently, so its counter
    /// begins wherever its own muxer started, and nothing inside the
    /// completing unit was lost — a `payload_unit_start` *ends* that unit.
    #[test]
    fn cc_restart_at_a_pes_start_keeps_an_unbounded_unit() {
        let au1 = data_au(0x10);
        let au2 = data_au(0x90);

        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_PES_PRIVATE),
        ));

        let pes1 = unbounded_pes(ES_STREAM_ID_PRIVATE_DATA, &au1);
        let pes2 = unbounded_pes(ES_STREAM_ID_PRIVATE_DATA, &au2);
        let pkt1 = pes_packets(ES_PID_UNDER_TEST, 0, &pes1);
        assert_eq!(pkt1.len(), 2 * TS_PACKET_SIZE, "unit 1 spans two packets");
        // Unit 2's counter restarts at 0, as an independently muxed segment's
        // does.
        let pkt2 = pes_packets(ES_PID_UNDER_TEST, 0, &pes2);
        input.extend_from_slice(&pkt1);
        input.extend_from_slice(&pkt2);

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let got: Vec<Vec<u8>> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => Some(sample.data.to_vec()),
                _ => None,
            })
            .collect();
        assert_eq!(
            got.len(),
            2,
            "both units are whole on the wire, so a counter restart between them must drop neither: {got:?}"
        );
        assert!(got[0].starts_with(&au1), "first unit preserved");
        assert!(got[1].starts_with(&au2), "second unit preserved");
    }

    /// The seam case for a **bounded** unit, checked through the real segmenter:
    /// concatenating independently muxed segments must keep every access unit
    /// of both. A counter restart at a `payload_unit_start` is not truncation.
    #[test]
    fn concatenated_segments_seam_keeps_every_access_unit() {
        use crate::TsMux;
        use crate::media::{Media, Track};
        use crate::pipeline::{CodecConfig, Sample, TrackSpec};
        use crate::rtp_sdp::avc_config_from_sprop;
        use broadcast_common::Package;

        let frame_dur = VIDEO_TIMESCALE / 25;
        let avc = avc_config_from_sprop("Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==").unwrap();
        let spec = TrackSpec::new(
            1,
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config: avc,
                width: 0,
                height: 0,
            },
        );
        let samples: Vec<Sample> = (0..6u32)
            .map(|i| {
                let nal = [0x65u8, 0xAA, i as u8];
                let mut data = (nal.len() as u32).to_be_bytes().to_vec();
                data.extend_from_slice(&nal);
                let dts = i64::from(i) * i64::from(frame_dur);
                Sample::new(data, Some(dts), Some(dts), Some(frame_dur), true)
            })
            .collect();
        let media = Media::new(vec![Track::new(spec, samples)], VIDEO_TIMESCALE);

        // Two separately muxed copies of the same track, concatenated as a
        // segmenter would emit (and as `transmux/tests/ts_hls.rs` does): each
        // begins with its own PAT/PMT and its own counter sequence.
        let segment = TsMux::default().package(&media).expect("mux to TS");
        let mut concatenated = segment.clone();
        concatenated.extend_from_slice(&segment);

        let mut demux = StreamingTsDemux::new();
        demux.feed(&concatenated);
        demux.finish();
        let video_samples = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| matches!(e, DemuxEvent::Sample { .. }))
            .count();
        assert_eq!(
            video_samples, 12,
            "a segment seam restarts the counter but loses nothing, so all 12 access units must survive"
        );
    }

    /// A gap *inside* an unbounded unit — on one of its own continuation
    /// packets — still drops it: that packet's bytes provably belong to the
    /// unit, and there is no declared length to show otherwise.
    #[test]
    fn cc_gap_inside_an_unbounded_unit_drops_it() {
        /// Bytes per TS payload.
        const PER_PACKET: usize = TS_PACKET_SIZE - 4;
        /// Four-packet unit, so a middle packet can be dropped.
        const PACKETS: usize = 4;
        /// Bytes of PES header [`unbounded_pes`] writes.
        const PES_HEADER_BYTES: usize = 9;

        let au: Vec<u8> = (0..PACKETS * PER_PACKET - PES_HEADER_BYTES)
            .map(|i| (i & 0xFF) as u8)
            .collect();
        let pes = unbounded_pes(ES_STREAM_ID_PRIVATE_DATA, &au);
        let packets = pes_packets(ES_PID_UNDER_TEST, 0, &pes);
        assert_eq!(packets.len(), PACKETS * TS_PACKET_SIZE);

        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_PES_PRIVATE),
        ));
        // Packet 0 starts the unit; packet 1 is lost; packets 2.. arrive.
        input.extend_from_slice(&packets[..TS_PACKET_SIZE]);
        input.extend_from_slice(&packets[2 * TS_PACKET_SIZE..]);

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let samples = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| matches!(e, DemuxEvent::Sample { .. }))
            .count();
        assert_eq!(
            samples, 0,
            "a unit missing one of its own packets must not be delivered"
        );
    }

    /// A signalled discontinuity is **not** on its own a reason to drop a unit
    /// (r04-W49 review, round 3). The rule is stated in
    /// [`StreamingTsDemux::process_packet`]: a unit is damaged only by a
    /// continuity-counter gap on one of *its own* packets, or by a bounded PES
    /// arriving short of its declared length. The indicator only says the
    /// source's byte stream changes *here* — a normal HLS or splice seam has an
    /// intact last unit of the old segment immediately before it, and dropping
    /// that unit lost one access unit at every seam.
    ///
    /// The fixture is deliberately **unbounded** (`PES_packet_length == 0`, the
    /// ordinary video shape): a bounded PES carries its own completeness proof,
    /// so it cannot distinguish "the indicator dropped it" from "the length did
    /// not add up". Two whole units are followed by a third, with the indicator
    /// on the packet that ends unit 2 and starts unit 3.
    #[test]
    fn signalled_discontinuity_does_not_drop_the_unit_it_ends() {
        /// Continuity counter of the packet that carries the indicator: unit 2
        /// used counters 1 and 2, so the indicator continues at 3.
        const SIGNALLED_CC: u8 = 3;
        let au1 = data_au(0x20);
        let au2 = data_au(0xA0);
        let au3 = data_au(0xF0);

        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_PES_PRIVATE),
        ));

        // Unbounded units, two TS packets each, all present.
        let mut cc = 0u8;
        for au in [&au1, &au2] {
            let pes = unbounded_pes(ES_STREAM_ID_PRIVATE_DATA, au);
            let packets = pes_packets(ES_PID_UNDER_TEST, cc, &pes);
            assert_eq!(
                packets.len(),
                2 * TS_PACKET_SIZE,
                "each unit spans two packets"
            );
            input.extend_from_slice(&packets);
            cc = cc.wrapping_add(2);
        }
        // The packet that starts unit 3 carries the indicator
        // (§2.4.3.5): `afc` '11', an 8-byte adaptation field, flags bit 7.
        let pes3 = unbounded_pes(ES_STREAM_ID_PRIVATE_DATA, &au3);
        input.extend_from_slice(&pes_packets_with_discontinuity(
            ES_PID_UNDER_TEST,
            SIGNALLED_CC,
            &pes3,
        ));

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let got: Vec<Vec<u8>> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => Some(sample.data.to_vec()),
                _ => None,
            })
            .collect();
        assert_eq!(
            got.len(),
            3,
            "every unit here is whole on the wire, so the indicator on the              packet starting unit 3 must not drop anything: {got:?}"
        );
        for (i, (got_au, want)) in got.iter().zip([&au1, &au2, &au3]).enumerate() {
            assert!(got_au.starts_with(want), "unit {i} must be delivered whole");
        }
    }

    /// The documented tradeoff at a seam, on an **unbounded** PES. A
    /// `payload_unit_start` ends the previous unit, so a counter restart there
    /// is the ordinary segment-concatenation shape and is *not* damage — which
    /// also means an unbounded unit that really did lose its tail is
    /// indistinguishable from one that ended cleanly, and is delivered
    /// (H.222.0 carries no length to tell them apart; §2.4.3.3/§2.4.3.7).
    ///
    /// This test pins that tradeoff explicitly, so the behaviour is a recorded
    /// decision rather than an accident, and so a future change that starts
    /// dropping here has to change this test deliberately. A **bounded** PES
    /// has no such tradeoff — its declared length settles it, which
    /// [`cc_jump_at_a_pes_start_keeps_the_previous_complete_access_unit`] and
    /// [`cc_gap_on_a_continuation_drops_only_that_access_unit`] cover.
    #[test]
    fn unbounded_unit_ended_by_a_counter_restart_is_delivered_by_design() {
        let au1 = data_au(0x20);
        let au2 = data_au(0xA0);

        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_PES_PRIVATE),
        ));

        // Unit 1's packet 1 is lost; unit 2 starts on a counter that therefore
        // jumps. Unbounded units, so nothing proves unit 1 was short.
        let pes1 = unbounded_pes(ES_STREAM_ID_PRIVATE_DATA, &au1);
        let pkt1 = pes_packets(ES_PID_UNDER_TEST, 0, &pes1);
        input.extend_from_slice(&pkt1[..TS_PACKET_SIZE]);
        let pes2 = unbounded_pes(ES_STREAM_ID_PRIVATE_DATA, &au2);
        input.extend_from_slice(&pes_packets(ES_PID_UNDER_TEST, 2, &pes2));

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let got: Vec<Vec<u8>> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => Some(sample.data.to_vec()),
                _ => None,
            })
            .collect();
        assert_eq!(
            got.len(),
            2,
            "both the truncated-tail unit and the whole one are delivered: an              unbounded PES gives no way to tell them apart at a seam"
        );
    }

    // ── r04-W48: a PES with no PTS/DTS is interpolated, not re-stamped ─────

    /// `PTS_DTS_flags == '00'` is legal (ISO/IEC 13818-1 §2.4.3.7) and the
    /// 2.7.4 interval constraint only requires the stamps periodically, so a
    /// video PID's access units routinely alternate stamped / unstamped. The
    /// demux used to hand an unstamped access unit the *previous* one's
    /// stamps verbatim, so every consecutive pair carried an identical `dts`:
    /// the one-behind duration rule then gave the earlier of the pair
    /// `duration = 0` and made the next stamped access unit absorb the whole
    /// gap, producing zero-duration samples plus one long one.
    ///
    /// Real elementary-stream bytes: the H.264 access units are the real
    /// `Media` samples recovered from `fixtures/ts/h264_aac.ts` and re-emitted
    /// through the crate's own `TsMux` PES packetiser, with the timing flags
    /// of every second PES cleared. The frame period is a real one (the
    /// fixture's own 25 fps cadence, 3600 ticks of 90 kHz).
    #[test]
    fn unstamped_pes_access_unit_is_interpolated_not_restamped() {
        // Real H.264 access units from a committed capture.
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("h264_aac.ts");
        let source =
            std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
        let media = TsDemux::new().demux(&source).expect("demux h264_aac.ts");

        /// Frame period of the fixture's video track, in 90 kHz ticks — the
        /// real cadence `TsMux` stamps its PES headers from.
        const FRAME_PERIOD: i64 = 3600;
        const UNSTAMPED_EVERY: usize = 2;

        let mut cc = 0u8;
        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_AVC),
        ));

        let video = media
            .tracks
            .iter()
            .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
            .expect("h264_aac.ts has an AVC track");
        let source_dts0 = video.samples[0]
            .dts
            .expect("the fixture's AVC track is stamped");
        let mut headers = 0usize;
        for (i, sample) in video.samples.iter().enumerate() {
            // The demux wants Annex B in each access unit; `TsMux` writes
            // length-prefixed samples, so rebuild the Annex B form from the
            // sample's own NALs.
            let mut au = Vec::new();
            let nals = crate::annexb::iter_length_prefixed_nals(&sample.data)
                .expect("TsMux writes valid 4-byte-length NAL prefixes");
            for nal in nals {
                au.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                au.extend_from_slice(nal);
            }
            // A constant, decode-ordered 90 kHz clock: the fixture's real
            // cadence (3600 ticks per access unit), anchored at its own first
            // dts. Both stamps are present on a stamped PES and neither is on
            // an unstamped one — the exact `PTS_DTS_flags` '11' / '00'
            // alternation §2.4.3.7 permits.
            // The first two access units must be stamped: the period is only
            // measurable from a stamped pair, and an unstamped access unit
            // arriving before then has nothing to interpolate from (the
            // documented fallback, exercised separately below).
            let stamped = i < UNSTAMPED_EVERY || i % UNSTAMPED_EVERY == 0;
            let dts = source_dts0 + i as i64 * FRAME_PERIOD;
            let pes = if stamped {
                pes_packet(ES_STREAM_ID_VIDEO, dts, Some(dts), &au)
            } else {
                pes_packet_untimed(ES_STREAM_ID_VIDEO, &au)
            };
            headers += 1;
            let packets = pes_packets(ES_PID_UNDER_TEST, cc, &pes);
            cc = cc.wrapping_add((packets.len() / TS_PACKET_SIZE) as u8);
            input.extend_from_slice(&packets);
        }
        assert!(
            video.samples.len() >= 20,
            "fixture too small for a meaningful interpolation test"
        );

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let samples: Vec<Sample> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => Some(sample),
                _ => None,
            })
            .collect();
        assert_eq!(
            samples.len(),
            headers,
            "one sample per fed access unit (video is one AU per PES here)"
        );

        // The dts series must advance by exactly one frame period per access
        // unit. The pre-fix behaviour gave every unstamped access unit the
        // previous one's dts, so the series alternated 0 / 3600 steps.
        let dts: Vec<i64> = samples.iter().map(|s| s.dts.expect("video dts")).collect();
        let steps: Vec<i64> = dts.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            steps.iter().all(|&d| d == FRAME_PERIOD),
            "every decode step must be one frame period ({FRAME_PERIOD} ticks); got {steps:?}"
        );
        assert!(
            samples
                .iter()
                .all(|s| s.duration == Some(FRAME_PERIOD as u32)),
            "no zero-duration sample may survive interpolation; durations: {:?}",
            samples.iter().map(|s| s.duration).collect::<Vec<_>>()
        );
    }

    // ── Audio re-anchor threshold (issue B5) ───────────────────────────────

    /// One 44.1 kHz stereo AAC-LC access unit: a real ADTS header (built by
    /// this crate's own `aac_asc::build_adts_header`, ISO/IEC 13818-7 §6.2,
    /// `sampling_frequency_index = 4` = 44100 Hz) plus filler payload. Content
    /// is irrelevant here — this test is about the timestamp anchor, and
    /// `emit_audio_au` only needs `split_adts_frames` to find the frame.
    fn aac_44100_access_unit() -> Vec<u8> {
        /// AAC-LC: `profile = audio_object_type - 1 = 1`.
        const ADTS_PROFILE_AAC_LC: u8 = 1;
        /// `sampling_frequency_index` for 44100 Hz (ISO/IEC 14496-3 Table 1.16).
        const SFI_44100: u8 = 4;
        /// `channel_configuration` = 2 (stereo).
        const CHANNELS_STEREO: u8 = 2;
        const PAYLOAD_BYTES: usize = 128;

        let frame_len = (ADTS_HEADER_SIZE + PAYLOAD_BYTES) as u16;
        let header = crate::aac_asc::build_adts_header(
            ADTS_PROFILE_AAC_LC,
            SFI_44100,
            CHANNELS_STEREO,
            frame_len,
        );
        let mut au = header.to_vec();
        au.resize(ADTS_HEADER_SIZE + PAYLOAD_BYTES, 0x21);
        au
    }

    /// PROVENANCE: synthesised, deliberately. The case under test is a
    /// **constant integer PES increment** at 44.1 kHz, and no committed
    /// capture here carries one — `fixtures/ts/h264_aac.ts` is 48 kHz, where
    /// 1024 samples is exactly 1920 ticks of 90 kHz and this class of drift
    /// cannot occur at all. The ADTS frames come from the crate's own
    /// spec-correct header builder, not hand-written bytes.
    ///
    /// The bug (issue B5 follow-up): the threshold was one intrinsic sample
    /// period — 3 ticks at 44.1 kHz — while `1024/44100 s` is `2089.795…`
    /// ticks, so a muxer stamping the rounded constant `2090` drifts `+0.204…`
    /// ticks per frame *on a perfectly continuous stream* and crossed the
    /// threshold roughly every 15 frames. `TimelineReanchored` was pure noise
    /// and the anchor was effectively inert.
    #[test]
    fn constant_increment_44100_aac_emits_no_timeline_reanchor() {
        /// What a muxer that rounds `1024 * 90000 / 44100` to an integer emits.
        const PES_INCREMENT_TICKS: i128 = 2090;
        const SAMPLE_RATE: u32 = 44_100;
        /// ~11.6 s of audio. Accumulated drift here is ~102 ticks: far past
        /// the old 3-tick threshold (which would have fired ~34 times), far
        /// short of the 1800-tick (20 ms) bound the fix derives.
        const FRAMES: i128 = 500;

        let au = aac_44100_access_unit();
        let mut anchor = AudioAnchor::default();
        let mut events: VecDeque<DemuxEvent> = VecDeque::new();
        for n in 0..FRAMES {
            let ts = n * PES_INCREMENT_TICKS;
            emit_audio_au(
                &AudioKind::Aac,
                SAMPLE_RATE,
                &mut anchor,
                &au,
                ts,
                ts,
                1,
                &mut events,
            );
        }

        assert!(
            events
                .iter()
                .any(|e| matches!(e, DemuxEvent::Sample { .. })),
            "sanity: the synthesised ADTS frames must actually split into samples"
        );
        let reanchors = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    DemuxEvent::Discontinuity {
                        kind: DiscontinuityKind::TimelineReanchored,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            reanchors, 0,
            "a constant-increment 44.1 kHz stream is continuous — the \
             rounding drift a real muxer accrues by construction must never be \
             reported as a discontinuity"
        );
    }

    /// The other half of the threshold contract: a **genuine** timeline gap
    /// (an encoder restart / splice) must still be reported, exactly once.
    #[test]
    fn a_real_timeline_gap_emits_exactly_one_reanchor() {
        const PES_INCREMENT_TICKS: i128 = 2090;
        const SAMPLE_RATE: u32 = 44_100;
        const FRAMES_BEFORE: i128 = 50;
        const FRAMES_AFTER: i128 = 50;
        /// Two seconds of 90 kHz — orders of magnitude past any muxer drift.
        const GAP_TICKS: i128 = 180_000;

        let au = aac_44100_access_unit();
        let mut anchor = AudioAnchor::default();
        let mut events: VecDeque<DemuxEvent> = VecDeque::new();
        let emit = |ts: i128, anchor: &mut AudioAnchor, events: &mut VecDeque<DemuxEvent>| {
            emit_audio_au(&AudioKind::Aac, SAMPLE_RATE, anchor, &au, ts, ts, 1, events);
        };
        for n in 0..FRAMES_BEFORE {
            emit(n * PES_INCREMENT_TICKS, &mut anchor, &mut events);
        }
        let resume = FRAMES_BEFORE * PES_INCREMENT_TICKS + GAP_TICKS;
        for n in 0..FRAMES_AFTER {
            emit(resume + n * PES_INCREMENT_TICKS, &mut anchor, &mut events);
        }

        let reanchors = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    DemuxEvent::Discontinuity {
                        kind: DiscontinuityKind::TimelineReanchored,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            reanchors, 1,
            "one genuine gap must produce exactly one TimelineReanchored — not \
             zero (the anchor silently absorbing a real splice) and not one \
             per following access unit"
        );
    }

    /// `split_adts_frames` must resync across PES payloads that don't start
    /// on a frame sync (issue #638 — the same defect reported for MP2, see
    /// `mpeg_legacy.rs`'s `mpeg_audio_resyncs_across_pes_boundaries`, applies
    /// identically to ADTS). Builds a real ADTS elementary stream from the
    /// real captured AAC frames in `fixtures/ts/h264_aac.ts` (re-synthesizing
    /// each frame's ADTS header from the track's real recovered config, the
    /// same way [`build_es_payload`] does for muxing), then re-chunks it at a
    /// fixed size that does not align to any real AAC frame length.
    #[test]
    fn adts_resyncs_across_pes_boundaries() {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("h264_aac.ts");
        let ts_bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));

        let mut demux = TsDemux::new();
        let media = demux.demux(&ts_bytes).expect("demux h264_aac.ts");
        let audio = media
            .tracks
            .iter()
            .find(|t| matches!(t.config(), CodecConfig::Aac { .. }))
            .expect("h264_aac.ts has an AAC track");
        let esds = match audio.config() {
            CodecConfig::Aac { esds, .. } => esds,
            _ => unreachable!(),
        };
        let dsi = esds
            .es_descriptor
            .decoder_config
            .as_ref()
            .and_then(|dc| dc.decoder_specific_info.as_ref())
            .expect("AAC esds carries a DecoderSpecificInfo");
        let asc = AudioSpecificConfig::parse(&dsi.data).expect("parse real AudioSpecificConfig");

        // Real ADTS ES: real captured AAC frame bytes, each with a freshly
        // synthesized (but spec-correct, config-derived) ADTS header.
        let mut es: Vec<u8> = Vec::new();
        for s in &audio.samples {
            let frame_len = (ADTS_HEADER_SIZE + s.data.len()) as u16;
            let hdr = asc
                .to_adts_header(frame_len)
                .expect("build real ADTS header");
            es.extend_from_slice(&hdr);
            es.extend_from_slice(&s.data);
        }
        assert!(
            audio.samples.len() >= 10,
            "fixture too small to be a meaningful resync test"
        );

        // Re-chunk at a fixed size (a realistic broadcast audio PES payload
        // size, several frames' worth) with no relation to any real AAC
        // frame length (~292 bytes average here), so PES payload boundaries
        // land mid-frame (issue #638).
        const CHUNK: usize = 2000;
        let mut recovered = 0usize;
        let mut off = 0usize;
        while off < es.len() {
            let end = (off + CHUNK).min(es.len());
            recovered += split_adts_frames(&es[off..end]).len();
            off = end;
        }

        // Before the #638-style fix, `split_adts_frames` bails at the first
        // byte that isn't a syncword and never resyncs, so only the chunks
        // that happen to start exactly on a frame boundary by chance yield
        // anything -- effectively none, for a chunk size unrelated to frame
        // length. Resync must recover the large majority of the real frames.
        assert!(
            recovered * 2 >= audio.samples.len(),
            "resync must recover most real AAC frames across misaligned \
             chunks (got {recovered} of {} real frames)",
            audio.samples.len()
        );
    }

    /// Loads the real AudioSpecificConfig recovered from `fixtures/ts/h264_aac.ts`
    /// plus its real per-frame AAC payloads, for building real (but
    /// re-headered) ADTS test frames — same real source `adts_resyncs_across_pes_boundaries`
    /// uses.
    fn real_aac_asc_and_samples() -> (AudioSpecificConfig, Vec<Vec<u8>>) {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("h264_aac.ts");
        let ts_bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
        let mut demux = TsDemux::new();
        let media = demux.demux(&ts_bytes).expect("demux h264_aac.ts");
        let audio = media
            .tracks
            .into_iter()
            .find(|t| matches!(t.config(), CodecConfig::Aac { .. }))
            .expect("h264_aac.ts has an AAC track");
        let esds = match audio.config() {
            CodecConfig::Aac { esds, .. } => esds.clone(),
            _ => unreachable!(),
        };
        let dsi = esds
            .es_descriptor
            .decoder_config
            .as_ref()
            .and_then(|dc| dc.decoder_specific_info.as_ref())
            .expect("AAC esds carries a DecoderSpecificInfo")
            .clone();
        let asc = AudioSpecificConfig::parse(&dsi.data).expect("parse real AudioSpecificConfig");
        let samples: Vec<Vec<u8>> = audio.samples.iter().map(|s| s.data.to_vec()).collect();
        (asc, samples)
    }

    /// C11 (#1012): a CRC-protected ADTS frame (`protection_absent == 0`) is
    /// 9 bytes of header (fixed+variable header, 7, plus `adts_error_check()`'s
    /// 16-bit `crc_check`, 2 — ISO/IEC 13818-7 §6.2) before the raw data
    /// block. No local tool can generate a real CRC-protected ADTS stream:
    /// ffmpeg's `adts` muxer has no CRC-write option (`ffmpeg -h muxer=adts`
    /// lists none) and always emits `protection_absent = 1`; TSDuck doesn't
    /// encode audio at all. Per the fixture-first rule, this is hand-built
    /// directly from that spec clause's field layout, using the crate's own
    /// spec-cited `build_adts_header`/`parse_adts_header` for every OTHER
    /// field (profile/sfi/channels/frame_length) and only overriding
    /// `protection_absent` — the real payload bytes are the genuine captured
    /// AAC frames from `fixtures/ts/h264_aac.ts`.
    ///
    /// Before the fix, `emit_audio_au` always stripped a fixed 7 bytes, so
    /// the emitted sample began with the 2 CRC bytes glued in front of the
    /// real payload — this test's oracle is the exact real payload bytes
    /// before they were wrapped, so that leak is directly visible as a
    /// length/content mismatch.
    #[test]
    fn adts_crc_protected_frame_strips_crc_not_sample_data() {
        let (asc, samples) = real_aac_asc_and_samples();
        assert!(samples.len() >= 2, "fixture must carry several AAC frames");

        let mut events: VecDeque<DemuxEvent> = VecDeque::new();
        let mut anchor = AudioAnchor::default();
        let mut dts_uw: i128 = 0;
        for payload in samples.iter().take(5) {
            // ADTS_CRC_SIZE(2) accounted in frame_length, per §6.2's
            // frame_length = "length of this ADTS frame including headers
            // and error_check in bytes".
            let frame_len = (ADTS_HEADER_SIZE + ADTS_CRC_SIZE + payload.len()) as u16;
            let mut header = asc.to_adts_header(frame_len).expect("build ADTS header");
            header[1] &= !0x01; // protection_absent = 0 (CRC present)
            let mut au = header.to_vec();
            au.extend_from_slice(&[0x00, 0x00]); // crc_check (value irrelevant here)
            au.extend_from_slice(payload);

            emit_audio_au(
                &AudioKind::Aac,
                44_100,
                &mut anchor,
                &au,
                dts_uw as u64 as i128,
                dts_uw,
                1,
                &mut events,
            );
            dts_uw += AAC_SAMPLES_PER_FRAME as i128;
        }

        let emitted: Vec<Vec<u8>> = events
            .iter()
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => Some(sample.data.to_vec()),
                _ => None,
            })
            .collect();
        assert_eq!(emitted.len(), 5, "one sample per CRC-protected ADTS frame");
        for (i, (got, want)) in emitted.iter().zip(samples.iter().take(5)).enumerate() {
            assert_eq!(
                got, want,
                "sample {i}: CRC bytes must not leak into (or truncate) the real payload"
            );
        }
    }

    /// C11 (#1012): `number_of_raw_data_blocks_in_frame > 0` (a legal,
    /// if rare, ADTS encoding — several AAC frames packed into one ADTS
    /// frame) must scale the emitted sample's duration by the number of
    /// raw data blocks, not always assume one. Same real-data provenance
    /// and same "no local tool" note as the CRC test above (ffmpeg's own
    /// AAC encoders never emit `number_of_raw_data_blocks_in_frame > 0`
    /// either); two real captured AAC frames are concatenated into a single
    /// ADTS frame's raw-data area to build a real (if hand-assembled at the
    /// framing level) 2-block frame.
    #[test]
    fn adts_multi_raw_data_block_duration_is_multiplied() {
        let (asc, samples) = real_aac_asc_and_samples();
        assert!(samples.len() >= 2, "fixture must carry several AAC frames");

        let mut combined = samples[0].clone();
        combined.extend_from_slice(&samples[1]);
        let frame_len = (ADTS_HEADER_SIZE + combined.len()) as u16;
        let mut header = asc.to_adts_header(frame_len).expect("build ADTS header");
        header[6] |= 0x01; // number_of_raw_data_blocks_in_frame = 1 (2 blocks)
        let mut au = header.to_vec();
        au.extend_from_slice(&combined);

        let mut events: VecDeque<DemuxEvent> = VecDeque::new();
        let mut anchor = AudioAnchor::default();
        emit_audio_au(
            &AudioKind::Aac,
            44_100,
            &mut anchor,
            &au,
            0,
            0,
            1,
            &mut events,
        );

        let emitted: Vec<(Vec<u8>, Option<u32>)> = events
            .iter()
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => Some((sample.data.to_vec(), sample.duration)),
                _ => None,
            })
            .collect();
        assert_eq!(
            emitted.len(),
            1,
            "the whole 2-raw-data-block frame is one emitted sample"
        );
        assert_eq!(
            emitted[0].0, combined,
            "both raw data blocks' real bytes must be present, undamaged"
        );
        assert_eq!(
            emitted[0].1,
            Some(AAC_SAMPLES_PER_FRAME * 2),
            "duration must be (num_raw_data_blocks + 1) * AAC_SAMPLES_PER_FRAME, not just 1024"
        );
    }

    /// Every DVB descriptor-disambiguated `stream_type` (`0x06`/`0x15`) with
    /// a recognised Dolby/DTS descriptor reclassifies from opaque data to the
    /// matching audio codec (issue #641). The end-to-end E-AC-3 case (real
    /// captured syncframes, full PMT-parse-through-sample-recovery path) is
    /// covered by `transmux/tests/dolby.rs`'s
    /// `dvb_0x06_enhanced_ac3_descriptor_classifies_as_eac3`; this covers the
    /// other two descriptor tags at the classification-function level.
    #[test]
    fn refine_with_descriptors_recognises_every_dolby_dts_tag() {
        let ac3_desc = [DESC_TAG_AC3, 1, 0x40];
        assert_eq!(
            Codec::Data(STREAM_TYPE_PES_PRIVATE)
                .refine_with_descriptors(STREAM_TYPE_PES_PRIVATE, &ac3_desc),
            Codec::Ac3
        );

        let dts_desc = [DESC_TAG_DTS, 1, 0x00];
        assert_eq!(
            Codec::Data(STREAM_TYPE_METADATA_PES)
                .refine_with_descriptors(STREAM_TYPE_METADATA_PES, &dts_desc),
            Codec::Dts,
            "0x15 (metadata in PES) is also descriptor-disambiguated"
        );

        let eac3_desc = [DESC_TAG_ENHANCED_AC3, 1, 0x00];
        assert_eq!(
            Codec::Data(STREAM_TYPE_PES_PRIVATE)
                .refine_with_descriptors(STREAM_TYPE_PES_PRIVATE, &eac3_desc),
            Codec::Eac3
        );
    }

    /// A `0x06`/`0x15` stream with no Dolby/DTS descriptor (e.g. DVB
    /// subtitles, tag `0x59`) must stay opaque data, not be misclassified as
    /// audio.
    #[test]
    fn refine_with_descriptors_leaves_non_audio_0x06_as_data() {
        const DESC_TAG_SUBTITLING: u8 = 0x59;
        let subtitle_desc = [DESC_TAG_SUBTITLING, 3, 0x65, 0x6E, 0x67];
        assert_eq!(
            Codec::Data(STREAM_TYPE_PES_PRIVATE)
                .refine_with_descriptors(STREAM_TYPE_PES_PRIVATE, &subtitle_desc),
            Codec::Data(STREAM_TYPE_PES_PRIVATE)
        );
    }

    /// A `stream_type` outside the descriptor-disambiguated set (`0x06`/
    /// `0x15`) must never be reclassified, even if its ES_info descriptor
    /// loop happens to contain a Dolby/DTS tag byte -- the descriptor scan
    /// only applies to the two `stream_type`s DVB actually disambiguates this
    /// way.
    #[test]
    fn refine_with_descriptors_ignores_other_stream_types() {
        let eac3_desc = [DESC_TAG_ENHANCED_AC3, 1, 0x00];
        assert_eq!(
            Codec::H264.refine_with_descriptors(STREAM_TYPE_AVC, &eac3_desc),
            Codec::H264
        );
    }

    /// F1: a PID declared by two PMTs (an ordinary shared audio/subtitle
    /// component across programs in a DVB multiplex) must not be torn down
    /// just because *one* declaring PMT reclassifies its codec while the
    /// other program's declaration is unchanged — `apply_pmt_diff`'s
    /// codec-changed branch must consult the same `es_declarers` refcount the
    /// "removed" branch already does. Must fail before the fix: without the
    /// check, `remove_track` ran unconditionally on a codec change,
    /// destroying the shared track (new `track_id`, spurious
    /// `TrackRemoved`/`TrackAdded`) even though the other program's
    /// `applied_es` still lists it.
    ///
    /// Also exercises the decided conflict policy for the two-programs/
    /// different-codecs case (documented on the fix): reclassification is
    /// refused while any other declarer remains, and only proceeds once this
    /// PMT is the *last* declarer.
    #[test]
    fn codec_change_on_shared_pid_does_not_tear_down_other_program_track() {
        const PMT_A: u16 = 0x1000;
        const PMT_B: u16 = 0x1001;
        const SHARED_PID: u16 = 0x0050;
        const STREAM_TYPE: u8 = 0x7F; // opaque data, PES-carried (see `data_carriage`)

        let mut demux = StreamingTsDemux::new();

        // PMT A declares the shared PID; PMT B declares it too (same codec).
        demux.apply_pmt_diff(
            PMT_A,
            &BTreeSet::new(),
            alloc::vec![(SHARED_PID, Codec::Data(STREAM_TYPE), Vec::new())],
        );
        demux.apply_pmt_diff(
            PMT_B,
            &BTreeSet::new(),
            alloc::vec![(SHARED_PID, Codec::Data(STREAM_TYPE), Vec::new())],
        );
        assert_eq!(
            demux.es_declarers.get(&SHARED_PID).map(|d| d.len()),
            Some(2),
            "both PMTs must be recorded as declarers of the shared PID"
        );

        // Promote it straight to `Live`, mirroring what real config recovery
        // would do: `ConfigProbe::Data` resolves on the very first access
        // unit (it needs no in-band header at all), so this is a faithful
        // shortcut, not a fabricated state.
        let carriage = data_carriage(STREAM_TYPE);
        assert_eq!(carriage, DataCarriage::Pes);
        demux.streams.get_mut(&SHARED_PID).unwrap().track = Some(TrackState::Parked {
            config: CodecConfig::Data {
                stream_type: STREAM_TYPE,
                descriptors: Vec::new(),
                carriage,
            },
            timescale: VIDEO_TIMESCALE,
            kind: LiveKind::Data {
                pending: None,
                last_duration: 0,
            },
            backlog: Vec::new(),
        });
        demux.try_promote_ready();
        let track_id_before = match demux.streams.get(&SHARED_PID).unwrap().track.as_ref() {
            Some(TrackState::Live(live)) => live.track_id,
            _ => panic!("expected the shared PID to be Live after promotion"),
        };
        while demux.poll_event().is_some() {} // drain TrackAdded — not under test here

        // PMT A reclassifies the PID's codec. PMT B's `applied_es` (the
        // diff baseline passed in on its own behalf) still lists the PID
        // unchanged — this call only ever represents PMT A's own view.
        let mut pmt_a_applied = BTreeSet::new();
        pmt_a_applied.insert(SHARED_PID);
        demux.apply_pmt_diff(
            PMT_A,
            &pmt_a_applied,
            alloc::vec![(SHARED_PID, Codec::Ac3, Vec::new())],
        );

        // The shared track must survive, unchanged, with its original
        // track_id — PMT B still declares it, so PMT A's reclassification
        // alone must not tear it down.
        match demux
            .streams
            .get(&SHARED_PID)
            .and_then(|s| s.track.as_ref())
        {
            Some(TrackState::Live(live)) => assert_eq!(
                live.track_id, track_id_before,
                "shared track must keep its original track_id"
            ),
            _ => {
                panic!("expected the shared track to survive PMT A's reclassification, still Live")
            }
        }
        assert!(
            !demux
                .events
                .iter()
                .any(|ev| matches!(ev, DemuxEvent::TrackRemoved { .. })),
            "PMT A's reclassification must not remove a track PMT B still declares"
        );
        assert_eq!(
            demux.streams.get(&SHARED_PID).unwrap().codec,
            Codec::Data(STREAM_TYPE),
            "the existing classification wins while another declarer disagrees"
        );

        // Now PMT B drops its declaration entirely — PMT A becomes the sole
        // (last) declarer.
        let mut pmt_b_applied = BTreeSet::new();
        pmt_b_applied.insert(SHARED_PID);
        demux.apply_pmt_diff(PMT_B, &pmt_b_applied, Vec::new());
        assert_eq!(
            demux.es_declarers.get(&SHARED_PID).map(|d| d.len()),
            Some(1),
            "PMT A must be the sole remaining declarer"
        );

        // PMT A reclassifies again: as the *last* declarer, the
        // teardown-and-rebuild now proceeds.
        demux.apply_pmt_diff(
            PMT_A,
            &pmt_a_applied,
            alloc::vec![(SHARED_PID, Codec::Aac, Vec::new())],
        );
        assert!(
            demux
                .events
                .iter()
                .any(|ev| matches!(ev, DemuxEvent::TrackRemoved { .. })),
            "once PMT A is the last declarer, its reclassification must actually tear down \
             the old track"
        );
        assert_eq!(demux.streams.get(&SHARED_PID).unwrap().codec, Codec::Aac);
    }

    /// F3: re-registering a PID after a codec-changed teardown (issue F1's
    /// `remove_track` + `register_new_es_at` pair) must preserve its original
    /// PMT-declaration-order slot, not lose it to the back of `codec_order`/
    /// `data_order` — the order backs `TrackAdded` emission order and gates
    /// promotion (`try_promote_ready`), so losing the slot reorders both.
    /// Must fail before the fix (the old `register_new_es` always appended).
    #[test]
    fn codec_change_reregistration_preserves_declaration_order_slot() {
        const PMT: u16 = 0x1000;
        const PID_X: u16 = 0x0050;
        const PID_Y: u16 = 0x0051;

        let mut demux = StreamingTsDemux::new();
        demux.apply_pmt_diff(
            PMT,
            &BTreeSet::new(),
            alloc::vec![
                (PID_X, Codec::Data(0x06), Vec::new()),
                (PID_Y, Codec::Data(0x07), Vec::new()),
            ],
        );
        assert_eq!(demux.data_order, alloc::vec![PID_X, PID_Y]);

        // PID X's stream_type changes (still opaque `Codec::Data`, so the
        // codec-changed teardown path runs) while PID Y is untouched.
        let mut old_applied = BTreeSet::new();
        old_applied.insert(PID_X);
        old_applied.insert(PID_Y);
        demux.apply_pmt_diff(
            PMT,
            &old_applied,
            alloc::vec![
                (PID_X, Codec::Data(0x08), Vec::new()),
                (PID_Y, Codec::Data(0x07), Vec::new()),
            ],
        );

        assert_eq!(
            demux.data_order,
            alloc::vec![PID_X, PID_Y],
            "PID X must keep its original (first) declaration-order slot, not move to the back"
        );
        assert_eq!(demux.streams.get(&PID_X).unwrap().codec, Codec::Data(0x08));
    }

    // ── InputDegradation tests (issue #778) ─────────────────────────────────

    /// A valid TS null packet (PID 0x1FFF, AFC=01, CC=0).
    fn null_packet() -> [u8; TS_PACKET_SIZE] {
        let mut p = [0xFFu8; TS_PACKET_SIZE];
        p[0] = 0x47;
        p[1] = 0x1F;
        p[2] = 0xFF;
        p[3] = 0x10;
        p
    }

    /// Helper: build a TS packet with explicit tei, pusi, pid, cc, adaptation
    /// field, and payload.
    fn ts_packet_degradation(
        tei: bool,
        pid: u16,
        cc: u8,
        afc: u8,
        adaptation: Option<&[u8]>,
        payload: &[u8],
    ) -> [u8; TS_PACKET_SIZE] {
        let mut p = [0xFFu8; TS_PACKET_SIZE];
        p[0] = 0x47;
        p[1] = (if tei { 0x80 } else { 0x00 }) | ((pid >> 8) as u8 & PID_HI_MASK);
        p[2] = (pid & 0xFF) as u8;
        p[3] = afc | (cc & 0x0F);

        let mut off = 4usize;
        if let Some(af) = adaptation {
            let af_len = af.len() as u8;
            p[off] = af_len;
            off += 1;
            p[off..off + af.len()].copy_from_slice(af);
            off += af.len();
        }
        // Copy payload into remaining space.
        let payload_end = off + payload.len().min(TS_PACKET_SIZE - off);
        p[off..payload_end].copy_from_slice(&payload[..payload_end - off]);
        p
    }

    // ── TEI test ────────────────────────────────────────────────────────────

    #[test]
    fn tei_set_packet_emits_transport_error() {
        let mut demux = StreamingTsDemux::new();
        // Bootstrap TsResync lock with enough null packets.
        let null = null_packet();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            demux.feed(&null);
        }
        let pkt = ts_packet_degradation(
            true, // tei
            0x0100,
            0,
            0x10, // AFC=01 (payload only)
            None,
            b"some payload",
        );
        demux.feed(&pkt);
        let events: Vec<_> = std::iter::from_fn(|| demux.poll_event()).collect();
        let degraded: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                DemuxEvent::InputDegraded {
                    kind, provenance, ..
                } => Some((*kind, *provenance)),
                _ => None,
            })
            .collect();
        assert_eq!(degraded.len(), 1, "expected exactly one InputDegraded");
        assert_eq!(degraded[0].0, InputDegradation::TransportError);
        assert_eq!(degraded[0].1.pid, Some(0x0100));
        // packet_index includes the bootstrap null packets.
        assert_eq!(
            degraded[0].1.packet_index,
            Some(mpeg_ts::resync::LOCK_CONFIRMATIONS as u64 + 1)
        );
    }

    // ── Clean fixture: h264_aac.ts produces zero InputDegraded ───────────────

    #[test]
    fn h264_aac_clean_fixture_produces_zero_input_degraded() {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("h264_aac.ts");
        let data = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));

        let mut demux = StreamingTsDemux::new();
        demux.feed(&data);
        demux.finish();
        let degraded: Vec<_> = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| matches!(e, DemuxEvent::InputDegraded { .. }))
            .collect();
        assert!(
            degraded.is_empty(),
            "h264_aac.ts must produce zero InputDegraded events, got {degraded:?}"
        );
    }

    // ── m6-discontinuity.ts smoke ───────────────────────────────────────────

    /// The m6-discontinuity fixture is a real, lossy capture — it has genuine
    /// CC gaps AND signalled discontinuities. This test asserts the gap count
    /// matches media-doctor's CcAnomalyCheck (877) — the two must agree.
    #[test]
    fn m6_discontinuity_fixture_gap_count_matches_media_doctor() {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("m6-discontinuity.ts");
        let data = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));

        let mut demux = StreamingTsDemux::new();
        demux.feed(&data);
        demux.finish();

        let gap_count = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| {
                matches!(
                    e,
                    DemuxEvent::InputDegraded {
                        kind: InputDegradation::ContinuityGap { .. },
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            gap_count, 877,
            "m6-discontinuity.ts must produce exactly 877 ContinuityGap events \
             (matching media-doctor CcAnomalyCheck); any divergence means the \
             exclusion rules disagree with the two in-repo reference \
             implementations"
        );
    }

    /// The same fixture — CC gaps AND signalled discontinuities. This test
    /// just confirms the fixture plays through without panicking and that
    /// signalled discontinuities are still observed as `Discontinuity` events.
    #[test]
    fn m6_discontinuity_fixture_plays_through() {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("..");
        path.push("fixtures");
        path.push("ts");
        path.push("m6-discontinuity.ts");
        let data = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));

        let mut demux = StreamingTsDemux::new();
        demux.feed(&data);
        demux.finish();

        let mut saw_discontinuity = false;
        while let Some(event) = demux.poll_event() {
            if matches!(event, DemuxEvent::Discontinuity { .. }) {
                saw_discontinuity = true;
            }
        }
        assert!(
            saw_discontinuity,
            "m6-discontinuity.ts must produce at least one Discontinuity event"
        );
    }

    // ── Legal duplicate: same CC + identical payload emits nothing ──────────

    #[test]
    fn legal_duplicate_same_cc_and_identical_payload_emits_nothing() {
        let mut demux = StreamingTsDemux::new();
        // Bootstrap TsResync lock.
        let null = null_packet();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            demux.feed(&null);
        }
        let payload = b"duplicate payload bytes";
        let pkt = |cc: u8| -> [u8; TS_PACKET_SIZE] {
            let mut p = [0xFFu8; TS_PACKET_SIZE];
            p[0] = 0x47;
            p[1] = 0x00; // PID=0
            p[2] = 0x31; // PID=0x0031
            p[3] = 0x10 | (cc & 0x0F); // AFC=01
            let end = 4 + payload.len().min(TS_PACKET_SIZE - 4);
            p[4..end].copy_from_slice(&payload[..end - 4]);
            p
        };

        // First packet: CC=0.
        demux.feed(&pkt(0));
        // Second packet: legal duplicate — same CC=0, identical payload.
        demux.feed(&pkt(0));

        let degraded: Vec<_> = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| matches!(e, DemuxEvent::InputDegraded { .. }))
            .collect();
        assert!(
            degraded.is_empty(),
            "legal duplicate must not emit InputDegraded, got {degraded:?}"
        );
    }

    // ── r04-W49: duplicates are not reassembled; gap AUs are dropped ────────

    /// ITU-T H.222.0 §2.4.3.3: "In transport streams, duplicate packets may be
    /// sent as two, and only two, consecutive transport stream packets of the
    /// same PID… In duplicate packets each byte of the original packet shall be
    /// duplicated, with the exception that in the program clock reference
    /// fields, if present, a valid value shall be encoded." The decoder shall
    /// discard the duplicate — the demux used to detect it for the CC check and
    /// then feed it to the reassembler anyway, so the PES contained those 184
    /// bytes twice and every access unit was corrupt.
    ///
    /// Real packet layout: a PES header + `PES_packet_length` spanning two TS
    /// packets on one PID, each sent twice (the second copy re-encoding its PCR
    /// so the pair is a legal duplicate only under the byte-compare-except-PCR
    /// rule the shared `ts_dup` helper implements). The delivered sample must be
    /// one whole PES payload, with no byte repeated.
    #[test]
    fn legal_duplicate_packets_are_not_reassembled_twice() {
        /// A private-PES data stream (`stream_type` 0x06) carried as one
        /// `Sample` per PES — the simplest carrier for a byte-exact check.
        const DATA_BYTES: usize = 1000;

        // Two TS payloads' worth of PES: 4-byte start code + length + flags +
        // `PES_header_data_length` 0, then the payload — long enough to span
        // two packets so a duplicated continuation really would append twice.
        let body: Vec<u8> = (0..DATA_BYTES).map(|i| (i & 0xFF) as u8).collect();
        let pes_len = (3 + body.len()) as u16;
        let mut pes = Vec::new();
        pes.extend_from_slice(&[0x00, 0x00, 0x01, 0xBD]);
        pes.extend_from_slice(&pes_len.to_be_bytes());
        pes.push(0x80); // '10' marker, not scrambled
        pes.push(0x00); // PTS_DTS_flags = '00'
        pes.push(0x00); // PES_header_data_length = 0
        pes.extend_from_slice(&body);
        assert!(
            pes.len() > TS_MAX_PAYLOAD_BYTES,
            "the PES must span more than one TS packet for this test to mean anything"
        );

        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_PES_PRIVATE),
        ));

        // Every packet of the PES is followed by its legal §2.4.3.3 duplicate
        // (same CC, same bytes). The duplicate of the *first* packet is the
        // decisive one: it must not be appended to the access unit in
        // progress, or the payload gains 184 spurious bytes in the middle.
        let mut cc = 0u8;
        let mut sent = 0usize;
        let mut dups = 0usize;
        while sent < pes.len() {
            let take = (pes.len() - sent).min(TS_MAX_PAYLOAD_BYTES);
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            pkt[0] = 0x47;
            pkt[1] = ((ES_PID_UNDER_TEST >> 8) as u8 & PID_HI_MASK)
                | if sent == 0 { 0x40 } else { 0x00 };
            pkt[2] = (ES_PID_UNDER_TEST & 0xFF) as u8;
            pkt[3] = 0x10 | (cc & 0x0F);
            pkt[4..4 + take].copy_from_slice(&pes[sent..sent + take]);
            input.extend_from_slice(&pkt);
            // The legal duplicate: same CC, byte-identical (a re-encoded PCR
            // would also be legal, but this PES carries no PCR).
            input.extend_from_slice(&pkt);
            dups += 1;
            sent += take;
            cc = cc.wrapping_add(1);
        }
        assert!(
            dups >= 3,
            "the PES must span several packets for a duplicate append to be observable ({dups} packets)"
        );

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.finish();
        let samples: Vec<Sample> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => Some(sample),
                _ => None,
            })
            .collect();
        assert!(
            dups >= 2,
            "the fixture must really contain duplicates ({dups} emitted)"
        );
        assert_eq!(samples.len(), 1, "one PES => one Data sample");
        assert_eq!(
            samples[0].data.as_ref(),
            body.as_slice(),
            "the delivered sample must be the PES *payload* exactly once — a reassembled duplicate appends its 184 bytes a second time"
        );
    }

    /// A continuity-counter gap loses at least one 184-byte payload, so the
    /// access unit being reassembled is missing bytes. The demux must drop it,
    /// not deliver it truncated — the IR cannot tell a complete PES from a
    /// truncated one, so every downstream muxer would write the corruption out.
    ///
    /// The *next* access unit, which begins with its own `payload_unit_start`,
    /// is intact from its first byte and must still be delivered.
    #[test]
    fn cc_gap_drops_the_access_unit_it_truncated_but_not_the_next_one() {
        let au1 = long_data_au();
        let au2 = long_data_au_alt();
        let pes1 = untimed_pes(ES_STREAM_ID_PRIVATE_DATA, &au1);
        let pes2 = untimed_pes(ES_STREAM_ID_PRIVATE_DATA, &au2);
        assert!(
            pes1.len() > TS_MAX_PAYLOAD_BYTES && pes2.len() > TS_MAX_PAYLOAD_BYTES,
            "both access units must span more than one TS packet"
        );

        let mut input = Vec::new();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            input.extend_from_slice(&null_packet());
        }
        input.extend_from_slice(&psi_section_packets(
            PAT_PID_UNDER_TEST,
            0,
            TABLE_ID_PAT,
            &pat_body(),
        ));
        input.extend_from_slice(&psi_section_packets(
            PMT_PID_UNDER_TEST,
            0,
            TABLE_ID_PMT,
            &pmt_body(ES_PID_UNDER_TEST, STREAM_TYPE_PES_PRIVATE),
        ));

        // AU 1: only its first packet survives; the continuation is lost, so
        // the next packet on the PID jumps the CC by two. AU 2 follows with
        // its own `payload_unit_start`, whole.
        let pkt1 = pes_packets(ES_PID_UNDER_TEST, 0, &pes1);
        assert_eq!(pkt1.len(), 2 * TS_PACKET_SIZE, "AU 1 must span two packets");
        input.extend_from_slice(&pkt1[..TS_PACKET_SIZE]);
        let pkt2 = pes_packets(ES_PID_UNDER_TEST, 2, &pes2);

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.feed(&pkt2);
        demux.finish();

        let degraded = std::iter::from_fn(|| demux.poll_event()).any(|e| {
            matches!(
                e,
                DemuxEvent::InputDegraded {
                    kind: InputDegradation::ContinuityGap { .. },
                    ..
                }
            )
        });
        assert!(
            degraded,
            "the lost continuation must be reported as a CC gap"
        );

        let mut demux = StreamingTsDemux::new();
        demux.feed(&input);
        demux.feed(&pkt2);
        demux.finish();
        let samples: Vec<Sample> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => Some(sample),
                _ => None,
            })
            .collect();
        assert_eq!(
            samples.len(),
            1,
            "only the intact access unit may be delivered, got {} samples",
            samples.len()
        );
        assert_eq!(
            samples[0].data.as_ref(),
            au2.as_slice(),
            "the delivered sample must be the second, whole access unit"
        );
    }

    // ── CC gap (synthetic) ──────────────────────────────────────────────────

    #[test]
    fn cc_gap_emits_continuity_gap() {
        let mut demux = StreamingTsDemux::new();
        // Bootstrap TsResync lock.
        let null = null_packet();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            demux.feed(&null);
        }
        let payload_a = b"first packet";
        let payload_b = b"second different";
        let pkt = |cc: u8, payload: &[u8]| -> [u8; TS_PACKET_SIZE] {
            let mut p = [0xFFu8; TS_PACKET_SIZE];
            p[0] = 0x47;
            p[1] = 0x00;
            p[2] = 0x42; // PID=0x0042
            p[3] = 0x10 | (cc & 0x0F); // AFC=01
            let end = 4 + payload.len().min(TS_PACKET_SIZE - 4);
            p[4..end].copy_from_slice(&payload[..end - 4]);
            p
        };

        // First packet: CC=0.
        demux.feed(&pkt(0, payload_a));
        // Second packet: CC=5 (gap: expected=1, got=5), different payload.
        demux.feed(&pkt(5, payload_b));

        let degraded: Vec<_> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::InputDegraded {
                    kind, provenance, ..
                } => Some((kind, provenance)),
                _ => None,
            })
            .collect();
        assert_eq!(
            degraded.len(),
            1,
            "expected exactly one InputDegraded on CC gap"
        );
        assert_eq!(
            degraded[0].0,
            InputDegradation::ContinuityGap {
                expected: 1,
                got: 5
            }
        );
        assert_eq!(degraded[0].1.pid, Some(0x0042));
        // packet_index of the second packet (the gap), offset by bootstrap nulls.
        assert_eq!(
            degraded[0].1.packet_index,
            Some(mpeg_ts::resync::LOCK_CONFIRMATIONS as u64 + 2)
        );
    }

    // ── Mutation proof: disabling duplicate check produces false positives ──

    /// Confirms that the duplicate-detection exclusion is load-bearing: if we
    /// craft a synthetic scenario where a legal duplicate would appear and
    /// assert that the *undecorated* CC gap fires, the test must PASS (the
    /// real implementation skips duplicates correctly, so this test documents
    /// that duplicates are NOT reported). The mutation proof is the inverse:
    /// if an engineer removes the duplicate check, the CC=0 duplicate packet
    /// WOULD fire a ContinuityGap — but since we're testing the actual code
    /// (not a mutated copy), we assert the gap is absent.
    #[test]
    fn mutation_proof_duplicate_exclusion_is_load_bearing() {
        // This test exercises the duplicate path: two packets on same PID,
        // same CC, identical payload. If duplicate detection were removed,
        // the second packet would trigger a ContinuityGap { expected: 1, got: 0 }.
        // Since the real code skips it, we expect zero InputDegraded.
        let mut demux = StreamingTsDemux::new();
        // Bootstrap TsResync lock.
        let null = null_packet();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            demux.feed(&null);
        }
        let payload = b"identical";
        let pkt = |cc: u8| -> [u8; TS_PACKET_SIZE] {
            let mut p = [0xFFu8; TS_PACKET_SIZE];
            p[0] = 0x47;
            p[1] = 0x00;
            p[2] = 0x55;
            p[3] = 0x10 | (cc & 0x0F);
            let end = 4 + payload.len().min(TS_PACKET_SIZE - 4);
            p[4..end].copy_from_slice(&payload[..end - 4]);
            p
        };
        demux.feed(&pkt(0));
        demux.feed(&pkt(0)); // legal duplicate — same CC, same payload

        let degraded: Vec<_> = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| matches!(e, DemuxEvent::InputDegraded { .. }))
            .collect();
        assert!(
            degraded.is_empty(),
            "legal duplicate must not fire InputDegraded; \
             would produce ContinuityGap {{ expected: 1, got: 0 }} if exclusion were removed"
        );
    }

    // ── Mutation proof: change duplicate payload, confirm gap fires ─────────

    /// If the payload changes but CC is the same, it is NOT a legal duplicate
    /// — it's a genuine gap (or at minimum, it's not the spec-blessed
    /// duplicate case). This test confirms the code does NOT treat it as a
    /// duplicate.
    #[test]
    fn mutation_proof_changed_payload_same_cc_is_not_a_duplicate() {
        let mut demux = StreamingTsDemux::new();
        // Bootstrap TsResync lock.
        let null = null_packet();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            demux.feed(&null);
        }
        let pkt = |cc: u8, payload: &[u8]| -> [u8; TS_PACKET_SIZE] {
            let mut p = [0xFFu8; TS_PACKET_SIZE];
            p[0] = 0x47;
            p[1] = 0x00;
            p[2] = 0x66;
            p[3] = 0x10 | (cc & 0x0F);
            let end = 4 + payload.len().min(TS_PACKET_SIZE - 4);
            p[4..end].copy_from_slice(&payload[..end - 4]);
            p
        };
        demux.feed(&pkt(0, b"first"));
        demux.feed(&pkt(0, b"second")); // same CC, DIFFERENT payload — NOT a duplicate

        let degraded: Vec<_> = std::iter::from_fn(|| demux.poll_event())
            .filter_map(|e| match e {
                DemuxEvent::InputDegraded { kind, .. } => Some(kind),
                _ => None,
            })
            .collect();
        assert_eq!(
            degraded.len(),
            1,
            "same CC + different payload must fire InputDegraded"
        );
        assert!(matches!(
            degraded[0],
            InputDegradation::ContinuityGap { .. }
        ));
    }

    // ── Mutation proof: signalled discontinuity skips CC gap ────────────────

    /// A packet whose adaptation field sets `discontinuity_indicator` must
    /// emit `Discontinuity`, not `InputDegraded::ContinuityGap`, even if its
    /// CC is a gap. This confirms the exclusion rule: if an engineer removes
    /// the `discontinuity_signalled` check, this test's assertion that no
    /// ContinuityGap appeared would fail.
    #[test]
    fn mutation_proof_signalled_discontinuity_suppresses_cc_gap() {
        let mut demux = StreamingTsDemux::new();
        // Bootstrap TsResync lock.
        let null = null_packet();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            demux.feed(&null);
        }
        let payload = b"payload";

        // First packet: CC=0, no adaptation, PID=0x0077.
        demux.feed(&ts_packet_degradation(
            false, 0x0077, 0, 0x10, None, payload,
        ));
        // Second packet: CC=5 (gap), BUT adaptation field with
        // discontinuity_indicator=1. Should emit Discontinuity, NOT
        // InputDegraded::ContinuityGap.
        let af = [
            0x80u8, // discontinuity_indicator=1, no other flags
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, // stuffing
        ];
        demux.feed(&ts_packet_degradation(
            false,
            0x0077,
            5,
            0x30,
            Some(&af),
            payload, // AFC=11
        ));

        let mut saw_discontinuity = false;
        let mut saw_cc_gap = false;
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::Discontinuity { .. } => saw_discontinuity = true,
                DemuxEvent::InputDegraded {
                    kind: InputDegradation::ContinuityGap { .. },
                    ..
                } => saw_cc_gap = true,
                _ => {}
            }
        }
        assert!(
            saw_discontinuity,
            "packet with discontinuity_indicator must emit Discontinuity"
        );
        assert!(
            !saw_cc_gap,
            "packet with discontinuity_indicator must NOT emit ContinuityGap — \
             the exclusion rule was removed or broken"
        );
    }

    // ── Mutation proof: CC wraps correctly ──────────────────────────────────

    #[test]
    fn cc_wraps_at_15_to_0_without_false_gap() {
        let mut demux = StreamingTsDemux::new();
        // Bootstrap TsResync lock.
        let null = null_packet();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            demux.feed(&null);
        }
        let pkt = |cc: u8| -> [u8; TS_PACKET_SIZE] {
            let mut p = [0xFFu8; TS_PACKET_SIZE];
            p[0] = 0x47;
            p[1] = 0x00;
            p[2] = 0x88;
            p[3] = 0x10 | (cc & 0x0F);
            p[4..11].copy_from_slice(b"payload");
            p
        };

        // CC 14, 15, 0 — no gap, normal wrap.
        demux.feed(&pkt(14));
        demux.feed(&pkt(15));
        demux.feed(&pkt(0));

        let degraded: Vec<_> = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| matches!(e, DemuxEvent::InputDegraded { .. }))
            .collect();
        assert!(
            degraded.is_empty(),
            "normal CC wrap 15→0 must not emit InputDegraded, got {degraded:?}"
        );
    }

    // ── Regression: post-discontinuity legal CC is not a false gap (defect 1) ─

    /// Regression test for the review-found false positive: a payload-bearing
    /// signalled discontinuity must NOT prevent `last_cc` from being updated.
    /// The next legal CC (discontinuity's CC + 1) must emit nothing.
    ///
    /// Derived from the real pid 0x0083 sequence in m6-discontinuity.ts:
    /// packet N carries discontinuity_indicator + CC=X; packet N+1 is the
    /// next legal payload-bearing packet with CC=(X+1) & 0x0F.
    #[test]
    fn post_discontinuity_legal_cc_emits_nothing() {
        let mut demux = StreamingTsDemux::new();
        let null = null_packet();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            demux.feed(&null);
        }

        let payload = b"payload";
        // First: normal packet, CC=0, PID=0x0083.
        demux.feed(&ts_packet_degradation(
            false, 0x0083, 0, 0x10, None, payload,
        ));
        // Second: discontinuity_indicator=1, CC=5 (gap — suppressed event, but state updated).
        let af = [0x80u8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        demux.feed(&ts_packet_degradation(
            false,
            0x0083,
            5,
            0x30,
            Some(&af),
            payload,
        ));
        // Third: legal follow-up, CC=6 ((5+1) & 0x0F). Must NOT fire ContinuityGap.
        demux.feed(&ts_packet_degradation(
            false, 0x0083, 6, 0x10, None, payload,
        ));

        let gaps: Vec<_> = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| {
                matches!(
                    e,
                    DemuxEvent::InputDegraded {
                        kind: InputDegradation::ContinuityGap { .. },
                        ..
                    }
                )
            })
            .collect();
        assert!(
            gaps.is_empty(),
            "post-discontinuity legal CC must not fire ContinuityGap; \
             discontinuity_indicator suppresses the event but updates last_cc. \
             Got {gaps:?}"
        );
    }

    // ── Regression: PCR-only adaptation-field change is NOT a false duplicate mismatch (defect 2) ─

    /// Regression test for the review-found false positive: a legal duplicate
    /// whose only difference is a re-encoded PCR in the adaptation field must
    /// still be recognised as a duplicate. The duplicate check compares
    /// `pkt.payload`, not `raw[4..]` (which includes the adaptation field).
    ///
    /// Derived from the real PCR-bearing PID behaviour in broadcast streams.
    #[test]
    fn pcr_variation_in_adaptation_field_is_still_a_legal_duplicate() {
        let mut demux = StreamingTsDemux::new();
        let null = null_packet();
        for _ in 0..mpeg_ts::resync::LOCK_CONFIRMATIONS + 1 {
            demux.feed(&null);
        }

        let payload = b"identical payload for duplicate test";

        // First: CC=0, PID=0x0100, adaptation field with PCR=100.
        let af_pcr_100 = [
            0x10u8, // PCR flag only
            0x00, 0x00, 0x00, 0x00, 0x7E, 0x64, // PCR = 100 (encoded as 6-byte PCR field)
        ];
        demux.feed(&ts_packet_degradation(
            false,
            0x0100,
            0,
            0x30, // AFC=11: adaptation + payload
            Some(&af_pcr_100),
            payload,
        ));

        // Second: CC=0 (legal duplicate), same payload, adaptation field with
        // PCR=200 (different PCR encoding — NOT a different payload).
        let af_pcr_200 = [
            0x10u8, // PCR flag only
            0x00, 0x00, 0x00, 0x00, 0x7E, 0xC8, // PCR = 200
        ];
        demux.feed(&ts_packet_degradation(
            false,
            0x0100,
            0, // same CC
            0x30,
            Some(&af_pcr_200),
            payload, // same payload
        ));

        let gaps: Vec<_> = std::iter::from_fn(|| demux.poll_event())
            .filter(|e| {
                matches!(
                    e,
                    DemuxEvent::InputDegraded {
                        kind: InputDegradation::ContinuityGap { .. },
                        ..
                    }
                )
            })
            .collect();
        assert!(
            gaps.is_empty(),
            "PCR-only adaptation-field change must not break duplicate detection; \
             duplicate check uses pkt.payload, not raw[4..]. Got {gaps:?}"
        );
    }
}
