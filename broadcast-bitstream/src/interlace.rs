//! Video interlace facts from the coded bytes: the sequence's scan mode and
//! the frame/field structure of one access unit.
//!
//! Both come from the parameter sets and the coded bytes — never guessed:
//!
//! 1. **Per sequence** — [`VideoScan`]: how the SPS says the sequence is
//!    coded.
//! 2. **Per access unit** — [`PictureFacts`]: frame vs field, first-field
//!    parity, and the `pic_struct` / `source_scan_type` of its picture timing
//!    SEI when the stream carries one ([`avc_facts`] / [`hevc_facts`]).
//!
//! The consumer supplies the parameter sets the AU is parsed against and the
//! NAL units of one sample; it owns the pairing of coded fields into whole
//! access units.
//!
//! Specs (transcribed in `private/specs-verified/mpeg/transmux/codec/h264-vui-pic-timing.md`,
//! `private/specs-verified/mpeg/transmux/codec/h265-vui-pic-timing.md`, `private/specs-verified/mpeg/transmux/codec/h264-slice-header.md`):
//! H.264 §7.3.2.1.1, §7.3.3, §D.1.3/D.2.3 + Table D-1, §E.1.1-E.1.2;
//! H.265 §7.3.7, §D.2.3/D.3.3 + Table D.2, §E.2.1-E.3.1.

use crate::bitreader::BitReader;
use crate::nal::sei_messages;
#[cfg(feature = "h264")]
use crate::nal::{AVC_NAL_HEADER_LEN, AVC_NAL_IDR, AVC_NAL_SEI, AVC_NAL_SLICE, NAL_TYPE_MASK};
#[cfg(feature = "hevc")]
use crate::nal::{HEVC_NAL_HEADER_LEN, HEVC_PREFIX_SEI_NUT};
#[cfg(feature = "h264")]
use crate::slice_header::{AvcParamSets, AvcSliceHeader};
#[cfg(feature = "hevc")]
use crate::sps::decode_hevc_interlace_sps;
#[cfg(feature = "h264")]
use crate::sps::{AvcPicTimingLayout, AvcSliceSps};

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

/// First HEVC non-VCL `nal_unit_type` (VCL types are 0..=31).
#[cfg(feature = "hevc")]
const HEVC_NAL_FIRST_NON_VCL: u8 = 32;

/// `payloadType` of `pic_timing` in both H.264 (§D.1.1) and H.265 (§D.2.1).
const SEI_PAYLOAD_PIC_TIMING: u32 = 1;

/// Bits of `pic_struct` (H.264 §D.1.3, H.265 §D.2.3).
const PIC_STRUCT_BITS: usize = 4;
/// Bits of HEVC `source_scan_type` (§D.2.3).
#[cfg(feature = "hevc")]
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
/// the access-unit decoders report `None` for them.
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
    #[cfg(feature = "hevc")]
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

/// The coded interlace structure of one video access unit.
///
/// `#[non_exhaustive]`: obtain it from [`avc_facts`] or [`hevc_facts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PictureFacts {
    field_pic: bool,
    fields: u8,
    bottom_field_first: Option<bool>,
    pic_struct: Option<PicStruct>,
    source_scan_type: Option<SourceScanType>,
}

impl PictureFacts {
    /// The sample's coded picture structure.
    #[must_use]
    pub fn new(
        field_pic: bool,
        fields: u8,
        bottom_field_first: Option<bool>,
        pic_struct: Option<PicStruct>,
        source_scan_type: Option<SourceScanType>,
    ) -> Self {
        Self {
            field_pic,
            fields,
            bottom_field_first,
            pic_struct,
            source_scan_type,
        }
    }

    /// The sample's picture is a field: H.264 `field_pic_flag`, or HEVC
    /// `field_seq_flag` (every picture of a field sequence is a field).
    #[must_use]
    pub fn field_pic(&self) -> bool {
        self.field_pic
    }

    /// Coded field pictures in the sample: 0 for a frame-coded picture, 1 for
    /// a lone field, 2 for a complementary field pair (what the PAFF pairing in
    /// a TS demuxer emits as one sample; H.264 only).
    #[must_use]
    pub fn fields(&self) -> u8 {
        self.fields
    }

    /// `Some(true)` when the bottom field is first in display order,
    /// `Some(false)` when the top field is. For an H.264 field/field-pair the
    /// parity of the first slice's field (authoritative, from the slice
    /// header); otherwise taken from `pic_struct` when it names an order;
    /// `None` when neither says (e.g. a progressive frame).
    #[must_use]
    pub fn bottom_field_first(&self) -> Option<bool> {
        self.bottom_field_first
    }

    /// `pic_struct` from the sample's picture timing SEI, when the SPS says it
    /// is carried (`pic_struct_present_flag` / `frame_field_info_present_flag`),
    /// the sample has one, and the value is not reserved.
    #[must_use]
    pub fn pic_struct(&self) -> Option<PicStruct> {
        self.pic_struct
    }

    /// HEVC `source_scan_type` from the picture timing SEI (`None` for H.264,
    /// whose pic_timing carries only `ct_type` inside clock timestamps, which
    /// is not read).
    #[must_use]
    pub fn source_scan_type(&self) -> Option<SourceScanType> {
        self.source_scan_type
    }
}

/// Read a `pic_timing` payload's `pic_struct` for H.264 given the SPS layout.
#[cfg(feature = "h264")]
fn avc_pic_struct(payload: &[u8], layout: AvcPicTimingLayout) -> Option<PicStruct> {
    let mut r = BitReader::from_rbsp(payload, "H.264 pic_timing").ok()?;
    if let Some((cpb, dpb)) = layout.delay_bits() {
        r.read_bits(usize::from(cpb), "cpb_removal_delay").ok()?;
        r.read_bits(usize::from(dpb), "dpb_output_delay").ok()?;
    }
    let code = r.read_bits(PIC_STRUCT_BITS, "pic_struct").ok()?;
    PicStruct::from_code(code as u8, false)
}

/// The coded interlace structure of one H.264 access unit. `ps` is the
/// parameter-set cache the slice headers parse against; `nals` are the NAL
/// units of one sample. `None` when no slice parses (missing SPS/PPS).
#[cfg(feature = "h264")]
pub fn avc_facts(ps: &AvcParamSets, nals: &[&[u8]]) -> Option<PictureFacts> {
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
                if f.field_pic_flag()
                    && h.field_pic_flag()
                    && f.bottom_field_flag() != h.bottom_field_flag()
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
    let bottom_field_first = if head.field_pic_flag() {
        Some(head.bottom_field_flag())
    } else {
        pic_struct.and_then(|p| p.bottom_first())
    };
    Some(PictureFacts::new(
        head.field_pic_flag(),
        match (head.field_pic_flag(), second_field) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        },
        bottom_field_first,
        pic_struct,
        None,
    ))
}

/// The coded interlace structure of one HEVC access unit, from its SPS NAL and
/// the NAL units of the sample. `None` when the SPS does not parse or the
/// sample carries no VCL NAL.
#[cfg(feature = "hevc")]
pub fn hevc_facts(sps_nal: &[u8], nals: &[&[u8]]) -> Option<PictureFacts> {
    let sps = decode_hevc_interlace_sps(sps_nal).ok()?;
    let nal_type = |n: &[u8]| crate::nal::nal_unit_type(crate::nal::NalCodec::Hevc, n);
    // A sample with no VCL NAL is not a picture.
    if !nals
        .iter()
        .any(|n| nal_type(n).is_some_and(|t| t < HEVC_NAL_FIRST_NON_VCL))
    {
        return None;
    }
    let mut pic_struct = None;
    let mut source_scan_type = None;
    if sps.frame_field_info_present() {
        let payload = nals
            .iter()
            .filter(|n| nal_type(n) == Some(HEVC_PREFIX_SEI_NUT) && n.len() > HEVC_NAL_HEADER_LEN)
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
    Some(PictureFacts::new(
        sps.field_seq(),
        u8::from(sps.field_seq()),
        pic_struct.and_then(|p| p.bottom_first()),
        pic_struct,
        source_scan_type,
    ))
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

    #[cfg(feature = "hevc")]
    #[test]
    fn source_scan_type_codes_map_per_d33() {
        assert_eq!(SourceScanType::from_code(0), SourceScanType::Interlaced);
        assert_eq!(SourceScanType::from_code(1), SourceScanType::Progressive);
        assert_eq!(SourceScanType::from_code(2), SourceScanType::Unknown);
        assert_eq!(SourceScanType::from_code(3), SourceScanType::Unknown);
    }
}

#[cfg(test)]
mod access_unit_facts {
    use super::*;
    #[cfg(feature = "h264")]
    use crate::slice_header::AvcParamSets;
    #[cfg(feature = "h264")]
    use crate::sps::decode_avc_slice_sps;
    use alloc::vec;
    use alloc::vec::Vec;
    use test_fixtures::bits::BitWriter;

    #[cfg(feature = "hevc")]
    /// Build an HEVC SPS NAL whose RPS section exercises inter-RPS prediction
    /// (so the VUI is only reachable with the exact (7-61)/(7-62) counts) and
    /// whose VUI carries `field_seq`/`frame_field_info_present`.
    fn hevc_interlace_sps(field_seq: bool, ffi: bool, vui: bool, ptl_bits: u64) -> Vec<u8> {
        hevc_sps_full(field_seq, ffi, vui, ptl_bits, None)
    }

    /// `aspect_ratio_idc` and, for Extended_SAR, `(sar_width, sar_height)`; the
    /// writer of the `aspect_ratio_info_present_flag` block (identical H.264
    /// §E.1.1 / H.265 §E.2.1 syntax).
    #[cfg(feature = "hevc")]
    type AspectBits = Option<(u8, Option<(u16, u16)>)>;

    #[cfg(feature = "hevc")]
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

    #[cfg(feature = "hevc")]
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

    #[cfg(feature = "hevc")]
    fn hevc_nal(nal_type: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![nal_type << 1, 0x01];
        v.extend_from_slice(body);
        v
    }

    /// A HEVC prefix SEI carrying a `pic_timing` with the given
    /// `pic_struct`/`source_scan_type`/`duplicate_flag` (§D.2.3).
    #[cfg(feature = "hevc")]
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

    #[cfg(feature = "hevc")]
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
    #[cfg(feature = "h264")]
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

    #[cfg(feature = "h264")]
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
    #[cfg(feature = "h264")]
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
    #[cfg(feature = "h264")]
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

    #[cfg(feature = "h264")]
    fn avc_ps(sps: &[u8]) -> AvcParamSets {
        let mut ps = AvcParamSets::new();
        ps.observe_nal(sps);
        ps.observe_nal(&avc_pps());
        ps
    }

    #[cfg(feature = "h264")]
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

    #[cfg(feature = "h264")]
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

    #[cfg(feature = "h264")]
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
