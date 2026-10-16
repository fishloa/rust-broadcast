//! Application Icons Descriptor — ETSI TS 102 809 V1.3.1 §5.3.5.6.2, Table 25
//! (AIT tag 0x0B); the `icon_flags` values are §5.2.8.2 Table 7 (PDF p.43 / p.23,
//! transcribed in `docs/descriptors/ts_102_809/25-*.md` and
//! `docs/enums/ts_102_809/7-*.md`, verified against the rendered pages).
//!
//! Zero or one instance in the "application" (inner) AIT loop. Body:
//! `icon_locator_length`(8), `icon_locator_byte`×N, `icon_flags`(16), then
//! `reserved_future_use`(8)×M up to `descriptor_length`.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::DvbText;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for application_icons_descriptor (AIT namespace).
pub const TAG: u8 = 0x0B;
const HEADER_LEN: usize = 2;
const LOCATOR_LEN_FIELD: usize = 1;
const ICON_FLAGS_LEN: usize = 2;

/// One icon size / pixel-aspect-ratio combination of `icon_flags`
/// (TS 102 809 §5.2.8.2 Table 7); each is one bit of the 16-bit field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum IconFormat {
    /// `0x0001` — 32 x 32 for square pixel display.
    Square32x32,
    /// `0x0002` — 32 x 32 for broadcast pixels on 4:3 display.
    Broadcast32x32Dar4x3,
    /// `0x0004` — 24 x 32 for broadcast pixels on 16:9 display.
    Broadcast24x32Dar16x9,
    /// `0x0008` — 64 x 64 for square pixel display.
    Square64x64,
    /// `0x0010` — 64 x 64 for broadcast pixels on 4:3 display.
    Broadcast64x64Dar4x3,
    /// `0x0020` — 48 x 64 for broadcast pixels on 16:9 display.
    Broadcast48x64Dar16x9,
    /// `0x0040` — 128 x 128 for square pixel display.
    Square128x128,
    /// `0x0080` — 128 x 128 for broadcast pixels on 4:3 display.
    Broadcast128x128Dar4x3,
    /// `0x0100` — 96 x 128 for broadcast pixels on 16:9 display.
    Broadcast96x128Dar16x9,
    /// `0x0200` — 256 x 256 for square pixel display.
    Square256x256,
    /// `0x0400` — 256 x 256 for broadcast pixels on 4:3 display.
    Broadcast256x256Dar4x3,
    /// `0x0800` — 192 x 256 for broadcast pixels on 16:9 display.
    Broadcast192x256Dar16x9,
}

impl IconFormat {
    /// Every defined format, in ascending bit order.
    pub const ALL: [IconFormat; 12] = [
        Self::Square32x32,
        Self::Broadcast32x32Dar4x3,
        Self::Broadcast24x32Dar16x9,
        Self::Square64x64,
        Self::Broadcast64x64Dar4x3,
        Self::Broadcast48x64Dar16x9,
        Self::Square128x128,
        Self::Broadcast128x128Dar4x3,
        Self::Broadcast96x128Dar16x9,
        Self::Square256x256,
        Self::Broadcast256x256Dar4x3,
        Self::Broadcast192x256Dar16x9,
    ];

    /// The `icon_flags` bit this format occupies.
    #[must_use]
    pub const fn mask(self) -> u16 {
        match self {
            Self::Square32x32 => 0x0001,
            Self::Broadcast32x32Dar4x3 => 0x0002,
            Self::Broadcast24x32Dar16x9 => 0x0004,
            Self::Square64x64 => 0x0008,
            Self::Broadcast64x64Dar4x3 => 0x0010,
            Self::Broadcast48x64Dar16x9 => 0x0020,
            Self::Square128x128 => 0x0040,
            Self::Broadcast128x128Dar4x3 => 0x0080,
            Self::Broadcast96x128Dar16x9 => 0x0100,
            Self::Square256x256 => 0x0200,
            Self::Broadcast256x256Dar4x3 => 0x0400,
            Self::Broadcast192x256Dar16x9 => 0x0800,
        }
    }

    /// Spec description of the icon size and pixel aspect ratio.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Square32x32 => "32 x 32 for square pixel display",
            Self::Broadcast32x32Dar4x3 => "32 x 32 for broadcast pixels on 4:3 display",
            Self::Broadcast24x32Dar16x9 => "24 x 32 for broadcast pixels on 16:9 display",
            Self::Square64x64 => "64 x 64 for square pixel display",
            Self::Broadcast64x64Dar4x3 => "64 x 64 for broadcast pixels on 4:3 display",
            Self::Broadcast48x64Dar16x9 => "48 x 64 for broadcast pixels on 16:9 display",
            Self::Square128x128 => "128 x 128 for square pixel display",
            Self::Broadcast128x128Dar4x3 => "128 x 128 for broadcast pixels on 4:3 display",
            Self::Broadcast96x128Dar16x9 => "96 x 128 for broadcast pixels on 16:9 display",
            Self::Square256x256 => "256 x 256 for square pixel display",
            Self::Broadcast256x256Dar4x3 => "256 x 256 for broadcast pixels on 4:3 display",
            Self::Broadcast192x256Dar16x9 => "192 x 256 for broadcast pixels on 16:9 display",
        }
    }
}
broadcast_common::impl_spec_display!(IconFormat);

/// The 16-bit `icon_flags` field (Table 7): one bit per [`IconFormat`]; the
/// top nibble (`xxxx 0000 0000 0000`) is `reserved_future_use` and is carried
/// verbatim so a parsed descriptor re-serializes byte-identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct IconFlags(u16);

/// Mask of the defined [`IconFormat`] bits (`0x0FFF`).
pub const ICON_FORMAT_MASK: u16 = 0x0FFF;

impl IconFlags {
    /// From the raw 16-bit wire value (reserved bits preserved).
    #[must_use]
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// The raw 16-bit wire value.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Is `format` signalled?
    #[must_use]
    pub const fn contains(self, format: IconFormat) -> bool {
        self.0 & format.mask() != 0
    }

    /// Signal `format` in addition to those already set.
    #[must_use]
    pub const fn with(self, format: IconFormat) -> Self {
        Self(self.0 | format.mask())
    }

    /// The `reserved_future_use` bits (`xxxx 0000 0000 0000`), right-aligned
    /// to the top nibble position (still masked with `0xF000`).
    #[must_use]
    pub const fn reserved_bits(self) -> u16 {
        self.0 & !ICON_FORMAT_MASK
    }

    /// The signalled formats in ascending bit order.
    pub fn formats(self) -> impl Iterator<Item = IconFormat> {
        IconFormat::ALL
            .into_iter()
            .filter(move |f| self.contains(*f))
    }
}

/// Application Icons Descriptor (AIT tag 0x0B).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationIconsDescriptor<'a> {
    /// `icon_locator` — the string prefixing the standard icon file names
    /// (`icon_locator "/dvb.icon." hex_string`, §5.2.8.1).
    pub icon_locator: DvbText<'a>,
    /// `icon_flags` — which icon sizes / aspect ratios are available.
    pub icon_flags: IconFlags,
    /// Trailing `reserved_future_use`(8) bytes (the spec's `for(i<N)` loop after
    /// `icon_flags`); empty in every defined edition, preserved if present.
    pub reserved_future_use: &'a [u8],
}

impl<'a> Parse<'a> for ApplicationIconsDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ApplicationIconsDescriptor",
            "unexpected tag for application_icons_descriptor",
        )?;
        let loc_len = *body.first().ok_or(Error::InvalidDescriptor {
            tag: TAG,
            reason: "application_icons_descriptor missing icon_locator_length",
        })? as usize;
        let flags_at = LOCATOR_LEN_FIELD + loc_len;
        if body.len() < flags_at + ICON_FLAGS_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "application_icons_descriptor icon_locator/icon_flags overrun body",
            });
        }
        Ok(Self {
            icon_locator: DvbText::new(&body[LOCATOR_LEN_FIELD..flags_at]),
            icon_flags: IconFlags::from_bits(u16::from_be_bytes([
                body[flags_at],
                body[flags_at + 1],
            ])),
            reserved_future_use: &body[flags_at + ICON_FLAGS_LEN..],
        })
    }
}

impl Serialize for ApplicationIconsDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + LOCATOR_LEN_FIELD
            + self.icon_locator.len()
            + ICON_FLAGS_LEN
            + self.reserved_future_use.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let loc_len =
            broadcast_common::len::fit_u8(self.icon_locator.len(), "icon_locator_length")?;
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        let mut pos = HEADER_LEN;
        buf[pos] = loc_len;
        pos += LOCATOR_LEN_FIELD;
        buf[pos..pos + self.icon_locator.len()].copy_from_slice(self.icon_locator.raw());
        pos += self.icon_locator.len();
        buf[pos..pos + ICON_FLAGS_LEN].copy_from_slice(&self.icon_flags.bits().to_be_bytes());
        pos += ICON_FLAGS_LEN;
        buf[pos..pos + self.reserved_future_use.len()].copy_from_slice(self.reserved_future_use);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ApplicationIconsDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "APPLICATION_ICONS";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec example, TS 102 809 §5.2.8.2: `icon_flags` = 0x0005 signals the
    /// 24 x 32 (0x0004) and 32 x 32 (0x0001) icon files.
    fn spec_example() -> Vec<u8> {
        let mut v = vec![TAG, 0x00, 0x06];
        v.extend_from_slice(b"/icons");
        v.extend_from_slice(&[0x00, 0x05]);
        v[1] = u8::try_from(v.len() - 2).unwrap();
        v
    }

    #[test]
    fn parse_spec_example() {
        let bytes = spec_example();
        let d = ApplicationIconsDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.icon_locator.raw(), b"/icons");
        assert_eq!(d.icon_flags.bits(), 0x0005);
        let f: Vec<_> = d.icon_flags.formats().collect();
        assert_eq!(
            f,
            vec![IconFormat::Square32x32, IconFormat::Broadcast24x32Dar16x9]
        );
        assert!(d.reserved_future_use.is_empty());
    }

    #[test]
    fn round_trip_byte_identical_with_reserved_bits_and_trailer() {
        // reserved top nibble set + two trailing reserved_future_use bytes.
        let mut bytes = vec![TAG, 0x00, 0x01, b'/', 0xF8, 0x01, 0xAA, 0xBB];
        bytes[1] = u8::try_from(bytes.len() - 2).unwrap();
        let d = ApplicationIconsDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.icon_flags.reserved_bits(), 0xF000);
        assert_eq!(d.reserved_future_use, &[0xAA, 0xBB]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
        assert_eq!(ApplicationIconsDescriptor::parse(&out).unwrap(), d);
    }

    #[test]
    fn round_trip_spec_example() {
        let bytes = spec_example();
        let d = ApplicationIconsDescriptor::parse(&bytes).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    #[test]
    fn truncated_locator_is_error() {
        assert!(ApplicationIconsDescriptor::parse(&[TAG, 2, 5, b'a']).is_err());
        assert!(ApplicationIconsDescriptor::parse(&[TAG, 0]).is_err());
        assert!(ApplicationIconsDescriptor::parse(&[TAG, 3, 1, b'a', 0x00]).is_err());
    }

    #[test]
    fn locator_over_255_is_overflow_error() {
        let big = vec![b'x'; 256];
        let d = ApplicationIconsDescriptor {
            icon_locator: DvbText::new(&big),
            icon_flags: IconFlags::default(),
            reserved_future_use: &[],
        };
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }

    #[test]
    fn icon_flags_builder_and_labels() {
        let f = IconFlags::default().with(IconFormat::Square256x256);
        assert!(f.contains(IconFormat::Square256x256));
        assert_eq!(f.bits(), 0x0200);
        for fmt in IconFormat::ALL {
            assert_eq!(fmt.mask().count_ones(), 1);
            assert!(!fmt.name().is_empty());
        }
    }
}
