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

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Demand, Serialize, Stage, Timestamp, Unpackage};
use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::resync::TsResync;
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};

use crate::aac_asc::{AdtsHeader, AudioSpecificConfig, parse_adts_header};
use crate::ac3::{
    AC3_SAMPLES_PER_SYNCFRAME, Ac3SyncframeInfo, Ec3SyncframeInfo, split_ac3_syncframes,
    split_eac3_syncframes,
};
use crate::annexb::{annexb_to_length_prefixed, iter_annexb_nals};
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::dts::{DtsCoreFrameInfo, split_dts_core_frames};
use crate::error::{Error, Result};
use crate::hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
use crate::media::{Media, PcrSample, Track};
use crate::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, ObjectTypeIndication,
    SLConfigDescriptor, StreamType as EsdsStreamType,
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
        if let Ok(hdr) = parse_adts_header(&data[off..]) {
            return Some((off, hdr));
        }
        off += 1;
    }
    None
}

/// Split a concatenated ADTS payload into individual frames (header + raw
/// data). Resyncs to the next frame boundary on a bad sync (see
/// [`find_adts_sync`]); stops once no further sync is found or a frame would
/// run past the end of `payload`, so a partial tail does not lose earlier
/// frames.
fn split_adts_frames(payload: &[u8]) -> Vec<&[u8]> {
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
    /// This track's already-recovered codec config, retained so a later PMT
    /// metadata change (issue #774) can rebuild a full [`TrackSpec`] for
    /// [`DemuxEvent::TrackUpdated`] without re-deriving it — codec config
    /// recovery itself stays single-shot and permanent, this field is only
    /// ever read, never re-probed.
    config: CodecConfig,
    /// This track's media timescale, for the same [`DemuxEvent::TrackUpdated`]
    /// reconstruction.
    timescale: u32,
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
    /// Previous access unit's resolved `(pts, dts)` — the fallback used when
    /// a PES carries neither (mirrors the old `push_access_unit` fallback).
    fallback: (u64, u64),
    has_any: bool,
    wrap: WrapState,
    /// Always `Some` except transiently inside [`advance_track`].
    track: Option<TrackState>,
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
            for frame in split_adts_frames(au_data) {
                if frame.len() > ADTS_HEADER_SIZE {
                    events.push_back(DemuxEvent::Sample {
                        track_id,
                        sample: audio_sample(
                            frame[ADTS_HEADER_SIZE..].to_vec(),
                            AAC_SAMPLES_PER_FRAME,
                            elapsed,
                        ),
                    });
                }
                elapsed += AAC_SAMPLES_PER_FRAME as u64;
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
    // The caller pushes the newest access unit immediately before calling
    // this, so the backlog is never empty here — degrade to "not resolvable
    // yet" rather than panicking if that ever stops holding.
    let latest = backlog.last()?;
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
            // Coded dimensions + high-profile chroma/bit-depth from the SPS
            // (ISO/IEC 14496-10 §7.3.2.1.1) — the TS in-band parameter set
            // carries them (0/None if undecodable).
            let info = crate::sps::decode_avc_sps(sps_bytes).ok();
            let (width, height) = info
                .as_ref()
                .map(|i| (i.width as u16, i.height as u16))
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
            // Resync within each buffered PES payload (issue #638) -- a real
            // broadcast payload is not guaranteed to start on a frame sync.
            let first = backlog
                .iter()
                .find_map(|au| find_mpeg_audio_sync(&au.data).map(|(_, hdr)| hdr))?;
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
                    anchor: AudioAnchor::default(),
                },
            ))
        }
        ConfigProbe::Aac => {
            // Resync within each buffered PES payload (issue #638) -- a real
            // broadcast payload is not guaranteed to start on a frame sync.
            let first_hdr = backlog
                .iter()
                .find_map(|au| find_adts_sync(&au.data).map(|(_, hdr)| hdr))?;
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
                    anchor: AudioAnchor::default(),
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
                    anchor: AudioAnchor::default(),
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
                    anchor: AudioAnchor::default(),
                },
            ))
        }
        ConfigProbe::Dts => {
            let info = backlog
                .iter()
                .find_map(|au| DtsCoreFrameInfo::from_es(&au.data).ok())?;
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
            // Scan the backlog for the first access unit whose MHAS packets
            // carry a PACTYP_MPEGH3DACFG (issue #579) — mirrors the
            // Ac3/Eac3/Dts `find_map` header scans above, just over MHAS
            // packets instead of a sync-frame header.
            let config_bytes = backlog
                .iter()
                .find_map(|au| find_mpegh3da_config(&au.data))?;
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
            match finalize_probe(stream.codec, &stream.descriptors, &mut probe, &backlog) {
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

/// Resolve a completed PES packet's `(pts, dts)` (mirrors the old
/// `push_access_unit` fallback rule) and drive it through [`advance_track`]
/// (parked/probing) or [`push_live_au`] (already live).
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
    /// Payload bytes of the last packet (for duplicate detection).
    last_payload: Vec<u8>,
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
                self.events.push_back(DemuxEvent::Discontinuity {
                    track: self.live_track_id(pkt.header.pid),
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

        // CC gap degradation (issue #778) — payload-bearing, non-null packets
        // only, after the discontinuity check (a signalled discontinuity is
        // `Discontinuity`, not `InputDegraded`). Legal duplicates (same CC +
        // identical payload) are excluded per ISO/IEC 13818-1 §2.4.3.3.
        if pkt.header.has_payload && pkt.header.pid != NULL_PACKET_PID && !discontinuity_signalled {
            self.check_cc(pkt.header.pid, pkt.header.continuity_counter, raw, idx);
        }

        let pid = pkt.header.pid;
        let pusi = pkt.header.pusi;
        let Some(payload) = pkt.payload else {
            return;
        };

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
            if let Some(completed) = completed_pes {
                on_completed_pes(stream, pid, &completed, &mut self.events);
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
    /// when a genuine gap is detected (issue #778).
    ///
    /// Does **not** fire for:
    /// - The first payload-bearing packet on this PID (`initialized` guard).
    /// - Legal duplicates: same CC + identical payload (ISO/IEC 13818-1 §2.4.3.3).
    ///   This is already pre-filtered by the caller: `discontinuity_signalled`
    ///   packets never reach here.
    /// - Non-payload-bearing or null-PID packets (already filtered by the caller).
    ///
    /// # Arguments
    /// - `pid` — the TS PID.
    /// - `cc` — the 4-bit continuity counter from the packet header.
    /// - `raw` — the full 188-byte TS packet (for duplicate detection: payload
    ///   bytes compared).
    /// - `packet_index` — 0-based index of this packet in the stream.
    fn check_cc(&mut self, pid: u16, cc: u8, raw: &[u8; TS_PACKET_SIZE], packet_index: u64) {
        use std::collections::btree_map::Entry;

        let payload = &raw[4..];
        let track = self.live_track_id(pid);
        match self.cc_states.entry(pid) {
            Entry::Occupied(mut e) => {
                let state = e.get_mut();
                if !state.initialized {
                    state.initialized = true;
                    state.last_cc = cc;
                    state.last_payload = payload.to_vec();
                    return;
                }
                let expected = (state.last_cc + 1) & 0x0F;
                if cc != expected {
                    // Check for legal duplicate: same CC + identical payload.
                    let is_dup = cc == state.last_cc
                        && payload.len() == state.last_payload.len()
                        && payload == state.last_payload.as_slice();
                    if !is_dup {
                        self.events.push_back(DemuxEvent::InputDegraded {
                            track,
                            kind: InputDegradation::ContinuityGap { expected, got: cc },
                            provenance: EventProvenance {
                                pid: Some(pid),
                                packet_index: Some(packet_index),
                            },
                        });
                    }
                }
                state.last_cc = cc;
                state.last_payload = payload.to_vec();
            }
            Entry::Vacant(e) => {
                e.insert(CcState {
                    initialized: true,
                    last_cc: cc,
                    last_payload: payload.to_vec(),
                });
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
            pes_bytes: 0,
            fallback: (0, 0),
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
        if let Some(stream) = self.streams.remove(&pid) {
            if let Some(TrackState::Live(live)) = stream.track {
                self.events.push_back(DemuxEvent::TrackRemoved {
                    track_id: live.track_id,
                    provenance: EventProvenance {
                        pid: Some(pid),
                        packet_index: None,
                    },
                });
            }
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
                    self.events.push_back(DemuxEvent::TrackAdded(spec));
                    let mut live = LiveTrack {
                        track_id,
                        kind,
                        config,
                        timescale,
                    };
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
                on_completed_pes(stream, pid, &completed, &mut self.events);
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
                DemuxEvent::Sample { track_id, sample } => {
                    if let Some(&i) = index_by_id.get(&track_id) {
                        let track = &mut tracks[i];
                        // `Track::start_decode_time` is no longer carried by
                        // `TrackAdded` (issue #774 reshape) — it is exactly
                        // the first sample's own `dts` (media plane step 2c
                        // invariant, unconditionally true for every track
                        // kind), so derive it here instead.
                        if track.samples.is_empty() {
                            if let Some(dts) = sample.dts {
                                track.start_decode_time = dts as u64;
                            }
                        }
                        track.samples.push(sample);
                    }
                }
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
            {
                if let Some(pid) = provenance.pid {
                    abandoned_pids.push(pid);
                }
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

    /// A negative unwrapped anchor is a legitimate value — reordering (or a
    /// capture starting mid-GOP) across the 2^33 boundary unwraps to a small
    /// negative absolute time — and every 90 kHz track kind carries it through
    /// verbatim. Rescaling into an audio track's own sample-rate timescale must
    /// not be the one path that clamps it to `0`, which fabricated `dts = 0`
    /// for the audio track alone and desynced it from the video it was muxed
    /// against.
    #[test]
    fn rescale_to_track_preserves_a_negative_anchor_for_audio_as_it_does_for_video() {
        const SAMPLE_RATE: u32 = 48_000;
        /// One second before zero, on the 90 kHz PES clock.
        const NEGATIVE_90K: i128 = -90_000;

        assert_eq!(
            rescale_to_track(NEGATIVE_90K, VIDEO_TIMESCALE),
            -90_000,
            "the 90 kHz identity path already preserved this"
        );
        assert_eq!(
            rescale_to_track(NEGATIVE_90K, SAMPLE_RATE),
            -48_000,
            "the audio rescale must preserve it too — one second before zero is \
             -48000 ticks at 48 kHz, not 0"
        );
        // Floor semantics hold on both sides of zero (what the doc claims).
        assert_eq!(rescale_to_track(-1, SAMPLE_RATE), -1);
        assert_eq!(rescale_to_track(1, SAMPLE_RATE), 0);
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
    /// CC gaps AND signalled discontinuities. This test just confirms the
    /// fixture plays through without panicking and that signalled
    /// discontinuities are still observed as `Discontinuity` events.
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
}
