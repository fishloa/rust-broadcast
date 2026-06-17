//! Hierarchy Descriptor — ISO/IEC 13818-1 §2.6.6 (tag 0x04).
//!
//! Identifies the hierarchy layer, embedded layer and channel of the
//! associated elementary stream. Scalability flags indicate which
//! scalability modes apply to the embedded layer.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for hierarchy_descriptor.
pub const TAG: u8 = 0x04;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 4;

/// Hierarchy type — ISO/IEC 13818-1 Table 2-50.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum HierarchyType {
    /// 0 — Reserved.
    Reserved0,
    /// 1 — Spatial Scalability.
    SpatialScalability,
    /// 2 — SNR Scalability.
    SnrScalability,
    /// 3 — Temporal Scalability.
    TemporalScalability,
    /// 4 — Data partitioning.
    DataPartitioning,
    /// 5 — Extension bitstream.
    ExtensionBitstream,
    /// 6 — Private Stream.
    PrivateStream,
    /// 7 — Multi-view Profile.
    MultiViewProfile,
    /// 8 — Combined Scalability or MV-HEVC sub-partition.
    CombinedScalabilityOrMvHevc,
    /// 9 — MVC video sub-bitstream or MVCD video sub-bitstream.
    MvcOrMvcdVideoSubBitstream,
    /// 10 — Auxiliary picture layer (Annex F of Rec. ITU-T H.265).
    AuxiliaryPictureLayer,
    /// 11–14 — Reserved.
    Reserved11To14(u8),
    /// 15 — Base layer or MVC base view sub-bitstream or …
    BaseLayerOrMvcBaseView,
}

impl HierarchyType {
    /// Construct from a raw byte; unknown values preserve the value
    /// for byte-identical round-trip.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Reserved0,
            1 => Self::SpatialScalability,
            2 => Self::SnrScalability,
            3 => Self::TemporalScalability,
            4 => Self::DataPartitioning,
            5 => Self::ExtensionBitstream,
            6 => Self::PrivateStream,
            7 => Self::MultiViewProfile,
            8 => Self::CombinedScalabilityOrMvHevc,
            9 => Self::MvcOrMvcdVideoSubBitstream,
            10 => Self::AuxiliaryPictureLayer,
            v @ 11..=14 => Self::Reserved11To14(v),
            15 => Self::BaseLayerOrMvcBaseView,
            _ => unreachable!("hierarchy_type is 4 bits (0–15)"),
        }
    }

    /// Return the raw byte value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Reserved0 => 0,
            Self::SpatialScalability => 1,
            Self::SnrScalability => 2,
            Self::TemporalScalability => 3,
            Self::DataPartitioning => 4,
            Self::ExtensionBitstream => 5,
            Self::PrivateStream => 6,
            Self::MultiViewProfile => 7,
            Self::CombinedScalabilityOrMvHevc => 8,
            Self::MvcOrMvcdVideoSubBitstream => 9,
            Self::AuxiliaryPictureLayer => 10,
            Self::Reserved11To14(v) => v,
            Self::BaseLayerOrMvcBaseView => 15,
        }
    }

    /// Returns a human-readable spec name for this value.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Reserved0 => "reserved",
            Self::SpatialScalability => "Spatial Scalability",
            Self::SnrScalability => "SNR Scalability",
            Self::TemporalScalability => "Temporal Scalability",
            Self::DataPartitioning => "Data partitioning",
            Self::ExtensionBitstream => "Extension bitstream",
            Self::PrivateStream => "Private Stream",
            Self::MultiViewProfile => "Multi-view Profile",
            Self::CombinedScalabilityOrMvHevc => {
                "Combined Scalability or MV-HEVC sub-partition"
            }
            Self::MvcOrMvcdVideoSubBitstream => {
                "MVC video sub-bitstream or MVCD video sub-bitstream"
            }
            Self::AuxiliaryPictureLayer => {
                "Auxiliary picture layer as defined in Annex F of Rec. ITU-T H.265 | ISO/IEC 23008-2"
            }
            Self::Reserved11To14(_) => "reserved",
            Self::BaseLayerOrMvcBaseView => {
                "Base layer or MVC base view sub-bitstream or AVC video sub-bitstream of MVC or HEVC temporal video sub-bitstream or HEVC base sub-partition or Base layer of MVCD base view sub-bitstream or AVC video sub-bitstream of MVCD or VVC temporal video sub-bitstream or EVC temporal video sub-bitstream"
            }
        }
    }
}
dvb_common::impl_spec_display!(HierarchyType, Reserved11To14);

/// Hierarchy Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct HierarchyDescriptor {
    /// No view scalability flag.
    pub no_view_scalability_flag: bool,
    /// No temporal scalability flag.
    pub no_temporal_scalability_flag: bool,
    /// No spatial scalability flag.
    pub no_spatial_scalability_flag: bool,
    /// No quality scalability flag.
    pub no_quality_scalability_flag: bool,
    /// Hierarchy type (Table 2-50).
    pub hierarchy_type: HierarchyType,
    /// Hierarchy layer index.
    pub hierarchy_layer_index: u8,
    /// Tref present flag.
    pub tref_present_flag: bool,
    /// Hierarchy embedded layer index.
    pub hierarchy_embedded_layer_index: u8,
    /// Hierarchy channel.
    pub hierarchy_channel: u8,
}

impl<'a> Parse<'a> for HierarchyDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "HierarchyDescriptor",
            "unexpected tag for hierarchy_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "hierarchy_descriptor length must equal 4",
            });
        }
        let b0 = body[0];
        let no_view_scalability_flag = (b0 & 0x80) != 0;
        let no_temporal_scalability_flag = (b0 & 0x40) != 0;
        let no_spatial_scalability_flag = (b0 & 0x20) != 0;
        let no_quality_scalability_flag = (b0 & 0x10) != 0;
        let hierarchy_type = HierarchyType::from_u8(b0 & 0x0F);
        let b1 = body[1];
        let hierarchy_layer_index = b1 & 0x3F;
        let b2 = body[2];
        let tref_present_flag = (b2 & 0x80) != 0;
        let hierarchy_embedded_layer_index = b2 & 0x3F;
        let b3 = body[3];
        let hierarchy_channel = b3 & 0x3F;
        Ok(Self {
            no_view_scalability_flag,
            no_temporal_scalability_flag,
            no_spatial_scalability_flag,
            no_quality_scalability_flag,
            hierarchy_type,
            hierarchy_layer_index,
            tref_present_flag,
            hierarchy_embedded_layer_index,
            hierarchy_channel,
        })
    }
}

impl Serialize for HierarchyDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + (BODY_LEN as usize)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        buf[HEADER_LEN] = ((self.no_view_scalability_flag as u8) << 7)
            | ((self.no_temporal_scalability_flag as u8) << 6)
            | ((self.no_spatial_scalability_flag as u8) << 5)
            | ((self.no_quality_scalability_flag as u8) << 4)
            | self.hierarchy_type.to_u8();
        buf[HEADER_LEN + 1] = self.hierarchy_layer_index & 0x3F;
        buf[HEADER_LEN + 2] =
            ((self.tref_present_flag as u8) << 7) | (self.hierarchy_embedded_layer_index & 0x3F);
        buf[HEADER_LEN + 3] = self.hierarchy_channel & 0x3F;
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for HierarchyDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "HIERARCHY";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let bytes = [
            TAG,
            4,
            0b1010_0011, // view=1, temporal=0, spatial=1, quality=0, type=3
            0x05,        // reserved(2) | hierarchy_layer_index(6)
            0b1001_0011, // tref=1, reserved=0(1b), embedded=19 (6b=010011)
            0b00_110010, // reserved(2) | channel(6)
        ];
        let d = HierarchyDescriptor::parse(&bytes).unwrap();
        assert!(d.no_view_scalability_flag);
        assert!(!d.no_temporal_scalability_flag);
        assert!(d.no_spatial_scalability_flag);
        assert!(!d.no_quality_scalability_flag);
        assert_eq!(d.hierarchy_type, HierarchyType::TemporalScalability);
        assert_eq!(d.hierarchy_layer_index, 5);
        assert!(d.tref_present_flag);
        assert_eq!(d.hierarchy_embedded_layer_index, 19);
        assert_eq!(d.hierarchy_channel, 50);
    }

    #[test]
    fn serialize_round_trip() {
        let d = HierarchyDescriptor {
            no_view_scalability_flag: false,
            no_temporal_scalability_flag: true,
            no_spatial_scalability_flag: false,
            no_quality_scalability_flag: true,
            hierarchy_type: HierarchyType::DataPartitioning,
            hierarchy_layer_index: 42,
            tref_present_flag: false,
            hierarchy_embedded_layer_index: 7,
            hierarchy_channel: 63,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = HierarchyDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn hierarchy_type_round_trip() {
        for v in 0u8..=15 {
            assert_eq!(
                HierarchyType::from_u8(v).to_u8(),
                v,
                "round-trip failed for {v:#04x}"
            );
        }
    }

    #[test]
    fn hierarchy_type_name() {
        assert_eq!(
            HierarchyType::SpatialScalability.name(),
            "Spatial Scalability"
        );
        assert_eq!(HierarchyType::BaseLayerOrMvcBaseView.name(), "Base layer or MVC base view sub-bitstream or AVC video sub-bitstream of MVC or HEVC temporal video sub-bitstream or HEVC base sub-partition or Base layer of MVCD base view sub-bitstream or AVC video sub-bitstream of MVCD or VVC temporal video sub-bitstream or EVC temporal video sub-bitstream");
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = HierarchyDescriptor::parse(&[0x05, 4, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x05, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = HierarchyDescriptor::parse(&[TAG, 3, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = HierarchyDescriptor {
            no_view_scalability_flag: false,
            no_temporal_scalability_flag: false,
            no_spatial_scalability_flag: false,
            no_quality_scalability_flag: false,
            hierarchy_type: HierarchyType::from_u8(0),
            hierarchy_layer_index: 0,
            tref_present_flag: false,
            hierarchy_embedded_layer_index: 0,
            hierarchy_channel: 0,
        };
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
