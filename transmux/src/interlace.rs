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
//! Specs (transcribed in `docs/codec/h264-vui-pic-timing.md`,
//! `docs/codec/h265-vui-pic-timing.md`, `docs/codec/h264-slice-header.md`):
//! H.264 §7.3.2.1.1, §7.3.3, §D.1.3/D.2.3 + Table D-1, §E.1.1-E.1.2;
//! H.265 §7.3.7, §D.2.3/D.3.3 + Table D.2, §E.2.1-E.3.1.

use crate::annexb::iter_length_prefixed_nals_with;
use crate::bitreader::BitReader;
use crate::hevc_config::HEVCDecoderConfigurationRecord;
use crate::ir::CodecConfig;
use crate::nal::sei_messages;
use crate::slice_header::{AvcParamSets, AvcSliceHeader};
use crate::sps::{
    AvcPicTimingLayout, AvcSliceSps, decode_avc_slice_sps, decode_hevc_interlace_sps,
};

/// `nal_unit_type` of an SPS in H.264 (Table 7-1) and the NAL header mask.
const AVC_NAL_TYPE_MASK: u8 = 0x1F;
/// H.264 coded slice of a non-IDR picture.
const AVC_NAL_SLICE: u8 = 1;
/// H.264 coded slice of an IDR picture.
const AVC_NAL_IDR: u8 = 5;
/// H.264 SEI.
const AVC_NAL_SEI: u8 = 6;
/// H.264 SPS.
const AVC_NAL_SPS: u8 = 7;
/// H.264 PPS.
const AVC_NAL_PPS: u8 = 8;
/// Length of the H.264 NAL header.
const AVC_NAL_HEADER_LEN: usize = 1;

/// Right shift to the HEVC `nal_unit_type` in header byte 0 (§7.3.1.2).
const HEVC_NAL_TYPE_SHIFT: u8 = 1;
/// Mask of the HEVC `nal_unit_type` after the shift.
const HEVC_NAL_TYPE_MASK: u8 = 0x3F;
/// HEVC SPS (Table 7-1).
const HEVC_NAL_SPS: u8 = 33;
/// HEVC prefix SEI (Table 7-1).
const HEVC_NAL_PREFIX_SEI: u8 = 39;
/// First HEVC non-VCL `nal_unit_type` (VCL types are 0..=31).
const HEVC_NAL_FIRST_NON_VCL: u8 = 32;
/// Length of the HEVC NAL header.
const HEVC_NAL_HEADER_LEN: usize = 2;

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

broadcast_common::impl_spec_display!(VideoScan);

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
            9 if hevc => PicStruct::TopPairedWithPreviousBottom,
            10 if hevc => PicStruct::BottomPairedWithPreviousTop,
            11 if hevc => PicStruct::TopPairedWithNextBottom,
            12 if hevc => PicStruct::BottomPairedWithNextTop,
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

broadcast_common::impl_spec_display!(PicStruct);

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

broadcast_common::impl_spec_display!(SourceScanType);

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
        .filter(|a| a.nal_unit_type == HEVC_NAL_SPS)
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
            Some(if s.frame_mbs_only {
                VideoScan::Progressive
            } else if s.mb_adaptive_frame_field {
                VideoScan::Mbaff
            } else {
                VideoScan::Paff
            })
        }
        CodecConfig::Hevc { .. } => {
            let s = decode_hevc_interlace_sps(sps).ok()?;
            Some(if s.field_seq {
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
                    nal.first().map(|b| b & AVC_NAL_TYPE_MASK),
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
                if hevc_type(nal) == Some(HEVC_NAL_SPS) {
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
    if let Some((cpb, dpb)) = layout.delay_bits {
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
            nal.first().map(|b| b & AVC_NAL_TYPE_MASK),
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
    let pic_struct = sps.pic_timing.and_then(|layout| {
        nals.iter()
            .filter(|n| n.first().map(|b| b & AVC_NAL_TYPE_MASK) == Some(AVC_NAL_SEI))
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
    if sps.frame_field_info_present {
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
        field_pic: sps.field_seq,
        fields: u8::from(sps.field_seq),
        bottom_field_first: pic_struct.and_then(|p| p.bottom_first()),
        pic_struct,
        source_scan_type,
    })
}
