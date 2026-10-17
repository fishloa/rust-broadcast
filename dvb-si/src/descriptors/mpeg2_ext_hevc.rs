//! MPEG-2 `Extension_descriptor` bodies for layered / tiled HEVC — Rec. ITU-T
//! H.222.0 (08/2023) | ISO/IEC 13818-1:2023:
//!
//! * `extension_descriptor_tag` 0x05 `HEVC_operation_point_descriptor()` —
//!   §2.6.100, Table 2-117 (page p.130). The page declares the per-operation-point
//!   count as `numEsInOp[i]` but bounds the loop with `NumESinOP[i]`; they are
//!   the same quantity.
//! * 0x06 `HEVC_hierarchy_extension_descriptor()` — §2.6.102, Table 2-118.
//! * 0x12 `HEVC_tile_substream_descriptor()` — §2.6.122, Table 2-130.
//! * 0x13 `HEVC_subregion_descriptor()` — §2.6.125, Table 2-131 (semantics §2.6.126).
//!
//! `HEVC_subregion_descriptor` follows the printed syntax and §2.6.126: the
//! inner `SubstreamOffset` loop runs `k < SubstreamCountMinus1[i]` times (the
//! field is "the number of HEVC tile substreams to be appended", so the loop
//! count is the field value), and the outer `N1` loop is implied by the
//! descriptor size (it runs to the end of the body). TSDuck 3.44 writes one
//! more offset per pattern than that and so disagrees; the divergence is
//! pinned in `tests/h222_descriptors.rs` and noted in the docs.
//!
//! Syntax transcriptions: `docs/descriptors/iso_13818_1/3f-extension-descriptor-bodies-0x02-0x07.md`
//! and `…-0x0f-0x19.md`. `profile_tier_level_info[i]` is typed as
//! [`ProfileTierLevelInfo`]: H.222.0 §2.6.101 fixes the field at **96 bits**
//! and says it is `profile_tier_level()` (Rec. ITU-T H.265 §7.3.3) with
//! `profilePresentFlag = 1`, i.e. exactly the *general* part of that syntax
//! (2+1+5+32+4+43+1+8 = 96 bits). The 44 bits between
//! `general_frame_only_constraint_flag` and `general_level_idc` are
//! profile-dependent in H.265 (`general_max_*` flags, reserved zeros,
//! `general_inbld_flag`); like `HEVC_video_descriptor` (H.222.0 Table 2-112)
//! they are carried as one `copied_44bits` value. The H.265 `sub_layer_*`
//! part that `maxNumSubLayersMinus1 = 6` would add (16 flag bits, 112 in all) lies beyond the 96 bits
//! H.222.0 allots and is **not** typed or invented here (see the docs
//! transcription's spec note).
//! Every `reserved` field is a typed field that is re-emitted verbatim
//! (default: all ones).

use crate::error::Result;
use crate::h222_bits::{BitReader, BitWriter, invalid};
use alloc::vec::Vec;

const TAG: u8 = super::mpeg2_extension::TAG;
/// `copied_44bits` width (the 43 profile-dependent bits + the 1-bit
/// `general_inbld_flag` / `general_reserved_zero_bit`).
const COPIED_BITS: usize = 44;
const RES1: u8 = 0x01;
const RES2: u8 = 0x03;
const RES4: u8 = 0x0F;

/// `profile_tier_level_info[i]` — the 96-bit `profile_tier_level()` of H.265
/// §7.3.3 with `profilePresentFlag = 1` (H.222.0 §2.6.101).
///
/// Bit layout (MSB first, 12 bytes):
/// `general_profile_space(2)` `general_tier_flag(1)` `general_profile_idc(5)`
/// `general_profile_compatibility_flag[0..32](32)`
/// `general_progressive_source_flag(1)` `general_interlaced_source_flag(1)`
/// `general_non_packed_constraint_flag(1)` `general_frame_only_constraint_flag(1)`
/// `copied_44bits(44)` `general_level_idc(8)`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ProfileTierLevelInfo {
    /// `general_profile_space` (2 bits).
    pub general_profile_space: u8,
    /// `general_tier_flag`.
    pub general_tier_flag: bool,
    /// `general_profile_idc` (5 bits).
    pub general_profile_idc: u8,
    /// `general_profile_compatibility_flag[j]`, `j = 0` the most significant bit.
    pub general_profile_compatibility_flags: u32,
    /// `general_progressive_source_flag`.
    pub general_progressive_source_flag: bool,
    /// `general_interlaced_source_flag`.
    pub general_interlaced_source_flag: bool,
    /// `general_non_packed_constraint_flag`.
    pub general_non_packed_constraint_flag: bool,
    /// `general_frame_only_constraint_flag`.
    pub general_frame_only_constraint_flag: bool,
    /// The 44 profile-dependent bits between `general_frame_only_constraint_flag`
    /// and `general_level_idc`, preserved verbatim (H.222.0 `copied_44bits`).
    pub copied_44bits: u64,
    /// `general_level_idc` (8 bits).
    pub general_level_idc: u8,
}

impl ProfileTierLevelInfo {
    /// Build with every flag clear and `copied_44bits` zero.
    #[must_use]
    pub const fn new(
        general_profile_space: u8,
        general_tier_flag: bool,
        general_profile_idc: u8,
        general_profile_compatibility_flags: u32,
        general_level_idc: u8,
    ) -> Self {
        Self {
            general_profile_space,
            general_tier_flag,
            general_profile_idc,
            general_profile_compatibility_flags,
            general_progressive_source_flag: false,
            general_interlaced_source_flag: false,
            general_non_packed_constraint_flag: false,
            general_frame_only_constraint_flag: false,
            copied_44bits: 0,
            general_level_idc,
        }
    }

    fn parse(r: &mut BitReader<'_>) -> Result<Self> {
        let general_profile_space = r.u8(2)?;
        let general_tier_flag = r.flag()?;
        let general_profile_idc = r.u8(5)?;
        let general_profile_compatibility_flags = r.bits(32)?;
        let general_progressive_source_flag = r.flag()?;
        let general_interlaced_source_flag = r.flag()?;
        let general_non_packed_constraint_flag = r.flag()?;
        let general_frame_only_constraint_flag = r.flag()?;
        let hi = u64::from(r.bits(COPIED_BITS - 32)?);
        let lo = u64::from(r.bits(32)?);
        let copied_44bits = (hi << 32) | lo;
        let general_level_idc = r.u8(8)?;
        Ok(Self {
            general_profile_space,
            general_tier_flag,
            general_profile_idc,
            general_profile_compatibility_flags,
            general_progressive_source_flag,
            general_interlaced_source_flag,
            general_non_packed_constraint_flag,
            general_frame_only_constraint_flag,
            copied_44bits,
            general_level_idc,
        })
    }

    fn write(&self, w: &mut BitWriter) -> Result<()> {
        w.put(
            u64::from(self.general_profile_space),
            2,
            "general_profile_space",
        )?;
        w.flag(self.general_tier_flag, "general_tier_flag")?;
        w.put(
            u64::from(self.general_profile_idc),
            5,
            "general_profile_idc",
        )?;
        w.put(
            u64::from(self.general_profile_compatibility_flags),
            32,
            "general_profile_compatibility_flag",
        )?;
        w.flag(
            self.general_progressive_source_flag,
            "general_progressive_source_flag",
        )?;
        w.flag(
            self.general_interlaced_source_flag,
            "general_interlaced_source_flag",
        )?;
        w.flag(
            self.general_non_packed_constraint_flag,
            "general_non_packed_constraint_flag",
        )?;
        w.flag(
            self.general_frame_only_constraint_flag,
            "general_frame_only_constraint_flag",
        )?;
        w.put(self.copied_44bits, COPIED_BITS, "copied_44bits")?;
        w.put(u64::from(self.general_level_idc), 8, "general_level_idc")
    }
}

// ───────────────────────── 0x05 HEVC_operation_point_descriptor ─────────────

/// One `ES_reference` of an operation point.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcOpEs {
    /// `reserved(1)`, default 1.
    pub reserved: u8,
    /// `prepend_dependencies[i][j]`.
    pub prepend_dependencies: bool,
    /// `ES_reference[i][j]` (6 bits).
    pub es_reference: u8,
}

impl HevcOpEs {
    /// Build with the spec `reserved` fill.
    #[must_use]
    pub const fn new(prepend_dependencies: bool, es_reference: u8) -> Self {
        Self {
            reserved: RES1,
            prepend_dependencies,
            es_reference,
        }
    }
}

/// One layer of an operation point (`numEsInOp` loop).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcOpLayer {
    /// `necessary_layer_flag[i][k]`.
    pub necessary_layer_flag: bool,
    /// `output_layer_flag[i][k]`.
    pub output_layer_flag: bool,
    /// `ptl_ref_idx[i][k]` (6 bits).
    pub ptl_ref_idx: u8,
}

impl HevcOpLayer {
    /// Build a layer entry.
    #[must_use]
    pub const fn new(necessary_layer_flag: bool, output_layer_flag: bool, ptl_ref_idx: u8) -> Self {
        Self {
            necessary_layer_flag,
            output_layer_flag,
            ptl_ref_idx,
        }
    }
}

/// `constant_frame_rate_info_idc > 0` block.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcOpFrameRate {
    /// `constant_frame_rate_info_idc[i]` (1..=3 here; 0 means this block is absent).
    pub constant_frame_rate_info_idc: u8,
    /// `reserved(4)`, default `0xF`.
    pub reserved: u8,
    /// `frame_rate_indicator[i]` (12 bits).
    pub frame_rate_indicator: u16,
}

impl HevcOpFrameRate {
    /// Build with the spec `reserved` fill.
    #[must_use]
    pub const fn new(constant_frame_rate_info_idc: u8, frame_rate_indicator: u16) -> Self {
        Self {
            constant_frame_rate_info_idc,
            reserved: RES4,
            frame_rate_indicator,
        }
    }
}

/// One HEVC operation point.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcOperationPointEntry {
    /// `target_ols[i]`.
    pub target_ols: u8,
    /// The `ES_count[i]` entries.
    pub es: Vec<HevcOpEs>,
    /// `reserved(2)` before `numEsInOp`, default 3.
    pub reserved_before_layers: u8,
    /// The `numEsInOp[i]` layers.
    pub layers: Vec<HevcOpLayer>,
    /// `reserved(1)` before `avg_bit_rate_info_flag`, default 1.
    pub reserved_before_flags: u8,
    /// `applicable_temporal_id[i]` (3 bits).
    pub applicable_temporal_id: u8,
    /// Frame-rate block, present iff `constant_frame_rate_info_idc > 0`.
    pub frame_rate: Option<HevcOpFrameRate>,
    /// `avg_bit_rate[i]` (24 bits, units of 1000 bit/s), present iff the info flag is set.
    pub avg_bit_rate: Option<u32>,
    /// `max_bit_rate[i]` (24 bits, units of 1000 bit/s), present iff the info flag is set.
    pub max_bit_rate: Option<u32>,
}

impl HevcOperationPointEntry {
    /// Build with the spec `reserved` fill and no optional blocks.
    #[must_use]
    pub fn new(
        target_ols: u8,
        es: Vec<HevcOpEs>,
        layers: Vec<HevcOpLayer>,
        applicable_temporal_id: u8,
    ) -> Self {
        Self {
            target_ols,
            es,
            reserved_before_layers: RES2,
            layers,
            reserved_before_flags: RES1,
            applicable_temporal_id,
            frame_rate: None,
            avg_bit_rate: None,
            max_bit_rate: None,
        }
    }
}

/// `HEVC_operation_point_descriptor()` — Table 2-117 (`extension_descriptor_tag` 0x05).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcOperationPoint {
    /// `reserved(2)` before `num_ptl`, default 3.
    pub reserved: u8,
    /// `profile_tier_level_info[i]` — 96 bits each, typed per H.265 §7.3.3
    /// (see [`ProfileTierLevelInfo`]).
    pub profile_tier_levels: Vec<ProfileTierLevelInfo>,
    /// The operation points.
    pub operation_points: Vec<HevcOperationPointEntry>,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl HevcOperationPoint {
    /// Build with the spec `reserved` fill and no trailing bytes.
    #[must_use]
    pub fn new(
        profile_tier_levels: Vec<ProfileTierLevelInfo>,
        operation_points: Vec<HevcOperationPointEntry>,
    ) -> Self {
        Self {
            reserved: RES2,
            profile_tier_levels,
            operation_points,
            trailing: Vec::new(),
        }
    }

    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "HevcOperationPoint");
        let reserved = r.u8(2)?;
        let num_ptl = r.bits(6)?;
        let mut profile_tier_levels = Vec::new();
        for _ in 0..num_ptl {
            profile_tier_levels.push(ProfileTierLevelInfo::parse(&mut r)?);
        }
        let count = r.bits(8)?;
        let mut operation_points = Vec::new();
        for _ in 0..count {
            let target_ols = r.u8(8)?;
            let es_count = r.bits(8)?;
            let mut es = Vec::new();
            for _ in 0..es_count {
                es.push(HevcOpEs {
                    reserved: r.u8(1)?,
                    prepend_dependencies: r.flag()?,
                    es_reference: r.u8(6)?,
                });
            }
            let reserved_before_layers = r.u8(2)?;
            let num_es_in_op = r.bits(6)?;
            let mut layers = Vec::new();
            for _ in 0..num_es_in_op {
                layers.push(HevcOpLayer {
                    necessary_layer_flag: r.flag()?,
                    output_layer_flag: r.flag()?,
                    ptl_ref_idx: r.u8(6)?,
                });
            }
            let reserved_before_flags = r.u8(1)?;
            let avg_flag = r.flag()?;
            let max_flag = r.flag()?;
            let idc = r.u8(2)?;
            let applicable_temporal_id = r.u8(3)?;
            let frame_rate = if idc > 0 {
                Some(HevcOpFrameRate {
                    constant_frame_rate_info_idc: idc,
                    reserved: r.u8(4)?,
                    frame_rate_indicator: r.u16(12)?,
                })
            } else {
                None
            };
            let avg_bit_rate = if avg_flag { Some(r.bits(24)?) } else { None };
            let max_bit_rate = if max_flag { Some(r.bits(24)?) } else { None };
            operation_points.push(HevcOperationPointEntry {
                target_ols,
                es,
                reserved_before_layers,
                layers,
                reserved_before_flags,
                applicable_temporal_id,
                frame_rate,
                avg_bit_rate,
                max_bit_rate,
            });
        }
        Ok(Self {
            reserved,
            profile_tier_levels,
            operation_points,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.put(u64::from(self.reserved), 2, "reserved")?;
        w.put(self.profile_tier_levels.len() as u64, 6, "num_ptl")?;
        for ptl in &self.profile_tier_levels {
            ptl.write(&mut w)?;
        }
        w.put(
            self.operation_points.len() as u64,
            8,
            "operation_points_count",
        )?;
        for op in &self.operation_points {
            w.put(u64::from(op.target_ols), 8, "target_ols")?;
            w.put(op.es.len() as u64, 8, "ES_count")?;
            for e in &op.es {
                w.put(u64::from(e.reserved), 1, "reserved")?;
                w.flag(e.prepend_dependencies, "prepend_dependencies")?;
                w.put(u64::from(e.es_reference), 6, "ES_reference")?;
            }
            w.put(u64::from(op.reserved_before_layers), 2, "reserved")?;
            w.put(op.layers.len() as u64, 6, "numEsInOp")?;
            for l in &op.layers {
                w.flag(l.necessary_layer_flag, "necessary_layer_flag")?;
                w.flag(l.output_layer_flag, "output_layer_flag")?;
                w.put(u64::from(l.ptl_ref_idx), 6, "ptl_ref_idx")?;
            }
            w.put(u64::from(op.reserved_before_flags), 1, "reserved")?;
            w.flag(op.avg_bit_rate.is_some(), "avg_bit_rate_info_flag")?;
            w.flag(op.max_bit_rate.is_some(), "max_bit_rate_info_flag")?;
            let idc = match &op.frame_rate {
                None => 0,
                Some(f) if (1..=3).contains(&f.constant_frame_rate_info_idc) => {
                    f.constant_frame_rate_info_idc
                }
                Some(_) => {
                    return Err(invalid(
                        TAG,
                        "HEVC_operation_point_descriptor constant_frame_rate_info_idc must be 1..=3 when a frame rate is present",
                    ));
                }
            };
            w.put(u64::from(idc), 2, "constant_frame_rate_info_idc")?;
            w.put(
                u64::from(op.applicable_temporal_id),
                3,
                "applicable_temporal_id",
            )?;
            if let Some(f) = &op.frame_rate {
                w.put(u64::from(f.reserved), 4, "reserved")?;
                w.put(
                    u64::from(f.frame_rate_indicator),
                    12,
                    "frame_rate_indicator",
                )?;
            }
            if let Some(v) = op.avg_bit_rate {
                w.put(u64::from(v), 24, "avg_bit_rate")?;
            }
            if let Some(v) = op.max_bit_rate {
                w.put(u64::from(v), 24, "max_bit_rate")?;
            }
        }
        w.bytes(&self.trailing)?;
        Ok(w.into_vec())
    }
}

// ───────────────────────── 0x06 HEVC_hierarchy_extension_descriptor ─────────

/// One `hierarchy_ext_embedded_layer_index[i]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcEmbeddedLayer {
    /// `reserved(2)`, default 3.
    pub reserved: u8,
    /// `hierarchy_ext_embedded_layer_index[i]` (6 bits).
    pub index: u8,
}

impl HevcEmbeddedLayer {
    /// Build with the spec `reserved` fill.
    #[must_use]
    pub const fn new(index: u8) -> Self {
        Self {
            reserved: RES2,
            index,
        }
    }
}

/// `HEVC_hierarchy_extension_descriptor()` — Table 2-118 (`extension_descriptor_tag` 0x06).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcHierarchyExtension {
    /// `extension_dimension_bits` (16 bits; bit index 0 multi-view, 1 spatial/SNR,
    /// 2 depth, 3 temporal, 4 auxiliary, 5..15 reserved — Table 2-119).
    pub extension_dimension_bits: u16,
    /// `hierarchy_layer_index` (6 bits).
    pub hierarchy_layer_index: u8,
    /// `temporal_id` (3 bits).
    pub temporal_id: u8,
    /// `nuh_layer_id` (6 bits).
    pub nuh_layer_id: u8,
    /// `tref_present_flag` (value `1` is reserved).
    pub tref_present_flag: bool,
    /// `reserved(2)` before `num_embedded_layers`, default 3.
    pub reserved_a: u8,
    /// `reserved(2)` before `hierarchy_channel`, default 3.
    pub reserved_b: u8,
    /// `hierarchy_channel` (6 bits).
    pub hierarchy_channel: u8,
    /// The `num_embedded_layers` entries.
    pub embedded_layers: Vec<HevcEmbeddedLayer>,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl HevcHierarchyExtension {
    /// Build with `tref_present_flag` clear, both `reserved(2)` fields all ones
    /// and no trailing bytes. Set the remaining public fields directly to override.
    #[must_use]
    pub fn new(
        extension_dimension_bits: u16,
        hierarchy_layer_index: u8,
        temporal_id: u8,
        nuh_layer_id: u8,
        hierarchy_channel: u8,
        embedded_layers: Vec<HevcEmbeddedLayer>,
    ) -> Self {
        Self {
            extension_dimension_bits,
            hierarchy_layer_index,
            temporal_id,
            nuh_layer_id,
            tref_present_flag: false,
            reserved_a: RES2,
            reserved_b: RES2,
            hierarchy_channel,
            embedded_layers,
            trailing: Vec::new(),
        }
    }

    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "HevcHierarchyExtension");
        let extension_dimension_bits = r.u16(16)?;
        let hierarchy_layer_index = r.u8(6)?;
        let temporal_id = r.u8(3)?;
        let nuh_layer_id = r.u8(6)?;
        let tref_present_flag = r.flag()?;
        let reserved_a = r.u8(2)?;
        let num = r.bits(6)?;
        let reserved_b = r.u8(2)?;
        let hierarchy_channel = r.u8(6)?;
        let mut embedded_layers = Vec::new();
        for _ in 0..num {
            embedded_layers.push(HevcEmbeddedLayer {
                reserved: r.u8(2)?,
                index: r.u8(6)?,
            });
        }
        Ok(Self {
            extension_dimension_bits,
            hierarchy_layer_index,
            temporal_id,
            nuh_layer_id,
            tref_present_flag,
            reserved_a,
            reserved_b,
            hierarchy_channel,
            embedded_layers,
            trailing: r.rest().to_vec(),
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.put(
            u64::from(self.extension_dimension_bits),
            16,
            "extension_dimension_bits",
        )?;
        w.put(
            u64::from(self.hierarchy_layer_index),
            6,
            "hierarchy_layer_index",
        )?;
        w.put(u64::from(self.temporal_id), 3, "temporal_id")?;
        w.put(u64::from(self.nuh_layer_id), 6, "nuh_layer_id")?;
        w.flag(self.tref_present_flag, "tref_present_flag")?;
        w.put(u64::from(self.reserved_a), 2, "reserved")?;
        w.put(self.embedded_layers.len() as u64, 6, "num_embedded_layers")?;
        w.put(u64::from(self.reserved_b), 2, "reserved")?;
        w.put(u64::from(self.hierarchy_channel), 6, "hierarchy_channel")?;
        for l in &self.embedded_layers {
            w.put(u64::from(l.reserved), 2, "reserved")?;
            w.put(u64::from(l.index), 6, "hierarchy_ext_embedded_layer_index")?;
        }
        w.bytes(&self.trailing)?;
        Ok(w.into_vec())
    }
}

// ───────────────────────── 0x12 HEVC_tile_substream_descriptor ──────────────

/// One `Flag[i]` / `AdditionalSubstreamID[i]` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AdditionalSubstream {
    /// `Flag[i]` (only `Flag[0]` has a meaning; the rest are reserved).
    pub flag: bool,
    /// `AdditionalSubstreamID[i]` (7 bits).
    pub substream_id: u8,
}

impl AdditionalSubstream {
    /// Build an entry.
    #[must_use]
    pub const fn new(flag: bool, substream_id: u8) -> Self {
        Self { flag, substream_id }
    }
}

/// What follows `SubstreamID` in the HEVC tile substream descriptor (Table 2-130).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum HevcTileSubstreamDetail {
    /// `descriptor_length == 1`: only `SubstreamID`; `ReferenceFlag` is reserved
    /// (carried here, re-emitted verbatim).
    Single {
        /// The reserved `ReferenceFlag` bit.
        reserved_reference_flag: bool,
    },
    /// `ReferenceFlag == '1'`: `PreambleFlag` + `PatternReference`.
    Pattern {
        /// `PreambleFlag`.
        preamble_flag: bool,
        /// `PatternReference` (7 bits).
        pattern_reference: u8,
        /// Bytes after the two defined ones, preserved verbatim.
        trailing: Vec<u8>,
    },
    /// `ReferenceFlag == '0'` and `N1 >= 1`: the additional SubstreamIDs
    /// (`N1` = `descriptor_length - 1` entries, never empty).
    Additional(Vec<AdditionalSubstream>),
}

/// `HEVC_tile_substream_descriptor()` — Table 2-130 (`extension_descriptor_tag` 0x12).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcTileSubstream {
    /// `SubstreamID` (7 bits).
    pub substream_id: u8,
    /// The remainder, selected by `ReferenceFlag` and the descriptor size.
    pub detail: HevcTileSubstreamDetail,
}

impl HevcTileSubstream {
    /// Build from the `SubstreamID` and the detail block.
    #[must_use]
    pub fn new(substream_id: u8, detail: HevcTileSubstreamDetail) -> Self {
        Self {
            substream_id,
            detail,
        }
    }
    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "HevcTileSubstream");
        let reference_flag = r.flag()?;
        let substream_id = r.u8(7)?;
        let detail = if r.remaining_bytes() == 0 {
            HevcTileSubstreamDetail::Single {
                reserved_reference_flag: reference_flag,
            }
        } else if reference_flag {
            HevcTileSubstreamDetail::Pattern {
                preamble_flag: r.flag()?,
                pattern_reference: r.u8(7)?,
                trailing: r.rest().to_vec(),
            }
        } else {
            let mut v = Vec::new();
            while r.remaining_bytes() > 0 {
                v.push(AdditionalSubstream {
                    flag: r.flag()?,
                    substream_id: r.u8(7)?,
                });
            }
            HevcTileSubstreamDetail::Additional(v)
        };
        Ok(Self {
            substream_id,
            detail,
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        let reference_flag = match &self.detail {
            HevcTileSubstreamDetail::Single {
                reserved_reference_flag,
            } => *reserved_reference_flag,
            HevcTileSubstreamDetail::Pattern { .. } => true,
            HevcTileSubstreamDetail::Additional(v) => {
                if v.is_empty() {
                    return Err(invalid(
                        TAG,
                        "HEVC_tile_substream_descriptor Additional list must not be empty (a one-byte body is the Single form)",
                    ));
                }
                false
            }
        };
        w.flag(reference_flag, "ReferenceFlag")?;
        w.put(u64::from(self.substream_id), 7, "SubstreamID")?;
        match &self.detail {
            HevcTileSubstreamDetail::Single { .. } => {}
            HevcTileSubstreamDetail::Pattern {
                preamble_flag,
                pattern_reference,
                trailing,
            } => {
                w.flag(*preamble_flag, "PreambleFlag")?;
                w.put(u64::from(*pattern_reference), 7, "PatternReference")?;
                w.bytes(trailing)?;
            }
            HevcTileSubstreamDetail::Additional(v) => {
                for a in v {
                    w.flag(a.flag, "Flag")?;
                    w.put(u64::from(a.substream_id), 7, "AdditionalSubstreamID")?;
                }
            }
        }
        Ok(w.into_vec())
    }
}

// ───────────────────────── 0x13 HEVC_subregion_descriptor ───────────────────

/// `reserved(1) + PreambleSubstreamID[i](7)`, present when
/// `SubstreamMarkingFlag == 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcSubregionPreamble {
    /// `reserved(1)`, default 1.
    pub reserved: bool,
    /// `PreambleSubstreamID[i]` (7 bits): 0 or `TotalSubstreamIDs + 1..=127`.
    pub preamble_substream_id: u8,
}

impl HevcSubregionPreamble {
    /// Build with the spec `reserved` fill.
    #[must_use]
    pub const fn new(preamble_substream_id: u8) -> Self {
        Self {
            reserved: true,
            preamble_substream_id,
        }
    }
}

/// One subregion layout (`i` loop entry) of the HEVC subregion descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcSubregionLayout {
    /// `PreambleSubstreamID[i]`, present iff `SubstreamMarkingFlag == 1`.
    pub preamble: Option<HevcSubregionPreamble>,
    /// `SubstreamCountMinus1[i]` (8 bits): the number of tile substreams to be
    /// appended (also the number of `SubstreamOffset`s in each pattern).
    pub substream_count_minus_1: u8,
    /// `Level[i]`.
    pub level: u8,
    /// `PictureSizeHor[i]`, pixels.
    pub picture_size_hor: u16,
    /// `PictureSizeVert[i]`, pixels.
    pub picture_size_vert: u16,
    /// `reserved(1)` before `PatternCount`, default 1.
    pub reserved: bool,
    /// `SubstreamOffset[k][j][i]` (8-bit `tcimsbf`): one inner vector per
    /// pattern `j` (`PatternCount[i]` of them, at most 127), each holding
    /// `substream_count_minus_1` offsets.
    pub patterns: Vec<Vec<i8>>,
}

impl HevcSubregionLayout {
    /// Build with the spec `reserved` fill.
    #[must_use]
    pub fn new(
        preamble: Option<HevcSubregionPreamble>,
        substream_count_minus_1: u8,
        level: u8,
        picture_size_hor: u16,
        picture_size_vert: u16,
        patterns: Vec<Vec<i8>>,
    ) -> Self {
        Self {
            preamble,
            substream_count_minus_1,
            level,
            picture_size_hor,
            picture_size_vert,
            reserved: true,
            patterns,
        }
    }
}

/// `HEVC_subregion_descriptor()` — Table 2-131 (`extension_descriptor_tag` 0x13).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HevcSubregion {
    /// `SubstreamMarkingFlag`: each tile substream is in its own ES.
    pub substream_marking_flag: bool,
    /// `SubstreamIDsPerLine` (7 bits).
    pub substream_ids_per_line: u8,
    /// `TotalSubstreamIDs` (8 bits).
    pub total_substream_ids: u8,
    /// `LevelFullPanorama` (8 bits).
    pub level_full_panorama: u8,
    /// The `N1` subregion layouts (`N1` is implied by the descriptor size).
    pub layouts: Vec<HevcSubregionLayout>,
}

impl HevcSubregion {
    /// Build a descriptor from its fields.
    #[must_use]
    pub fn new(
        substream_marking_flag: bool,
        substream_ids_per_line: u8,
        total_substream_ids: u8,
        level_full_panorama: u8,
        layouts: Vec<HevcSubregionLayout>,
    ) -> Self {
        Self {
            substream_marking_flag,
            substream_ids_per_line,
            total_substream_ids,
            level_full_panorama,
            layouts,
        }
    }

    pub(crate) fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "HevcSubregion");
        let substream_marking_flag = r.flag()?;
        let substream_ids_per_line = r.u8(7)?;
        let total_substream_ids = r.u8(8)?;
        let level_full_panorama = r.u8(8)?;
        let mut layouts = Vec::new();
        while r.remaining_bytes() > 0 {
            let preamble = if substream_marking_flag {
                Some(HevcSubregionPreamble {
                    reserved: r.flag()?,
                    preamble_substream_id: r.u8(7)?,
                })
            } else {
                None
            };
            let substream_count_minus_1 = r.u8(8)?;
            let level = r.u8(8)?;
            let picture_size_hor = r.u16(16)?;
            let picture_size_vert = r.u16(16)?;
            let reserved = r.flag()?;
            let pattern_count = r.u8(7)?;
            let mut patterns = Vec::new();
            for _ in 0..pattern_count {
                let mut offsets = Vec::new();
                for _ in 0..substream_count_minus_1 {
                    offsets.push(i8::from_be_bytes([r.u8(8)?]));
                }
                patterns.push(offsets);
            }
            layouts.push(HevcSubregionLayout {
                preamble,
                substream_count_minus_1,
                level,
                picture_size_hor,
                picture_size_vert,
                reserved,
                patterns,
            });
        }
        Ok(Self {
            substream_marking_flag,
            substream_ids_per_line,
            total_substream_ids,
            level_full_panorama,
            layouts,
        })
    }

    pub(crate) fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.flag(self.substream_marking_flag, "SubstreamMarkingFlag")?;
        w.put(
            u64::from(self.substream_ids_per_line),
            7,
            "SubstreamIDsPerLine",
        )?;
        w.put(u64::from(self.total_substream_ids), 8, "TotalSubstreamIDs")?;
        w.put(u64::from(self.level_full_panorama), 8, "LevelFullPanorama")?;
        for l in &self.layouts {
            match (self.substream_marking_flag, &l.preamble) {
                (true, Some(p)) => {
                    w.flag(p.reserved, "reserved")?;
                    w.put(u64::from(p.preamble_substream_id), 7, "PreambleSubstreamID")?;
                }
                (false, None) => {}
                _ => {
                    return Err(invalid(
                        TAG,
                        "HEVC_subregion_descriptor PreambleSubstreamID must be present exactly when SubstreamMarkingFlag is 1",
                    ));
                }
            }
            w.put(
                u64::from(l.substream_count_minus_1),
                8,
                "SubstreamCountMinus1",
            )?;
            w.put(u64::from(l.level), 8, "Level")?;
            w.put(u64::from(l.picture_size_hor), 16, "PictureSizeHor")?;
            w.put(u64::from(l.picture_size_vert), 16, "PictureSizeVert")?;
            w.flag(l.reserved, "reserved")?;
            w.put(l.patterns.len() as u64, 7, "PatternCount")?;
            for pattern in &l.patterns {
                if pattern.len() != usize::from(l.substream_count_minus_1) {
                    return Err(invalid(
                        TAG,
                        "HEVC_subregion_descriptor pattern length differs from SubstreamCountMinus1",
                    ));
                }
                for o in pattern {
                    w.put(u64::from(o.to_be_bytes()[0]), 8, "SubstreamOffset")?;
                }
            }
        }
        Ok(w.into_vec())
    }
}

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)] // bit-field-wise literals mirror the syntax tables
mod tests {
    use super::*;

    // ── Table 2-131 spec vectors (hand-built) ──

    #[test]
    fn subregion_marked_two_layouts() {
        // SubstreamMarkingFlag=1 SubstreamIDsPerLine=4                  -> 84
        // TotalSubstreamIDs=0x10, LevelFullPanorama=0x5A                -> 10 5A
        // layout 0: reserved=1 PreambleSubstreamID=0x11                 -> 91
        //   SubstreamCountMinus1=2 Level=0x3C hor=1920 vert=1080        -> 02 3C 07 80 04 38
        //   reserved=1 PatternCount=2                                   -> 82
        //   pattern0 (2 offsets)=[1,-1]  pattern1=[2,-2]                -> 01 FF 02 FE
        // layout 1: reserved=1 PreambleSubstreamID=0x7F                 -> FF
        //   SubstreamCountMinus1=0 Level=0x1E hor=1280 vert=720         -> 00 1E 05 00 02 D0
        //   reserved=1 PatternCount=1 (one pattern, zero offsets)       -> 81
        let b = [
            0x84, 0x10, 0x5A, 0x91, 0x02, 0x3C, 0x07, 0x80, 0x04, 0x38, 0x82, 0x01, 0xFF, 0x02,
            0xFE, 0xFF, 0x00, 0x1E, 0x05, 0x00, 0x02, 0xD0, 0x81,
        ];
        let d = HevcSubregion::parse_body(&b).unwrap();
        assert!(d.substream_marking_flag);
        assert_eq!(
            (
                d.substream_ids_per_line,
                d.total_substream_ids,
                d.level_full_panorama
            ),
            (4, 0x10, 0x5A)
        );
        assert_eq!(d.layouts.len(), 2);
        let l0 = &d.layouts[0];
        assert_eq!(l0.preamble.unwrap().preamble_substream_id, 0x11);
        assert_eq!((l0.substream_count_minus_1, l0.level), (2, 0x3C));
        assert_eq!((l0.picture_size_hor, l0.picture_size_vert), (1920, 1080));
        assert_eq!(l0.patterns, [vec![1i8, -1], vec![2, -2]]);
        let l1 = &d.layouts[1];
        assert_eq!(l1.preamble.unwrap().preamble_substream_id, 0x7F);
        assert_eq!(l1.patterns, [Vec::<i8>::new()]);
        assert_eq!(d.write_body().unwrap(), b);
    }

    #[test]
    fn subregion_unmarked_has_no_preamble() {
        // SubstreamMarkingFlag=0 IDsPerLine=4 -> 04 ; total 0x10 level 0x5A
        // layout: SubstreamCountMinus1=1 Level=0x3C 1920x1080 reserved=1 PatternCount=1 -> 01 3C 07 80 04 38 81
        //   one pattern of ONE offset (the loop is `k < SubstreamCountMinus1`): -5 = FB
        let b = [
            0x04, 0x10, 0x5A, 0x01, 0x3C, 0x07, 0x80, 0x04, 0x38, 0x81, 0xFB,
        ];
        let d = HevcSubregion::parse_body(&b).unwrap();
        assert!(!d.substream_marking_flag);
        assert!(d.layouts[0].preamble.is_none());
        assert_eq!(d.layouts[0].patterns, [vec![-5i8]]);
        assert_eq!(d.write_body().unwrap(), b);
        // No layouts at all (N1 = 0) is the bare 3-byte header.
        let bare = HevcSubregion::parse_body(&[0x04, 0x10, 0x5A]).unwrap();
        assert!(bare.layouts.is_empty());
        assert_eq!(bare.write_body().unwrap(), [0x04, 0x10, 0x5A]);
    }

    #[test]
    fn subregion_rejects_inconsistent_values() {
        // truncated layout
        assert!(HevcSubregion::parse_body(&[0x04, 0x10, 0x5A, 0x01, 0x3C]).is_err());
        // pattern length != SubstreamCountMinus1
        let mut d = HevcSubregion::parse_body(&[
            0x04, 0x10, 0x5A, 0x01, 0x3C, 0x07, 0x80, 0x04, 0x38, 0x81, 0xFB,
        ])
        .unwrap();
        d.layouts[0].patterns[0].push(1);
        assert!(d.write_body().is_err());
        // preamble presence must match SubstreamMarkingFlag
        d.layouts[0].patterns[0].pop();
        d.layouts[0].preamble = Some(HevcSubregionPreamble::new(0x11));
        assert!(d.write_body().is_err());
        // PatternCount is 7 bits
        d.layouts[0].preamble = None;
        d.layouts[0].substream_count_minus_1 = 0;
        d.layouts[0].patterns = vec![Vec::new(); 128];
        assert!(d.write_body().is_err());
    }

    // Hand-built from Table 2-117 (see docs): reserved bits all ones.
    #[test]
    fn operation_point_spec_vector() {
        let ptl = ProfileTierLevelInfo::new(0, false, 1, 0, 0);
        // 96 bits: space=10 tier=1 idc=00101 -> 0xA5, then 0xAA fill.
        let mut other = ProfileTierLevelInfo::new(2, true, 5, 0xAAAA_AAAA, 0xAA);
        other.general_progressive_source_flag = true;
        other.general_non_packed_constraint_flag = true;
        other.copied_44bits = 0xAAA_AAAA_AAAA;
        let mut e = HevcOperationPointEntry::new(
            2,
            vec![HevcOpEs::new(true, 5), HevcOpEs::new(false, 6)],
            vec![
                HevcOpLayer::new(true, true, 0),
                HevcOpLayer::new(true, false, 1),
            ],
            6,
        );
        e.frame_rate = Some(HevcOpFrameRate::new(2, 50));
        e.avg_bit_rate = Some(8000);
        e.max_bit_rate = Some(10000);
        let d = HevcOperationPoint::new(vec![ptl, other], vec![e]);
        let b = d.write_body().unwrap();
        // 2 reserved + num_ptl(2) = 0xC2
        assert_eq!(b[0], 0xC2);
        assert_eq!(&b[1..13], &[0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            &b[13..25],
            &[
                0xA5, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA
            ]
        );
        assert_eq!(b[25], 1); // operation_points_count
        assert_eq!(b[26], 2); // target_ols
        assert_eq!(b[27], 2); // ES_count
        assert_eq!(&b[28..30], &[0b1_1_000101, 0b1_0_000110]);
        assert_eq!(b[30], 0b11_000010); // reserved + numEsInOp
        assert_eq!(&b[31..33], &[0b11_000000, 0b10_000001]);
        // reserved(1)=1, avg=1, max=1, idc=10, tid=110
        assert_eq!(b[33], 0b1_1_1_10_110);
        assert_eq!(&b[34..36], &[0xF0, 50]);
        assert_eq!(&b[36..39], &[0, 0x1F, 0x40]);
        assert_eq!(&b[39..42], &[0, 0x27, 0x10]);
        assert_eq!(HevcOperationPoint::parse_body(&b).unwrap(), d);
    }

    #[test]
    fn operation_point_rejects_bad_idc_and_truncation() {
        let mut e = HevcOperationPointEntry::new(0, vec![], vec![], 0);
        e.frame_rate = Some(HevcOpFrameRate::new(0, 1));
        assert!(
            HevcOperationPoint::new(vec![], vec![e])
                .write_body()
                .is_err()
        );
        assert!(HevcOperationPoint::parse_body(&[0xC1, 0x00]).is_err());
    }

    #[test]
    fn hierarchy_extension_round_trip() {
        // Table 2-118: dims=0x0003 layer=5 tid=2 nuh=1 tref=0 res=11 num=2 res=11 chan=4
        let b = [
            0x00,
            0x03,
            0b000101_01,
            0b0_000001_0,
            0b11_000010,
            0b11_000100,
            0xC1,
            0xC3,
        ];
        let d = HevcHierarchyExtension::parse_body(&b).unwrap();
        assert_eq!(d.hierarchy_layer_index, 5);
        assert_eq!(d.temporal_id, 2);
        assert_eq!(d.nuh_layer_id, 1);
        assert!(!d.tref_present_flag);
        assert_eq!(d.embedded_layers.len(), 2);
        assert_eq!(d.embedded_layers[1].index, 3);
        assert_eq!(d.write_body().unwrap(), b);
    }

    #[test]
    fn tile_substream_three_forms() {
        // size 1: ReferenceFlag reserved bit survives
        let single = HevcTileSubstream::parse_body(&[0x85]).unwrap();
        assert_eq!(
            single.detail,
            HevcTileSubstreamDetail::Single {
                reserved_reference_flag: true
            }
        );
        assert_eq!(single.write_body().unwrap(), [0x85]);
        let pat = HevcTileSubstream::parse_body(&[0x85, 0x83]).unwrap();
        assert!(matches!(
            pat.detail,
            HevcTileSubstreamDetail::Pattern {
                preamble_flag: true,
                pattern_reference: 3,
                ..
            }
        ));
        assert_eq!(pat.write_body().unwrap(), [0x85, 0x83]);
        let add = HevcTileSubstream::parse_body(&[0x05, 0x81, 0x02]).unwrap();
        let HevcTileSubstreamDetail::Additional(v) = &add.detail else {
            panic!()
        };
        assert_eq!(
            v,
            &[
                AdditionalSubstream::new(true, 1),
                AdditionalSubstream::new(false, 2)
            ]
        );
        assert_eq!(add.write_body().unwrap(), [0x05, 0x81, 0x02]);
        let empty = HevcTileSubstream {
            substream_id: 1,
            detail: HevcTileSubstreamDetail::Additional(vec![]),
        };
        assert!(empty.write_body().is_err());
    }

    // Bit layout of the hand-computed vector (MSB first):
    //   byte 0    : space=10 tier=1 idc=00101        -> 0b10_1_00101 = 0xA5
    //   bytes 1-4 : compat flags 0x60000000          -> 60 00 00 00
    //   byte 5    : progressive=1 interlaced=0 non_packed=1 frame_only=0,
    //               copied_44bits[43:40] = 0x9       -> 0b1010_1001 = 0xA9
    //   bytes 6-10: copied_44bits[39:0] = 0x12_3456_789A -> 12 34 56 78 9A
    //   byte 11   : general_level_idc = 93 (0x5D)
    const PTL_VECTOR: [u8; 12] = [
        0xA5, 0x60, 0x00, 0x00, 0x00, 0xA9, 0x12, 0x34, 0x56, 0x78, 0x9A, 0x5D,
    ];

    type PtlCase = (fn(&mut ProfileTierLevelInfo), usize, u8);

    fn parse_ptl(b: &[u8]) -> Result<ProfileTierLevelInfo> {
        ProfileTierLevelInfo::parse(&mut BitReader::new(b, "ptl"))
    }

    fn write_ptl(p: &ProfileTierLevelInfo) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        p.write(&mut w)?;
        Ok(w.into_vec())
    }

    #[test]
    fn profile_tier_level_decodes_hand_computed_vector() {
        let p = parse_ptl(&PTL_VECTOR).unwrap();
        assert_eq!(p.general_profile_space, 2);
        assert!(p.general_tier_flag);
        assert_eq!(p.general_profile_idc, 5);
        assert_eq!(p.general_profile_compatibility_flags, 0x6000_0000);
        assert!(p.general_progressive_source_flag);
        assert!(!p.general_interlaced_source_flag);
        assert!(p.general_non_packed_constraint_flag);
        assert!(!p.general_frame_only_constraint_flag);
        assert_eq!(p.copied_44bits, 0x9_1234_5678_9A);
        assert_eq!(p.general_level_idc, 93);
        assert_eq!(write_ptl(&p).unwrap(), PTL_VECTOR);
    }

    #[test]
    fn profile_tier_level_each_field_lands_in_its_bit() {
        let base = ProfileTierLevelInfo::new(0, false, 0, 0, 0);
        assert_eq!(write_ptl(&base).unwrap(), [0u8; 12]);
        let cases: [PtlCase; 12] = [
            (|p| p.general_profile_space = 3, 0, 0xC0),
            (|p| p.general_tier_flag = true, 0, 0x20),
            (|p| p.general_profile_idc = 0x1F, 0, 0x1F),
            (
                |p| p.general_profile_compatibility_flags = 0x8000_0000,
                1,
                0x80,
            ),
            (|p| p.general_profile_compatibility_flags = 1, 4, 0x01),
            (|p| p.general_progressive_source_flag = true, 5, 0x80),
            (|p| p.general_interlaced_source_flag = true, 5, 0x40),
            (|p| p.general_non_packed_constraint_flag = true, 5, 0x20),
            (|p| p.general_frame_only_constraint_flag = true, 5, 0x10),
            (|p| p.copied_44bits = 1 << 43, 5, 0x08),
            (|p| p.copied_44bits = 1, 10, 0x01),
            (|p| p.general_level_idc = 0x80, 11, 0x80),
        ];
        for (set, idx, val) in cases {
            let mut p = base.clone();
            set(&mut p);
            let mut want = [0u8; 12];
            want[idx] = val;
            assert_eq!(write_ptl(&p).unwrap(), want);
            assert_eq!(parse_ptl(&want).unwrap(), p);
        }
    }

    #[test]
    fn profile_tier_level_rejects_out_of_range_fields() {
        let mut p = ProfileTierLevelInfo::new(0, false, 0, 0, 0);
        p.copied_44bits = 1 << 44;
        assert!(write_ptl(&p).is_err());
        assert!(write_ptl(&ProfileTierLevelInfo::new(4, false, 0, 0, 0)).is_err());
        assert!(write_ptl(&ProfileTierLevelInfo::new(0, false, 32, 0, 0)).is_err());
        assert!(parse_ptl(&PTL_VECTOR[..11]).is_err());
    }
}
