//! Stereoscopic 3D descriptors — Rec. ITU-T H.222.0 (08/2023) | ISO/IEC
//! 13818-1:2023:
//!
//! * `MPEG2_stereoscopic_video_format_descriptor` (tag 0x34) — §2.6.84–2.6.85, Table 2-104.
//! * `Stereoscopic_program_info_descriptor` (tag 0x35) — §2.6.86–2.6.87, Tables 2-105/2-106.
//! * `Stereoscopic_video_info_descriptor` (tag 0x36) — §2.6.88–2.6.89, Tables 2-107/2-108.
//!
//! Syntax: `docs/descriptors/iso_13818_1/33-37-mvc-operation-point-stereoscopic-and-transport-profile-descriptors.md`.
//! Every `reserved` field is a typed field re-emitted verbatim (default: all ones).

use crate::error::Result;
use crate::h222_bits::{BitReader, BitWriter, impl_h222_descriptor};
use alloc::vec::Vec;

/// Tag of `MPEG2_stereoscopic_video_format_descriptor` (Table 2-45: 52).
pub const TAG_VIDEO_FORMAT: u8 = 0x34;
/// Tag of `Stereoscopic_program_info_descriptor` (Table 2-45: 53).
pub const TAG_PROGRAM_INFO: u8 = 0x35;
/// Tag of `Stereoscopic_video_info_descriptor` (Table 2-45: 54).
pub const TAG_VIDEO_INFO: u8 = 0x36;
const RESERVED_7_DEFAULT: u8 = 0x7F;
const RESERVED_5_DEFAULT: u8 = 0x1F;

// ───────────────────────── 0x34 ─────────────────────────────────────────────

/// `MPEG2_stereoscopic_video_format_descriptor()` — Table 2-104.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct Mpeg2StereoscopicVideoFormatDescriptor {
    /// `arrangement_type` (7 bits, Table L-1 of ISO/IEC 13818-2 Amd.4 — external
    /// code); `Some` iff `stereo_video_arrangement_type_present == '1'`.
    pub arrangement_type: Option<u8>,
    /// `reserved(7)` carried when no `arrangement_type` is present (default
    /// `0x7F`); ignored when `arrangement_type` is `Some`.
    pub reserved: u8,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl Mpeg2StereoscopicVideoFormatDescriptor {
    /// Build with the spec `reserved` fill and no trailing bytes.
    #[must_use]
    pub fn new(arrangement_type: Option<u8>) -> Self {
        Self {
            arrangement_type,
            reserved: RESERVED_7_DEFAULT,
            trailing: Vec::new(),
        }
    }

    fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "Mpeg2StereoscopicVideoFormatDescriptor");
        let present = r.flag()?;
        let v = r.u8(7)?;
        Ok(Self {
            arrangement_type: present.then_some(v),
            reserved: if present { RESERVED_7_DEFAULT } else { v },
            trailing: r.rest()?.to_vec(),
        })
    }

    fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.flag(
            self.arrangement_type.is_some(),
            "stereo_video_arrangement_type_present",
        )?;
        match self.arrangement_type {
            Some(a) => w.put(u64::from(a), 7, "arrangement_type")?,
            None => w.put(u64::from(self.reserved), 7, "reserved")?,
        }
        w.bytes(&self.trailing)?;
        Ok(w.into_vec())
    }
}

impl_h222_descriptor!(
    Mpeg2StereoscopicVideoFormatDescriptor,
    TAG_VIDEO_FORMAT,
    "Mpeg2StereoscopicVideoFormatDescriptor",
    "MPEG2_STEREOSCOPIC_VIDEO_FORMAT"
);

// ───────────────────────── 0x35 ─────────────────────────────────────────────

/// `stereoscopic_service_type` (Table 2-106).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StereoscopicServiceType {
    /// `000` — unspecified.
    Unspecified,
    /// `001` — 2D-only (monoscopic) service.
    Monoscopic2d,
    /// `010` — frame-compatible stereoscopic 3D service.
    FrameCompatible,
    /// `011` — service-compatible stereoscopic 3D service.
    ServiceCompatible,
    /// `100`..`111` — reserved by ITU-T H.222.0 | ISO/IEC 13818-1 (value carried).
    Reserved(u8),
}

impl StereoscopicServiceType {
    /// Decode the 3-bit field (lossless).
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Unspecified,
            1 => Self::Monoscopic2d,
            2 => Self::FrameCompatible,
            3 => Self::ServiceCompatible,
            other => Self::Reserved(other),
        }
    }

    /// The wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Unspecified => 0,
            Self::Monoscopic2d => 1,
            Self::FrameCompatible => 2,
            Self::ServiceCompatible => 3,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label (Table 2-106).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Monoscopic2d => "2D-only (monoscopic) service",
            Self::FrameCompatible => "frame-compatible stereoscopic 3D service",
            Self::ServiceCompatible => "service-compatible stereoscopic 3D service",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(StereoscopicServiceType, Reserved);

/// `Stereoscopic_program_info_descriptor()` — Table 2-105.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct StereoscopicProgramInfoDescriptor {
    /// `reserved(5)`, default `0x1F`.
    pub reserved: u8,
    /// `stereoscopic_service_type` (Table 2-106).
    pub stereoscopic_service_type: StereoscopicServiceType,
    /// Bytes after the syntax (`descriptor_length` shall be 1, so none are
    /// expected), preserved verbatim.
    pub trailing: Vec<u8>,
}

impl StereoscopicProgramInfoDescriptor {
    /// Build with the spec `reserved` fill and no trailing bytes.
    #[must_use]
    pub fn new(stereoscopic_service_type: StereoscopicServiceType) -> Self {
        Self {
            reserved: RESERVED_5_DEFAULT,
            stereoscopic_service_type,
            trailing: Vec::new(),
        }
    }

    fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "StereoscopicProgramInfoDescriptor");
        Ok(Self {
            reserved: r.u8(5)?,
            stereoscopic_service_type: StereoscopicServiceType::from_u8(r.u8(3)?),
            trailing: r.rest()?.to_vec(),
        })
    }

    fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.put(u64::from(self.reserved), 5, "reserved")?;
        w.put(
            u64::from(self.stereoscopic_service_type.to_u8()),
            3,
            "stereoscopic_service_type",
        )?;
        w.bytes(&self.trailing)?;
        Ok(w.into_vec())
    }
}

impl_h222_descriptor!(
    StereoscopicProgramInfoDescriptor,
    TAG_PROGRAM_INFO,
    "StereoscopicProgramInfoDescriptor",
    "STEREOSCOPIC_PROGRAM_INFO"
);

// ───────────────────────── 0x36 ─────────────────────────────────────────────

/// Upsampling factor (Table 2-108), 4 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum UpsamplingFactor {
    /// `0000` — forbidden.
    Forbidden,
    /// `0001` — unspecified (use the information in the video elementary stream).
    Unspecified,
    /// `0010` — coded resolution is the same as the base view.
    SameAsBaseView,
    /// `0011` — coded resolution is 3/4 of the base view.
    ThreeQuarters,
    /// `0100` — coded resolution is 2/3 of the base view.
    TwoThirds,
    /// `0101` — coded resolution is 1/2 of the base view.
    Half,
    /// `0110`..`1000` — reserved (value carried).
    Reserved(u8),
    /// `1001`..`1111` — user private (value carried).
    UserPrivate(u8),
}

impl UpsamplingFactor {
    /// Decode the 4-bit field (lossless).
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Forbidden,
            1 => Self::Unspecified,
            2 => Self::SameAsBaseView,
            3 => Self::ThreeQuarters,
            4 => Self::TwoThirds,
            5 => Self::Half,
            6..=8 => Self::Reserved(v),
            other => Self::UserPrivate(other),
        }
    }

    /// The wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Forbidden => 0,
            Self::Unspecified => 1,
            Self::SameAsBaseView => 2,
            Self::ThreeQuarters => 3,
            Self::TwoThirds => 4,
            Self::Half => 5,
            Self::Reserved(v) | Self::UserPrivate(v) => v,
        }
    }

    /// Spec label (Table 2-108).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::Unspecified => "unspecified",
            Self::SameAsBaseView => "same coded resolution as base view",
            Self::ThreeQuarters => "3/4 coded resolution of base view",
            Self::TwoThirds => "2/3 coded resolution of base view",
            Self::Half => "1/2 coded resolution of base view",
            Self::Reserved(_) => "reserved",
            Self::UserPrivate(_) => "user_private",
        }
    }
}
broadcast_common::impl_spec_display!(UpsamplingFactor, Reserved, UserPrivate);

/// The `base_video_flag`-selected part of `Stereoscopic_video_info_descriptor()`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StereoscopicVideoView {
    /// `base_video_flag == '1'`: the base view video stream.
    Base {
        /// `reserved(7)` before `leftview_flag`, default `0x7F`.
        reserved: u8,
        /// `leftview_flag`: the stream is the left (true) or right view.
        leftview_flag: bool,
    },
    /// `base_video_flag == '0'`: the additional view video stream.
    Additional {
        /// `reserved(7)` before `usable_as_2D`, default `0x7F`.
        reserved: u8,
        /// `usable_as_2D`: the additional view may also serve a 2D service.
        usable_as_2d: bool,
        /// `horizontal_upsampling_factor` (Table 2-108).
        horizontal_upsampling_factor: UpsamplingFactor,
        /// `vertical_upsampling_factor` (Table 2-108).
        vertical_upsampling_factor: UpsamplingFactor,
    },
}

/// `Stereoscopic_video_info_descriptor()` — Table 2-107.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct StereoscopicVideoInfoDescriptor {
    /// `reserved(7)` before `base_video_flag`, default `0x7F`.
    pub reserved: u8,
    /// The part selected by `base_video_flag`.
    pub view: StereoscopicVideoView,
    /// Bytes after the syntax, preserved verbatim.
    pub trailing: Vec<u8>,
}

impl StereoscopicVideoInfoDescriptor {
    /// Build with the spec `reserved` fill and no trailing bytes.
    #[must_use]
    pub fn new(view: StereoscopicVideoView) -> Self {
        Self {
            reserved: RESERVED_7_DEFAULT,
            view,
            trailing: Vec::new(),
        }
    }

    fn parse_body(body: &[u8]) -> Result<Self> {
        let mut r = BitReader::new(body, "StereoscopicVideoInfoDescriptor");
        let reserved = r.u8(7)?;
        let base = r.flag()?;
        let view = if base {
            StereoscopicVideoView::Base {
                reserved: r.u8(7)?,
                leftview_flag: r.flag()?,
            }
        } else {
            StereoscopicVideoView::Additional {
                reserved: r.u8(7)?,
                usable_as_2d: r.flag()?,
                horizontal_upsampling_factor: UpsamplingFactor::from_u8(r.u8(4)?),
                vertical_upsampling_factor: UpsamplingFactor::from_u8(r.u8(4)?),
            }
        };
        Ok(Self {
            reserved,
            view,
            trailing: r.rest()?.to_vec(),
        })
    }

    fn write_body(&self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        w.put(u64::from(self.reserved), 7, "reserved")?;
        match &self.view {
            StereoscopicVideoView::Base {
                reserved,
                leftview_flag,
            } => {
                w.flag(true, "base_video_flag")?;
                w.put(u64::from(*reserved), 7, "reserved")?;
                w.flag(*leftview_flag, "leftview_flag")?;
            }
            StereoscopicVideoView::Additional {
                reserved,
                usable_as_2d,
                horizontal_upsampling_factor,
                vertical_upsampling_factor,
            } => {
                w.flag(false, "base_video_flag")?;
                w.put(u64::from(*reserved), 7, "reserved")?;
                w.flag(*usable_as_2d, "usable_as_2D")?;
                w.put(
                    u64::from(horizontal_upsampling_factor.to_u8()),
                    4,
                    "horizontal_upsampling_factor",
                )?;
                w.put(
                    u64::from(vertical_upsampling_factor.to_u8()),
                    4,
                    "vertical_upsampling_factor",
                )?;
            }
        }
        w.bytes(&self.trailing)?;
        Ok(w.into_vec())
    }
}

impl_h222_descriptor!(
    StereoscopicVideoInfoDescriptor,
    TAG_VIDEO_INFO,
    "StereoscopicVideoInfoDescriptor",
    "STEREOSCOPIC_VIDEO_INFO"
);

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)] // bit-field-wise literals mirror the syntax tables
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    fn rt<T>(b: &[u8]) -> T
    where
        T: for<'a> Parse<'a, Error = crate::error::Error> + Serialize<Error = crate::error::Error>,
    {
        let d = T::parse(b).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, b);
        d
    }

    #[test]
    fn video_format_present_and_absent() {
        // Table 2-104: present, arrangement_type 0x03
        let d: Mpeg2StereoscopicVideoFormatDescriptor = rt(&[TAG_VIDEO_FORMAT, 1, 0x83]);
        assert_eq!(d.arrangement_type, Some(3));
        // absent: the 7 reserved bits survive (here all ones)
        let d: Mpeg2StereoscopicVideoFormatDescriptor = rt(&[TAG_VIDEO_FORMAT, 1, 0x7F]);
        assert_eq!(d.arrangement_type, None);
        assert_eq!(d.reserved, 0x7F);
        // non-default reserved bits round-trip
        let d: Mpeg2StereoscopicVideoFormatDescriptor = rt(&[TAG_VIDEO_FORMAT, 1, 0x00]);
        assert_eq!(d.reserved, 0);
        assert!(Mpeg2StereoscopicVideoFormatDescriptor::parse(&[TAG_VIDEO_FORMAT, 0]).is_err());
        let mut bad = Mpeg2StereoscopicVideoFormatDescriptor::new(Some(0x80));
        assert!(bad.serialize_into(&mut [0u8; 8]).is_err());
        bad.arrangement_type = Some(0x7F);
        assert!(bad.serialize_into(&mut [0u8; 8]).is_ok());
    }

    #[test]
    fn program_info_service_types() {
        for (b, t) in [
            (0xF8u8, StereoscopicServiceType::Unspecified),
            (0xF9, StereoscopicServiceType::Monoscopic2d),
            (0xFA, StereoscopicServiceType::FrameCompatible),
            (0xFB, StereoscopicServiceType::ServiceCompatible),
            (0xFC, StereoscopicServiceType::Reserved(4)),
        ] {
            let d: StereoscopicProgramInfoDescriptor = rt(&[TAG_PROGRAM_INFO, 1, b]);
            assert_eq!(d.stereoscopic_service_type, t);
            assert_eq!(d.reserved, 0x1F);
        }
    }

    #[test]
    fn video_info_base_and_additional() {
        // reserved=7F base=1 | reserved=7F leftview=1
        let d: StereoscopicVideoInfoDescriptor = rt(&[TAG_VIDEO_INFO, 2, 0xFF, 0xFF]);
        assert!(matches!(
            d.view,
            StereoscopicVideoView::Base {
                leftview_flag: true,
                ..
            }
        ));
        // additional view: usable_as_2D=1, hup=0010 (same), vup=0101 (half)
        let d: StereoscopicVideoInfoDescriptor = rt(&[TAG_VIDEO_INFO, 3, 0xFE, 0xFF, 0x25]);
        let StereoscopicVideoView::Additional {
            usable_as_2d,
            horizontal_upsampling_factor,
            vertical_upsampling_factor,
            ..
        } = d.view
        else {
            panic!()
        };
        assert!(usable_as_2d);
        assert_eq!(
            horizontal_upsampling_factor,
            UpsamplingFactor::SameAsBaseView
        );
        assert_eq!(vertical_upsampling_factor, UpsamplingFactor::Half);
        // user-private / reserved factors are preserved
        let d: StereoscopicVideoInfoDescriptor = rt(&[TAG_VIDEO_INFO, 3, 0xFE, 0xFE, 0x7F]);
        assert!(matches!(
            d.view,
            StereoscopicVideoView::Additional {
                horizontal_upsampling_factor: UpsamplingFactor::Reserved(7),
                vertical_upsampling_factor: UpsamplingFactor::UserPrivate(15),
                ..
            }
        ));
        // base view needs its second byte; additional view its third
        assert!(StereoscopicVideoInfoDescriptor::parse(&[TAG_VIDEO_INFO, 1, 0xFF]).is_err());
        assert!(StereoscopicVideoInfoDescriptor::parse(&[TAG_VIDEO_INFO, 2, 0xFE, 0xFF]).is_err());
    }
}
