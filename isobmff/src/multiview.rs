//! The multiview view-attribute boxes — ISO/IEC 14496-15:2010 F.7, F.8.
//!
//! The attribute table (F.8.4) defines a syntax for each attribute that a
//! `MultiviewGroupBox` or `MultiviewRelationBox` names. Five carry fields and
//! are typed here: `prfl`/`levl` (a reserved 24-bit prefix and an 8-bit
//! indication, one structure), `ilvi`/`elvi` (the view-array order, one
//! structure), and `rtvi` (the rectangular array dimensions). The table states
//! `geom`, `plvi` and `spvi` as "Unspecified", so they have no box layout and
//! are pinned as not applicable rather than guessed.
//!
//! Sources: `private/specs-verified/mpeg/isobmff/iso-iec-14496-15-2010.md`
//! F.7.3 and F.8.

use broadcast_core::len::{need_in, need_out};
use broadcast_core::wire::{be_u16, u32_at};
use broadcast_core::{Parse, Serialize};

use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, box_len, write_box_header};
use crate::error::{Error, Result};

/// `prfl`: the profile attribute (F.8.4).
pub const PRFL: [u8; BOX_TYPE_SIZE] = *b"prfl";
/// `levl`: the level attribute (F.8.4).
pub const LEVL: [u8; BOX_TYPE_SIZE] = *b"levl";
/// `ilvi`: the inline view array attribute (F.8.4).
pub const ILVI: [u8; BOX_TYPE_SIZE] = *b"ilvi";
/// `elvi`: the elliptical view array attribute (F.8.4).
pub const ELVI: [u8; BOX_TYPE_SIZE] = *b"elvi";
/// `rtvi`: the rectangular view array attribute (F.8.4).
pub const RTVI: [u8; BOX_TYPE_SIZE] = *b"rtvi";
/// `vwdi`: the multiview scene info box (F.8.6).
pub const VWDI: [u8; BOX_TYPE_SIZE] = *b"vwdi";
/// `mvci`: the multiview information box (F.8.3).
pub const MVCI: [u8; BOX_TYPE_SIZE] = *b"mvci";

/// Bytes of a profile/level attribute: reserved(24) + indication(8).
const PROFILE_LEVEL_LEN: usize = core::mem::size_of::<u32>();
/// Offset of the 8-bit indication in the profile/level body.
const PROFILE_LEVEL_OFFSET: usize = PROFILE_LEVEL_LEN - 1;
/// Bytes of a view-array order attribute: reserved(28) + two 2-bit orders.
const VIEW_ARRAY_ORDER_LEN: usize = core::mem::size_of::<u32>();
/// Bytes of a rectangular view array: two 16-bit counts.
const RECTANGULAR_ARRAY_LEN: usize = core::mem::size_of::<u16>() * 2;

/// The profile or level attribute (`prfl`, `levl`) — F.8.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ProfileLevelAttribute {
    box_type: [u8; BOX_TYPE_SIZE],
    reserved: u32,
    indication: u8,
}

impl ProfileLevelAttribute {
    /// Build one for `codec_type` (`prfl` or `levl`) with the reserved bits 0.
    ///
    /// # Errors
    /// `codec_type` is neither code.
    pub fn new(codec_type: [u8; BOX_TYPE_SIZE], indication: u8) -> Result<Self> {
        if codec_type != PRFL && codec_type != LEVL {
            return Err(Error::UnexpectedBox {
                expected: "prfl or levl",
            });
        }
        Ok(Self {
            box_type: codec_type,
            reserved: 0,
            indication,
        })
    }

    /// The `profileIndication` or `levelIndication` value.
    #[must_use]
    pub const fn indication(self) -> u8 {
        self.indication
    }

    /// The 24 reserved bits, carried as read.
    #[must_use]
    pub const fn reserved(self) -> u32 {
        self.reserved
    }

    /// Parse the payload after the box header.
    pub fn parse_body(codec_type: [u8; BOX_TYPE_SIZE], body: &[u8]) -> Result<Self> {
        if codec_type != PRFL && codec_type != LEVL {
            return Err(Error::UnexpectedBox {
                expected: "prfl or levl",
            });
        }
        need_in(body, PROFILE_LEVEL_LEN, "prfl/levl")?;
        Ok(Self {
            box_type: codec_type,
            reserved: u32_at(body, 0) >> u8::BITS,
            indication: body[PROFILE_LEVEL_OFFSET],
        })
    }

    /// Parse a whole box, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        Self::parse_body(item.four_cc(), item.body)
    }
}

impl<'a> Parse<'a> for ProfileLevelAttribute {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for ProfileLevelAttribute {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + PROFILE_LEVEL_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &self.box_type, need)?;
        let word = (self.reserved << u8::BITS) | u32::from(self.indication);
        buf[c..c + PROFILE_LEVEL_LEN].copy_from_slice(&word.to_be_bytes());
        c += PROFILE_LEVEL_LEN;
        Ok(c)
    }
}

/// The view-array order attribute (`ilvi`, `elvi`) — F.8.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ViewArrayOrder {
    box_type: [u8; BOX_TYPE_SIZE],
    reserved: u8,
    horizontal_order: u8,
    vertical_order: u8,
}

/// Value of the 2-bit `horizontal_order`/`vertical_order` fields.
const VIEW_ORDER_MASK: u8 = 0b11;
/// Shift of `horizontal_order` in the final byte (above `vertical_order`).
const HORIZONTAL_ORDER_SHIFT: u32 = 2;
/// The 4 reserved high bits of the final byte.
const VIEW_ORDER_RESERVED_MASK: u8 = 0xf0;

impl ViewArrayOrder {
    /// Build one for `codec_type` (`ilvi` or `elvi`).
    ///
    /// # Errors
    /// `codec_type` is neither code.
    pub fn new(
        codec_type: [u8; BOX_TYPE_SIZE],
        horizontal_order: u8,
        vertical_order: u8,
    ) -> Result<Self> {
        if codec_type != ILVI && codec_type != ELVI {
            return Err(Error::UnexpectedBox {
                expected: "ilvi or elvi",
            });
        }
        Ok(Self {
            box_type: codec_type,
            reserved: 0,
            horizontal_order,
            vertical_order,
        })
    }

    /// The horizontal view order (0..=3).
    #[must_use]
    pub const fn horizontal_order(self) -> u8 {
        self.horizontal_order
    }

    /// The vertical view order (0..=3).
    #[must_use]
    pub const fn vertical_order(self) -> u8 {
        self.vertical_order
    }

    /// Parse the payload after the box header.
    pub fn parse_body(codec_type: [u8; BOX_TYPE_SIZE], body: &[u8]) -> Result<Self> {
        if codec_type != ILVI && codec_type != ELVI {
            return Err(Error::UnexpectedBox {
                expected: "ilvi or elvi",
            });
        }
        need_in(body, VIEW_ARRAY_ORDER_LEN, "ilvi/elvi")?;
        let last = body[VIEW_ARRAY_ORDER_LEN - 1];
        Ok(Self {
            box_type: codec_type,
            reserved: last & VIEW_ORDER_RESERVED_MASK,
            horizontal_order: (last >> HORIZONTAL_ORDER_SHIFT) & VIEW_ORDER_MASK,
            vertical_order: last & VIEW_ORDER_MASK,
        })
    }

    /// Parse a whole box, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        Self::parse_body(item.four_cc(), item.body)
    }
}

impl<'a> Parse<'a> for ViewArrayOrder {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for ViewArrayOrder {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + VIEW_ARRAY_ORDER_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &self.box_type, need)?;
        buf[c..c + VIEW_ARRAY_ORDER_LEN - 1].fill(0);
        c += VIEW_ARRAY_ORDER_LEN - 1;
        buf[c] = (self.horizontal_order & VIEW_ORDER_MASK) << HORIZONTAL_ORDER_SHIFT
            | (self.vertical_order & VIEW_ORDER_MASK)
            | self.reserved;
        Ok(c + 1)
    }
}

/// The rectangular view array attribute (`rtvi`) — F.8.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RectangularViewArray {
    row_view_count: u16,
    col_view_count: u16,
}

impl RectangularViewArray {
    /// Build one from the row and column counts.
    #[must_use]
    pub const fn new(row_view_count: u16, col_view_count: u16) -> Self {
        Self {
            row_view_count,
            col_view_count,
        }
    }

    /// The number of rows.
    #[must_use]
    pub const fn row_view_count(self) -> u16 {
        self.row_view_count
    }

    /// The number of columns.
    #[must_use]
    pub const fn col_view_count(self) -> u16 {
        self.col_view_count
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, RECTANGULAR_ARRAY_LEN, "rtvi")?;
        Ok(Self {
            row_view_count: be_u16(body, 0, "rtvi row_view_count")?,
            col_view_count: be_u16(body, core::mem::size_of::<u16>(), "rtvi col_view_count")?,
        })
    }

    /// Parse a whole box, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        if item.four_cc() != RTVI {
            return Err(Error::UnexpectedBox { expected: "rtvi" });
        }
        Self::parse_body(item.body)
    }
}

impl<'a> Parse<'a> for RectangularViewArray {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for RectangularViewArray {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + RECTANGULAR_ARRAY_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_box_header(buf, &RTVI, need)?;
        buf[c..c + core::mem::size_of::<u16>()].copy_from_slice(&self.row_view_count.to_be_bytes());
        c += core::mem::size_of::<u16>();
        buf[c..c + core::mem::size_of::<u16>()].copy_from_slice(&self.col_view_count.to_be_bytes());
        c += core::mem::size_of::<u16>();
        Ok(c)
    }
}

/// The multiview scene info box (`vwdi`) — F.8.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MultiviewSceneInfoBox {
    max_disparity: u8,
}

impl MultiviewSceneInfoBox {
    /// Build one from the maximum disparity.
    #[must_use]
    pub const fn new(max_disparity: u8) -> Self {
        Self { max_disparity }
    }

    /// The maximum disparity in integer luma samples.
    #[must_use]
    pub const fn max_disparity(self) -> u8 {
        self.max_disparity
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, 1, "vwdi")?;
        Ok(Self {
            max_disparity: body[0],
        })
    }

    /// Parse a whole box, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        if item.four_cc() != VWDI {
            return Err(Error::UnexpectedBox { expected: "vwdi" });
        }
        Self::parse_body(item.body)
    }
}

impl<'a> Parse<'a> for MultiviewSceneInfoBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for MultiviewSceneInfoBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + 1)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let c = write_box_header(buf, &VWDI, need)?;
        buf[c] = self.max_disparity;
        Ok(c + 1)
    }
}

/// The multiview information box (`mvci`) — F.8.3: a version-0 FullBox with
/// no fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MultiviewInformationBox {
    version: u8,
    flags: u32,
}

impl MultiviewInformationBox {
    /// The FullBox version.
    #[must_use]
    pub const fn version(self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(self) -> u32 {
        self.flags
    }

    /// Parse the payload after the box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, crate::box_types::FULLBOX_EXTRA_SIZE, "mvci")?;
        Ok(Self {
            version: body[0],
            flags: broadcast_core::wire::u24_at(body, 1),
        })
    }

    /// Parse a whole box, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        if item.four_cc() != MVCI {
            return Err(Error::UnexpectedBox { expected: "mvci" });
        }
        Self::parse_body(item.body)
    }
}

impl<'a> Parse<'a> for MultiviewInformationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for MultiviewInformationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + crate::box_types::FULLBOX_EXTRA_SIZE)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        crate::box_types::write_full_box_header(buf, &MVCI, need, self.version, self.flags, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn round<T>(value: &T) -> T
    where
        T: Serialize<Error = Error>,
        for<'a> T: Parse<'a, Error = Error>,
    {
        let mut out = vec![0u8; value.serialized_len()];
        value.serialize_into(&mut out).expect("serializes");
        T::parse(&out).expect("parses")
    }

    #[test]
    fn profile_and_level_attributes_round_trip() {
        let value = ProfileLevelAttribute::new(PRFL, 100).expect("builds");
        assert_eq!(round(&value), value);
        let value = ProfileLevelAttribute::new(LEVL, 51).expect("builds");
        assert_eq!(round(&value).indication(), 51);
    }

    #[test]
    fn view_array_orders_round_trip() {
        let value = ViewArrayOrder::new(ILVI, 1, 2).expect("builds");
        let back = round(&value);
        assert_eq!((back.horizontal_order(), back.vertical_order()), (1, 2));
        let value = ViewArrayOrder::new(ELVI, 3, 0).expect("builds");
        assert_eq!(round(&value), value);
    }

    #[test]
    fn rectangular_arrays_and_scene_info_round_trip() {
        let value = RectangularViewArray::new(2, 3);
        assert_eq!(round(&value), value);
        let value = MultiviewSceneInfoBox::new(7);
        assert_eq!(round(&value).max_disparity(), 7);
        let value = MultiviewInformationBox::default();
        assert_eq!(round(&value), value);
    }
}
