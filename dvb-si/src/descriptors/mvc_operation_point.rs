//! MVC operation point descriptor — Rec. ITU-T H.222.0 (08/2023) |
//! ISO/IEC 13818-1:2023 §2.6.82–2.6.83, Table 2-103 (tag 0x33); syntax in
//! `docs/descriptors/iso_13818_1/33-37-mvc-operation-point-stereoscopic-and-transport-profile-descriptors.md`.
//!
//! ```text
//! profile_idc(8) constraint_set0..5_flag(1 each) AVC_compatible_flags(2) level_count(8)
//! { level_idc(8) operation_points_count(8)
//!   { reserved(5) applicable_temporal_id(3) num_target_output_views(8) ES_count(8)
//!     { reserved(2) ES_reference(6) }* }* }*
//! ```

use crate::error::Result;
use crate::h222_bits::{BitReader, BitWriter, impl_h222_descriptor};
use alloc::vec::Vec;

/// Descriptor tag of `MVC_operation_point_descriptor` (Table 2-45: 51).
pub const TAG: u8 = 0x33;
const RESERVED_5_DEFAULT: u8 = 0x1F;
const RESERVED_2_DEFAULT: u8 = 0x03;

/// One `ES_reference` of an operation point.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MvcEsReference {
    /// `reserved(2)`, default 3.
    pub reserved: u8,
    /// `ES_reference` (6 bits): a `hierarchy_layer_index` of the hierarchy descriptor.
    pub es_reference: u8,
}

impl MvcEsReference {
    /// Build with the spec `reserved` fill.
    #[must_use]
    pub const fn new(es_reference: u8) -> Self {
        Self {
            reserved: RESERVED_2_DEFAULT,
            es_reference,
        }
    }
}

/// One operation point of a level.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MvcOperationPointEntry {
    /// `reserved(5)`, default `0x1F`.
    pub reserved: u8,
    /// `applicable_temporal_id` (3 bits).
    pub applicable_temporal_id: u8,
    /// `num_target_output_views`.
    pub num_target_output_views: u8,
    /// The `ES_count` references (`0xFF` is reserved).
    pub es_references: Vec<MvcEsReference>,
}

impl MvcOperationPointEntry {
    /// Build with the spec `reserved` fill.
    #[must_use]
    pub fn new(
        applicable_temporal_id: u8,
        num_target_output_views: u8,
        es_references: Vec<MvcEsReference>,
    ) -> Self {
        Self {
            reserved: RESERVED_5_DEFAULT,
            applicable_temporal_id,
            num_target_output_views,
            es_references,
        }
    }
}

/// One level and its operation points.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MvcLevel {
    /// `level_idc` (H.264 level, external code).
    pub level_idc: u8,
    /// The `operation_points_count` operation points.
    pub operation_points: Vec<MvcOperationPointEntry>,
}

impl MvcLevel {
    /// Build from the level and its operation points.
    #[must_use]
    pub fn new(level_idc: u8, operation_points: Vec<MvcOperationPointEntry>) -> Self {
        Self {
            level_idc,
            operation_points,
        }
    }
}

/// `MVC_operation_point_descriptor()` — Table 2-103.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct MvcOperationPointDescriptor {
    /// `profile_idc` (H.264 profile, external code).
    pub profile_idc: u8,
    /// `constraint_set0_flag` … `constraint_set5_flag` (H.264 semantics), in order.
    pub constraint_set_flags: [bool; 6],
    /// `AVC_compatible_flags` (2 bits, H.264 SPS semantics).
    pub avc_compatible_flags: u8,
    /// The `level_count` levels.
    pub levels: Vec<MvcLevel>,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl MvcOperationPointDescriptor {
    /// Build with no trailing bytes.
    #[must_use]
    pub fn new(
        profile_idc: u8,
        constraint_set_flags: [bool; 6],
        avc_compatible_flags: u8,
        levels: Vec<MvcLevel>,
    ) -> Self {
        Self {
            profile_idc,
            constraint_set_flags,
            avc_compatible_flags,
            levels,
            trailing: Vec::new(),
        }
    }

    fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "MvcOperationPointDescriptor");
        let profile_idc = r.u8(8)?;
        let mut constraint_set_flags = [false; 6];
        for f in &mut constraint_set_flags {
            *f = r.flag()?;
        }
        let avc_compatible_flags = r.u8(2)?;
        let level_count = r.bits(8)?;
        let mut levels = Vec::new();
        for _ in 0..level_count {
            let level_idc = r.u8(8)?;
            let ops = r.bits(8)?;
            let mut operation_points = Vec::new();
            for _ in 0..ops {
                let reserved = r.u8(5)?;
                let applicable_temporal_id = r.u8(3)?;
                let num_target_output_views = r.u8(8)?;
                let es_count = r.bits(8)?;
                let mut es_references = Vec::new();
                for _ in 0..es_count {
                    es_references.push(MvcEsReference {
                        reserved: r.u8(2)?,
                        es_reference: r.u8(6)?,
                    });
                }
                operation_points.push(MvcOperationPointEntry {
                    reserved,
                    applicable_temporal_id,
                    num_target_output_views,
                    es_references,
                });
            }
            levels.push(MvcLevel {
                level_idc,
                operation_points,
            });
        }
        Ok(Self {
            profile_idc,
            constraint_set_flags,
            avc_compatible_flags,
            levels,
            trailing: r.rest()?.to_vec(),
        })
    }

    fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.put(u64::from(self.profile_idc), 8, "profile_idc")?;
        for f in &self.constraint_set_flags {
            w.flag(*f, "constraint_set_flag")?;
        }
        w.put(
            u64::from(self.avc_compatible_flags),
            2,
            "AVC_compatible_flags",
        )?;
        w.put(self.levels.len() as u64, 8, "level_count")?;
        for l in &self.levels {
            w.put(u64::from(l.level_idc), 8, "level_idc")?;
            w.put(l.operation_points.len() as u64, 8, "operation_points_count")?;
            for op in &l.operation_points {
                w.put(u64::from(op.reserved), 5, "reserved")?;
                w.put(
                    u64::from(op.applicable_temporal_id),
                    3,
                    "applicable_temporal_id",
                )?;
                w.put(
                    u64::from(op.num_target_output_views),
                    8,
                    "num_target_output_views",
                )?;
                w.put(op.es_references.len() as u64, 8, "ES_count")?;
                for e in &op.es_references {
                    w.put(u64::from(e.reserved), 2, "reserved")?;
                    w.put(u64::from(e.es_reference), 6, "ES_reference")?;
                }
            }
        }
        w.bytes(&self.trailing)?;
        Ok(w.into_vec())
    }
}

impl_h222_descriptor!(
    MvcOperationPointDescriptor,
    TAG,
    "MvcOperationPointDescriptor",
    "MVC_OPERATION_POINT"
);

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)] // bit-field-wise literals mirror the syntax tables
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    #[test]
    fn spec_vector_round_trip() {
        // profile 118, set0 set2 set5, compat=01, 1 level (0x28) with 1 op point:
        // reserved 11111 tid=3, 2 views, 2 ES refs (3, 4).
        let b = [
            TAG,
            10,
            118,
            0b1_0_1_0_0_1_01,
            1,
            0x28,
            1,
            0b11111_011,
            2,
            2,
            0b11_000011,
            0b11_000100,
        ];
        let d = MvcOperationPointDescriptor::parse(&b).unwrap();
        assert_eq!(d.profile_idc, 118);
        assert_eq!(
            d.constraint_set_flags,
            [true, false, true, false, false, true]
        );
        assert_eq!(d.avc_compatible_flags, 1);
        assert_eq!(
            d.levels[0].operation_points[0].es_references[1].es_reference,
            4
        );
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, &b[..]);
    }

    #[test]
    fn rejects_truncation_overflow_and_empty() {
        assert!(MvcOperationPointDescriptor::parse(&[TAG, 3, 118, 0, 1]).is_err());
        let mut d = MvcOperationPointDescriptor::new(1, [false; 6], 0, vec![]);
        let mut out = [0u8; 16];
        assert!(d.serialize_into(&mut out).is_ok()); // zero levels is legal
        d.avc_compatible_flags = 4;
        assert!(d.serialize_into(&mut out).is_err());
    }
}
