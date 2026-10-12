//! H.264 slice-header prefix, PPS prefix and field-pair (PAFF) detection —
//! ITU-T H.264 (08/2021) §7.3.3 (slice header), §7.3.2.2 (PPS), §7.4.1.2.4
//! (first VCL NAL unit of a primary coded picture), §3.35-3.37 (complementary
//! field pairs). Transcription: `docs/codec/h264-slice-header.md`.
//!
//! Field-coded (PAFF) H.264 is broadcast as one coded picture per *field*, one
//! PES per field. ISO/IEC 14496-15 samples are whole access units, i.e. both
//! fields of a frame ([`AvcFieldInfo::pairs_with`]); the demuxer uses
//! [`AvcParamSets::classify_au`] to find them. Only the entropy-agnostic prefix
//! of the slice header (through `redundant_pic_cnt`) is parsed. MBAFF pictures
//! are *frames* (`field_pic_flag == 0`) and classify as [`AvcPicture::Frame`].

use crate::annexb::iter_annexb_nals;
use crate::bitreader::BitReader;
use crate::error::{Error, Result};
use crate::sps::{AvcSliceSps, decode_avc_slice_sps};
use alloc::collections::BTreeMap;

/// `nal_unit_type` of a non-IDR coded slice (Table 7-1).
const NAL_SLICE: u8 = 1;
/// `nal_unit_type` of an IDR coded slice (Table 7-1).
const NAL_IDR: u8 = 5;
/// `nal_unit_type` of a sequence parameter set.
const NAL_SPS: u8 = 7;
/// `nal_unit_type` of a picture parameter set.
const NAL_PPS: u8 = 8;
/// `nal_unit_type` of an access unit delimiter.
pub(crate) const NAL_AUD: u8 = 9;
/// Low five bits of the NAL header byte.
const NAL_TYPE_MASK: u8 = 0x1F;
/// `nal_ref_idc` bits of the NAL header byte.
const NAL_REF_IDC_MASK: u8 = 0x60;
/// Shift of `nal_ref_idc` in the NAL header byte.
const NAL_REF_IDC_SHIFT: u8 = 5;
/// How many NAL bytes (after the header) a slice-header prefix is read from.
/// The prefix is a handful of Exp-Golomb codes; this bounds the unescape copy
/// so a multi-megabyte I slice is not duplicated per access unit.
const SLICE_PREFIX_BYTES: usize = 64;
/// Bits of `colour_plane_id` (§7.3.3).
const COLOUR_PLANE_ID_BITS: usize = 2;

/// The PPS fields a slice-header parser needs (§7.3.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvcSlicePps {
    /// `pic_parameter_set_id`.
    pub pps_id: u8,
    /// `seq_parameter_set_id`.
    pub sps_id: u8,
    /// `entropy_coding_mode_flag`.
    pub entropy_coding_mode: bool,
    /// `bottom_field_pic_order_in_frame_present_flag`.
    pub bottom_field_pic_order_in_frame_present: bool,
    /// `redundant_pic_cnt_present_flag`.
    pub redundant_pic_cnt_present: bool,
}

/// Decode the slice-header-relevant prefix of a PPS NAL unit (1-byte NAL
/// header included), walking the slice-group syntax to reach
/// `redundant_pic_cnt_present_flag` (§7.3.2.2).
pub fn decode_avc_slice_pps(pps_nal: &[u8]) -> Result<AvcSlicePps> {
    let body = pps_nal.get(1..).unwrap_or(&[]);
    let mut r = BitReader::with_unescape(body, "H.264 PPS")?;
    let pps_id = r.read_ue("pic_parameter_set_id")?;
    let sps_id = r.read_ue("seq_parameter_set_id")?;
    if pps_id > 255 || sps_id > 31 {
        return Err(Error::InvalidValue {
            field: "pic_parameter_set_id/seq_parameter_set_id",
            value: pps_id.max(sps_id),
            reason: "out of range (H.264 §7.4.2.2: 0..=255 / 0..=31)",
        });
    }
    let entropy_coding_mode = r.read_flag("entropy_coding_mode_flag")?;
    let bottom_field_pic_order_in_frame_present =
        r.read_flag("bottom_field_pic_order_in_frame_present_flag")?;
    let num_slice_groups_minus1 = r.read_ue("num_slice_groups_minus1")?;
    if num_slice_groups_minus1 > 7 {
        return Err(Error::InvalidValue {
            field: "num_slice_groups_minus1",
            value: num_slice_groups_minus1,
            reason: "out of range 0..=7 (H.264 §7.4.2.2)",
        });
    }
    if num_slice_groups_minus1 > 0 {
        let map_type = r.read_ue("slice_group_map_type")?;
        match map_type {
            0 => {
                for _ in 0..=num_slice_groups_minus1 {
                    r.read_ue("run_length_minus1")?;
                }
            }
            2 => {
                for _ in 0..num_slice_groups_minus1 {
                    r.read_ue("top_left")?;
                    r.read_ue("bottom_right")?;
                }
            }
            3..=5 => {
                r.read_flag("slice_group_change_direction_flag")?;
                r.read_ue("slice_group_change_rate_minus1")?;
            }
            6 => {
                let n = r.read_ue("pic_size_in_map_units_minus1")?;
                // Ceil(Log2(num_slice_groups_minus1 + 1)) bits per id (§7.4.2.2).
                let bits = (num_slice_groups_minus1 + 1)
                    .next_power_of_two()
                    .trailing_zeros();
                for _ in 0..=n {
                    r.read_bits(bits as usize, "slice_group_id")?;
                }
            }
            _ => {}
        }
    }
    r.read_ue("num_ref_idx_l0_default_active_minus1")?;
    r.read_ue("num_ref_idx_l1_default_active_minus1")?;
    r.read_flag("weighted_pred_flag")?;
    r.read_bits(2, "weighted_bipred_idc")?;
    r.read_se("pic_init_qp_minus26")?;
    r.read_se("pic_init_qs_minus26")?;
    r.read_se("chroma_qp_index_offset")?;
    r.read_flag("deblocking_filter_control_present_flag")?;
    r.read_flag("constrained_intra_pred_flag")?;
    let redundant_pic_cnt_present = r.read_flag("redundant_pic_cnt_present_flag")?;
    Ok(AvcSlicePps {
        pps_id: pps_id as u8,
        sps_id: sps_id as u8,
        entropy_coding_mode,
        bottom_field_pic_order_in_frame_present,
        redundant_pic_cnt_present,
    })
}

/// The slice-header prefix of one coded slice NAL (§7.3.3 through
/// `redundant_pic_cnt`), plus the NAL-header facts the §7.4.1.2.4 test uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvcSliceHeader {
    /// `nal_ref_idc` (0 = non-reference).
    pub nal_ref_idc: u8,
    /// `IdrPicFlag` (`nal_unit_type == 5`).
    pub idr: bool,
    /// `first_mb_in_slice`.
    pub first_mb_in_slice: u32,
    /// `slice_type` as coded (0..=9, Table 7-6).
    pub slice_type: u8,
    /// `pic_parameter_set_id`.
    pub pps_id: u8,
    /// `colour_plane_id` (only when `separate_colour_plane_flag`).
    pub colour_plane_id: Option<u8>,
    /// `frame_num`.
    pub frame_num: u32,
    /// `field_pic_flag` (inferred `false` when absent).
    pub field_pic_flag: bool,
    /// `bottom_field_flag` (inferred `false` when absent).
    pub bottom_field_flag: bool,
    /// `idr_pic_id` (IDR slices only).
    pub idr_pic_id: Option<u32>,
    /// `pic_order_cnt_lsb` (POC type 0 only).
    pub pic_order_cnt_lsb: Option<u32>,
    /// `delta_pic_order_cnt_bottom` (inferred 0).
    pub delta_pic_order_cnt_bottom: i32,
    /// `delta_pic_order_cnt[0..=1]` (inferred 0).
    pub delta_pic_order_cnt: [i32; 2],
    /// `redundant_pic_cnt` (inferred 0).
    pub redundant_pic_cnt: u32,
}

fn to_u32(v: u64, field: &'static str) -> Result<u32> {
    u32::try_from(v).map_err(|_| Error::InvalidValue {
        field,
        value: v,
        reason: "does not fit in u32",
    })
}

fn to_i32(v: i64, field: &'static str) -> Result<i32> {
    i32::try_from(v).map_err(|_| Error::InvalidValue {
        field,
        value: v as u64,
        reason: "does not fit in i32",
    })
}

/// Parse the §7.3.3 prefix of one slice NAL (`nal_unit_type` 1 or 5; 1-byte NAL
/// header included) against its SPS and PPS.
pub fn parse_avc_slice_header(
    nal: &[u8],
    sps: &AvcSliceSps,
    pps: &AvcSlicePps,
) -> Result<AvcSliceHeader> {
    let header = *nal.first().ok_or(Error::BufferTooShort {
        need: 1,
        have: 0,
        what: "H.264 slice NAL header",
    })?;
    let nal_type = header & NAL_TYPE_MASK;
    if nal_type != NAL_SLICE && nal_type != NAL_IDR {
        return Err(Error::InvalidValue {
            field: "nal_unit_type",
            value: nal_type as u64,
            reason: "not a non-partitioned coded slice (1 or 5)",
        });
    }
    let idr = nal_type == NAL_IDR;
    let nal_ref_idc = (header & NAL_REF_IDC_MASK) >> NAL_REF_IDC_SHIFT;
    let end = nal.len().min(1 + SLICE_PREFIX_BYTES);
    let mut r = BitReader::with_unescape(&nal[1..end], "H.264 slice header")?;
    let first_mb_in_slice = to_u32(r.read_ue("first_mb_in_slice")?, "first_mb_in_slice")?;
    let slice_type = r.read_ue("slice_type")?;
    if slice_type > 9 {
        return Err(Error::InvalidValue {
            field: "slice_type",
            value: slice_type,
            reason: "out of range 0..=9 (H.264 Table 7-6)",
        });
    }
    let pps_id = r.read_ue("pic_parameter_set_id")?;
    if pps_id != pps.pps_id as u64 {
        return Err(Error::InvalidValue {
            field: "pic_parameter_set_id",
            value: pps_id,
            reason: "slice refers to a different PPS than the one supplied",
        });
    }
    let colour_plane_id = if sps.separate_colour_plane {
        Some(r.read_bits(COLOUR_PLANE_ID_BITS, "colour_plane_id")? as u8)
    } else {
        None
    };
    let frame_num = r.read_bits(sps.log2_max_frame_num as usize, "frame_num")? as u32;
    let (mut field_pic_flag, mut bottom_field_flag) = (false, false);
    if !sps.frame_mbs_only {
        field_pic_flag = r.read_flag("field_pic_flag")?;
        if field_pic_flag {
            bottom_field_flag = r.read_flag("bottom_field_flag")?;
        }
    }
    let idr_pic_id = if idr {
        Some(to_u32(r.read_ue("idr_pic_id")?, "idr_pic_id")?)
    } else {
        None
    };
    let bottom_poc_present = pps.bottom_field_pic_order_in_frame_present && !field_pic_flag;
    let mut pic_order_cnt_lsb = None;
    let mut delta_pic_order_cnt_bottom = 0;
    let mut delta_pic_order_cnt = [0i32; 2];
    if sps.pic_order_cnt_type == 0 {
        pic_order_cnt_lsb =
            Some(r.read_bits(sps.log2_max_poc_lsb as usize, "pic_order_cnt_lsb")? as u32);
        if bottom_poc_present {
            delta_pic_order_cnt_bottom = to_i32(
                r.read_se("delta_pic_order_cnt_bottom")?,
                "delta_pic_order_cnt_bottom",
            )?;
        }
    }
    if sps.pic_order_cnt_type == 1 && !sps.delta_pic_order_always_zero {
        delta_pic_order_cnt[0] = to_i32(
            r.read_se("delta_pic_order_cnt[0]")?,
            "delta_pic_order_cnt[0]",
        )?;
        if bottom_poc_present {
            delta_pic_order_cnt[1] = to_i32(
                r.read_se("delta_pic_order_cnt[1]")?,
                "delta_pic_order_cnt[1]",
            )?;
        }
    }
    let redundant_pic_cnt = if pps.redundant_pic_cnt_present {
        to_u32(r.read_ue("redundant_pic_cnt")?, "redundant_pic_cnt")?
    } else {
        0
    };
    Ok(AvcSliceHeader {
        nal_ref_idc,
        idr,
        first_mb_in_slice,
        slice_type: slice_type as u8,
        pps_id: pps_id as u8,
        colour_plane_id,
        frame_num,
        field_pic_flag,
        bottom_field_flag,
        idr_pic_id,
        pic_order_cnt_lsb,
        delta_pic_order_cnt_bottom,
        delta_pic_order_cnt,
        redundant_pic_cnt,
    })
}

/// What an access unit's coded picture is, for field-pairing purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AvcPicture {
    /// A coded frame (progressive, or interlaced with `field_pic_flag == 0`,
    /// including MBAFF), or an AU holding more than one picture/parity — never
    /// merged with a neighbour.
    Frame,
    /// A single coded field.
    Field(AvcFieldInfo),
    /// The access unit carries no parseable slice (missing SPS/PPS, malformed
    /// header, non-VCL only): treated like [`AvcPicture::Frame`] by the demuxer.
    Unknown,
}

/// The identity of a coded field picture (the facts §3.36/§3.37 pair on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvcFieldInfo {
    /// `bottom_field_flag`.
    pub bottom: bool,
    /// `frame_num`.
    pub frame_num: u32,
    /// `nal_ref_idc != 0`.
    pub is_reference: bool,
    /// The field is an IDR picture.
    pub idr: bool,
}

impl AvcFieldInfo {
    /// Whether `second` (the next access unit in decoding order) completes
    /// `self` as a complementary field pair (§3.35-3.37): opposite parity, the
    /// same `frame_num`, both reference or both non-reference, and the second
    /// field not an IDR picture. (The mmco-5 exclusion of §3.37 is not
    /// checked — see the module docs. "First field not already paired" is the
    /// caller's state.)
    pub fn pairs_with(&self, second: &AvcFieldInfo) -> bool {
        self.bottom != second.bottom
            && self.frame_num == second.frame_num
            && self.is_reference == second.is_reference
            && !second.idr
    }
}

/// SPS/PPS cache for classifying access units, fed from the stream's in-band
/// parameter sets.
#[derive(Debug, Clone, Default)]
pub struct AvcParamSets {
    sps: BTreeMap<u8, AvcSliceSps>,
    pps: BTreeMap<u8, AvcSlicePps>,
}

impl AvcParamSets {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an SPS or PPS NAL unit (any other NAL type, or a malformed one,
    /// is ignored; the previous entry for that id is kept on a parse failure).
    pub fn observe_nal(&mut self, nal: &[u8]) {
        match nal.first().map(|b| b & NAL_TYPE_MASK) {
            Some(NAL_SPS) => {
                if let Ok(s) = decode_avc_slice_sps(nal) {
                    self.sps.insert(s.sps_id, s);
                }
            }
            Some(NAL_PPS) => {
                if let Ok(p) = decode_avc_slice_pps(nal) {
                    self.pps.insert(p.pps_id, p);
                }
            }
            _ => {}
        }
    }

    /// Update the cache from the parameter sets of an Annex B access unit and
    /// classify its coded picture ([`AvcPicture`]). Every VCL slice NAL is
    /// parsed; slices that disagree on field/frame, parity or `frame_num`
    /// make the access unit a [`AvcPicture::Frame`] (it already holds more
    /// than one picture, so it is a complete sample on its own).
    pub fn classify_au(&mut self, annexb_au: &[u8]) -> AvcPicture {
        let mut result: Option<AvcPicture> = None;
        for nal in iter_annexb_nals(annexb_au) {
            let t = nal.first().map(|b| b & NAL_TYPE_MASK);
            match t {
                Some(NAL_SPS) | Some(NAL_PPS) => self.observe_nal(nal),
                Some(NAL_SLICE) | Some(NAL_IDR) => {
                    let this = self.classify_slice(nal);
                    result = Some(match (result, this) {
                        (None, p) => p,
                        (Some(a), b) if a == b => a,
                        _ => AvcPicture::Frame,
                    });
                }
                _ => {}
            }
        }
        result.unwrap_or(AvcPicture::Unknown)
    }

    fn classify_slice(&self, nal: &[u8]) -> AvcPicture {
        // pic_parameter_set_id is the third ue(v); peek it to select the PPS.
        let Some(pps_id) = peek_pps_id(nal) else {
            return AvcPicture::Unknown;
        };
        let Some(pps) = self.pps.get(&pps_id) else {
            return AvcPicture::Unknown;
        };
        let Some(sps) = self.sps.get(&pps.sps_id) else {
            return AvcPicture::Unknown;
        };
        match parse_avc_slice_header(nal, sps, pps) {
            Ok(h) if h.field_pic_flag => AvcPicture::Field(AvcFieldInfo {
                bottom: h.bottom_field_flag,
                frame_num: h.frame_num,
                is_reference: h.nal_ref_idc != 0,
                idr: h.idr,
            }),
            Ok(_) => AvcPicture::Frame,
            Err(_) => AvcPicture::Unknown,
        }
    }
}

fn peek_pps_id(nal: &[u8]) -> Option<u8> {
    let end = nal.len().min(1 + SLICE_PREFIX_BYTES);
    let mut r = BitReader::with_unescape(nal.get(1..end)?, "H.264 slice header").ok()?;
    r.read_ue("first_mb_in_slice").ok()?;
    r.read_ue("slice_type").ok()?;
    u8::try_from(r.read_ue("pic_parameter_set_id").ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Minimal MSB-first bit writer for building spec-shaped test NALs.
    #[derive(Default)]
    struct W {
        out: Vec<u8>,
        cur: u8,
        n: u8,
    }
    impl W {
        fn u(&mut self, bits: u32, v: u32) {
            for i in (0..bits).rev() {
                self.cur = (self.cur << 1) | ((v >> i) & 1) as u8;
                self.n += 1;
                if self.n == 8 {
                    self.out.push(self.cur);
                    self.cur = 0;
                    self.n = 0;
                }
            }
        }
        fn ue(&mut self, v: u32) {
            let x = v + 1;
            let len = 32 - x.leading_zeros();
            self.u(len - 1, 0);
            self.u(len, x);
        }
        fn se(&mut self, v: i32) {
            self.ue(if v > 0 {
                (2 * v - 1) as u32
            } else {
                (-2 * v) as u32
            });
        }
        fn finish(mut self) -> Vec<u8> {
            self.u(1, 1);
            while self.n != 0 {
                self.u(1, 0);
            }
            self.out
        }
    }

    fn nal(ref_idc: u8, t: u8, rbsp: Vec<u8>) -> Vec<u8> {
        let mut v = vec![(ref_idc << 5) | t];
        v.extend(rbsp);
        v
    }

    fn sps_nal(poc_type: u32, frame_mbs_only: bool) -> Vec<u8> {
        let mut w = W::default();
        w.u(8, 77);
        w.u(8, 0);
        w.u(8, 30);
        w.ue(0); // sps id
        w.ue(0); // log2_max_frame_num_minus4 -> 4 bits
        w.ue(poc_type);
        if poc_type == 0 {
            w.ue(2); // log2_max_poc_lsb_minus4 -> 6 bits
        } else if poc_type == 1 {
            w.u(1, 0); // delta_pic_order_always_zero_flag
            w.se(0);
            w.se(0);
            w.ue(0);
        }
        w.ue(1);
        w.u(1, 0);
        w.ue(3);
        w.ue(1);
        w.u(1, frame_mbs_only as u32);
        if !frame_mbs_only {
            w.u(1, 0);
        }
        w.u(1, 1);
        w.u(1, 0);
        w.u(1, 0);
        nal(3, 7, w.finish())
    }

    fn pps_nal(id: u32, bottom_poc: bool, redundant: bool, slice_groups: u32) -> Vec<u8> {
        let mut w = W::default();
        w.ue(id);
        w.ue(0);
        w.u(1, 0);
        w.u(1, bottom_poc as u32);
        w.ue(slice_groups);
        if slice_groups > 0 {
            w.ue(6); // map type 6
            w.ue(3); // pic_size_in_map_units_minus1 -> 4 ids
            for _ in 0..4 {
                w.u(2, 1); // ceil(log2(3)) = 2 bits
            }
        }
        w.ue(0);
        w.ue(0);
        w.u(1, 0);
        w.u(2, 0);
        w.se(0);
        w.se(0);
        w.se(0);
        w.u(1, 1);
        w.u(1, 0);
        w.u(1, redundant as u32);
        nal(3, 8, w.finish())
    }

    #[allow(clippy::too_many_arguments)]
    fn slice(
        ref_idc: u8,
        idr: bool,
        frame_num: u32,
        field: Option<bool>,
        poc_lsb: u32,
        extra_after_poc: &dyn Fn(&mut W),
    ) -> Vec<u8> {
        let mut w = W::default();
        w.ue(0); // first_mb
        w.ue(7); // slice_type I
        w.ue(0); // pps id
        w.u(4, frame_num);
        if let Some(bottom) = field {
            w.u(1, 1);
            w.u(1, bottom as u32);
        } else {
            w.u(1, 0);
        }
        if idr {
            w.ue(9);
        }
        w.u(6, poc_lsb);
        extra_after_poc(&mut w);
        w.ue(1); // trailing slice data stand-in
        nal(ref_idc, if idr { 5 } else { 1 }, w.finish())
    }

    fn annexb(nals: &[Vec<u8>]) -> Vec<u8> {
        let mut v = Vec::new();
        for n in nals {
            v.extend([0, 0, 0, 1]);
            v.extend(n);
        }
        v
    }

    #[test]
    fn parses_every_prefix_field_bit_exact() {
        let sps = decode_avc_slice_sps(&sps_nal(0, false)).unwrap();
        assert_eq!(sps.log2_max_frame_num, 4);
        assert_eq!(sps.log2_max_poc_lsb, 6);
        assert!(!sps.frame_mbs_only);
        let pps = decode_avc_slice_pps(&pps_nal(0, true, true, 0)).unwrap();
        assert!(pps.bottom_field_pic_order_in_frame_present && pps.redundant_pic_cnt_present);
        // Frame picture (field_pic_flag = 0): delta_pic_order_cnt_bottom + redundant_pic_cnt present.
        let n = slice(2, true, 9, None, 37, &|w| {
            w.se(-5);
            w.ue(3);
        });
        let h = parse_avc_slice_header(&n, &sps, &pps).unwrap();
        assert_eq!(h.nal_ref_idc, 2);
        assert!(h.idr);
        assert_eq!((h.first_mb_in_slice, h.slice_type, h.frame_num), (0, 7, 9));
        assert!(!h.field_pic_flag && !h.bottom_field_flag);
        assert_eq!(h.idr_pic_id, Some(9));
        assert_eq!(h.pic_order_cnt_lsb, Some(37));
        assert_eq!(h.delta_pic_order_cnt_bottom, -5);
        assert_eq!(h.redundant_pic_cnt, 3);
        // Field picture: no delta_pic_order_cnt_bottom even though the PPS flag is set.
        let n = slice(2, false, 15, Some(true), 63, &|w| w.ue(2));
        let h = parse_avc_slice_header(&n, &sps, &pps).unwrap();
        assert!(h.field_pic_flag && h.bottom_field_flag);
        assert_eq!((h.frame_num, h.pic_order_cnt_lsb), (15, Some(63)));
        assert_eq!((h.delta_pic_order_cnt_bottom, h.redundant_pic_cnt), (0, 2));
        assert_eq!(h.idr_pic_id, None);
    }

    #[test]
    fn poc_type_1_and_progressive_sps() {
        let sps = decode_avc_slice_sps(&sps_nal(1, true)).unwrap();
        assert!(sps.frame_mbs_only);
        assert_eq!(sps.pic_order_cnt_type, 1);
        let pps = decode_avc_slice_pps(&pps_nal(0, false, false, 0)).unwrap();
        // frame_mbs_only: no field_pic_flag bit at all; POC type 1: delta_pic_order_cnt[0].
        let mut w = W::default();
        w.ue(0);
        w.ue(0);
        w.ue(0);
        w.u(4, 3);
        w.se(7);
        let n = nal(1, 1, w.finish());
        let h = parse_avc_slice_header(&n, &sps, &pps).unwrap();
        assert!(!h.field_pic_flag);
        assert_eq!((h.frame_num, h.delta_pic_order_cnt), (3, [7, 0]));
    }

    #[test]
    fn pps_slice_group_type_6_walked() {
        let pps = decode_avc_slice_pps(&pps_nal(0, false, true, 2)).unwrap();
        assert!(pps.redundant_pic_cnt_present);
    }

    #[test]
    fn truncated_or_wrong_pps_is_an_error_not_a_panic() {
        let sps = decode_avc_slice_sps(&sps_nal(0, false)).unwrap();
        let pps = decode_avc_slice_pps(&pps_nal(0, false, false, 0)).unwrap();
        let n = slice(1, false, 1, Some(false), 1, &|_| {});
        for cut in 1..4 {
            assert!(parse_avc_slice_header(&n[..cut], &sps, &pps).is_err());
        }
        let other = AvcSlicePps { pps_id: 4, ..pps };
        assert!(parse_avc_slice_header(&n, &sps, &other).is_err());
        assert!(parse_avc_slice_header(&[0x67, 0x01], &sps, &pps).is_err());
    }

    fn params() -> (Vec<u8>, Vec<u8>) {
        (sps_nal(0, false), pps_nal(0, false, false, 0))
    }

    #[test]
    fn classify_field_frame_and_unknown() {
        let (s, p) = params();
        let mut ps = AvcParamSets::new();
        // No parameter sets seen yet: Unknown.
        let f = slice(1, false, 2, Some(false), 4, &|_| {});
        assert_eq!(
            ps.classify_au(&annexb(std::slice::from_ref(&f))),
            AvcPicture::Unknown
        );
        // With them: a field...
        let au = annexb(&[s, p, f.clone()]);
        assert_eq!(
            ps.classify_au(&au),
            AvcPicture::Field(AvcFieldInfo {
                bottom: false,
                frame_num: 2,
                is_reference: true,
                idr: false
            })
        );
        // ...and a frame picture (field_pic_flag = 0, MBAFF-like) is not a field.
        let fr = slice(1, false, 2, None, 4, &|_| {});
        assert_eq!(ps.classify_au(&annexb(&[fr])), AvcPicture::Frame);
        // An AU carrying both parities (two pictures in one PES) is a Frame.
        let b = slice(1, false, 2, Some(true), 5, &|_| {});
        assert_eq!(ps.classify_au(&annexb(&[f, b])), AvcPicture::Frame);
    }

    #[test]
    fn pairing_rules() {
        let a = AvcFieldInfo {
            bottom: false,
            frame_num: 3,
            is_reference: true,
            idr: true,
        };
        let b = AvcFieldInfo {
            bottom: true,
            frame_num: 3,
            is_reference: true,
            idr: false,
        };
        assert!(a.pairs_with(&b)); // IDR first field + non-IDR second
        assert!(!a.pairs_with(&AvcFieldInfo { bottom: false, ..b })); // same parity
        assert!(!a.pairs_with(&AvcFieldInfo { frame_num: 4, ..b })); // frame_num
        assert!(!a.pairs_with(&AvcFieldInfo {
            is_reference: false,
            ..b
        })); // ref/non-ref mix
        assert!(!a.pairs_with(&AvcFieldInfo { idr: true, ..b })); // second field IDR
    }
}
