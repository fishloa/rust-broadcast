//! Regions and region annotations — ISO/IEC 23008-12:2025 clause 11.
//!
//! [`RegionItem`] types the data of a region item (`rgan`, 11.3.2),
//! [`RegionGeometry`] the geometry structure 11.2.1 shared by region items and
//! region-track samples, [`MaskConfigurationProperty`](crate::image_props::MaskConfigurationProperty)
//! the mask configuration of a mask item (`mski`, 11.2.2),
//! [`RegionTrackConfigBox`] (`rgaC`), [`RegionSampleEntry`] (`rgan`) and
//! [`RegionSample`] the region-track configuration, sample entry and sample
//! format (11.4.2), and
//! [`RegionAnnotationMap`] (`regm`) and
//! [`UserDescriptionSampleGroupEntry`] (`udes`) the two region-track sample
//! groups (11.4.3).
//!
//! A merge region item (Amd 1 11.3.4) adds no syntax: it is a region item
//! carrying `base` item references, which the item model already reports.
//! Deflate-compressed inline masks (mask coding method 1) are carried as
//! opaque bits: the crate never inflates them.
//!
//! Sources: `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025.md`
//! clause 11 and `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025-amd1-2025.md`
//! 11.3.4.

use alloc::vec::Vec;

use broadcast_core::len::{fit_u8, fit_u16, fit_u32, need_in, need_out};
use broadcast_core::wire::{be_u16, be_u32, i16_at, i32_at, u16_at, u32_at};
use broadcast_core::{Parse, Serialize};

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, box_len, write_full_box_header,
};
use crate::error::{Error, Result};
use crate::image::full_flags;
use crate::meta_track::SAMPLE_ENTRY_RESERVED_LEN;

/// `flags & 1`: 32-bit geometry fields (11.2.1.3).
const GEOMETRY_WIDE_FIELDS: u32 = 1;
/// Fields in a coordinate pair.
const GEOMETRY_PAIR_FIELDS: usize = 2;
/// Fields in an `x, y, width, height` group.
const GEOMETRY_QUAD_FIELDS: usize = 4;
/// Fixed prefix fields of a region item body (11.3.2.2).
const REGION_ITEM_PREFIX_LEN: usize = 2;
/// Mask of `annotation_container_type` (11.4.3.1.2).
const ANNOTATION_CONTAINER_MASK: u8 = 0x7;
/// Width of `annotation_container_type` (11.4.3.1.2).
const ANNOTATION_CONTAINER_TYPE_BITS: u32 = 3;
/// Mask of the `reserved` value an association carries (11.4.3.1.2).
const ANNOTATION_RESERVED_VALUE_MASK: u8 = 0x1f;
/// Width of one `regm` association (11.4.3.1.2).
const REGM_ASSOCIATION_LEN: usize = GEOMETRY_TYPE_LEN + 2 * WIDE_FIELD_LEN;
/// Width of a narrow geometry field (11.2.1.3).
const NARROW_FIELD_LEN: usize = core::mem::size_of::<u16>();
/// Width of a wide geometry field (11.2.1.3).
const WIDE_FIELD_LEN: usize = core::mem::size_of::<u32>();
/// Width of `geometry_type` (11.2.1.2).
const GEOMETRY_TYPE_LEN: usize = core::mem::size_of::<u8>();
/// Strings a `udes` entry carries, each NUL-terminated (11.4.3.2.2).
const UDES_CSTRING_COUNT: usize = 4;
/// Bits in one byte.
const BITS_PER_BYTE: u32 = 8;
/// Bits one inline-mask pixel uses (11.2.1.3).
const INLINE_MASK_BITS_PER_PIXEL: u32 = 1;
/// `mask_coding_method` 0: uncompressed (11.2.1.3).
pub const MASK_CODING_NONE: u8 = 0;
/// `mask_coding_method` 1: deflate (11.2.1.3).
pub const MASK_CODING_DEFLATE: u8 = 1;
/// `geometry_type` 0: point (11.2.1.3).
pub const GEOMETRY_POINT: u8 = 0;
/// `geometry_type` 1: rectangle (11.2.1.3).
pub const GEOMETRY_RECTANGLE: u8 = 1;
/// `geometry_type` 2: ellipse (11.2.1.3).
pub const GEOMETRY_ELLIPSE: u8 = 2;
/// `geometry_type` 3: polygon (11.2.1.3).
pub const GEOMETRY_POLYGON: u8 = 3;
/// `geometry_type` 4: referenced mask (11.2.1.3).
pub const GEOMETRY_REFERENCED_MASK: u8 = 4;
/// `geometry_type` 5: inline mask (11.2.1.3).
pub const GEOMETRY_INLINE_MASK: u8 = 5;
/// `geometry_type` 6: polyline (11.2.1.3).
pub const GEOMETRY_POLYLINE: u8 = 6;
/// Region track flag: 32-bit fields (11.4.2.2.1).
pub const RGA_FIELD_LENGTH_SIZE: u32 = 0x000001;
/// Region track flag: region identifiers persist across samples (11.4.2.2.1).
pub const RGA_PERSISTENT_REGION_IDS: u32 = 0x000100;
/// `annotation_container_type` 0: a track or item (11.4.3.1.3).
pub const ANNOTATION_TRACK_OR_ITEM: u8 = 0;
/// `annotation_container_type` 1: a sample group description (11.4.3.1.3).
pub const ANNOTATION_SAMPLE_GROUP: u8 = 1;

/// One region geometry (11.2.1.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RegionGeometry {
    /// A point.
    Point {
        /// `x`.
        x: i32,
        /// `y`.
        y: i32,
    },
    /// A rectangle.
    Rectangle {
        /// `x`.
        x: i32,
        /// `y`.
        y: i32,
        /// `width`.
        width: u32,
        /// `height`.
        height: u32,
    },
    /// An ellipse.
    Ellipse {
        /// `x`.
        x: i32,
        /// `y`.
        y: i32,
        /// `radius_x`.
        radius_x: u32,
        /// `radius_y`.
        radius_y: u32,
    },
    /// A polygon.
    Polygon(Vec<(i32, i32)>),
    /// A mask carried in another entity.
    ReferencedMask {
        /// `x`.
        x: i32,
        /// `y`.
        y: i32,
        /// `width` (0 means the mask entity's own width).
        width: u32,
        /// `height` (0 means the mask entity's own height).
        height: u32,
        /// `mask_ref_idx`, present only in a region-track sample.
        mask_ref_idx: Option<u32>,
    },
    /// A mask carried inline.
    InlineMask {
        /// `x`.
        x: i32,
        /// `y`.
        y: i32,
        /// `width`.
        width: u32,
        /// `height`.
        height: u32,
        /// `mask_coding_method`.
        mask_coding_method: u8,
        /// `mask_coding_parameters`, present when the coding method is not 0.
        mask_coding_parameters: Option<u32>,
        /// The (possibly compressed) mask bits.
        data: Vec<u8>,
    },
    /// A polyline.
    Polyline(Vec<(i32, i32)>),
    /// A geometry type this crate does not model, with the bytes it read.
    Opaque {
        /// `geometry_type`.
        geometry_type: u8,
        /// The bytes after `geometry_type`.
        data: Vec<u8>,
    },
}

impl RegionGeometry {
    /// `geometry_type`.
    #[must_use]
    pub const fn geometry_type(&self) -> u8 {
        match self {
            Self::Point { .. } => GEOMETRY_POINT,
            Self::Rectangle { .. } => GEOMETRY_RECTANGLE,
            Self::Ellipse { .. } => GEOMETRY_ELLIPSE,
            Self::Polygon(_) => GEOMETRY_POLYGON,
            Self::ReferencedMask { .. } => GEOMETRY_REFERENCED_MASK,
            Self::InlineMask { .. } => GEOMETRY_INLINE_MASK,
            Self::Polyline(_) => GEOMETRY_POLYLINE,
            Self::Opaque { geometry_type, .. } => *geometry_type,
        }
    }

    /// Parse one geometry from `body` starting at `cursor`, advancing it.
    pub fn parse(
        body: &[u8],
        cursor: &mut usize,
        wide: bool,
        is_region_track: bool,
    ) -> Result<Self> {
        need_in(body, *cursor + GEOMETRY_TYPE_LEN, "region geometry")?;
        let geometry_type = body[*cursor];
        *cursor += GEOMETRY_TYPE_LEN;
        let width = field_len(wide);
        let geometry = match geometry_type {
            GEOMETRY_POINT => {
                let x = read_signed(body, cursor, width, "region point x")?;
                let y = read_signed(body, cursor, width, "region point y")?;
                Self::Point { x, y }
            }
            GEOMETRY_RECTANGLE => {
                let x = read_signed(body, cursor, width, "region rectangle x")?;
                let y = read_signed(body, cursor, width, "region rectangle y")?;
                let w = read_unsigned(body, cursor, width, "region rectangle width")?;
                let h = read_unsigned(body, cursor, width, "region rectangle height")?;
                Self::Rectangle {
                    x,
                    y,
                    width: w,
                    height: h,
                }
            }
            GEOMETRY_ELLIPSE => {
                let x = read_signed(body, cursor, width, "region ellipse x")?;
                let y = read_signed(body, cursor, width, "region ellipse y")?;
                let radius_x = read_unsigned(body, cursor, width, "region ellipse radius_x")?;
                let radius_y = read_unsigned(body, cursor, width, "region ellipse radius_y")?;
                Self::Ellipse {
                    x,
                    y,
                    radius_x,
                    radius_y,
                }
            }
            GEOMETRY_POLYGON | GEOMETRY_POLYLINE => {
                let count = read_unsigned(body, cursor, width, "region point_count")?;
                let count = usize::try_from(count)
                    .map_err(|_| Error::InvalidInput("region point_count"))?;
                crate::box_support::check_entry_count(
                    body.len() - *cursor,
                    GEOMETRY_PAIR_FIELDS * width,
                    count,
                    "region points",
                )?;
                let mut points = Vec::with_capacity(count);
                for _ in 0..count {
                    let px = read_signed(body, cursor, width, "region point x")?;
                    let py = read_signed(body, cursor, width, "region point y")?;
                    points.push((px, py));
                }
                if geometry_type == GEOMETRY_POLYGON {
                    Self::Polygon(points)
                } else {
                    Self::Polyline(points)
                }
            }
            GEOMETRY_REFERENCED_MASK => {
                let x = read_signed(body, cursor, width, "region mask x")?;
                let y = read_signed(body, cursor, width, "region mask y")?;
                let w = read_unsigned(body, cursor, width, "region mask width")?;
                let h = read_unsigned(body, cursor, width, "region mask height")?;
                let mask_ref_idx = if is_region_track {
                    Some(read_unsigned(body, cursor, width, "region mask_ref_idx")?)
                } else {
                    None
                };
                Self::ReferencedMask {
                    x,
                    y,
                    width: w,
                    height: h,
                    mask_ref_idx,
                }
            }
            GEOMETRY_INLINE_MASK => {
                let x = read_signed(body, cursor, width, "region mask x")?;
                let y = read_signed(body, cursor, width, "region mask y")?;
                let w = read_unsigned(body, cursor, width, "region mask width")?;
                let h = read_unsigned(body, cursor, width, "region mask height")?;
                need_in(body, *cursor + 1, "region inline mask coding method")?;
                let mask_coding_method = body[*cursor];
                *cursor += 1;
                let mask_coding_parameters = if mask_coding_method == MASK_CODING_NONE {
                    None
                } else {
                    let value = be_u32(body, *cursor, "region inline mask parameters")?;
                    *cursor += WIDE_FIELD_LEN;
                    Some(value)
                };
                let data_len = inline_mask_len(w, h, mask_coding_method, mask_coding_parameters)?;
                need_in(body, *cursor + data_len, "region inline mask data")?;
                let data = body[*cursor..*cursor + data_len].to_vec();
                *cursor += data_len;
                Self::InlineMask {
                    x,
                    y,
                    width: w,
                    height: h,
                    mask_coding_method,
                    mask_coding_parameters,
                    data,
                }
            }
            other => {
                let data = body.get(*cursor..).unwrap_or_default().to_vec();
                *cursor = body.len();
                Self::Opaque {
                    geometry_type: other,
                    data,
                }
            }
        };
        Ok(geometry)
    }

    /// Write this geometry, returning the bytes written.
    pub fn serialize_into(&self, buf: &mut [u8], wide: bool) -> Result<usize> {
        let width = field_len(wide);
        let need = self.serialized_len(wide);
        need_out(buf, need)?;
        let mut cursor = 0usize;
        buf[cursor] = self.geometry_type();
        cursor += GEOMETRY_TYPE_LEN;
        match self {
            Self::Point { x, y } => {
                write_signed(buf, &mut cursor, *x, width)?;
                write_signed(buf, &mut cursor, *y, width)?;
            }
            Self::Rectangle {
                x,
                y,
                width: w,
                height: h,
            }
            | Self::ReferencedMask {
                x,
                y,
                width: w,
                height: h,
                ..
            }
            | Self::InlineMask {
                x,
                y,
                width: w,
                height: h,
                ..
            } => {
                write_signed(buf, &mut cursor, *x, width)?;
                write_signed(buf, &mut cursor, *y, width)?;
                write_unsigned(buf, &mut cursor, *w, width)?;
                write_unsigned(buf, &mut cursor, *h, width)?;
                if let Self::ReferencedMask {
                    mask_ref_idx: Some(index),
                    ..
                } = self
                {
                    write_unsigned(buf, &mut cursor, *index, width)?;
                }
                if let Self::InlineMask {
                    mask_coding_method,
                    mask_coding_parameters,
                    data,
                    ..
                } = self
                {
                    need_out(buf, cursor + 1)?;
                    buf[cursor] = *mask_coding_method;
                    cursor += 1;
                    if let Some(parameters) = mask_coding_parameters {
                        need_out(buf, cursor + WIDE_FIELD_LEN)?;
                        buf[cursor..cursor + WIDE_FIELD_LEN]
                            .copy_from_slice(&parameters.to_be_bytes());
                        cursor += WIDE_FIELD_LEN;
                    }
                    need_out(buf, cursor + data.len())?;
                    let end = cursor + data.len();
                    buf[cursor..end].copy_from_slice(data);
                    cursor = end;
                }
            }
            Self::Ellipse {
                x,
                y,
                radius_x,
                radius_y,
            } => {
                write_signed(buf, &mut cursor, *x, width)?;
                write_signed(buf, &mut cursor, *y, width)?;
                write_unsigned(buf, &mut cursor, *radius_x, width)?;
                write_unsigned(buf, &mut cursor, *radius_y, width)?;
            }
            Self::Polygon(points) | Self::Polyline(points) => {
                write_unsigned(
                    buf,
                    &mut cursor,
                    fit_u32(points.len(), "region point_count")?,
                    width,
                )?;
                for (px, py) in points {
                    write_signed(buf, &mut cursor, *px, width)?;
                    write_signed(buf, &mut cursor, *py, width)?;
                }
            }
            Self::Opaque { data, .. } => {
                need_out(buf, cursor + data.len())?;
                let end = cursor + data.len();
                buf[cursor..end].copy_from_slice(data);
                cursor = end;
            }
        }
        Ok(cursor)
    }

    /// The length this geometry writes at `wide`.
    #[must_use]
    pub fn serialized_len(&self, wide: bool) -> usize {
        let width = field_len(wide);
        let fixed = match self {
            Self::Point { .. } => GEOMETRY_PAIR_FIELDS * width,
            Self::Rectangle { .. } => GEOMETRY_PAIR_FIELDS * width + GEOMETRY_PAIR_FIELDS * width,
            Self::Ellipse { .. } => GEOMETRY_PAIR_FIELDS * width + GEOMETRY_PAIR_FIELDS * width,
            Self::ReferencedMask { mask_ref_idx, .. } => {
                GEOMETRY_QUAD_FIELDS * width + if mask_ref_idx.is_some() { width } else { 0 }
            }
            Self::InlineMask {
                mask_coding_parameters,
                data,
                ..
            } => {
                GEOMETRY_QUAD_FIELDS * width
                    + 1
                    + if mask_coding_parameters.is_some() {
                        WIDE_FIELD_LEN
                    } else {
                        0
                    }
                    + data.len()
            }
            Self::Polygon(points) | Self::Polyline(points) => {
                width + points.len() * GEOMETRY_PAIR_FIELDS * width
            }
            Self::Opaque { data, .. } => data.len(),
        };
        GEOMETRY_TYPE_LEN + fixed
    }
}

fn field_len(wide: bool) -> usize {
    if wide {
        WIDE_FIELD_LEN
    } else {
        NARROW_FIELD_LEN
    }
}

fn inline_mask_len(
    width: u32,
    height: u32,
    coding_method: u8,
    parameters: Option<u32>,
) -> Result<usize> {
    match coding_method {
        MASK_CODING_NONE => {
            let bits = u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|value| value.checked_mul(u64::from(INLINE_MASK_BITS_PER_PIXEL)))
                .ok_or(Error::InvalidInput("inline mask size"))?;
            usize::try_from(bits.div_ceil(u64::from(BITS_PER_BYTE)))
                .map_err(|_| Error::InvalidInput("inline mask size"))
        }
        MASK_CODING_DEFLATE => parameters
            .and_then(|value| usize::try_from(value).ok())
            .ok_or(Error::InvalidInput("inline mask parameters")),
        _ => Err(Error::UnsupportedFeature("inline mask coding method")),
    }
}

fn read_signed(body: &[u8], cursor: &mut usize, width: usize, what: &'static str) -> Result<i32> {
    need_in(body, *cursor + width, what)?;
    let value = if width == WIDE_FIELD_LEN {
        i32_at(body, *cursor)
    } else {
        i32::from(i16_at(body, *cursor))
    };
    *cursor += width;
    Ok(value)
}

fn read_unsigned(body: &[u8], cursor: &mut usize, width: usize, what: &'static str) -> Result<u32> {
    need_in(body, *cursor + width, what)?;
    let value = if width == WIDE_FIELD_LEN {
        u32_at(body, *cursor)
    } else {
        u32::from(u16_at(body, *cursor))
    };
    *cursor += width;
    Ok(value)
}

fn write_signed(buf: &mut [u8], cursor: &mut usize, value: i32, width: usize) -> Result<()> {
    need_out(buf, *cursor + width)?;
    if width == WIDE_FIELD_LEN {
        buf[*cursor..*cursor + WIDE_FIELD_LEN].copy_from_slice(&value.to_be_bytes());
    } else {
        let narrowed = i16::try_from(value).map_err(|_| Error::InvalidInput("region geometry"))?;
        buf[*cursor..*cursor + NARROW_FIELD_LEN].copy_from_slice(&narrowed.to_be_bytes());
    }
    *cursor += width;
    Ok(())
}

fn write_unsigned(buf: &mut [u8], cursor: &mut usize, value: u32, width: usize) -> Result<()> {
    need_out(buf, *cursor + width)?;
    if width == WIDE_FIELD_LEN {
        buf[*cursor..*cursor + WIDE_FIELD_LEN].copy_from_slice(&value.to_be_bytes());
    } else {
        let narrowed = u16::try_from(value).map_err(|_| Error::InvalidInput("region geometry"))?;
        buf[*cursor..*cursor + NARROW_FIELD_LEN].copy_from_slice(&narrowed.to_be_bytes());
    }
    *cursor += width;
    Ok(())
}

/// Region Item data (`rgan`) — ISO/IEC 23008-12:2025 11.3.2.2.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RegionItem {
    version: u8,
    flags: u32,
    reference_width: u32,
    reference_height: u32,
    regions: Vec<RegionGeometry>,
}

impl RegionItem {
    /// A region item in a `reference_width` x `reference_height` space.
    #[must_use]
    pub fn new(reference_width: u32, reference_height: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            reference_width,
            reference_height,
            regions: Vec::new(),
        }
    }

    /// Use 32-bit geometry fields.
    #[must_use]
    pub const fn with_wide_fields(mut self) -> Self {
        self.flags |= GEOMETRY_WIDE_FIELDS;
        self
    }

    /// Add a region geometry.
    #[must_use]
    pub fn with_region(mut self, region: RegionGeometry) -> Self {
        self.regions.push(region);
        self
    }

    /// `version`.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `reference_width`.
    #[must_use]
    pub const fn reference_width(&self) -> u32 {
        self.reference_width
    }

    /// `reference_height`.
    #[must_use]
    pub const fn reference_height(&self) -> u32 {
        self.reference_height
    }

    /// The regions, in declaration order.
    #[must_use]
    pub fn regions(&self) -> &[RegionGeometry] {
        &self.regions
    }

    fn wide(&self) -> bool {
        self.flags & GEOMETRY_WIDE_FIELDS != 0
    }
}

impl<'a> Parse<'a> for RegionItem {
    type Error = Error;

    fn parse(body: &'a [u8]) -> Result<Self> {
        need_in(body, REGION_ITEM_PREFIX_LEN, "region item")?;
        let version = body[0];
        let flags = u32::from(body[1]);
        let wide = flags & GEOMETRY_WIDE_FIELDS != 0;
        let width = field_len(wide);
        let mut cursor = REGION_ITEM_PREFIX_LEN;
        let reference_width = read_unsigned(body, &mut cursor, width, "region reference_width")?;
        let reference_height = read_unsigned(body, &mut cursor, width, "region reference_height")?;
        need_in(body, cursor + 1, "region_count")?;
        let region_count = usize::from(body[cursor]);
        cursor += 1;
        let mut regions = Vec::with_capacity(region_count);
        for _ in 0..region_count {
            regions.push(RegionGeometry::parse(body, &mut cursor, wide, false)?);
        }
        if cursor != body.len() {
            return Err(Error::InvalidInput("region item trailing bytes"));
        }
        Ok(Self {
            version,
            flags,
            reference_width,
            reference_height,
            regions,
        })
    }
}

impl Serialize for RegionItem {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let width = field_len(self.wide());
        REGION_ITEM_PREFIX_LEN
            + GEOMETRY_PAIR_FIELDS * width
            + 1
            + self
                .regions
                .iter()
                .map(|region| region.serialized_len(self.wide()))
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let wide = self.wide();
        let width = field_len(wide);
        let need = self.serialized_len();
        need_out(buf, need)?;
        buf[0] = self.version;
        buf[1] = fit_u8(
            usize::try_from(self.flags).map_err(|_| Error::InvalidInput("region item flags"))?,
            "region item flags",
        )?;
        let mut cursor = REGION_ITEM_PREFIX_LEN;
        write_unsigned(buf, &mut cursor, self.reference_width, width)?;
        write_unsigned(buf, &mut cursor, self.reference_height, width)?;
        buf[cursor] = fit_u8(self.regions.len(), "region_count")?;
        cursor += 1;
        for region in &self.regions {
            cursor += region.serialize_into(&mut buf[cursor..], wide)?;
        }
        Ok(cursor)
    }
}

/// Region Track Configuration box (`rgaC`) — ISO/IEC 23008-12:2025 11.4.2.2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RegionTrackConfigBox {
    version: u8,
    flags: u32,
    reference_width: u32,
    reference_height: u32,
}

impl RegionTrackConfigBox {
    /// A configuration in a `reference_width` x `reference_height` space.
    #[must_use]
    pub const fn new(reference_width: u32, reference_height: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            reference_width,
            reference_height,
        }
    }

    /// Use 32-bit fields (`field_length_size`).
    #[must_use]
    pub const fn with_large_fields(mut self) -> Self {
        self.flags |= RGA_FIELD_LENGTH_SIZE;
        self
    }

    /// Region identifiers persist across samples (`persistent_region_ids`).
    #[must_use]
    pub const fn with_persistent_region_ids(mut self) -> Self {
        self.flags |= RGA_PERSISTENT_REGION_IDS;
        self
    }

    /// `version`.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `reference_width`.
    #[must_use]
    pub const fn reference_width(&self) -> u32 {
        self.reference_width
    }

    /// `reference_height`.
    #[must_use]
    pub const fn reference_height(&self) -> u32 {
        self.reference_height
    }

    /// Whether region identifiers persist across samples.
    #[must_use]
    pub const fn persistent_region_ids(&self) -> bool {
        self.flags & RGA_PERSISTENT_REGION_IDS != 0
    }

    fn wide(&self) -> bool {
        self.flags & RGA_FIELD_LENGTH_SIZE != 0
    }
}

impl<'a> Parse<'a> for RegionTrackConfigBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        crate::image::expect_type(bytes, &RGA_CONFIG, "rgaC")?;
        need_in(bytes, BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE, "rgaC")?;
        let body = crate::box_types::box_body(bytes, FULLBOX_EXTRA_SIZE, "rgaC")?;
        let version = body[0];
        let flags = full_flags(body);
        let wide = flags & RGA_FIELD_LENGTH_SIZE != 0;
        let width = field_len(wide);
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let reference_width = read_unsigned(body, &mut cursor, width, "rgaC reference_width")?;
        let reference_height = read_unsigned(body, &mut cursor, width, "rgaC reference_height")?;
        if cursor != body.len() {
            return Err(Error::InvalidInput("rgaC trailing bytes"));
        }
        Ok(Self {
            version,
            flags,
            reference_width,
            reference_height,
        })
    }
}

impl Serialize for RegionTrackConfigBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + GEOMETRY_PAIR_FIELDS * field_len(self.wide()),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let width = field_len(self.wide());
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor =
            write_full_box_header(buf, &RGA_CONFIG, need, self.version, self.flags, false)?;
        write_unsigned(buf, &mut cursor, self.reference_width, width)?;
        write_unsigned(buf, &mut cursor, self.reference_height, width)?;
        Ok(cursor)
    }
}

/// Four-CC of the Region Track Configuration box (11.4.2.2.1).
pub const RGA_CONFIG: [u8; BOX_TYPE_SIZE] = *b"rgaC";

/// Four-CC of the region-track sample entry (11.4.2.2.2).
pub const RGAN: [u8; BOX_TYPE_SIZE] = *b"rgan";

/// The region-track sample entry (`rgan`) — ISO/IEC 23008-12:2025 11.4.2.2.2.
///
/// A `MetadataSampleEntry` whose mandatory `rgaC` child states the reference
/// space its samples' regions are placed in. The SampleEntry's six reserved
/// bytes come back verbatim (ISO/IEC 14496-12:2026 §8.5.2.2), every sibling
/// child keeps its wire position and header form, and a child region a walk
/// cannot represent is kept as a verbatim tail.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RegionSampleEntry {
    reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    data_reference_index: u16,
    config: RegionTrackConfigBox,
    other_boxes: Vec<OpaqueBox>,
    config_position: usize,
    trailing: Vec<u8>,
}

impl RegionSampleEntry {
    /// An entry carrying `config`.
    #[must_use]
    pub fn new(config: RegionTrackConfigBox) -> Self {
        Self {
            reserved: [0u8; SAMPLE_ENTRY_RESERVED_LEN],
            data_reference_index: 1,
            config,
            other_boxes: Vec::new(),
            config_position: 0,
            trailing: Vec::new(),
        }
    }

    /// The entry's `data_reference_index` (ISO/IEC 14496-12:2026 §8.5.2.2).
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// The mandatory `rgaC` configuration.
    #[must_use]
    pub const fn config(&self) -> &RegionTrackConfigBox {
        &self.config
    }

    /// Child boxes beside the `rgaC` configuration, in wire order.
    #[must_use]
    pub fn other_boxes(&self) -> &[OpaqueBox] {
        &self.other_boxes
    }

    /// Bytes after the last complete child, kept verbatim.
    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }

    /// Parse the complete box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let (reserved, data_reference_index, rest) =
            crate::meta_track::parse_sample_entry_base(bytes, "rgan")?;
        let (children, trailing) = crate::box_support::capture_children(rest).into_parts();
        let mut config = None;
        let mut other_boxes = Vec::new();
        let mut config_position = 0;
        for child in children {
            if config.is_none() && child.box_type == RGA_CONFIG {
                config = Some(RegionTrackConfigBox::parse(&child.try_to_bytes()?)?);
                continue;
            }
            if config.is_none() {
                config_position += 1;
            }
            other_boxes.push(child);
        }
        Ok(Self {
            reserved,
            data_reference_index,
            config: config.ok_or(Error::InvalidInput("rgan missing rgaC"))?,
            other_boxes,
            config_position,
            trailing,
        })
    }
}

impl<'a> Parse<'a> for RegionSampleEntry {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        crate::image::expect_type(bytes, &RGAN, "rgan")?;
        Self::parse_box(bytes)
    }
}

impl Serialize for RegionSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(
            BOX_HEADER_MIN_SIZE
                + crate::meta_track::SAMPLE_ENTRY_FIXED_LEN
                + self.config.serialized_len()
                + self
                    .other_boxes
                    .iter()
                    .map(Serialize::serialized_len)
                    .sum::<usize>()
                + self.trailing.len(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        let mut cursor = crate::meta_track::write_sample_entry_base(
            buf,
            &RGAN,
            need,
            &self.reserved,
            self.data_reference_index,
        )?;
        let mut others = self.other_boxes.iter();
        for slot in 0..=self.other_boxes.len() {
            if slot == self.config_position {
                cursor += self.config.serialize_into(&mut buf[cursor..])?;
            } else if let Some(child) = others.next() {
                cursor += child.serialize_into(&mut buf[cursor..])?;
            }
        }
        buf[cursor..cursor + self.trailing.len()].copy_from_slice(&self.trailing);
        cursor += self.trailing.len();
        Ok(cursor)
    }
}

/// A region-track sample (11.4.2.3.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RegionSample {
    regions: Vec<(u32, RegionGeometry)>,
}

impl RegionSample {
    /// A sample carrying `regions`, one identifier and geometry each.
    #[must_use]
    pub fn new(regions: Vec<(u32, RegionGeometry)>) -> Self {
        Self { regions }
    }

    /// The regions.
    #[must_use]
    pub fn regions(&self) -> &[(u32, RegionGeometry)] {
        &self.regions
    }

    /// Parse a sample using its sample entry's configuration.
    pub fn parse_with_config(sample: &[u8], config: &RegionTrackConfigBox) -> Result<Self> {
        let wide = config.wide();
        let mut cursor = 0usize;
        let region_count = be_u32(sample, cursor, "region sample count")?;
        cursor += WIDE_FIELD_LEN;
        let count =
            usize::try_from(region_count).map_err(|_| Error::InvalidInput("region sample"))?;
        crate::box_support::check_entry_count(
            sample.len() - cursor,
            WIDE_FIELD_LEN + GEOMETRY_TYPE_LEN,
            count,
            "region sample",
        )?;
        let mut regions = Vec::with_capacity(count);
        for _ in 0..count {
            let identifier = be_u32(sample, cursor, "region_identifier")?;
            cursor += WIDE_FIELD_LEN;
            let geometry = RegionGeometry::parse(sample, &mut cursor, wide, true)?;
            regions.push((identifier, geometry));
        }
        if cursor != sample.len() {
            return Err(Error::InvalidInput("region sample trailing bytes"));
        }
        Ok(Self { regions })
    }

    /// Write a sample using its sample entry's configuration.
    pub fn serialize_with_config(&self, config: &RegionTrackConfigBox) -> Result<Vec<u8>> {
        let wide = config.wide();
        let mut out = alloc::vec![0u8; 0];
        out.extend_from_slice(&fit_u32(self.regions.len(), "region sample count")?.to_be_bytes());
        for (identifier, geometry) in &self.regions {
            out.extend_from_slice(&identifier.to_be_bytes());
            let start = out.len();
            out.resize(start + geometry.serialized_len(wide), 0);
            geometry.serialize_into(&mut out[start..], wide)?;
        }
        Ok(out)
    }
}

/// One annotation association of a [`RegionAnnotationMap`] (11.4.3.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RegionAnnotationAssociation {
    reserved: u8,
    container_type: AnnotationContainerType,
    reference_type: [u8; BOX_TYPE_SIZE],
    index: u32,
}

impl RegionAnnotationAssociation {
    /// An association of `container_type` naming `reference_type` and `index`.
    #[must_use]
    pub const fn new(
        container_type: AnnotationContainerType,
        reference_type: [u8; BOX_TYPE_SIZE],
        index: u32,
    ) -> Self {
        Self {
            reserved: 0,
            container_type,
            reference_type,
            index,
        }
    }

    /// The `reserved` run preceding `annotation_container_type`, kept verbatim.
    #[must_use]
    pub const fn reserved(&self) -> u8 {
        self.reserved
    }

    /// `annotation_container_type`.
    #[must_use]
    pub const fn container_type(&self) -> AnnotationContainerType {
        self.container_type
    }

    /// `annotation_reference_type`.
    #[must_use]
    pub const fn reference_type(&self) -> [u8; BOX_TYPE_SIZE] {
        self.reference_type
    }

    /// `annotation_idx`.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }
}

/// The `annotation_container_type` of a region association (11.4.3.1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AnnotationContainerType {
    /// `0`: a track or item named by a track reference.
    #[default]
    TrackOrItem,
    /// `1`: a sample group description entry.
    SampleGroup,
    /// A reserved value.
    Reserved(u8),
}

impl AnnotationContainerType {
    /// The wire value.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::TrackOrItem => ANNOTATION_TRACK_OR_ITEM,
            Self::SampleGroup => ANNOTATION_SAMPLE_GROUP,
            Self::Reserved(value) => value,
        }
    }

    /// The type a wire value names.
    #[must_use]
    pub const fn from_value(value: u8) -> Self {
        match value {
            ANNOTATION_TRACK_OR_ITEM => Self::TrackOrItem,
            ANNOTATION_SAMPLE_GROUP => Self::SampleGroup,
            other => Self::Reserved(other),
        }
    }

    /// The spec token.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TrackOrItem => "track-or-item",
            Self::SampleGroup => "sample-group",
            Self::Reserved(_) => "reserved",
        }
    }
}

broadcast_core::impl_spec_display!(AnnotationContainerType, Reserved);

/// The sample-to-region-id mapping (`regm`, 11.4.3.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RegionAnnotationMap {
    entries: Vec<RegionAnnotation>,
}

impl RegionAnnotationMap {
    /// A mapping with these region entries.
    #[must_use]
    pub fn new(entries: Vec<RegionAnnotation>) -> Self {
        Self { entries }
    }

    /// The region entries.
    #[must_use]
    pub fn entries(&self) -> &[RegionAnnotation] {
        &self.entries
    }

    /// Parse a `regm` sample group entry body.
    pub fn parse(body: &[u8]) -> Result<Self> {
        let mut cursor = 0usize;
        let entry_count = be_u32(body, cursor, "regm entry_count")?;
        cursor += WIDE_FIELD_LEN;
        let count = usize::try_from(entry_count).map_err(|_| Error::InvalidInput("regm"))?;
        crate::box_support::check_entry_count(
            body.len() - cursor,
            WIDE_FIELD_LEN + NARROW_FIELD_LEN,
            count,
            "regm entries",
        )?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let region_id = be_u32(body, cursor, "regm region_id")?;
            cursor += WIDE_FIELD_LEN;
            let association_count = be_u16(body, cursor, "regm association_count")?;
            cursor += NARROW_FIELD_LEN;
            let associations = usize::from(association_count);
            crate::box_support::check_entry_count(
                body.len() - cursor,
                REGM_ASSOCIATION_LEN,
                associations,
                "regm associations",
            )?;
            let mut mapped = Vec::with_capacity(associations);
            for _ in 0..associations {
                need_in(body, cursor + REGM_ASSOCIATION_LEN, "regm association")?;
                let packed = body[cursor];
                let container_type =
                    AnnotationContainerType::from_value(packed & ANNOTATION_CONTAINER_MASK);
                let reserved = packed >> ANNOTATION_CONTAINER_TYPE_BITS;
                cursor += 1;
                let mut reference_type = [0u8; BOX_TYPE_SIZE];
                reference_type.copy_from_slice(&body[cursor..cursor + BOX_TYPE_SIZE]);
                cursor += BOX_TYPE_SIZE;
                let index = be_u32(body, cursor, "regm annotation_idx")?;
                cursor += WIDE_FIELD_LEN;
                mapped.push(RegionAnnotationAssociation {
                    reserved,
                    container_type,
                    reference_type,
                    index,
                });
            }
            entries.push(RegionAnnotation {
                region_id,
                associations: mapped,
            });
        }
        if cursor != body.len() {
            return Err(Error::InvalidInput("regm trailing bytes"));
        }
        Ok(Self { entries })
    }

    /// Bytes the entry body occupies.
    #[must_use]
    pub fn body_len(&self) -> usize {
        self.entries.iter().fold(WIDE_FIELD_LEN, |total, entry| {
            total
                + WIDE_FIELD_LEN
                + NARROW_FIELD_LEN
                + entry.associations.len() * REGM_ASSOCIATION_LEN
        })
    }

    /// Write the `regm` sample group entry body.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        out.extend_from_slice(&fit_u32(self.entries.len(), "regm entry_count")?.to_be_bytes());
        for entry in &self.entries {
            out.extend_from_slice(&entry.region_id.to_be_bytes());
            out.extend_from_slice(
                &fit_u16(entry.associations.len(), "regm association_count")?.to_be_bytes(),
            );
            for association in &entry.associations {
                let reserved = broadcast_core::len::fit_mask(
                    association.reserved,
                    ANNOTATION_RESERVED_VALUE_MASK,
                    "regm reserved",
                )? << ANNOTATION_CONTAINER_TYPE_BITS;
                out.push(
                    reserved | (association.container_type.value() & ANNOTATION_CONTAINER_MASK),
                );
                out.extend_from_slice(&association.reference_type);
                out.extend_from_slice(&association.index.to_be_bytes());
            }
        }
        Ok(out)
    }
}

/// One region of a [`RegionAnnotationMap`] (11.4.3.1.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RegionAnnotation {
    region_id: u32,
    associations: Vec<RegionAnnotationAssociation>,
}

impl RegionAnnotation {
    /// The region identifier and its annotations.
    #[must_use]
    pub fn new(region_id: u32, associations: Vec<RegionAnnotationAssociation>) -> Self {
        Self {
            region_id,
            associations,
        }
    }

    /// `region_id`.
    #[must_use]
    pub const fn region_id(&self) -> u32 {
        self.region_id
    }

    /// The associations.
    #[must_use]
    pub fn associations(&self) -> &[RegionAnnotationAssociation] {
        &self.associations
    }
}

/// The user description sample group entry (`udes`, 11.4.3.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct UserDescriptionSampleGroupEntry {
    lang: Vec<u8>,
    name: Vec<u8>,
    description: Vec<u8>,
    tags: Vec<u8>,
}

impl UserDescriptionSampleGroupEntry {
    /// A description in `lang`.
    #[must_use]
    pub fn new(lang: &[u8], name: &[u8], description: &[u8], tags: &[u8]) -> Self {
        Self {
            lang: lang.to_vec(),
            name: name.to_vec(),
            description: description.to_vec(),
            tags: tags.to_vec(),
        }
    }

    /// `lang`.
    #[must_use]
    pub fn lang(&self) -> &[u8] {
        &self.lang
    }

    /// `name`.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    /// `description`.
    #[must_use]
    pub fn description(&self) -> &[u8] {
        &self.description
    }

    /// `tags`.
    #[must_use]
    pub fn tags(&self) -> &[u8] {
        &self.tags
    }

    /// Parse the sample group entry body.
    pub fn parse(body: &[u8]) -> Result<Self> {
        let strings = crate::image_props::split_cstrings(body, 4, "udes sample group")?;
        let mut strings = strings.into_iter();
        Ok(Self {
            lang: strings.next().unwrap_or_default(),
            name: strings.next().unwrap_or_default(),
            description: strings.next().unwrap_or_default(),
            tags: strings.next().unwrap_or_default(),
        })
    }

    /// Bytes the entry body occupies.
    #[must_use]
    pub fn body_len(&self) -> usize {
        self.lang.len()
            + self.name.len()
            + self.description.len()
            + self.tags.len()
            + UDES_CSTRING_COUNT
    }

    /// Write the sample group entry body.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let strings = alloc::vec![
            self.lang.clone(),
            self.name.clone(),
            self.description.clone(),
            self.tags.clone(),
        ];
        let mut out = alloc::vec![0u8; crate::image_props::cstrings_len(&strings)];
        crate::image_props::write_cstrings(&mut out, &strings)?;
        Ok(out)
    }
}

impl RegionItem {
    /// The body bytes, for a writer that adds a region item whole.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = alloc::vec![0u8; self.serialized_len()];
        let written = self.serialize_into(&mut out)?;
        out.truncate(written);
        Ok(out)
    }
}
