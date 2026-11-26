//! Video interlace facts for live-player consumers (issue #1162, R3).
//!
//! Two layers, both derived from the codec config and the coded bytes — never
//! guessed:
//!
//! 1. **Per track** — [`video_scan`] / [`CodecConfig::video_scan`]: how the
//!    SPS says the sequence is coded ([`VideoScan`]).
//! 2. **Per sample** — [`video_picture_facts`]: for one sample of a track (as
//!    [`StreamingTsDemux`](crate::StreamingTsDemux) emits it, i.e. a
//!    length-prefixed access unit) the coded structure of its picture
//!    ([`PictureFacts`]): frame vs field, first-field parity, and the
//!    `pic_struct` / `source_scan_type` of its picture timing SEI when the
//!    stream carries one.
//!
//! Why a free function over `(config, sample bytes)` rather than a field on
//! [`Sample`](crate::Sample): `Sample` is the hot, widely constructed IR
//! type and is shared with every demuxer and muxer; the facts need the
//! track's parameter sets anyway, are only wanted by display-side consumers,
//! and costing a slice-header parse per sample is not something every
//! consumer should pay. The function is additive and touches no existing API.
//!
//! Specs (transcribed in `private/specs-verified/mpeg/transmux/codec/h264-vui-pic-timing.md`,
//! `private/specs-verified/mpeg/transmux/codec/h265-vui-pic-timing.md`, `private/specs-verified/mpeg/transmux/codec/h264-slice-header.md`):
//! H.264 §7.3.2.1.1, §7.3.3, §D.1.3/D.2.3 + Table D-1, §E.1.1-E.1.2;
//! H.265 §7.3.7, §D.2.3/D.3.3 + Table D.2, §E.2.1-E.3.1.

use crate::ir::CodecConfig;
use crate::slice_header::{AvcParamSets, AvcSliceHeader};
use broadcast_bitstream::annexb::iter_length_prefixed_nals_with;
use broadcast_bitstream::bitreader::BitReader;
use broadcast_bitstream::nal::{
    AVC_NAL_IDR, AVC_NAL_PPS, AVC_NAL_SEI, AVC_NAL_SPS, HEVC_NAL_HEADER_LEN, HEVC_NAL_TYPE_MASK,
    NAL_TYPE_MASK, sei_messages,
};
use broadcast_bitstream::sps::{
    AvcPicTimingLayout, AvcSliceSps, decode_avc_slice_sps, decode_hevc_interlace_sps,
};
use isobmff::hevc_config::HEVCDecoderConfigurationRecord;
use isobmff::nalu_types::HEVC_SPS_NUT;

/// `pic_struct` 9 (H.265 Table D.2): top field paired with the previous
/// bottom field in output order.
const PIC_STRUCT_TOP_PAIRED_PREVIOUS_BOTTOM: u8 = 9;
/// `pic_struct` 10 (H.265 Table D.2): bottom field paired with the previous
/// top field.
const PIC_STRUCT_BOTTOM_PAIRED_PREVIOUS_TOP: u8 = 10;
/// `pic_struct` 11 (H.265 Table D.2): top field paired with the next bottom
/// field.
const PIC_STRUCT_TOP_PAIRED_NEXT_BOTTOM: u8 = 11;
/// `pic_struct` 12 (H.265 Table D.2): bottom field paired with the next top
/// field.
const PIC_STRUCT_BOTTOM_PAIRED_NEXT_TOP: u8 = 12;

/// H.264 coded slice of a non-IDR picture.
const AVC_NAL_SLICE: u8 = 1;
/// Length of the H.264 NAL header.
const AVC_NAL_HEADER_LEN: usize = 1;

/// Right shift to the HEVC `nal_unit_type` in header byte 0 (§7.3.1.2).
const HEVC_NAL_TYPE_SHIFT: u8 = 1;
/// HEVC prefix SEI (Table 7-1).
const HEVC_NAL_PREFIX_SEI: u8 = 39;
/// First HEVC non-VCL `nal_unit_type` (VCL types are 0..=31).
const HEVC_NAL_FIRST_NON_VCL: u8 = 32;

/// `payloadType` of `pic_timing` in both H.264 (§D.1.1) and H.265 (§D.2.1).
const SEI_PAYLOAD_PIC_TIMING: u32 = 1;

/// Bits of `pic_struct` (H.264 §D.1.3, H.265 §D.2.3).
const PIC_STRUCT_BITS: usize = 4;
/// Bits of HEVC `source_scan_type` (§D.2.3).
const SOURCE_SCAN_TYPE_BITS: usize = 2;

/// How a video sequence is coded, from its SPS.
///
/// These are properties of the *coding*, not of the source content: a
/// [`VideoScan::Progressive`] H.264 stream may carry interlaced-origin material
/// coded as frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum VideoScan {
    /// Frame-coded only: H.264 `frame_mbs_only_flag == 1`, or HEVC
    /// `field_seq_flag == 0` (every picture is a frame).
    Progressive,
    /// H.264 `frame_mbs_only_flag == 0` and `mb_adaptive_frame_field_flag == 0`:
    /// each picture is a frame or a field (picture-adaptive frame/field,
    /// PAFF), chosen per picture by `field_pic_flag`.
    Paff,
    /// H.264 `frame_mbs_only_flag == 0` and `mb_adaptive_frame_field_flag == 1`:
    /// frames are coded with macroblock-adaptive frame/field (MBAFF); pictures
    /// may still be single fields (`field_pic_flag == 1`) — "mixed".
    Mbaff,
    /// HEVC `field_seq_flag == 1`: the sequence conveys pictures that
    /// represent fields.
    FieldSequence,
}

impl VideoScan {
    /// The short spec-token label.
    pub fn name(&self) -> &'static str {
        match self {
            VideoScan::Progressive => "progressive",
            VideoScan::Paff => "PAFF",
            VideoScan::Mbaff => "MBAFF",
            VideoScan::FieldSequence => "field_seq",
        }
    }
}

broadcast_core::impl_spec_display!(VideoScan);

/// `pic_struct` of a picture timing SEI: H.264 Table D-1 (values 0..=8) or
/// H.265 Table D.2 (values 0..=12). Reserved values are not representable;
/// [`video_picture_facts`] reports `None` for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PicStruct {
    /// 0: (progressive) frame.
    Frame,
    /// 1: top field.
    TopField,
    /// 2: bottom field.
    BottomField,
    /// 3: top field, bottom field, in that order.
    TopBottom,
    /// 4: bottom field, top field, in that order.
    BottomTop,
    /// 5: top field, bottom field, top field repeated.
    TopBottomTop,
    /// 6: bottom field, top field, bottom field repeated.
    BottomTopBottom,
    /// 7: frame doubling.
    FrameDoubling,
    /// 8: frame tripling.
    FrameTripling,
    /// 9 (HEVC only): top field paired with the previous bottom field in
    /// output order.
    TopPairedWithPreviousBottom,
    /// 10 (HEVC only): bottom field paired with the previous top field.
    BottomPairedWithPreviousTop,
    /// 11 (HEVC only): top field paired with the next bottom field.
    TopPairedWithNextBottom,
    /// 12 (HEVC only): bottom field paired with the next top field.
    BottomPairedWithNextTop,
}

impl PicStruct {
    /// Decode a coded `pic_struct`. `hevc` selects the wider H.265 table;
    /// reserved values yield `None`.
    pub fn from_code(code: u8, hevc: bool) -> Option<Self> {
        Some(match code {
            0 => PicStruct::Frame,
            1 => PicStruct::TopField,
            2 => PicStruct::BottomField,
            3 => PicStruct::TopBottom,
            4 => PicStruct::BottomTop,
            5 => PicStruct::TopBottomTop,
            6 => PicStruct::BottomTopBottom,
            7 => PicStruct::FrameDoubling,
            8 => PicStruct::FrameTripling,
            PIC_STRUCT_TOP_PAIRED_PREVIOUS_BOTTOM if hevc => PicStruct::TopPairedWithPreviousBottom,
            PIC_STRUCT_BOTTOM_PAIRED_PREVIOUS_TOP if hevc => PicStruct::BottomPairedWithPreviousTop,
            PIC_STRUCT_TOP_PAIRED_NEXT_BOTTOM if hevc => PicStruct::TopPairedWithNextBottom,
            PIC_STRUCT_BOTTOM_PAIRED_NEXT_TOP if hevc => PicStruct::BottomPairedWithNextTop,
            _ => return None,
        })
    }

    /// The coded value (Table D-1 / D.2).
    pub fn code(&self) -> u8 {
        match self {
            PicStruct::Frame => 0,
            PicStruct::TopField => 1,
            PicStruct::BottomField => 2,
            PicStruct::TopBottom => 3,
            PicStruct::BottomTop => 4,
            PicStruct::TopBottomTop => 5,
            PicStruct::BottomTopBottom => 6,
            PicStruct::FrameDoubling => 7,
            PicStruct::FrameTripling => 8,
            PicStruct::TopPairedWithPreviousBottom => 9,
            PicStruct::BottomPairedWithPreviousTop => 10,
            PicStruct::TopPairedWithNextBottom => 11,
            PicStruct::BottomPairedWithNextTop => 12,
        }
    }

    /// The short spec-token label.
    pub fn name(&self) -> &'static str {
        match self {
            PicStruct::Frame => "frame",
            PicStruct::TopField => "top field",
            PicStruct::BottomField => "bottom field",
            PicStruct::TopBottom => "top, bottom",
            PicStruct::BottomTop => "bottom, top",
            PicStruct::TopBottomTop => "top, bottom, top repeated",
            PicStruct::BottomTopBottom => "bottom, top, bottom repeated",
            PicStruct::FrameDoubling => "frame doubling",
            PicStruct::FrameTripling => "frame tripling",
            PicStruct::TopPairedWithPreviousBottom => "top paired with previous bottom",
            PicStruct::BottomPairedWithPreviousTop => "bottom paired with previous top",
            PicStruct::TopPairedWithNextBottom => "top paired with next bottom",
            PicStruct::BottomPairedWithNextTop => "bottom paired with next top",
        }
    }

    /// Which field comes first in display order, when this `pic_struct` says:
    /// `Some(false)` top first, `Some(true)` bottom first.
    fn bottom_first(&self) -> Option<bool> {
        match self {
            PicStruct::TopField
            | PicStruct::TopBottom
            | PicStruct::TopBottomTop
            | PicStruct::TopPairedWithPreviousBottom
            | PicStruct::TopPairedWithNextBottom => Some(false),
            PicStruct::BottomField
            | PicStruct::BottomTop
            | PicStruct::BottomTopBottom
            | PicStruct::BottomPairedWithPreviousTop
            | PicStruct::BottomPairedWithNextTop => Some(true),
            PicStruct::Frame | PicStruct::FrameDoubling | PicStruct::FrameTripling => None,
        }
    }
}

broadcast_core::impl_spec_display!(PicStruct);

/// HEVC `source_scan_type` of a picture timing SEI (H.265 §D.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SourceScanType {
    /// 0: interlaced source.
    Interlaced,
    /// 1: progressive source.
    Progressive,
    /// 2: unknown/unspecified (the reserved value 3 is interpreted as 2).
    Unknown,
}

impl SourceScanType {
    fn from_code(code: u8) -> Self {
        match code {
            0 => SourceScanType::Interlaced,
            1 => SourceScanType::Progressive,
            _ => SourceScanType::Unknown,
        }
    }

    /// The short spec-token label.
    pub fn name(&self) -> &'static str {
        match self {
            SourceScanType::Interlaced => "interlaced",
            SourceScanType::Progressive => "progressive",
            SourceScanType::Unknown => "unknown",
        }
    }
}

broadcast_core::impl_spec_display!(SourceScanType);

/// The coded interlace structure of one video sample.
///
/// `#[non_exhaustive]`: obtain it from [`video_picture_facts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PictureFacts {
    /// The sample's picture is a field: H.264 `field_pic_flag`, or HEVC
    /// `field_seq_flag` (every picture of a field sequence is a field).
    pub field_pic: bool,
    /// Coded field pictures in the sample: 0 for a frame-coded picture, 1 for
    /// a lone field, 2 for a complementary field pair (what the PAFF pairing in
    /// the TS demuxer emits as one sample; H.264 only).
    pub fields: u8,
    /// `Some(true)` when the bottom field is first in display order,
    /// `Some(false)` when the top field is. For an H.264 field/field-pair the
    /// parity of the first slice's field (authoritative, from the slice
    /// header); otherwise taken from `pic_struct` when it names an order;
    /// `None` when neither says (e.g. a progressive frame).
    pub bottom_field_first: Option<bool>,
    /// `pic_struct` from the sample's picture timing SEI, when the SPS says it
    /// is carried (`pic_struct_present_flag` / `frame_field_info_present_flag`),
    /// the sample has one, and the value is not reserved.
    pub pic_struct: Option<PicStruct>,
    /// HEVC `source_scan_type` from the picture timing SEI (`None` for H.264,
    /// whose pic_timing carries only `ct_type` inside clock timestamps, which
    /// is not read).
    pub source_scan_type: Option<SourceScanType>,
}

impl CodecConfig {
    /// How this track's video is coded, from its SPS ([`video_scan`]); `None`
    /// for non-AVC/HEVC configs or an unparseable SPS.
    pub fn video_scan(&self) -> Option<VideoScan> {
        video_scan(self)
    }
}

/// The first SPS NAL of an `hvcC` record.
fn hevc_config_sps(rec: &HEVCDecoderConfigurationRecord) -> Option<&[u8]> {
    rec.arrays
        .iter()
        .filter(|a| a.nal_unit_type == HEVC_SPS_NUT)
        .flat_map(|a| a.nalus.iter())
        .next()
        .map(|n| n.0.as_slice())
}

/// The first SPS NAL of an AVC/HEVC config.
fn first_sps(config: &CodecConfig) -> Option<&[u8]> {
    match config {
        CodecConfig::Avc { config, .. } => config.config.sps.first().map(|s| s.0.as_slice()),
        CodecConfig::Hevc { config, .. } => hevc_config_sps(&config.config),
        _ => None,
    }
}

/// How the track's SPS says its video is coded ([`VideoScan`]); `None` for a
/// non-AVC/HEVC config, or when the SPS does not parse.
pub fn video_scan(config: &CodecConfig) -> Option<VideoScan> {
    let sps = first_sps(config)?;
    match config {
        CodecConfig::Avc { .. } => {
            let s = decode_avc_slice_sps(sps).ok()?;
            Some(if s.frame_mbs_only() {
                VideoScan::Progressive
            } else if s.mb_adaptive_frame_field() {
                VideoScan::Mbaff
            } else {
                VideoScan::Paff
            })
        }
        CodecConfig::Hevc { .. } => {
            let s = decode_hevc_interlace_sps(sps).ok()?;
            Some(if s.field_seq() {
                VideoScan::FieldSequence
            } else {
                VideoScan::Progressive
            })
        }
        _ => None,
    }
}

/// The coded interlace structure of one sample of an AVC/HEVC track.
///
/// `sample` is the length-prefixed access unit exactly as the demuxers emit it
/// (the prefix width comes from the config's `lengthSizeMinusOne`). Parameter
/// sets are taken from `config` and from any in-band SPS/PPS in the sample.
/// Returns `None` for a non-AVC/HEVC config, a malformed sample, a sample with
/// no parseable picture (e.g. an H.264 slice whose SPS/PPS is unknown), or an
/// HEVC track whose SPS does not parse.
pub fn video_picture_facts(config: &CodecConfig, sample: &[u8]) -> Option<PictureFacts> {
    match config {
        CodecConfig::Avc { config, .. } => {
            let rec = &config.config;
            let nals =
                iter_length_prefixed_nals_with(sample, usize::from(rec.length_size_minus_one) + 1)
                    .ok()?;
            let mut ps = AvcParamSets::new();
            for sps in &rec.sps {
                ps.observe_nal(&sps.0);
            }
            for pps in &rec.pps {
                ps.observe_nal(&pps.0);
            }
            for nal in &nals {
                if matches!(
                    nal.first().map(|b| b & NAL_TYPE_MASK),
                    Some(AVC_NAL_SPS | AVC_NAL_PPS)
                ) {
                    ps.observe_nal(nal);
                }
            }
            avc_facts(&ps, &nals)
        }
        CodecConfig::Hevc { config, .. } => {
            let rec = &config.config;
            let nals =
                iter_length_prefixed_nals_with(sample, usize::from(rec.length_size_minus_one) + 1)
                    .ok()?;
            let mut sps_nal = hevc_config_sps(rec)?;
            // An in-band SPS supersedes the config's.
            for nal in &nals {
                if hevc_type(nal) == Some(HEVC_SPS_NUT) {
                    sps_nal = nal;
                }
            }
            hevc_facts(sps_nal, &nals)
        }
        _ => None,
    }
}

fn hevc_type(nal: &[u8]) -> Option<u8> {
    nal.first()
        .map(|b| (b >> HEVC_NAL_TYPE_SHIFT) & HEVC_NAL_TYPE_MASK)
}

/// Read a `pic_timing` payload's `pic_struct` for H.264 given the SPS layout.
fn avc_pic_struct(payload: &[u8], layout: AvcPicTimingLayout) -> Option<PicStruct> {
    let mut r = BitReader::from_rbsp(payload, "H.264 pic_timing").ok()?;
    if let Some((cpb, dpb)) = layout.delay_bits() {
        r.read_bits(usize::from(cpb), "cpb_removal_delay").ok()?;
        r.read_bits(usize::from(dpb), "dpb_output_delay").ok()?;
    }
    let code = r.read_bits(PIC_STRUCT_BITS, "pic_struct").ok()?;
    PicStruct::from_code(code as u8, false)
}

pub(crate) fn avc_facts(ps: &AvcParamSets, nals: &[&[u8]]) -> Option<PictureFacts> {
    let mut first: Option<(AvcSliceHeader, AvcSliceSps)> = None;
    let mut second_field = false;
    for nal in nals {
        if !matches!(
            nal.first().map(|b| b & NAL_TYPE_MASK),
            Some(AVC_NAL_SLICE | AVC_NAL_IDR)
        ) {
            continue;
        }
        let Some((h, sps)) = ps.parse_slice(nal) else {
            continue;
        };
        match &first {
            None => first = Some((h, sps)),
            Some((f, _)) => {
                if f.field_pic_flag
                    && h.field_pic_flag
                    && f.bottom_field_flag != h.bottom_field_flag
                {
                    second_field = true;
                }
            }
        }
    }
    let (head, sps) = first?;
    let pic_struct = sps.pic_timing().and_then(|layout| {
        nals.iter()
            .filter(|n| n.first().map(|b| b & NAL_TYPE_MASK) == Some(AVC_NAL_SEI))
            .flat_map(|n| sei_messages(&n[AVC_NAL_HEADER_LEN..]))
            .find(|(t, _)| *t == SEI_PAYLOAD_PIC_TIMING)
            .and_then(|(_, p)| avc_pic_struct(&p, layout))
    });
    let bottom_field_first = if head.field_pic_flag {
        Some(head.bottom_field_flag)
    } else {
        pic_struct.and_then(|p| p.bottom_first())
    };
    Some(PictureFacts {
        field_pic: head.field_pic_flag,
        fields: match (head.field_pic_flag, second_field) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        },
        bottom_field_first,
        pic_struct,
        source_scan_type: None,
    })
}

pub(crate) fn hevc_facts(sps_nal: &[u8], nals: &[&[u8]]) -> Option<PictureFacts> {
    let sps = decode_hevc_interlace_sps(sps_nal).ok()?;
    // A sample with no VCL NAL is not a picture.
    if !nals
        .iter()
        .any(|n| hevc_type(n).is_some_and(|t| t < HEVC_NAL_FIRST_NON_VCL))
    {
        return None;
    }
    let mut pic_struct = None;
    let mut source_scan_type = None;
    if sps.frame_field_info_present() {
        let payload = nals
            .iter()
            .filter(|n| hevc_type(n) == Some(HEVC_NAL_PREFIX_SEI) && n.len() > HEVC_NAL_HEADER_LEN)
            .flat_map(|n| sei_messages(&n[HEVC_NAL_HEADER_LEN..]))
            .find(|(t, _)| *t == SEI_PAYLOAD_PIC_TIMING);
        if let Some((_, p)) = payload
            && let Ok(mut r) = BitReader::from_rbsp(&p, "H.265 pic_timing")
        {
            pic_struct = r
                .read_bits(PIC_STRUCT_BITS, "pic_struct")
                .ok()
                .and_then(|c| PicStruct::from_code(c as u8, true));
            source_scan_type = r
                .read_bits(SOURCE_SCAN_TYPE_BITS, "source_scan_type")
                .ok()
                .map(|c| SourceScanType::from_code(c as u8));
        }
    }
    Some(PictureFacts {
        field_pic: sps.field_seq(),
        fields: u8::from(sps.field_seq()),
        bottom_field_first: pic_struct.and_then(|p| p.bottom_first()),
        pic_struct,
        source_scan_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pic_struct_codes_match_tables_d1_and_d2() {
        for code in 0..=8u8 {
            let h264 = PicStruct::from_code(code, false)
                .unwrap_or_else(|| panic!("H.264 Table D-1 code {code}"));
            assert_eq!(h264.code(), code);
            let h265 = PicStruct::from_code(code, true)
                .unwrap_or_else(|| panic!("H.265 Table D.2 code {code}"));
            assert_eq!(h265, h264);
        }
        for code in 9..=12u8 {
            assert_eq!(
                PicStruct::from_code(code, false),
                None,
                "H.264 Table D-1 reserved {code}"
            );
            let h265 = PicStruct::from_code(code, true)
                .unwrap_or_else(|| panic!("H.265 Table D.2 code {code}"));
            assert_eq!(h265.code(), code);
        }
        assert_eq!(PicStruct::from_code(13, false), None);
        assert_eq!(PicStruct::from_code(13, true), None);
    }

    #[test]
    fn source_scan_type_codes_map_per_d33() {
        assert_eq!(SourceScanType::from_code(0), SourceScanType::Interlaced);
        assert_eq!(SourceScanType::from_code(1), SourceScanType::Progressive);
        assert_eq!(SourceScanType::from_code(2), SourceScanType::Unknown);
        assert_eq!(SourceScanType::from_code(3), SourceScanType::Unknown);
    }

    use crate::ir::CodecConfig;
    use alloc::vec;
    use alloc::vec::Vec;
    use isobmff::hevc_config::{HEVCConfigurationBox, HEVCDecoderConfigurationRecord};
    use isobmff::nalu_types::{HevcNalArray, HevcNalUnit};
    use test_fixtures::bits::BitWriter;

    fn hevc_nal(nal_type: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![nal_type << 1, 0x01];
        v.extend_from_slice(body);
        v
    }

    /// An HEVC SPS whose VUI carries `frame_field_info_present_flag = 1` and
    /// `field_seq_flag = 0` (H.265 §7.3.2.3 / §E.2.1), written field by field.
    fn hevc_sps_frame_field_info() -> Vec<u8> {
        let mut nal = vec![0x42u8, 0x01];
        let mut s = BitWriter::new();
        s.u(4, 0); // sps_video_parameter_set_id
        s.u(3, 0); // sps_max_sub_layers_minus1
        s.bit(true); // sps_temporal_id_nesting_flag
        s.u(2, 0); // general_profile_space
        s.bit(false); // general_tier_flag
        s.u(5, 1); // general_profile_idc
        s.u(32, 0); // general_profile_compatibility_flags
        s.u(48, 0); // general_constraint_indicator_flags
        s.u(8, 93); // general_level_idc
        s.ue(0); // sps_seq_parameter_set_id
        s.ue(1); // chroma_format_idc
        s.ue(16); // pic_width_in_luma_samples
        s.ue(16); // pic_height_in_luma_samples
        s.bit(false); // conformance_window_flag
        s.ue(0); // bit_depth_luma_minus8
        s.ue(0); // bit_depth_chroma_minus8
        s.ue(3); // log2_max_pic_order_cnt_lsb_minus4
        s.bit(false); // sps_sub_layer_ordering_info_present_flag
        s.ue(0); // sps_max_dec_pic_buffering_minus1
        s.ue(0); // sps_max_num_reorder_pics
        s.ue(0); // sps_max_latency_increase_plus1
        s.ue(0); // log2_min_luma_coding_block_size_minus3
        s.ue(3); // log2_diff_max_min_luma_coding_block_size
        s.ue(0); // log2_min_luma_transform_block_size_minus2
        s.ue(3); // log2_diff_max_min_luma_transform_block_size
        s.ue(1); // max_transform_hierarchy_depth_inter
        s.ue(1); // max_transform_hierarchy_depth_intra
        s.bit(false); // scaling_list_enabled_flag
        s.bit(false); // amp_enabled_flag
        s.bit(false); // sample_adaptive_offset_enabled_flag
        s.bit(false); // pcm_enabled_flag
        s.ue(3); // num_short_term_ref_pic_sets
        // set 0: explicit, DeltaPocS0 = [-1], DeltaPocS1 = [+2]
        s.ue(1);
        s.ue(1);
        s.ue(0);
        s.bit(true);
        s.ue(1);
        s.bit(true);
        // set 1: inter from set 0
        s.bit(true);
        s.bit(true);
        s.ue(2);
        s.bit(true);
        s.bit(false);
        s.bit(false);
        s.bit(true);
        // set 2: inter from set 1
        s.bit(true);
        s.bit(false);
        s.ue(0);
        for _ in 0..3 {
            s.bit(true);
        }
        s.bit(false); // long_term_ref_pics_present_flag
        s.bit(false); // sps_temporal_mvp_enabled_flag
        s.bit(false); // strong_intra_smoothing_enabled_flag
        s.bit(true); // vui_parameters_present_flag
        s.bit(false); // aspect_ratio_info_present_flag
        s.bit(false); // overscan_info_present_flag
        s.bit(false); // video_signal_type_present_flag
        s.bit(false); // chroma_loc_info_present_flag
        s.bit(false); // neutral_chroma_indication_flag
        s.bit(false); // field_seq_flag
        s.bit(true); // frame_field_info_present_flag
        s.bit(false); // default_display_window_flag
        s.bit(false); // vui_timing_info_present_flag
        s.bit(false); // bitstream_restriction_flag
        s.bit(true); // rbsp_stop_one_bit
        nal.extend(s.finish());
        nal
    }

    /// A prefix SEI NAL carrying a `pic_timing` (§D.2.3) with the given
    /// `pic_struct` and `source_scan_type`.
    fn hevc_pic_timing_sei(pic_struct: u64, source_scan_type: u64) -> Vec<u8> {
        let mut p = BitWriter::new();
        p.u(4, pic_struct);
        p.u(2, source_scan_type);
        p.bit(false); // duplicate_flag
        let payload = p.finish();
        let mut body = vec![1u8, payload.len() as u8];
        body.extend_from_slice(&payload);
        body.push(0x80); // rbsp_trailing_bits
        hevc_nal(39, &body)
    }

    /// An hvcC carrying a VPS array before the SPS array (the order in a real
    /// record) and 4-byte NAL length prefixes.
    fn hevc_config() -> CodecConfig {
        let record = HEVCDecoderConfigurationRecord {
            configuration_version: 1,
            general_profile_space: 0,
            general_tier_flag: false,
            general_profile_idc: 1,
            general_profile_compatibility_flags: 0,
            general_constraint_indicator_flags: 0,
            general_level_idc: 93,
            min_spatial_segmentation_idc: 0,
            parallelism_type: 0,
            chroma_format_idc: 1,
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            avg_frame_rate: 0,
            constant_frame_rate: 0,
            num_temporal_layers: 1,
            temporal_id_nested: true,
            length_size_minus_one: 3,
            arrays: vec![
                HevcNalArray::new(
                    true,
                    32,
                    vec![HevcNalUnit::new(hevc_nal(32, &[0x0C, 0x01]))],
                ),
                HevcNalArray::new(
                    true,
                    33,
                    vec![HevcNalUnit::new(hevc_sps_frame_field_info())],
                ),
            ],
        };
        CodecConfig::Hevc {
            config: HEVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        }
    }

    fn access_unit(nals: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in nals {
            out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            out.extend_from_slice(nal);
        }
        out
    }

    #[test]
    fn hevc_facts_come_from_the_config_sps_and_prefix_sei() {
        let config = hevc_config();

        // A sample with only a VPS (nal_unit_type 32) is not a picture.
        let vps = hevc_nal(32, &[0x0C, 0x01]);
        assert_eq!(video_picture_facts(&config, &access_unit(&[vps])), None);

        // A 4-byte length prefix (lengthSizeMinusOne 3) over a VCL NAL and the
        // pic_timing prefix SEI. The VCL NAL is longer than 64 KiB, so reading
        // the prefixes with a 2-byte width desynchronizes the whole sample.
        let mut vcl = vec![0x02, 0x01];
        vcl.resize(65_536, 0xFF);
        let sei = hevc_pic_timing_sei(3, 0);
        let facts = video_picture_facts(&config, &access_unit(&[vcl, sei]))
            .expect("VCL + pic_timing SEI yields facts");
        assert!(!facts.field_pic);
        assert_eq!(facts.fields, 0);
        assert_eq!(facts.pic_struct, Some(PicStruct::TopBottom));
        assert_eq!(facts.bottom_field_first, Some(false));
        assert_eq!(facts.source_scan_type, Some(SourceScanType::Interlaced));
    }

    #[test]
    fn hevc_only_prefix_sei_nals_are_read_as_sei() {
        let config = hevc_config();
        // A VCL NAL whose body happens to look like a pic_timing message
        // (payloadType 1, payload 0x20 = pic_struct 2) must not be read as SEI.
        let disguised_vcl = hevc_nal(1, &[0x01, 0x01, 0x20, 0x80]);
        let sei = hevc_pic_timing_sei(3, 0);
        let facts = video_picture_facts(&config, &access_unit(&[disguised_vcl, sei.clone()]))
            .expect("the real prefix SEI still decides");
        assert_eq!(facts.pic_struct, Some(PicStruct::TopBottom));

        // A header-only (2-byte) prefix SEI NAL carries no message and is skipped.
        let vcl = hevc_nal(1, &[0x80]);
        let empty_sei = hevc_nal(39, &[]);
        let facts = video_picture_facts(&config, &access_unit(&[vcl, empty_sei, sei]))
            .expect("the real prefix SEI still decides");
        assert_eq!(facts.pic_struct, Some(PicStruct::TopBottom));
    }
}

#[cfg(test)]
mod access_unit_facts {
    use super::*;
    use crate::slice_header::AvcParamSets;
    use alloc::vec;
    use alloc::vec::Vec;
    use broadcast_bitstream::sps::decode_avc_slice_sps;
    use test_fixtures::bits::BitWriter;

    /// Build an HEVC SPS NAL whose RPS section exercises inter-RPS prediction
    /// (so the VUI is only reachable with the exact (7-61)/(7-62) counts) and
    /// whose VUI carries `field_seq`/`frame_field_info_present`.
    fn hevc_interlace_sps(field_seq: bool, ffi: bool, vui: bool, ptl_bits: u64) -> Vec<u8> {
        hevc_sps_full(field_seq, ffi, vui, ptl_bits, None)
    }

    /// `aspect_ratio_idc` and, for Extended_SAR, `(sar_width, sar_height)`; the
    /// writer of the `aspect_ratio_info_present_flag` block (identical H.264
    /// §E.1.1 / H.265 §E.2.1 syntax).
    type AspectBits = Option<(u8, Option<(u16, u16)>)>;

    fn put_aspect(s: &mut BitWriter, aspect: AspectBits) {
        s.bit(aspect.is_some()); // aspect_ratio_info_present_flag
        if let Some((idc, ext)) = aspect {
            s.u(8, u64::from(idc)); // aspect_ratio_idc
            if let Some((w, h)) = ext {
                s.u(16, u64::from(w)); // sar_width
                s.u(16, u64::from(h)); // sar_height
            }
        }
    }

    fn hevc_sps_full(
        field_seq: bool,
        ffi: bool,
        vui: bool,
        ptl_bits: u64,
        aspect: AspectBits,
    ) -> Vec<u8> {
        let mut nal = vec![0x42u8, 0x01];
        let mut s = BitWriter::new();
        s.u(4, 0); // sps_video_parameter_set_id
        s.u(3, 0); // sps_max_sub_layers_minus1
        s.bit(true); // sps_temporal_id_nesting_flag
        s.u(2, 0); // general_profile_space
        s.bit(false); // general_tier_flag
        s.u(5, 1); // general_profile_idc
        s.u(32, 0); // general_profile_compatibility_flags
        s.u(48, ptl_bits); // general_constraint_indicator_flags
        s.u(8, 93); // general_level_idc
        s.ue(0); // sps_seq_parameter_set_id
        s.ue(1); // chroma_format_idc
        s.ue(16); // pic_width_in_luma_samples
        s.ue(16); // pic_height_in_luma_samples
        s.bit(false); // conformance_window_flag
        s.ue(0); // bit_depth_luma_minus8
        s.ue(0); // bit_depth_chroma_minus8
        s.ue(3); // log2_max_pic_order_cnt_lsb_minus4
        s.bit(false); // sps_sub_layer_ordering_info_present_flag
        s.ue(0);
        s.ue(0);
        s.ue(0); // sps_max_dec_pic_buffering/num_reorder/latency
        s.ue(0);
        s.ue(3);
        s.ue(0);
        s.ue(3); // coding/transform block sizes
        s.ue(1);
        s.ue(1); // max_transform_hierarchy_depth_inter/intra
        s.bit(false); // scaling_list_enabled_flag
        s.bit(false); // amp_enabled_flag
        s.bit(false); // sample_adaptive_offset_enabled_flag
        s.bit(false); // pcm_enabled_flag
        s.ue(3); // num_short_term_ref_pic_sets
        // set 0: explicit, DeltaPocS0 = [-1], DeltaPocS1 = [+2]
        s.ue(1); // num_negative_pics
        s.ue(1); // num_positive_pics
        s.ue(0); // delta_poc_s0_minus1 -> -1
        s.bit(true); // used_by_curr_pic_s0_flag
        s.ue(1); // delta_poc_s1_minus1 -> +2
        s.bit(true); // used_by_curr_pic_s1_flag
        // set 1: inter from set 0 (NumDeltaPocs = 2 -> 3 flag groups),
        // deltaRps = -3. Candidates: s1[0] -> -1 (j = 1, dropped below),
        // deltaRps entry (j = 2) -> -3, s0[0] -> -4 (j = 0): derives
        // S0 = [-3, -4], S1 = [] (2 entries).
        s.bit(true); // inter_ref_pic_set_prediction_flag
        s.bit(true); // delta_rps_sign
        s.ue(2); // abs_delta_rps_minus1
        s.bit(true); // used_by_curr_pic_flag[0]
        s.bit(false); // used_by_curr_pic_flag[1]
        s.bit(false); // use_delta_flag[1] = 0: -1 is not kept
        s.bit(true); // used_by_curr_pic_flag[2]
        // set 2: inter from set 1 (NumDeltaPocs = 2 -> 3 flags)
        s.bit(true); // inter_ref_pic_set_prediction_flag
        s.bit(false); // delta_rps_sign
        s.ue(0); // abs_delta_rps_minus1
        for _ in 0..3 {
            s.bit(true); // used_by_curr_pic_flag[j]
        }
        s.bit(false); // long_term_ref_pics_present_flag
        s.bit(false); // sps_temporal_mvp_enabled_flag
        s.bit(false); // strong_intra_smoothing_enabled_flag
        s.bit(vui); // vui_parameters_present_flag
        if vui {
            put_aspect(&mut s, aspect);
            s.bit(false); // overscan_info_present_flag
            s.bit(false); // video_signal_type_present_flag
            s.bit(false); // chroma_loc_info_present_flag
            s.bit(false); // neutral_chroma_indication_flag
            s.bit(field_seq); // field_seq_flag
            s.bit(ffi); // frame_field_info_present_flag
            s.bit(false); // default_display_window_flag
            s.bit(false); // vui_timing_info_present_flag
            s.bit(false); // bitstream_restriction_flag
        }
        s.bit(true); // rbsp_stop_one_bit
        nal.extend(s.finish());
        nal
    }

    fn hevc_nal(nal_type: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![nal_type << 1, 0x01];
        v.extend_from_slice(body);
        v
    }

    /// A HEVC prefix SEI carrying a `pic_timing` with the given
    /// `pic_struct`/`source_scan_type`/`duplicate_flag` (§D.2.3).
    fn hevc_pic_timing_sei(pic_struct: u64, scan: u64) -> Vec<u8> {
        let mut p = BitWriter::new();
        p.u(4, pic_struct);
        p.u(2, scan);
        p.bit(false); // duplicate_flag
        let payload = p.finish(); // one byte (7 bits + pad)
        let mut body = vec![1u8, payload.len() as u8]; // payloadType 1, size
        body.extend_from_slice(&payload);
        body.push(0x80); // rbsp_trailing_bits
        hevc_nal(39, &body)
    }

    #[test]
    fn hevc_pic_struct_and_scan_type_are_read_from_prefix_sei() {
        let sps = hevc_interlace_sps(true, true, true, 0);
        let vcl = hevc_nal(1, &[0x80]);
        for (code, want, bottom_first) in [
            (1u64, PicStruct::TopField, Some(false)),
            (2, PicStruct::BottomField, Some(true)),
            (10, PicStruct::BottomPairedWithPreviousTop, Some(true)),
            (11, PicStruct::TopPairedWithNextBottom, Some(false)),
        ] {
            let sei = hevc_pic_timing_sei(code, 0);
            let f = hevc_facts(&sps, &[sei.as_slice(), vcl.as_slice()]).expect("facts");
            assert!(f.field_pic);
            assert_eq!(f.fields, 1);
            assert_eq!(f.pic_struct, Some(want));
            assert_eq!(f.bottom_field_first, bottom_first);
            assert_eq!(f.source_scan_type, Some(SourceScanType::Interlaced));
        }
        // source_scan_type 3 (reserved) reads as unknown (§D.3.3).
        let sei = hevc_pic_timing_sei(1, 3);
        let f = hevc_facts(&sps, &[sei.as_slice(), vcl.as_slice()]).unwrap();
        assert_eq!(f.source_scan_type, Some(SourceScanType::Unknown));
        // Reserved pic_struct (13) is reported as None, not guessed.
        let sei = hevc_pic_timing_sei(13, 0);
        let f = hevc_facts(&sps, &[sei.as_slice(), vcl.as_slice()]).unwrap();
        assert_eq!(f.pic_struct, None);
        // A SEI-less picture of a field sequence is still a field, parity unknown.
        let f = hevc_facts(&sps, &[vcl.as_slice()]).unwrap();
        assert!(f.field_pic);
        assert_eq!((f.pic_struct, f.bottom_field_first), (None, None));
        // No VCL NAL at all: not a picture.
        assert!(hevc_facts(&sps, &[sei.as_slice()]).is_none());
        // frame_field_info_present = 0: the SEI is not interpreted.
        let sps0 = hevc_interlace_sps(false, false, true, 0);
        let sei = hevc_pic_timing_sei(4, 0);
        let f = hevc_facts(&sps0, &[sei.as_slice(), vcl.as_slice()]).unwrap();
        assert!(!f.field_pic);
        assert_eq!(f.pic_struct, None);
    }

    /// Build an H.264 SPS (profile 66, `pic_order_cnt_type` 2 so slice headers
    /// carry no POC syntax). `hrd`: optional `(cpb_removal_delay_length_minus1,
    /// dpb_output_delay_length_minus1)` for the NAL HRD, `vcl` likewise.
    fn avc_interlace_sps(
        mbaff: bool,
        pic_struct_present: bool,
        nal_hrd: Option<(u64, u64)>,
        vcl_hrd: Option<(u64, u64)>,
    ) -> Vec<u8> {
        fn hrd(s: &mut BitWriter, (cpb, dpb): (u64, u64)) {
            s.ue(1); // cpb_cnt_minus1 (two schedules, so the loop must iterate)
            s.u(4, 0); // bit_rate_scale
            s.u(4, 0); // cpb_size_scale
            for _ in 0..2 {
                s.ue(7); // bit_rate_value_minus1
                s.ue(9); // cpb_size_value_minus1
                s.bit(true); // cbr_flag
            }
            s.u(5, 22); // initial_cpb_removal_delay_length_minus1
            s.u(5, cpb); // cpb_removal_delay_length_minus1
            s.u(5, dpb); // dpb_output_delay_length_minus1
            s.u(5, 24); // time_offset_length
        }
        let mut nal = vec![0x67u8, 66, 0x00, 30];
        let mut s = BitWriter::new();
        s.ue(0); // seq_parameter_set_id
        s.ue(0); // log2_max_frame_num_minus4 -> frame_num is 4 bits
        s.ue(2); // pic_order_cnt_type
        s.ue(1); // max_num_ref_frames
        s.bit(false); // gaps_in_frame_num_value_allowed_flag
        s.ue(3); // pic_width_in_mbs_minus1
        s.ue(1); // pic_height_in_map_units_minus1
        s.bit(false); // frame_mbs_only_flag
        s.bit(mbaff); // mb_adaptive_frame_field_flag
        s.bit(true); // direct_8x8_inference_flag
        s.bit(false); // frame_cropping_flag
        s.bit(true); // vui_parameters_present_flag
        s.bit(false); // aspect_ratio_info_present_flag
        s.bit(false); // overscan_info_present_flag
        s.bit(false); // video_signal_type_present_flag
        s.bit(false); // chroma_loc_info_present_flag
        s.bit(true); // timing_info_present_flag
        s.u(32, 1); // num_units_in_tick
        s.u(32, 50); // time_scale
        s.bit(true); // fixed_frame_rate_flag
        s.bit(nal_hrd.is_some()); // nal_hrd_parameters_present_flag
        if let Some(h) = nal_hrd {
            hrd(&mut s, h);
        }
        s.bit(vcl_hrd.is_some()); // vcl_hrd_parameters_present_flag
        if let Some(h) = vcl_hrd {
            hrd(&mut s, h);
        }
        if nal_hrd.is_some() || vcl_hrd.is_some() {
            s.bit(false); // low_delay_hrd_flag
        }
        s.bit(pic_struct_present); // pic_struct_present_flag
        s.bit(false); // bitstream_restriction_flag
        s.bit(true); // rbsp_stop_one_bit
        nal.extend(s.finish());
        nal
    }

    fn avc_pps() -> Vec<u8> {
        let mut s = BitWriter::new();
        s.ue(0); // pic_parameter_set_id
        s.ue(0); // seq_parameter_set_id
        s.bit(false); // entropy_coding_mode_flag
        s.bit(false); // bottom_field_pic_order_in_frame_present_flag
        s.ue(0); // num_slice_groups_minus1
        s.ue(0);
        s.ue(0); // num_ref_idx_l0/l1_default_active_minus1
        s.bit(false); // weighted_pred_flag
        s.u(2, 0); // weighted_bipred_idc
        s.ue(0); // pic_init_qp_minus26 se(0)
        s.ue(0); // pic_init_qs_minus26
        s.ue(0); // chroma_qp_index_offset
        s.bit(false); // deblocking_filter_control_present_flag
        s.bit(false); // constrained_intra_pred_flag
        s.bit(false); // redundant_pic_cnt_present_flag
        s.bit(true);
        let mut nal = vec![0x68u8];
        nal.extend(s.finish());
        nal
    }

    /// An I slice (type 7), `frame_num` 0, as a frame or as a top/bottom field.
    fn avc_slice(field: Option<bool>) -> Vec<u8> {
        let mut s = BitWriter::new();
        s.ue(0); // first_mb_in_slice
        s.ue(7); // slice_type (I)
        s.ue(0); // pic_parameter_set_id
        s.u(4, 0); // frame_num
        match field {
            None => {
                s.bit(false); // field_pic_flag
            }
            Some(bottom) => {
                s.bit(true); // field_pic_flag
                s.bit(bottom); // bottom_field_flag
            }
        }
        s.u(8, 0); // slice data stand-in (never parsed)
        let mut nal = vec![0x61u8]; // nal_ref_idc 3, type 1
        nal.extend(s.finish());
        nal
    }

    /// An H.264 `pic_timing` SEI NAL: `cpb`/`dpb` delay bit lengths, then
    /// `pic_struct` (§D.1.3), then a zero clock-timestamp-flag tail.
    fn avc_pic_timing_sei(delays: Option<(u32, u32)>, pic_struct: u64) -> Vec<u8> {
        let mut p = BitWriter::new();
        if let Some((cpb, dpb)) = delays {
            p.u(cpb, 0x15555 & ((1 << cpb) - 1)); // cpb_removal_delay
            p.u(dpb, 0x2AAAA & ((1 << dpb) - 1)); // dpb_output_delay
        }
        p.u(4, pic_struct);
        p.u(3, 0); // clock_timestamp_flag[0..] = 0
        let payload = p.finish();
        let mut nal = vec![0x06u8, 1, payload.len() as u8];
        nal.extend_from_slice(&payload);
        nal.push(0x80);
        nal
    }

    fn avc_ps(sps: &[u8]) -> AvcParamSets {
        let mut ps = AvcParamSets::new();
        ps.observe_nal(sps);
        ps.observe_nal(&avc_pps());
        ps
    }

    #[test]
    fn avc_sps_scan_flags_and_pic_timing_layout() {
        let paff = decode_avc_slice_sps(&avc_interlace_sps(false, false, None, None)).unwrap();
        assert!(!paff.frame_mbs_only() && !paff.mb_adaptive_frame_field());
        assert_eq!(paff.pic_timing(), None, "pic_struct_present_flag = 0");
        let mbaff = decode_avc_slice_sps(&avc_interlace_sps(true, true, None, None)).unwrap();
        assert!(mbaff.mb_adaptive_frame_field());
        assert_eq!(mbaff.pic_timing(), Some(AvcPicTimingLayout::new(None)));
        let hrd = decode_avc_slice_sps(&avc_interlace_sps(true, true, Some((4, 3)), None)).unwrap();
        assert_eq!(
            hrd.pic_timing(),
            Some(AvcPicTimingLayout::new(Some((5, 4))))
        );
        let both = decode_avc_slice_sps(&avc_interlace_sps(true, true, Some((4, 3)), Some((4, 3))))
            .unwrap();
        assert_eq!(both.pic_timing(), hrd.pic_timing());
        // NAL and VCL disagree: the spec names no winner, so no pic_struct.
        let clash =
            decode_avc_slice_sps(&avc_interlace_sps(true, true, Some((4, 3)), Some((6, 3))))
                .unwrap();
        assert_eq!(clash.pic_timing(), None);
    }

    #[test]
    fn avc_pic_struct_is_read_after_the_hrd_delays() {
        let sps = avc_interlace_sps(true, true, Some((4, 3)), None);
        let ps = avc_ps(&sps);
        let frame = avc_slice(None);
        let sei = avc_pic_timing_sei(Some((5, 4)), 4);
        let f = avc_facts(&ps, &[sei.as_slice(), frame.as_slice()]).expect("facts");
        assert!(!f.field_pic);
        assert_eq!(f.fields, 0);
        assert_eq!(f.pic_struct, Some(PicStruct::BottomTop));
        assert_eq!(f.bottom_field_first, Some(true));
        // Reserved pic_struct (9 is HEVC-only) is None for H.264.
        let sei = avc_pic_timing_sei(Some((5, 4)), 9);
        let f = avc_facts(&ps, &[sei.as_slice(), frame.as_slice()]).unwrap();
        assert_eq!(f.pic_struct, None);
        // No HRD: pic_struct is the first field of the payload.
        let sps = avc_interlace_sps(false, true, None, None);
        let ps = avc_ps(&sps);
        let sei = avc_pic_timing_sei(None, 3);
        let f = avc_facts(&ps, &[sei.as_slice(), frame.as_slice()]).unwrap();
        assert_eq!(f.pic_struct, Some(PicStruct::TopBottom));
        assert_eq!(f.bottom_field_first, Some(false));
    }

    #[test]
    fn avc_field_pair_parity_comes_from_the_first_slice() {
        let ps = avc_ps(&avc_interlace_sps(false, false, None, None));
        let (top, bottom) = (avc_slice(Some(false)), avc_slice(Some(true)));
        let pair = avc_facts(&ps, &[top.as_slice(), bottom.as_slice()]).unwrap();
        assert_eq!(
            (pair.field_pic, pair.fields, pair.bottom_field_first),
            (true, 2, Some(false))
        );
        let pair = avc_facts(&ps, &[bottom.as_slice(), top.as_slice()]).unwrap();
        assert_eq!(
            (pair.field_pic, pair.fields, pair.bottom_field_first),
            (true, 2, Some(true))
        );
        let lone = avc_facts(&ps, &[bottom.as_slice()]).unwrap();
        assert_eq!((lone.fields, lone.bottom_field_first), (1, Some(true)));
        // Two slices of the SAME field are one picture, not a pair.
        let same = avc_facts(&ps, &[top.as_slice(), top.as_slice()]).unwrap();
        assert_eq!(same.fields, 1);
        // Unknown PPS: no facts.
        assert!(avc_facts(&AvcParamSets::new(), &[top.as_slice()]).is_none());
    }
}
