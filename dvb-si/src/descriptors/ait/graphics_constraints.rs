//! Graphics Constraints Descriptor — ETSI TS 102 809 V1.3.1 §5.3.5.8, Table 27
//! (AIT tag 0x14; same layout as §5.2.9.2 Table 8, byte values Table 9).
//! PDF p.44 / p.25, transcribed in `docs/descriptors/ts_102_809/27-*.md` and
//! `docs/enums/ts_102_809/9-*.md`, verified against the rendered pages.
//!
//! Body: `reserved_future_use`(5), `can_run_without_visible_ui`(1),
//! `handles_configuration_changed`(1), `handles_externally_controlled_video`(1),
//! then `graphics_configuration_byte`(8)×N up to `descriptor_length`.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for graphics_constraints_descriptor (AIT namespace).
pub const TAG: u8 = 0x14;
const HEADER_LEN: usize = 2;
const FLAGS_LEN: usize = 1;

const RESERVED_MASK: u8 = 0x1F;
/// Bit position of the 5-bit `reserved_future_use` (the top five bits).
const RESERVED_SHIFT: u32 = 3;
const CAN_RUN_WITHOUT_VISIBLE_UI: u8 = 0x04;
const HANDLES_CONFIGURATION_CHANGED: u8 = 0x02;
const HANDLES_EXTERNALLY_CONTROLLED_VIDEO: u8 = 0x01;

/// Spec default of the 5-bit `reserved_future_use` (all ones, TS 102 809
/// §5.3.3.2 convention).
pub const RESERVED_FUTURE_USE_DEFAULT: u8 = 0x1F;

/// `graphics_configuration_byte` (TS 102 809 §5.2.9.2 Table 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum GraphicsConfiguration {
    /// 1 — full screen standard definition.
    FullScreenStandardDefinition,
    /// 2 — full screen 960x540.
    FullScreen960x540,
    /// 3 — full screen 1280x720.
    FullScreen1280x720,
    /// 4 — full screen 1920x1080.
    FullScreen1920x1080,
    /// 0 (reserved), 5..=31 (reserved for future use by the DVB project) and
    /// 32..=255 (reserved for future use).
    Reserved(u8),
}

impl GraphicsConfiguration {
    /// Decode from the 8-bit wire value.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::FullScreenStandardDefinition,
            2 => Self::FullScreen960x540,
            3 => Self::FullScreen1280x720,
            4 => Self::FullScreen1920x1080,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 8-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::FullScreenStandardDefinition => 1,
            Self::FullScreen960x540 => 2,
            Self::FullScreen1280x720 => 3,
            Self::FullScreen1920x1080 => 4,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::FullScreenStandardDefinition => "Full screen standard definition",
            Self::FullScreen960x540 => "Full screen 960x540",
            Self::FullScreen1280x720 => "Full screen 1280x720",
            Self::FullScreen1920x1080 => "Full screen 1920x1080",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(GraphicsConfiguration, Reserved);

/// Graphics Constraints Descriptor (AIT tag 0x14).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GraphicsConstraintsDescriptor {
    /// `reserved_future_use`(5), right-aligned. Parsed value is kept so
    /// re-serialization is byte-identical; default
    /// [`RESERVED_FUTURE_USE_DEFAULT`].
    pub reserved_future_use: u8,
    /// `can_run_without_visible_ui` (§5.2.9.1.2).
    pub can_run_without_visible_ui: bool,
    /// `handles_configuration_changed` (§5.2.9.1.3).
    pub handles_configuration_changed: bool,
    /// `handles_externally_controlled_video` (§5.2.9.1.4).
    pub handles_externally_controlled_video: bool,
    /// `graphics_configuration_byte` list, most preferred first (§5.2.9.1.1).
    /// Empty = not constrained to particular graphics configurations.
    pub graphics_configurations: Vec<GraphicsConfiguration>,
}

impl Default for GraphicsConstraintsDescriptor {
    /// No flags, no configurations, spec-default reserved bits.
    fn default() -> Self {
        Self {
            reserved_future_use: RESERVED_FUTURE_USE_DEFAULT,
            can_run_without_visible_ui: false,
            handles_configuration_changed: false,
            handles_externally_controlled_video: false,
            graphics_configurations: Vec::new(),
        }
    }
}

impl<'a> Parse<'a> for GraphicsConstraintsDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "GraphicsConstraintsDescriptor",
            "unexpected tag for graphics_constraints_descriptor",
        )?;
        let (&flags, rest) = body.split_first().ok_or(Error::InvalidDescriptor {
            tag: TAG,
            reason: "graphics_constraints_descriptor body shorter than its flags byte",
        })?;
        Ok(Self {
            reserved_future_use: flags >> RESERVED_SHIFT,
            can_run_without_visible_ui: flags & CAN_RUN_WITHOUT_VISIBLE_UI != 0,
            handles_configuration_changed: flags & HANDLES_CONFIGURATION_CHANGED != 0,
            handles_externally_controlled_video: flags & HANDLES_EXTERNALLY_CONTROLLED_VIDEO != 0,
            graphics_configurations: rest
                .iter()
                .map(|&b| GraphicsConfiguration::from_u8(b))
                .collect(),
        })
    }
}

impl Serialize for GraphicsConstraintsDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FLAGS_LEN + self.graphics_configurations.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let reserved = broadcast_common::len::fit_mask(
            self.reserved_future_use,
            RESERVED_MASK,
            "reserved_future_use",
        )?;
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2] = (reserved << RESERVED_SHIFT)
            | (u8::from(self.can_run_without_visible_ui) * CAN_RUN_WITHOUT_VISIBLE_UI)
            | (u8::from(self.handles_configuration_changed) * HANDLES_CONFIGURATION_CHANGED)
            | (u8::from(self.handles_externally_controlled_video)
                * HANDLES_EXTERNALLY_CONTROLLED_VIDEO);
        for (slot, c) in buf[HEADER_LEN + FLAGS_LEN..len]
            .iter_mut()
            .zip(&self.graphics_configurations)
        {
            *slot = c.to_u8();
        }
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for GraphicsConstraintsDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "GRAPHICS_CONSTRAINTS";
}

#[cfg(test)]
mod tests {
    use super::*;

    // Table 27: reserved 0x1F, can_run=1, changed=0, ext_video=1; SD + 1920x1080.
    const BYTES: [u8; 5] = [TAG, 3, 0xFD, 0x01, 0x04];

    #[test]
    fn parse_fields() {
        let d = GraphicsConstraintsDescriptor::parse(&BYTES).unwrap();
        assert_eq!(d.reserved_future_use, 0x1F);
        assert!(d.can_run_without_visible_ui);
        assert!(!d.handles_configuration_changed);
        assert!(d.handles_externally_controlled_video);
        assert_eq!(
            d.graphics_configurations,
            vec![
                GraphicsConfiguration::FullScreenStandardDefinition,
                GraphicsConfiguration::FullScreen1920x1080
            ]
        );
    }

    #[test]
    fn round_trip_byte_identical_including_zero_reserved_and_reserved_config() {
        for bytes in [
            BYTES.to_vec(),
            vec![TAG, 1, 0x00],
            vec![TAG, 3, 0x02, 0x00, 0xFF],
        ] {
            let d = GraphicsConstraintsDescriptor::parse(&bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
            assert_eq!(GraphicsConstraintsDescriptor::parse(&out).unwrap(), d);
        }
    }

    #[test]
    fn default_reserved_is_spec_value_and_overflow_rejected() {
        let d = GraphicsConstraintsDescriptor::default();
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 1, 0xF8]);
        let bad = GraphicsConstraintsDescriptor {
            reserved_future_use: 0x20,
            ..Default::default()
        };
        assert!(bad.serialize_into(&mut out).is_err());
        assert!(GraphicsConstraintsDescriptor::parse(&[TAG, 0]).is_err());
    }

    #[test]
    fn configuration_round_trips_every_byte() {
        for v in 0..=255u8 {
            assert_eq!(GraphicsConfiguration::from_u8(v).to_u8(), v);
        }
    }
}
