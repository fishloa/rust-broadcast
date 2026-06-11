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

/// Stream type coding — ISO/IEC 13818-1 Table 2-34.
///
/// Identifies the elementary-stream type carried in the associated PID.
/// Values 0x80–0xFF are user private.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StreamType {
    /// 0x00 — ITU-T | ISO/IEC Reserved.
    Reserved,
    /// 0x01 — ISO/IEC 11172-2 Video (MPEG-1 video).
    Mpeg1Video,
    /// 0x02 — ISO/IEC 13818-2 Video (MPEG-2 video).
    Mpeg2Video,
    /// 0x03 — ISO/IEC 11172-3 Audio (MPEG-1 audio).
    Mpeg1Audio,
    /// 0x04 — ISO/IEC 13818-3 Audio (MPEG-2 audio).
    Mpeg2Audio,
    /// 0x05 — ISO/IEC 13818-2 private_sections.
    PrivateSections,
    /// 0x06 — ISO/IEC 13818-1 PES packets containing private data.
    PesPrivateData,
    /// 0x07 — ISO/IEC 13522 MHEG.
    Mheg,
    /// 0x08 — ISO/IEC 13818-1 Annex A DSM-CC.
    DsmCc,
    /// 0x09 — ITU-T Rec. H.222.1.
    H222_1,
    /// 0x0A — ISO/IEC 13818-6 type A.
    Iso13818_6TypeA,
    /// 0x0B — ISO/IEC 13818-6 type B.
    Iso13818_6TypeB,
    /// 0x0C — ISO/IEC 13818-6 type C.
    Iso13818_6TypeC,
    /// 0x0D — ISO/IEC 13818-6 type D.
    Iso13818_6TypeD,
    /// 0x0E — ITU-T Rec. H.222.1 auxiliary.
    H222_1Aux,
    /// 0x0F — ISO/IEC 13818-7 Audio with ADTS transport syntax (AAC).
    AacAdts,
    /// 0x10 — ISO/IEC 14496-2 Visual (MPEG-4 video).
    Mpeg4Video,
    /// 0x11 — ISO/IEC 14496-3 Audio with LATM transport syntax (AAC LATM).
    AacLatm,
    /// 0x12 — ISO/IEC 14496-1 SL-packetized stream or FlexMux in PES.
    SlFlexMuxPes,
    /// 0x13 — ISO/IEC 14496-1 SL-packetized stream or FlexMux in ISO/IEC 13818-6 sections.
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
    /// 0x1A — IPMP stream (ISO/IEC 13818-11 MPEG-2 IPMP).
    Ipmp,
    /// 0x1B — ITU-T Rec. H.264 | ISO/IEC 14496-10 Video (AVC/H.264).
    H264,
    /// 0x1C — ISO/IEC 14496-3 Audio without additional transport syntax.
    Iso14496_3Audio,
    /// 0x1D — ISO/IEC 14496-17 Text.
    Iso14496_17Text,
    /// 0x1E — ISO/IEC 23002-3 Auxiliary Video.
    AuxiliaryVideo,
    /// 0x1F — ISO/IEC 14496-10 SVC sub-bitstream.
    Svc,
    /// 0x20 — ISO/IEC 14496-10 MVC sub-bitstream.
    Mvc,
    /// 0x21 — ITU-T Rec. T.800 | ISO/IEC 15444 JPEG 2000 Video.
    Jpeg2000,
    /// 0x22 — ISO/IEC 14496-2 Additional view.
    AdditionalViewRec14496_2,
    /// 0x23 — ISO/IEC 14496-10 MVC base view sub-bitstream.
    MvcBaseView,
    /// 0x24 — ITU-T Rec. H.265 | ISO/IEC 23008-2 Video (HEVC/H.265).
    Hevc,
    /// 0x25 — ISO/IEC 23008-2 HEVC Temporal Video sub-bitstream.
    HevcTemporal,
    /// 0x26 — ISO/IEC 23008-2 HEVC Temporal Video subset of HEVC Annex A.
    HevcTemporalAnnexA,
    /// 0x27 — ITU-T Rec. H.265 Annex I video sub-bitstream.
    HevcAnnexI,
    /// 0x28 — ITU-T Rec. H.265 Annex I video.
    HevcAnnexIMain,
    /// 0x29 — ISO/IEC 23008-2 HEVC Temporal Video sub-bitstream of HEVC Annex I video.
    HevcAnnexITemporal,
    /// 0x2A — ISO/IEC 23008-2 HEVC Temporal Video sub-bitstream of HEVC Annex A enhanced range extension.
    HevcTemporalEnhanced,
    /// 0x2B — ISO/IEC 23008-2 HEVC Temporal Video sub-bitstream of HEVC Annex I enhanced range extension.
    HevcAnnexITemporalEnhanced,
    /// 0x30 — ISO/IEC 23090-3 Video (VVC/H.266).
    Vvc,
    /// 0x33 — ISO/IEC 23090-3 Video (VVC/H.266).
    VvcAlt,
    /// 0x80 — User private (range 0x80..=0xFF).
    Private(u8),
    /// 0x81 — ATSC AC-3 audio.
    Ac3,
    /// 0x84 — ATSC Dolby Digital Plus (E-AC-3).
    EnhancedAc3,
    /// 0x85 — ATSC DTS-HD audio.
    DtsHd,
    /// 0x86 — ATSC DTS audio.
    Dts,
    /// 0x87 — ATSC E-AC-3 / Dolby Digital Plus audio.
    EAc3Alt,
    /// 0x8A — DTS audio.
    DtsAlt,
    /// 0x8B — DTS-HD audio.
    DtsHdAlt,
    /// 0x8C — Dolby MAT (Metadata-enhanced Audio Transmission).
    DolbyMat,
    /// 0x90 — SCTE subtitling.
    ScteSubtitling,
    /// 0x91 — ARIB subtitling.
    AribSubtitling,
    /// 0x92 — TTML subtitling.
    TtmlSubtitling,
    /// 0xEA — User private (VC-1).
    PrivateVc1,
    /// Catch-all for unallocated / reserved values not named above.
    Unallocated(u8),
}

impl StreamType {
    /// Decode from the wire byte.  Every byte maps to a variant (lossless).
    /// Decode from the wire byte.  Every byte maps to a variant (lossless).
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
            0x0E => Self::H222_1Aux,
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
            0x22 => Self::AdditionalViewRec14496_2,
            0x23 => Self::MvcBaseView,
            0x24 => Self::Hevc,
            0x25 => Self::HevcTemporal,
            0x26 => Self::HevcTemporalAnnexA,
            0x27 => Self::HevcAnnexI,
            0x28 => Self::HevcAnnexIMain,
            0x29 => Self::HevcAnnexITemporal,
            0x2A => Self::HevcTemporalEnhanced,
            0x2B => Self::HevcAnnexITemporalEnhanced,
            0x30 => Self::Vvc,
            0x33 => Self::VvcAlt,
            0x80 => Self::Private(0x80),
            0x81 => Self::Ac3,
            0x84 => Self::EnhancedAc3,
            0x85 => Self::DtsHd,
            0x86 => Self::Dts,
            0x87 => Self::EAc3Alt,
            0x8A => Self::DtsAlt,
            0x8B => Self::DtsHdAlt,
            0x8C => Self::DolbyMat,
            0x90 => Self::ScteSubtitling,
            0x91 => Self::AribSubtitling,
            0x92 => Self::TtmlSubtitling,
            0xEA => Self::PrivateVc1,
            v if v >= 0x80 => Self::Private(v),
            _ => Self::Unallocated(v),
        }
    }

    /// Encode to the wire byte.  Inverse of `from_u8`.
    /// Encode to the wire byte.  Inverse of `from_u8`.
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
            Self::H222_1Aux => 0x0E,
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
            Self::AdditionalViewRec14496_2 => 0x22,
            Self::MvcBaseView => 0x23,
            Self::Hevc => 0x24,
            Self::HevcTemporal => 0x25,
            Self::HevcTemporalAnnexA => 0x26,
            Self::HevcAnnexI => 0x27,
            Self::HevcAnnexIMain => 0x28,
            Self::HevcAnnexITemporal => 0x29,
            Self::HevcTemporalEnhanced => 0x2A,
            Self::HevcAnnexITemporalEnhanced => 0x2B,
            Self::Vvc => 0x30,
            Self::VvcAlt => 0x33,
            Self::Ac3 => 0x81,
            Self::EnhancedAc3 => 0x84,
            Self::DtsHd => 0x85,
            Self::Dts => 0x86,
            Self::EAc3Alt => 0x87,
            Self::DtsAlt => 0x8A,
            Self::DtsHdAlt => 0x8B,
            Self::DolbyMat => 0x8C,
            Self::ScteSubtitling => 0x90,
            Self::AribSubtitling => 0x91,
            Self::TtmlSubtitling => 0x92,
            Self::PrivateVc1 => 0xEA,
            Self::Private(v) | Self::Unallocated(v) => v,
        }
    }

    /// Human-readable spec display name.
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
            Self::H222_1Aux => "H.222.1 Auxiliary",
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
            Self::AdditionalViewRec14496_2 => "Additional View Rec. 14496-2",
            Self::MvcBaseView => "MVC Base View",
            Self::Hevc => "HEVC/H.265",
            Self::HevcTemporal => "HEVC Temporal",
            Self::HevcTemporalAnnexA => "HEVC Temporal Annex A",
            Self::HevcAnnexI => "HEVC Annex I",
            Self::HevcAnnexIMain => "HEVC Annex I Main",
            Self::HevcAnnexITemporal => "HEVC Annex I Temporal",
            Self::HevcTemporalEnhanced => "HEVC Temporal Enhanced",
            Self::HevcAnnexITemporalEnhanced => "HEVC Annex I Temporal Enhanced",
            Self::Vvc => "VVC/H.266",
            Self::VvcAlt => "VVC/H.266 (alt)",
            Self::Ac3 => "AC-3",
            Self::EnhancedAc3 => "Enhanced AC-3",
            Self::DtsHd => "DTS-HD",
            Self::Dts => "DTS",
            Self::EAc3Alt => "E-AC-3",
            Self::DtsAlt => "DTS (alt)",
            Self::DtsHdAlt => "DTS-HD (alt)",
            Self::DolbyMat => "Dolby MAT",
            Self::ScteSubtitling => "SCTE Subtitling",
            Self::AribSubtitling => "ARIB Subtitling",
            Self::TtmlSubtitling => "TTML Subtitling",
            Self::PrivateVc1 => "VC-1 (Private)",
            Self::Private(_) => "User Private",
            Self::Unallocated(_) => "Unallocated",
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
    /// 13-bit PCR PID.
    pub pcr_pid: u16,
    /// Raw program_info descriptor bytes.
    /// Program-info descriptor loop. Serializes as the typed descriptor
    /// sequence; `.raw()` yields the wire bytes.
    pub program_info: DescriptorLoop<'a>,
    /// Elementary streams in wire order.
    pub streams: Vec<PmtStream<'a>>,
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
        buf[6] = 0;
        buf[7] = 0;
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
        assert_eq!(StreamType::VvcAlt.to_u8(), 0x33);
        assert_eq!(StreamType::Ac3.to_u8(), 0x81);
        assert_eq!(StreamType::EAc3Alt.to_u8(), 0x87);
        assert_eq!(StreamType::AacAdts.to_u8(), 0x0F);
    }

    #[test]
    fn stream_type_names() {
        assert_eq!(StreamType::Mpeg2Video.name(), "MPEG-2 Video");
        assert_eq!(StreamType::H264.name(), "H.264/AVC");
        assert_eq!(StreamType::Hevc.name(), "HEVC/H.265");
        assert_eq!(StreamType::Vvc.name(), "VVC/H.266");
        assert_eq!(StreamType::DsmCc.name(), "DSM-CC");
        assert_eq!(StreamType::Ac3.name(), "AC-3");
    }
}
