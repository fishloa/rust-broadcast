//! MPEG-2 `Extension_descriptor` bodies for layered / tiled HEVC — Rec. ITU-T
//! H.222.0 (08/2023) | ISO/IEC 13818-1:2023:
//!
//! * `extension_descriptor_tag` 0x05 `HEVC_operation_point_descriptor()` —
//!   §2.6.100, Table 2-117 (page p.130). The page declares the per-operation-point
//!   count as `numEsInOp[i]` but bounds the loop with `NumESinOP[i]`; they are
//!   the same quantity.
//! * 0x06 `HEVC_hierarchy_extension_descriptor()` — §2.6.102, Table 2-118.
//! * 0x12 `HEVC_tile_substream_descriptor()` — §2.6.122, Table 2-130.
//!
//! (0x13 `HEVC_subregion_descriptor()`, Table 2-131, is deliberately not typed:
//! its inner `SubstreamOffset` loop is bounded by `k < SubstreamCountMinus1[i]`
//! as printed, which TSDuck 3.44 contradicts — see the docs.)
//!
//! Syntax transcriptions: `docs/descriptors/iso_13818_1/3f-extension-descriptor-bodies-0x02-0x07.md`
//! and `…-0x0f-0x19.md`. `profile_tier_level_info[i]` is the 96-bit H.265
//! §7.3.3 structure (not vendored) and is carried as 12 opaque bytes.
//! Every `reserved` field is a typed field that is re-emitted verbatim
//! (default: all ones).

use crate::error::Result;
use crate::h222_bits::{BitReader, BitWriter, invalid};
use alloc::vec::Vec;

const TAG: u8 = super::mpeg2_extension::TAG;
/// Size of one `profile_tier_level_info[i]` (96 bits).
pub const PROFILE_TIER_LEVEL_LEN: usize = 12;
const RES1: u8 = 0x01;
const RES2: u8 = 0x03;
const RES4: u8 = 0x0F;

// ───────────────────────── 0x05 HEVC_operation_point_descriptor ─────────────

/// One `ES_reference` of an operation point.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
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
pub struct HevcOperationPoint {
    /// `reserved(2)` before `num_ptl`, default 3.
    pub reserved: u8,
    /// `profile_tier_level_info[i]` — 96 bits each, coded per H.265 §7.3.3
    /// (not vendored: opaque).
    pub profile_tier_levels: Vec<[u8; PROFILE_TIER_LEVEL_LEN]>,
    /// The operation points.
    pub operation_points: Vec<HevcOperationPointEntry>,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl HevcOperationPoint {
    /// Build with the spec `reserved` fill and no trailing bytes.
    #[must_use]
    pub fn new(
        profile_tier_levels: Vec<[u8; PROFILE_TIER_LEVEL_LEN]>,
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
            let mut ptl = [0u8; PROFILE_TIER_LEVEL_LEN];
            ptl.copy_from_slice(r.bytes(PROFILE_TIER_LEVEL_LEN)?);
            profile_tier_levels.push(ptl);
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
            w.bytes(ptl);
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
        w.bytes(&self.trailing);
        Ok(w.into_vec())
    }
}

// ───────────────────────── 0x06 HEVC_hierarchy_extension_descriptor ─────────

/// One `hierarchy_ext_embedded_layer_index[i]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
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
        w.bytes(&self.trailing);
        Ok(w.into_vec())
    }
}

// ───────────────────────── 0x12 HEVC_tile_substream_descriptor ──────────────

/// One `Flag[i]` / `AdditionalSubstreamID[i]` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
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
pub struct HevcTileSubstream {
    /// `SubstreamID` (7 bits).
    pub substream_id: u8,
    /// The remainder, selected by `ReferenceFlag` and the descriptor size.
    pub detail: HevcTileSubstreamDetail,
}

impl HevcTileSubstream {
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
                w.bytes(trailing);
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

#[cfg(test)]
mod tests {
    use super::*;

    // Hand-built from Table 2-117 (see docs): reserved bits all ones.
    #[test]
    fn operation_point_spec_vector() {
        let mut ptl = [0u8; 12];
        ptl[0] = 0x01;
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
        let d = HevcOperationPoint::new(vec![ptl, [0xAA; 12]], vec![e]);
        let b = d.write_body().unwrap();
        // 2 reserved + num_ptl(2) = 0xC2
        assert_eq!(b[0], 0xC2);
        assert_eq!(&b[1..13], &ptl);
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
}
