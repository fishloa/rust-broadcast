//! VVC video descriptor — Rec. ITU-T H.222.0 (08/2023) | ISO/IEC 13818-1:2023
//! §2.6.129–2.6.130, Tables 2-133…2-138 (tag 0x39); syntax in
//! `docs/descriptors/iso_13818_1/39-3a-vvc-and-evc-video-descriptors.md`.
//!
//! ```text
//! profile_idc(7) tier_flag(1) num_sub_profiles(8) { sub_profile_idc(32) }*
//! progressive_source_flag(1) interlaced_source_flag(1) non_packed_constraint_flag(1)
//! frame_only_constraint_flag(1) reserved_zero_4bits(4) level_idc(8)
//! temporal_layer_subset_flag(1) VVC_still_present_flag(1) VVC_24hr_picture_present_flag(1)
//! reserved(5) HDR_WCG_idc(2) reserved(2) video_properties_tag(4)
//! [ reserved(5) temporal_id_min(3) reserved(5) temporal_id_max(3) ]  -- iff temporal_layer_subset_flag
//! ```

use super::hdr_wcg_idc::HdrWcgIdc;
use crate::error::Result;
use crate::h222_bits::{BitReader, BitWriter, impl_h222_descriptor};
use alloc::vec::Vec;

/// Descriptor tag of `VVC_video_descriptor` (Table 2-45: 57).
pub const TAG: u8 = 0x39;
const RESERVED_5_DEFAULT: u8 = 0x1F;
const RESERVED_2_DEFAULT: u8 = 0x03;

/// The `temporal_id_min` / `temporal_id_max` block (present iff
/// `temporal_layer_subset_flag == '1'`), shared by the VVC and EVC descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TemporalIdRange {
    /// `reserved(5)` before `temporal_id_min`, default `0x1F`.
    pub reserved_min: u8,
    /// `temporal_id_min` (3 bits).
    pub temporal_id_min: u8,
    /// `reserved(5)` before `temporal_id_max`, default `0x1F`.
    pub reserved_max: u8,
    /// `temporal_id_max` (3 bits).
    pub temporal_id_max: u8,
}

impl TemporalIdRange {
    /// Build with the spec `reserved` fill.
    #[must_use]
    pub const fn new(temporal_id_min: u8, temporal_id_max: u8) -> Self {
        Self {
            reserved_min: RESERVED_5_DEFAULT,
            temporal_id_min,
            reserved_max: RESERVED_5_DEFAULT,
            temporal_id_max,
        }
    }

    pub(crate) fn read(r: &mut BitReader<'_>) -> Result<Self> {
        Ok(Self {
            reserved_min: r.u8(5)?,
            temporal_id_min: r.u8(3)?,
            reserved_max: r.u8(5)?,
            temporal_id_max: r.u8(3)?,
        })
    }

    pub(crate) fn write(&self, w: &mut BitWriter) -> Result<()> {
        w.put(u64::from(self.reserved_min), 5, "reserved")?;
        w.put(u64::from(self.temporal_id_min), 3, "temporal_id_min")?;
        w.put(u64::from(self.reserved_max), 5, "reserved")?;
        w.put(u64::from(self.temporal_id_max), 3, "temporal_id_max")
    }
}

/// `VVC_video_descriptor()` — Table 2-133.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct VvcVideoDescriptor {
    /// `profile_idc` (7 bits; `general_profile_idc` of H.266, external code).
    pub profile_idc: u8,
    /// `tier_flag`.
    pub tier_flag: bool,
    /// `sub_profile_idc[i]` (32 bits each; `num_sub_profiles` is 8 bits).
    pub sub_profile_idcs: Vec<u32>,
    /// `progressive_source_flag`.
    pub progressive_source_flag: bool,
    /// `interlaced_source_flag`.
    pub interlaced_source_flag: bool,
    /// `non_packed_constraint_flag`.
    pub non_packed_constraint_flag: bool,
    /// `frame_only_constraint_flag`.
    pub frame_only_constraint_flag: bool,
    /// `reserved_zero_4bits`: "shall be set to '0000'"; re-emitted verbatim, default 0.
    pub reserved_zero_4bits: u8,
    /// `level_idc` (H.266 `general_level_idc`, external code).
    pub level_idc: u8,
    /// `VVC_still_present_flag`.
    pub still_present_flag: bool,
    /// `VVC_24hr_picture_present_flag`.
    pub picture_24hr_present_flag: bool,
    /// `reserved(5)` after the three flags, default `0x1F`.
    pub reserved_5: u8,
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

impl VvcVideoDescriptor {
    /// Build with no sub-profiles, all four source/constraint flags and both
    /// presence flags clear, `reserved_zero_4bits` zero, the spec `reserved`
    /// fills, no temporal subset and no trailing bytes. Set the remaining public
    /// fields directly to override.
    #[must_use]
    pub fn new(
        profile_idc: u8,
        tier_flag: bool,
        level_idc: u8,
        hdr_wcg_idc: HdrWcgIdc,
        video_properties_tag: u8,
    ) -> Self {
        Self {
            profile_idc,
            tier_flag,
            sub_profile_idcs: Vec::new(),
            progressive_source_flag: false,
            interlaced_source_flag: false,
            non_packed_constraint_flag: false,
            frame_only_constraint_flag: false,
            reserved_zero_4bits: 0,
            level_idc,
            still_present_flag: false,
            picture_24hr_present_flag: false,
            reserved_5: RESERVED_5_DEFAULT,
            hdr_wcg_idc,
            reserved_2: RESERVED_2_DEFAULT,
            video_properties_tag,
            temporal_layer_subset: None,
            trailing: Vec::new(),
        }
    }

    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "VvcVideoDescriptor");
        let profile_idc = r.u8(7)?;
        let tier_flag = r.flag()?;
        let n = r.bits(8)?;
        let mut sub_profile_idcs = Vec::new();
        for _ in 0..n {
            sub_profile_idcs.push(r.bits(32)?);
        }
        let progressive_source_flag = r.flag()?;
        let interlaced_source_flag = r.flag()?;
        let non_packed_constraint_flag = r.flag()?;
        let frame_only_constraint_flag = r.flag()?;
        let reserved_zero_4bits = r.u8(4)?;
        let level_idc = r.u8(8)?;
        let subset = r.flag()?;
        let still_present_flag = r.flag()?;
        let picture_24hr_present_flag = r.flag()?;
        let reserved_5 = r.u8(5)?;
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
            tier_flag,
            sub_profile_idcs,
            progressive_source_flag,
            interlaced_source_flag,
            non_packed_constraint_flag,
            frame_only_constraint_flag,
            reserved_zero_4bits,
            level_idc,
            still_present_flag,
            picture_24hr_present_flag,
            reserved_5,
            hdr_wcg_idc,
            reserved_2,
            video_properties_tag,
            temporal_layer_subset,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.put(u64::from(self.profile_idc), 7, "profile_idc")?;
        w.flag(self.tier_flag, "tier_flag")?;
        w.put(self.sub_profile_idcs.len() as u64, 8, "num_sub_profiles")?;
        for s in &self.sub_profile_idcs {
            w.put(u64::from(*s), 32, "sub_profile_idc")?;
        }
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
        w.put(
            u64::from(self.reserved_zero_4bits),
            4,
            "reserved_zero_4bits",
        )?;
        w.put(u64::from(self.level_idc), 8, "level_idc")?;
        w.flag(
            self.temporal_layer_subset.is_some(),
            "temporal_layer_subset_flag",
        )?;
        w.flag(self.still_present_flag, "VVC_still_present_flag")?;
        w.flag(
            self.picture_24hr_present_flag,
            "VVC_24hr_picture_present_flag",
        )?;
        w.put(u64::from(self.reserved_5), 5, "reserved")?;
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
        w.bytes(&self.trailing)?;
        Ok(w.into_vec())
    }

    /// Default of `reserved(5)` (all ones).
    pub const RESERVED_5_DEFAULT: u8 = RESERVED_5_DEFAULT;
    /// Default of `reserved(2)` (all ones).
    pub const RESERVED_2_DEFAULT: u8 = RESERVED_2_DEFAULT;
}

impl_h222_descriptor!(VvcVideoDescriptor, TAG, "VvcVideoDescriptor", "VVC_VIDEO");

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)] // bit-field-wise literals mirror the syntax tables
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    #[test]
    fn spec_vector_with_subprofile_and_temporal_subset() {
        // profile 1, tier 0, 1 sub profile 0x0000_0002, prog=1 inter=0 npc=0 fo=1 zero=0,
        // level 0x33, subset=1 still=0 24hr=0 res=11111, HDR=2 res=11 vpt=2,
        // reserved 11111 min=1, reserved 11111 max=3
        let b = [
            TAG,
            12,
            0b0000001_0,
            1,
            0,
            0,
            0,
            2,
            0b1001_0000,
            0x33,
            0b1_0_0_11111,
            0b10_11_0010,
            0b11111_001,
            0b11111_011,
        ];
        let d = VvcVideoDescriptor::parse(&b).unwrap();
        assert_eq!(d.profile_idc, 1);
        assert_eq!(d.sub_profile_idcs, [2]);
        assert!(d.progressive_source_flag && d.frame_only_constraint_flag);
        assert_eq!(d.hdr_wcg_idc, HdrWcgIdc::HdrAndWcg);
        assert_eq!(d.video_properties_tag, 2);
        assert_eq!(d.temporal_layer_subset, Some(TemporalIdRange::new(1, 3)));
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, &b[..]);
    }

    #[test]
    fn without_subset_keeps_nonzero_reserved_zero_bits() {
        let body = [
            0b0000010_1,
            0,
            0b0000_1010,
            0x40,
            0b0_1_1_00000,
            0b11_00_0000,
        ];
        let mut full = vec![TAG, body.len() as u8];
        full.extend_from_slice(&body);
        let d = VvcVideoDescriptor::parse(&full).unwrap();
        assert!(d.tier_flag);
        assert_eq!(d.reserved_zero_4bits, 0b1010);
        assert!(d.temporal_layer_subset.is_none());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, full);
        let mut bad = d.clone();
        bad.profile_idc = 0x80;
        assert!(bad.serialize_into(&mut [0u8; 32]).is_err());
        assert!(VvcVideoDescriptor::parse(&full[..full.len() - 1]).is_err());
    }
}
