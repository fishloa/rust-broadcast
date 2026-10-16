//! EVC video descriptor — Rec. ITU-T H.222.0 (08/2023) | ISO/IEC 13818-1:2023
//! §2.6.133–2.6.134, Table 2-140 (tag 0x3A); syntax in
//! `docs/descriptors/iso_13818_1/39-3a-vvc-and-evc-video-descriptors.md`.
//!
//! **Spec erratum:** Table 2-140 prints `descriptor_length` as 16 bits (every
//! other descriptor, including the VVC one, uses 8). The standard 8-bit
//! descriptor framing is kept and only the body fields are typed (the md has
//! the verbatim note).
//!
//! ```text
//! profile_idc(8) level_idc(8) toolset_idc_h(32) toolset_idc_l(32)
//! progressive_source_flag(1) interlaced_source_flag(1) non_packed_constraint_flag(1)
//! frame_only_constraint_flag(1) reserved(1) temporal_layer_subset_flag(1)
//! EVC_still_present_flag(1) EVC_24hr_picture_present_flag(1)
//! HDR_WCG_idc(2) reserved(2) video_properties_tag(4)
//! [ reserved(5) temporal_id_min(3) reserved(5) temporal_id_max(3) ]  -- iff temporal_layer_subset_flag
//! ```

use super::hdr_wcg_idc::HdrWcgIdc;
use super::vvc_video::TemporalIdRange;
use crate::error::Result;
use crate::h222_bits::{BitReader, BitWriter, impl_h222_descriptor};
use alloc::vec::Vec;

/// Descriptor tag of `EVC_video_descriptor` (Table 2-45: 58).
pub const TAG: u8 = 0x3A;
const RESERVED_1_DEFAULT: u8 = 0x01;
const RESERVED_2_DEFAULT: u8 = 0x03;

/// `EVC_video_descriptor()` — Table 2-140.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct EvcVideoDescriptor {
    /// `profile_idc` (ISO/IEC 23094-1 SPS code, external).
    pub profile_idc: u8,
    /// `level_idc` (ISO/IEC 23094-1 SPS code, external).
    pub level_idc: u8,
    /// `toolset_idc_h` (ISO/IEC 23094-1 SPS code, external).
    pub toolset_idc_h: u32,
    /// `toolset_idc_l` (ISO/IEC 23094-1 SPS code, external).
    pub toolset_idc_l: u32,
    /// `progressive_source_flag`.
    pub progressive_source_flag: bool,
    /// `interlaced_source_flag`.
    pub interlaced_source_flag: bool,
    /// `non_packed_constraint_flag`.
    pub non_packed_constraint_flag: bool,
    /// `frame_only_constraint_flag`.
    pub frame_only_constraint_flag: bool,
    /// `reserved(1)` before `temporal_layer_subset_flag`, default 1.
    pub reserved_1: u8,
    /// `EVC_still_present_flag`.
    pub still_present_flag: bool,
    /// `EVC_24hr_picture_present_flag`.
    pub picture_24hr_present_flag: bool,
    /// `HDR_WCG_idc` (Table 2-134).
    pub hdr_wcg_idc: HdrWcgIdc,
    /// `reserved(2)` before `video_properties_tag`, default 3.
    pub reserved_2: u8,
    /// `video_properties_tag` (4 bits; Tables 2-135…2-138 select its meaning by
    /// `HDR_WCG_idc`).
    pub video_properties_tag: u8,
    /// `temporal_id_min` / `temporal_id_max`, present iff
    /// `temporal_layer_subset_flag == '1'`.
    pub temporal_layer_subset: Option<TemporalIdRange>,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl EvcVideoDescriptor {
    /// Default of `reserved(1)` (all ones).
    pub const RESERVED_1_DEFAULT: u8 = RESERVED_1_DEFAULT;
    /// Default of `reserved(2)` (all ones).
    pub const RESERVED_2_DEFAULT: u8 = RESERVED_2_DEFAULT;

    fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "EvcVideoDescriptor");
        let profile_idc = r.u8(8)?;
        let level_idc = r.u8(8)?;
        let toolset_idc_h = r.bits(32)?;
        let toolset_idc_l = r.bits(32)?;
        let progressive_source_flag = r.flag()?;
        let interlaced_source_flag = r.flag()?;
        let non_packed_constraint_flag = r.flag()?;
        let frame_only_constraint_flag = r.flag()?;
        let reserved_1 = r.u8(1)?;
        let subset = r.flag()?;
        let still_present_flag = r.flag()?;
        let picture_24hr_present_flag = r.flag()?;
        let hdr_wcg_idc = HdrWcgIdc::from_u8(r.u8(2)?);
        let reserved_2 = r.u8(2)?;
        let video_properties_tag = r.u8(4)?;
        let temporal_layer_subset = if subset {
            Some(TemporalIdRange::read(&mut r)?)
        } else {
            None
        };
        Ok(Self {
            profile_idc,
            level_idc,
            toolset_idc_h,
            toolset_idc_l,
            progressive_source_flag,
            interlaced_source_flag,
            non_packed_constraint_flag,
            frame_only_constraint_flag,
            reserved_1,
            still_present_flag,
            picture_24hr_present_flag,
            hdr_wcg_idc,
            reserved_2,
            video_properties_tag,
            temporal_layer_subset,
            trailing: r.rest().to_vec(),
        })
    }

    fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.put(u64::from(self.profile_idc), 8, "profile_idc")?;
        w.put(u64::from(self.level_idc), 8, "level_idc")?;
        w.put(u64::from(self.toolset_idc_h), 32, "toolset_idc_h")?;
        w.put(u64::from(self.toolset_idc_l), 32, "toolset_idc_l")?;
        w.flag(self.progressive_source_flag, "progressive_source_flag")?;
        w.flag(self.interlaced_source_flag, "interlaced_source_flag")?;
        w.flag(
            self.non_packed_constraint_flag,
            "non_packed_constraint_flag",
        )?;
        w.flag(
            self.frame_only_constraint_flag,
            "frame_only_constraint_flag",
        )?;
        w.put(u64::from(self.reserved_1), 1, "reserved")?;
        w.flag(
            self.temporal_layer_subset.is_some(),
            "temporal_layer_subset_flag",
        )?;
        w.flag(self.still_present_flag, "EVC_still_present_flag")?;
        w.flag(
            self.picture_24hr_present_flag,
            "EVC_24hr_picture_present_flag",
        )?;
        w.put(u64::from(self.hdr_wcg_idc.to_u8()), 2, "HDR_WCG_idc")?;
        w.put(u64::from(self.reserved_2), 2, "reserved")?;
        w.put(
            u64::from(self.video_properties_tag),
            4,
            "video_properties_tag",
        )?;
        if let Some(t) = &self.temporal_layer_subset {
            t.write(&mut w)?;
        }
        w.bytes(&self.trailing);
        Ok(w.into_vec())
    }
}

impl_h222_descriptor!(EvcVideoDescriptor, TAG, "EvcVideoDescriptor", "EVC_VIDEO");

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    #[test]
    fn spec_vector_round_trip() {
        // profile 0, level 0x33, toolset h=1 l=2, prog=1 inter=0 npc=0 fo=1 res=1
        // subset=1 still=0 24hr=1, HDR=1 res=11 vpt=1, [res=11111 min=0 res=11111 max=2]
        let body = [
            0,
            0x33,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            2,
            0b1001_1_1_0_1,
            0b01_11_0001,
            0b11111_000,
            0b11111_010,
        ];
        let mut b = vec![TAG, body.len() as u8];
        b.extend_from_slice(&body);
        let d = EvcVideoDescriptor::parse(&b).unwrap();
        assert_eq!((d.toolset_idc_h, d.toolset_idc_l), (1, 2));
        assert!(d.progressive_source_flag && d.frame_only_constraint_flag);
        assert_eq!(d.hdr_wcg_idc, HdrWcgIdc::WcgOnly);
        assert_eq!(d.temporal_layer_subset, Some(TemporalIdRange::new(0, 2)));
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, b);
        // body that stops inside the temporal-id block is rejected
        assert!(EvcVideoDescriptor::parse(&b[..b.len() - 1]).is_err());
        let mut bad = d.clone();
        bad.video_properties_tag = 16;
        assert!(bad.serialize_into(&mut [0u8; 32]).is_err());
    }
}
