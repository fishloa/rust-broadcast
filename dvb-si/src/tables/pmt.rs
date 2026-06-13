//! Program Map Table — MPEG-2 ISO/IEC 13818-1 §2.4.4.8.
//!
//! PMT describes the elementary streams that make up one programme.
//! Carried on a per-programme PID signalled by the PAT, with table_id 0x02.
//! Descriptor parsing is out of scope for this commit — raw bytes only.

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// PMT table_id (ISO/IEC 13818-1 Table 2-30).
pub const TABLE_ID: u8 = 0x02;
/// PMT PIDs are programme-specific and signalled via PAT; 0x0000 is a
/// placeholder meaning "no well-known PID".
pub const PID: u16 = 0x0000;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const PCR_PID_LEN: usize = 2;
const PROG_INFO_LEN_BYTES: usize = 2;
const CRC_LEN: usize = 4;
const MIN_SECTION_LEN: usize =
    MIN_HEADER_LEN + EXTENSION_HEADER_LEN + PCR_PID_LEN + PROG_INFO_LEN_BYTES + CRC_LEN;
const STREAM_HEADER_LEN: usize = 5;

/// Stream type coding — Rec. ITU-T H.222.0 (06/2021) Table 2-34.
///
/// Identifies the elementary-stream type carried in the associated PID.
/// Values `0x80`–`0xFF` are user private; only well-established entries
/// cited to their own specs are named — the rest fall through to
/// [`UserPrivate`](Self::UserPrivate).
///
/// # Examples
/// ```
/// use dvb_si::tables::pmt::StreamType;
///
/// assert_eq!(StreamType::from_u8(0x02).name(), "MPEG-2 Video");
/// assert_eq!(StreamType::from_u8(0x1B).to_u8(), 0x1B); // H.264, lossless round-trip
/// ```
/// A parsed `PmtStream.stream_type` is already a `StreamType` — match on it directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StreamType {
    /// 0x00 — ITU-T | ISO/IEC Reserved.
    Reserved,
    /// 0x01 — ISO/IEC 11172-2 Video (MPEG-1 video).
    Mpeg1Video,
    /// 0x02 — Rec. ITU-T H.262 | ISO/IEC 13818-2 Video (MPEG-2 video).
    Mpeg2Video,
    /// 0x03 — ISO/IEC 11172-3 Audio (MPEG-1 audio).
    Mpeg1Audio,
    /// 0x04 — ISO/IEC 13818-3 Audio (MPEG-2 audio).
    Mpeg2Audio,
    /// 0x05 — Rec. ITU-T H.222.0 | ISO/IEC 13818-1 private_sections.
    PrivateSections,
    /// 0x06 — Rec. ITU-T H.222.0 | ISO/IEC 13818-1 PES packets containing private data.
    PesPrivateData,
    /// 0x07 — ISO/IEC 13522 MHEG.
    Mheg,
    /// 0x08 — Rec. ITU-T H.222.0 | ISO/IEC 13818-1 Annex A DSM-CC.
    DsmCc,
    /// 0x09 — Rec. ITU-T H.222.1.
    H222_1,
    /// 0x0A — ISO/IEC 13818-6 type A.
    Iso13818_6TypeA,
    /// 0x0B — ISO/IEC 13818-6 type B.
    Iso13818_6TypeB,
    /// 0x0C — ISO/IEC 13818-6 type C.
    Iso13818_6TypeC,
    /// 0x0D — ISO/IEC 13818-6 type D.
    Iso13818_6TypeD,
    /// 0x0E — Rec. ITU-T H.222.0 | ISO/IEC 13818-1 auxiliary.
    Auxiliary,
    /// 0x0F — ISO/IEC 13818-7 Audio with ADTS transport syntax (AAC).
    AacAdts,
    /// 0x10 — ISO/IEC 14496-2 Visual (MPEG-4 video).
    Mpeg4Video,
    /// 0x11 — ISO/IEC 14496-3 Audio with LATM transport syntax (AAC LATM).
    AacLatm,
    /// 0x12 — ISO/IEC 14496-1 SL-packetized / FlexMux stream in PES.
    SlFlexMuxPes,
    /// 0x13 — ISO/IEC 14496-1 SL-packetized / FlexMux stream in sections.
    SlFlexMuxSections,
    /// 0x14 — ISO/IEC 13818-6 Synchronized Download Protocol.
    SyncDownload,
    /// 0x15 — Metadata carried in PES packets.
    MetadataPes,
    /// 0x16 — Metadata carried in metadata_sections.
    MetadataSections,
    /// 0x17 — Metadata carried in ISO/IEC 13818-6 Data Carousel.
    MetadataDataCarousel,
    /// 0x18 — Metadata carried in ISO/IEC 13818-6 Object Carousel.
    MetadataObjectCarousel,
    /// 0x19 — Metadata carried in ISO/IEC 13818-6 Synchronized Download Protocol.
    MetadataSyncDownload,
    /// 0x1A — IPMP stream (ISO/IEC 13818-11, MPEG-2 IPMP).
    Ipmp,
    /// 0x1B — AVC video stream (Rec. ITU-T H.264 | ISO/IEC 14496-10).
    H264,
    /// 0x1C — ISO/IEC 14496-3 Audio without additional transport syntax (DST, ALS, SLS).
    Iso14496_3Audio,
    /// 0x1D — ISO/IEC 14496-17 Text.
    Iso14496_17Text,
    /// 0x1E — Auxiliary video stream (ISO/IEC 23002-3).
    AuxiliaryVideo,
    /// 0x1F — SVC video sub-bitstream of an AVC video stream (H.264 Annex G).
    Svc,
    /// 0x20 — MVC video sub-bitstream of an AVC video stream (H.264 Annex H).
    Mvc,
    /// 0x21 — JPEG 2000 video (Rec. ITU-T T.800 | ISO/IEC 15444-1).
    Jpeg2000,
    /// 0x22 — Additional view H.262 | ISO/IEC 13818-2 video for service-compatible stereoscopic 3D.
    AdditionalViewH262,
    /// 0x23 — Additional view H.264 | ISO/IEC 14496-10 video for service-compatible stereoscopic 3D.
    AdditionalViewH264,
    /// 0x24 — Rec. ITU-T H.265 | ISO/IEC 23008-2 video (HEVC) or HEVC temporal sub-bitstream.
    Hevc,
    /// 0x25 — HEVC temporal video subset (H.265 Annex A profiles).
    HevcTemporalSubset,
    /// 0x26 — MVCD video sub-bitstream of an AVC video stream (H.264 Annex I).
    Mvcd,
    /// 0x27 — Timeline and External Media Information (TEMI, H.222.0 Annex U).
    Temi,
    /// 0x28 — HEVC enhancement sub-partition incl. TemporalId 0 (H.265 Annex G).
    HevcAnnexG,
    /// 0x29 — HEVC temporal enhancement sub-partition (H.265 Annex G).
    HevcAnnexGTemporal,
    /// 0x2A — HEVC enhancement sub-partition incl. TemporalId 0 (H.265 Annex H).
    HevcAnnexH,
    /// 0x2B — HEVC temporal enhancement sub-partition (H.265 Annex H).
    HevcAnnexHTemporal,
    /// 0x2C — Green access units carried in MPEG-2 sections.
    GreenAccessUnits,
    /// 0x2D — ISO/IEC 23008-3 Audio with MHAS transport syntax — main stream.
    MhasAudioMain,
    /// 0x2E — ISO/IEC 23008-3 Audio with MHAS transport syntax — auxiliary stream.
    MhasAudioAux,
    /// 0x2F — Quality access units carried in sections.
    QualityAccessUnits,
    /// 0x30 — Media Orchestration Access Units carried in sections.
    MediaOrchestration,
    /// 0x31 — MCTS substream of an H.265 | ISO/IEC 23008-2 video stream.
    MctsHevc,
    /// 0x32 — JPEG XS video stream (ISO/IEC 21122-2 profiles).
    JpegXs,
    /// 0x33 — VVC video stream (Rec. ITU-T H.266 | ISO/IEC 23090-3) or VVC temporal sub-bitstream.
    Vvc,
    /// 0x34 — VVC temporal video subset (H.266 Annex A profiles).
    VvcTemporalSubset,
    /// 0x35 — EVC video stream or EVC temporal sub-bitstream (ISO/IEC 23094-1).
    Evc,
    /// 0x81 — ATSC AC-3 audio (A/52).
    Ac3,
    /// 0x86 — SCTE-35 splice_info_section (ANSI/SCTE 35).
    Scte35,
    /// 0x87 — ATSC E-AC-3 / Dolby Digital Plus audio (A/52B).
    EAc3,
    /// 0x7F — IPMP stream (H.222.0 Table 2-34).
    IpmpHigh,
    /// Rec. ITU-T H.222.0 reserved range `0x36`..=`0x7E`.
    ReservedRange(u8),
    /// User private range `0x80`..=`0xFF` (except named entries).
    UserPrivate(u8),
}

impl StreamType {
    /// Decode from the wire byte. Every byte maps to a variant (lossless).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Reserved,
            0x01 => Self::Mpeg1Video,
            0x02 => Self::Mpeg2Video,
            0x03 => Self::Mpeg1Audio,
            0x04 => Self::Mpeg2Audio,
            0x05 => Self::PrivateSections,
            0x06 => Self::PesPrivateData,
            0x07 => Self::Mheg,
            0x08 => Self::DsmCc,
            0x09 => Self::H222_1,
            0x0A => Self::Iso13818_6TypeA,
            0x0B => Self::Iso13818_6TypeB,
            0x0C => Self::Iso13818_6TypeC,
            0x0D => Self::Iso13818_6TypeD,
            0x0E => Self::Auxiliary,
            0x0F => Self::AacAdts,
            0x10 => Self::Mpeg4Video,
            0x11 => Self::AacLatm,
            0x12 => Self::SlFlexMuxPes,
            0x13 => Self::SlFlexMuxSections,
            0x14 => Self::SyncDownload,
            0x15 => Self::MetadataPes,
            0x16 => Self::MetadataSections,
            0x17 => Self::MetadataDataCarousel,
            0x18 => Self::MetadataObjectCarousel,
            0x19 => Self::MetadataSyncDownload,
            0x1A => Self::Ipmp,
            0x1B => Self::H264,
            0x1C => Self::Iso14496_3Audio,
            0x1D => Self::Iso14496_17Text,
            0x1E => Self::AuxiliaryVideo,
            0x1F => Self::Svc,
            0x20 => Self::Mvc,
            0x21 => Self::Jpeg2000,
            0x22 => Self::AdditionalViewH262,
            0x23 => Self::AdditionalViewH264,
            0x24 => Self::Hevc,
            0x25 => Self::HevcTemporalSubset,
            0x26 => Self::Mvcd,
            0x27 => Self::Temi,
            0x28 => Self::HevcAnnexG,
            0x29 => Self::HevcAnnexGTemporal,
            0x2A => Self::HevcAnnexH,
            0x2B => Self::HevcAnnexHTemporal,
            0x2C => Self::GreenAccessUnits,
            0x2D => Self::MhasAudioMain,
            0x2E => Self::MhasAudioAux,
            0x2F => Self::QualityAccessUnits,
            0x30 => Self::MediaOrchestration,
            0x31 => Self::MctsHevc,
            0x32 => Self::JpegXs,
            0x33 => Self::Vvc,
            0x34 => Self::VvcTemporalSubset,
            0x35 => Self::Evc,
            0x36..=0x7E => Self::ReservedRange(v),
            0x7F => Self::IpmpHigh,
            0x81 => Self::Ac3,
            0x86 => Self::Scte35,
            0x87 => Self::EAc3,
            _ => Self::UserPrivate(v),
        }
    }

    /// Encode to the wire byte. Inverse of `from_u8`.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Reserved => 0x00,
            Self::Mpeg1Video => 0x01,
            Self::Mpeg2Video => 0x02,
            Self::Mpeg1Audio => 0x03,
            Self::Mpeg2Audio => 0x04,
            Self::PrivateSections => 0x05,
            Self::PesPrivateData => 0x06,
            Self::Mheg => 0x07,
            Self::DsmCc => 0x08,
            Self::H222_1 => 0x09,
            Self::Iso13818_6TypeA => 0x0A,
            Self::Iso13818_6TypeB => 0x0B,
            Self::Iso13818_6TypeC => 0x0C,
            Self::Iso13818_6TypeD => 0x0D,
            Self::Auxiliary => 0x0E,
            Self::AacAdts => 0x0F,
            Self::Mpeg4Video => 0x10,
            Self::AacLatm => 0x11,
            Self::SlFlexMuxPes => 0x12,
            Self::SlFlexMuxSections => 0x13,
            Self::SyncDownload => 0x14,
            Self::MetadataPes => 0x15,
            Self::MetadataSections => 0x16,
            Self::MetadataDataCarousel => 0x17,
            Self::MetadataObjectCarousel => 0x18,
            Self::MetadataSyncDownload => 0x19,
            Self::Ipmp => 0x1A,
            Self::H264 => 0x1B,
            Self::Iso14496_3Audio => 0x1C,
            Self::Iso14496_17Text => 0x1D,
            Self::AuxiliaryVideo => 0x1E,
            Self::Svc => 0x1F,
            Self::Mvc => 0x20,
            Self::Jpeg2000 => 0x21,
            Self::AdditionalViewH262 => 0x22,
            Self::AdditionalViewH264 => 0x23,
            Self::Hevc => 0x24,
            Self::HevcTemporalSubset => 0x25,
            Self::Mvcd => 0x26,
            Self::Temi => 0x27,
            Self::HevcAnnexG => 0x28,
            Self::HevcAnnexGTemporal => 0x29,
            Self::HevcAnnexH => 0x2A,
            Self::HevcAnnexHTemporal => 0x2B,
            Self::GreenAccessUnits => 0x2C,
            Self::MhasAudioMain => 0x2D,
            Self::MhasAudioAux => 0x2E,
            Self::QualityAccessUnits => 0x2F,
            Self::MediaOrchestration => 0x30,
            Self::MctsHevc => 0x31,
            Self::JpegXs => 0x32,
            Self::Vvc => 0x33,
            Self::VvcTemporalSubset => 0x34,
            Self::Evc => 0x35,
            Self::IpmpHigh => 0x7F,
            Self::Ac3 => 0x81,
            Self::Scte35 => 0x86,
            Self::EAc3 => 0x87,
            Self::ReservedRange(v) | Self::UserPrivate(v) => v,
        }
    }

    /// Human-readable spec display name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Reserved => "Reserved",
            Self::Mpeg1Video => "MPEG-1 Video",
            Self::Mpeg2Video => "MPEG-2 Video",
            Self::Mpeg1Audio => "MPEG-1 Audio",
            Self::Mpeg2Audio => "MPEG-2 Audio",
            Self::PrivateSections => "Private Sections",
            Self::PesPrivateData => "PES Private Data",
            Self::Mheg => "MHEG",
            Self::DsmCc => "DSM-CC",
            Self::H222_1 => "H.222.1",
            Self::Iso13818_6TypeA => "ISO/IEC 13818-6 Type A",
            Self::Iso13818_6TypeB => "ISO/IEC 13818-6 Type B",
            Self::Iso13818_6TypeC => "ISO/IEC 13818-6 Type C",
            Self::Iso13818_6TypeD => "ISO/IEC 13818-6 Type D",
            Self::Auxiliary => "Auxiliary",
            Self::AacAdts => "AAC ADTS",
            Self::Mpeg4Video => "MPEG-4 Video",
            Self::AacLatm => "AAC LATM",
            Self::SlFlexMuxPes => "SL/FlexMux in PES",
            Self::SlFlexMuxSections => "SL/FlexMux in Sections",
            Self::SyncDownload => "Sync Download Protocol",
            Self::MetadataPes => "Metadata in PES",
            Self::MetadataSections => "Metadata in Sections",
            Self::MetadataDataCarousel => "Metadata Data Carousel",
            Self::MetadataObjectCarousel => "Metadata Object Carousel",
            Self::MetadataSyncDownload => "Metadata Sync Download",
            Self::Ipmp => "IPMP",
            Self::H264 => "H.264/AVC",
            Self::Iso14496_3Audio => "ISO/IEC 14496-3 Audio",
            Self::Iso14496_17Text => "ISO/IEC 14496-17 Text",
            Self::AuxiliaryVideo => "Auxiliary Video",
            Self::Svc => "SVC",
            Self::Mvc => "MVC",
            Self::Jpeg2000 => "JPEG 2000",
            Self::AdditionalViewH262 => "Additional View H.262 (3D)",
            Self::AdditionalViewH264 => "Additional View H.264 (3D)",
            Self::Hevc => "HEVC/H.265",
            Self::HevcTemporalSubset => "HEVC Temporal Subset",
            Self::Mvcd => "MVCD (H.264 Annex I)",
            Self::Temi => "TEMI",
            Self::HevcAnnexG => "HEVC Annex G",
            Self::HevcAnnexGTemporal => "HEVC Annex G Temporal",
            Self::HevcAnnexH => "HEVC Annex H",
            Self::HevcAnnexHTemporal => "HEVC Annex H Temporal",
            Self::GreenAccessUnits => "Green Access Units",
            Self::MhasAudioMain => "MHAS Audio Main",
            Self::MhasAudioAux => "MHAS Audio Aux",
            Self::QualityAccessUnits => "Quality Access Units",
            Self::MediaOrchestration => "Media Orchestration",
            Self::MctsHevc => "MCTS HEVC",
            Self::JpegXs => "JPEG XS",
            Self::Vvc => "VVC/H.266",
            Self::VvcTemporalSubset => "VVC Temporal Subset",
            Self::Evc => "EVC",
            Self::IpmpHigh => "IPMP (0x7F)",
            Self::Ac3 => "AC-3",
            Self::Scte35 => "SCTE-35",
            Self::EAc3 => "E-AC-3",
            Self::ReservedRange(_) => "Reserved",
            Self::UserPrivate(_) => "User Private",
        }
    }
}

/// One elementary stream entry in the PMT's ES loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct PmtStream<'a> {
    /// MPEG-2 stream_type byte (ISO/IEC 13818-1 Table 2-34).
    pub stream_type: StreamType,
    /// 13-bit elementary stream PID.
    pub elementary_pid: u16,
    /// Raw ES_info descriptor bytes; parsing lives in crate::descriptors.
    /// Elementary-stream descriptor loop. Serializes as the typed descriptor
    /// sequence; `.raw()` yields the wire bytes.
    pub es_info: DescriptorLoop<'a>,
}

/// Program Map Table.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct PmtSection<'a> {
    /// Programme number from the table_id_extension field.
    pub program_number: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence (ISO/IEC 13818-1 §2.4.4.8;
    /// shall be 0x00 for conformant PMTs but preserved for round-trip fidelity).
    pub section_number: u8,
    /// last_section_number in the sub-table sequence (ISO/IEC 13818-1 §2.4.4.8;
    /// shall be 0x00 for conformant PMTs but preserved for round-trip fidelity).
    pub last_section_number: u8,
    /// 13-bit PCR PID.
    pub pcr_pid: u16,
    /// Raw program_info descriptor bytes.
    /// Program-info descriptor loop. Serializes as the typed descriptor
    /// sequence; `.raw()` yields the wire bytes.
    pub program_info: DescriptorLoop<'a>,
    /// Elementary streams in wire order.
    pub streams: Vec<PmtStream<'a>>,
}

impl<'a> PmtSection<'a> {
    /// Construct a `PmtSection` from its fields.
    ///
    /// This is the canonical constructor for external code. `PmtSection` is
    /// `#[non_exhaustive]` so struct literal syntax is not available outside
    /// the crate; use this function instead.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        program_number: u16,
        version_number: u8,
        current_next_indicator: bool,
        section_number: u8,
        last_section_number: u8,
        pcr_pid: u16,
        program_info: DescriptorLoop<'a>,
        streams: Vec<PmtStream<'a>>,
    ) -> Self {
        Self {
            program_number,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            pcr_pid,
            program_info,
            streams,
        }
    }
}

impl<'a> Parse<'a> for PmtSection<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len =
            MIN_HEADER_LEN + EXTENSION_HEADER_LEN + PCR_PID_LEN + PROG_INFO_LEN_BYTES + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "PmtSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "PmtSection",
                expected: &[TABLE_ID],
            });
        }

        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let total = super::check_section_length(
            bytes.len(),
            MIN_HEADER_LEN,
            section_length as usize,
            MIN_SECTION_LEN,
        )?;

        let program_number = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        let pcr_pid = (((bytes[8] & 0x1F) as u16) << 8) | bytes[9] as u16;
        let program_info_length = (((bytes[10] & 0x0F) as usize) << 8) | bytes[11] as usize;

        let prog_info_start =
            MIN_HEADER_LEN + EXTENSION_HEADER_LEN + PCR_PID_LEN + PROG_INFO_LEN_BYTES;
        let prog_info_end = prog_info_start + program_info_length;
        let stream_loop_end = total - CRC_LEN;
        if prog_info_end > stream_loop_end {
            return Err(Error::SectionLengthOverflow {
                declared: program_info_length,
                available: stream_loop_end.saturating_sub(prog_info_start),
            });
        }
        let program_info = DescriptorLoop::new(&bytes[prog_info_start..prog_info_end]);

        let mut streams = Vec::new();
        let mut pos = prog_info_end;
        while pos + STREAM_HEADER_LEN <= stream_loop_end {
            let stream_type = StreamType::from_u8(bytes[pos]);
            let elementary_pid = (((bytes[pos + 1] & 0x1F) as u16) << 8) | bytes[pos + 2] as u16;
            let es_info_length =
                (((bytes[pos + 3] & 0x0F) as usize) << 8) | bytes[pos + 4] as usize;
            let es_start = pos + STREAM_HEADER_LEN;
            let es_end = es_start + es_info_length;
            if es_end > stream_loop_end {
                return Err(Error::SectionLengthOverflow {
                    declared: es_info_length,
                    available: stream_loop_end.saturating_sub(es_start),
                });
            }
            streams.push(PmtStream {
                stream_type,
                elementary_pid,
                es_info: DescriptorLoop::new(&bytes[es_start..es_end]),
            });
            pos = es_end;
        }

        Ok(PmtSection {
            program_number,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            pcr_pid,
            program_info,
            streams,
        })
    }
}

impl Serialize for PmtSection<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let streams_bytes: usize = self
            .streams
            .iter()
            .map(|s| STREAM_HEADER_LEN + s.es_info.len())
            .sum();
        MIN_HEADER_LEN
            + EXTENSION_HEADER_LEN
            + PCR_PID_LEN
            + PROG_INFO_LEN_BYTES
            + self.program_info.len()
            + streams_bytes
            + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        let section_length: u16 = (len - MIN_HEADER_LEN) as u16;
        buf[0] = TABLE_ID;
        buf[1] = super::SECTION_B1_FLAGS_PSI | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..5].copy_from_slice(&self.program_number.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        buf[8] = 0xE0 | ((self.pcr_pid >> 8) as u8 & 0x1F);
        buf[9] = (self.pcr_pid & 0xFF) as u8;
        let pil = self.program_info.len() as u16;
        buf[10] = 0xF0 | ((pil >> 8) as u8 & 0x0F);
        buf[11] = (pil & 0xFF) as u8;

        let prog_info_start =
            MIN_HEADER_LEN + EXTENSION_HEADER_LEN + PCR_PID_LEN + PROG_INFO_LEN_BYTES;
        buf[prog_info_start..prog_info_start + self.program_info.len()]
            .copy_from_slice(self.program_info.raw());

        let mut pos = prog_info_start + self.program_info.len();
        for stream in &self.streams {
            buf[pos] = stream.stream_type.to_u8();
            buf[pos + 1] = 0xE0 | ((stream.elementary_pid >> 8) as u8 & 0x1F);
            buf[pos + 2] = (stream.elementary_pid & 0xFF) as u8;
            let esl = stream.es_info.len() as u16;
            buf[pos + 3] = 0xF0 | ((esl >> 8) as u8 & 0x0F);
            buf[pos + 4] = (esl & 0xFF) as u8;
            let es_start = pos + STREAM_HEADER_LEN;
            buf[es_start..es_start + stream.es_info.len()].copy_from_slice(stream.es_info.raw());
            pos = es_start + stream.es_info.len();
        }

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for PmtSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "PROGRAM_MAP";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a PMT section with given fields. Placeholder CRC.
    fn build_pmt(
        program_number: u16,
        version: u8,
        pcr_pid: u16,
        program_info: &[u8],
        streams: &[(u8, u16, Vec<u8>)],
    ) -> Vec<u8> {
        let streams_bytes: usize = streams
            .iter()
            .map(|(_, _, es)| STREAM_HEADER_LEN + es.len())
            .sum();
        let section_length: u16 = (EXTENSION_HEADER_LEN
            + PCR_PID_LEN
            + PROG_INFO_LEN_BYTES
            + program_info.len()
            + streams_bytes
            + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(TABLE_ID);
        v.push(super::super::SECTION_B1_FLAGS_PSI | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&program_number.to_be_bytes());
        v.push(0xC0 | ((version & 0x1F) << 1) | 0x01);
        v.push(0);
        v.push(0);
        v.push(0xE0 | ((pcr_pid >> 8) as u8 & 0x1F));
        v.push((pcr_pid & 0xFF) as u8);
        v.push(0xF0 | ((program_info.len() >> 8) as u8 & 0x0F));
        v.push((program_info.len() & 0xFF) as u8);
        v.extend_from_slice(program_info);
        for (stype, pid, es) in streams {
            v.push(*stype);
            v.push(0xE0 | ((pid >> 8) as u8 & 0x1F));
            v.push((pid & 0xFF) as u8);
            v.push(0xF0 | ((es.len() >> 8) as u8 & 0x0F));
            v.push((es.len() & 0xFF) as u8);
            v.extend_from_slice(es);
        }
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_extracts_pcr_pid_and_program_info() {
        let bytes = build_pmt(42, 5, 0x0100, &[0xAA, 0xBB], &[]);
        let pmt = PmtSection::parse(&bytes).unwrap();
        assert_eq!(pmt.program_number, 42);
        assert_eq!(pmt.version_number, 5);
        assert!(pmt.current_next_indicator);
        assert_eq!(pmt.pcr_pid, 0x0100);
        assert_eq!(pmt.program_info.raw(), &[0xAA, 0xBB]);
        assert_eq!(pmt.streams.len(), 0);
    }

    #[test]
    fn parse_elementary_streams_and_es_info_slices() {
        let bytes = build_pmt(
            1,
            0,
            0x101,
            &[],
            &[(0x02, 0x102, vec![0x11, 0x22]), (0x1B, 0x103, vec![0x33])],
        );
        let pmt = PmtSection::parse(&bytes).unwrap();
        assert_eq!(pmt.streams.len(), 2);
        assert_eq!(pmt.streams[0].stream_type, StreamType::Mpeg2Video);
        assert_eq!(pmt.streams[0].elementary_pid, 0x102);
        assert_eq!(pmt.streams[0].es_info.raw(), &[0x11, 0x22]);
        assert_eq!(pmt.streams[1].stream_type, StreamType::H264);
        assert_eq!(pmt.streams[1].elementary_pid, 0x103);
        assert_eq!(pmt.streams[1].es_info.raw(), &[0x33]);
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_pmt(1, 0, 0x100, &[], &[]);
        bytes[0] = 0x00;
        let err = PmtSection::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x00, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = PmtSection::parse(&[0x02, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip_empty_program() {
        let pmt = PmtSection {
            program_number: 1,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            pcr_pid: 0x100,
            program_info: DescriptorLoop::new(&[]),
            streams: vec![],
        };
        let mut buf = vec![0u8; pmt.serialized_len()];
        pmt.serialize_into(&mut buf).unwrap();
        let re = PmtSection::parse(&buf).unwrap();
        assert_eq!(pmt, re);
    }

    #[test]
    fn serialize_round_trip_with_streams_and_descriptors() {
        let prog_info: [u8; 3] = [0x09, 0x01, 0xFF];
        let es1: [u8; 4] = [0x52, 0x02, 0xAA, 0xBB];
        let es2: [u8; 2] = [0x0A, 0x00];
        let pmt = PmtSection {
            program_number: 0xABCD,
            version_number: 7,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            pcr_pid: 0x1F0,
            program_info: DescriptorLoop::new(&prog_info),
            streams: vec![
                PmtStream {
                    stream_type: StreamType::Mpeg2Video,
                    elementary_pid: 0x100,
                    es_info: DescriptorLoop::new(&es1),
                },
                PmtStream {
                    stream_type: StreamType::Mpeg1Audio,
                    elementary_pid: 0x101,
                    es_info: DescriptorLoop::new(&es2),
                },
                PmtStream {
                    stream_type: StreamType::H264,
                    elementary_pid: 0x102,
                    es_info: DescriptorLoop::new(&[]),
                },
            ],
        };
        let mut buf = vec![0u8; pmt.serialized_len()];
        pmt.serialize_into(&mut buf).unwrap();
        let re = PmtSection::parse(&buf).unwrap();
        assert_eq!(pmt, re);
    }

    #[test]
    fn zero_elementary_streams_is_valid() {
        let bytes = build_pmt(99, 0, 0x0100, &[], &[]);
        let pmt = PmtSection::parse(&bytes).unwrap();
        assert_eq!(pmt.streams.len(), 0);
    }

    #[test]
    fn parse_preserves_raw_program_info_bytes() {
        let pi = vec![0x09, 0x04, 0x01, 0x02, 0x03, 0x04];
        let bytes = build_pmt(1, 0, 0x100, &pi, &[]);
        let pmt = PmtSection::parse(&bytes).unwrap();
        assert_eq!(pmt.program_info.raw(), &pi[..]);
    }

    #[test]
    fn parse_rejects_zero_section_length() {
        let mut buf = vec![0u8; 64];
        buf[0] = TABLE_ID;
        buf[1] = 0xF0;
        buf[2] = 0x00;
        for b in &mut buf[3..] {
            *b = 0xFF;
        }
        assert!(matches!(
            PmtSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn stream_type_full_range_round_trip() {
        for byte in 0u8..=0xFF {
            let st = StreamType::from_u8(byte);
            assert_eq!(
                st.to_u8(),
                byte,
                "StreamType round-trip failed for {byte:#04x}"
            );
        }
    }

    #[test]
    fn stream_type_named_values() {
        assert_eq!(StreamType::Mpeg2Video.to_u8(), 0x02);
        assert_eq!(StreamType::H264.to_u8(), 0x1B);
        assert_eq!(StreamType::Hevc.to_u8(), 0x24);
        assert_eq!(StreamType::Vvc.to_u8(), 0x33);
        assert_eq!(StreamType::MediaOrchestration.to_u8(), 0x30);
        assert_eq!(StreamType::Mvcd.to_u8(), 0x26);
        assert_eq!(StreamType::Temi.to_u8(), 0x27);
        assert_eq!(StreamType::Ac3.to_u8(), 0x81);
        assert_eq!(StreamType::Scte35.to_u8(), 0x86);
        assert_eq!(StreamType::EAc3.to_u8(), 0x87);
        assert_eq!(StreamType::AacAdts.to_u8(), 0x0F);
        assert_eq!(StreamType::IpmpHigh.to_u8(), 0x7F);
    }

    #[test]
    fn stream_type_names() {
        assert_eq!(StreamType::Mpeg2Video.name(), "MPEG-2 Video");
        assert_eq!(StreamType::H264.name(), "H.264/AVC");
        assert_eq!(StreamType::Hevc.name(), "HEVC/H.265");
        assert_eq!(StreamType::Vvc.name(), "VVC/H.266");
        assert_eq!(StreamType::MediaOrchestration.name(), "Media Orchestration");
        assert_eq!(StreamType::Mvcd.name(), "MVCD (H.264 Annex I)");
        assert_eq!(StreamType::Temi.name(), "TEMI");
        assert_eq!(StreamType::DsmCc.name(), "DSM-CC");
        assert_eq!(StreamType::Ac3.name(), "AC-3");
        assert_eq!(StreamType::Scte35.name(), "SCTE-35");
    }

    #[test]
    fn stream_type_wire_to_name() {
        assert_eq!(StreamType::from_u8(0x02).name(), "MPEG-2 Video");
        assert_eq!(StreamType::from_u8(0x1B).name(), "H.264/AVC");
        assert_eq!(StreamType::from_u8(0x24).name(), "HEVC/H.265");
        assert_eq!(StreamType::from_u8(0x00).name(), "Reserved");
        assert_eq!(StreamType::from_u8(0x81).name(), "AC-3");
    }

    /// Regression test for #181: non-zero section_number/last_section_number must
    /// survive a serialize → parse round-trip. Before the fix, serialize_into
    /// hardcoded buf[6]=0 and buf[7]=0, so this test would fail.
    #[test]
    fn section_number_round_trip_nonzero() {
        let pmt = PmtSection {
            program_number: 42,
            version_number: 1,
            current_next_indicator: true,
            section_number: 3,
            last_section_number: 7,
            pcr_pid: 0x0200,
            program_info: DescriptorLoop::new(&[]),
            streams: vec![],
        };
        let mut buf = vec![0u8; pmt.serialized_len()];
        pmt.serialize_into(&mut buf).unwrap();
        // Wire bytes[6] and [7] must carry the encoded values.
        assert_eq!(buf[6], 3, "wire byte[6] must be section_number=3");
        assert_eq!(buf[7], 7, "wire byte[7] must be last_section_number=7");
        // Re-parse must recover the same values.
        let re = PmtSection::parse(&buf).unwrap();
        assert_eq!(re.section_number, 3);
        assert_eq!(re.last_section_number, 7);
        assert_eq!(pmt, re);
    }
}
