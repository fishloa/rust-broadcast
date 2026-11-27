//! Typed bodies of the entity groups ISO/IEC 23008-12:2025 and Amendment 1
//! define beyond the bare entity list: the image pyramid (`pymd`, Amd 1
//! 6.8.12) and the region partition group (`rgpa`, Amd 1 6.8.13).
//!
//! A group whose only body is its entity list (`altr`, `ster`, `stem`, the
//! bracketing groups, `pano`, `slid`, `prgr`, `brst`, `tsyn`, `iaug`, `albc`,
//! `favc`) is carried by [`crate::item::EntityToGroupBox`] with no trailing
//! bytes. A grouping type this crate does not type keeps its trailing bytes
//! opaque, so an unknown or private group round-trips unchanged.
//!
//! Sources: `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025.md`
//! clause 6.8 and `private/specs-verified/mpeg/heif/iso-iec-23008-12-2025-amd1-2025.md`
//! 6.8.11-6.8.13.

use alloc::vec::Vec;

use broadcast_core::len::need_out;
use broadcast_core::wire::{be_u16, be_u32};

use crate::error::{Error, Result};
use crate::item::EntityGroupType;

/// `pymd` flag: tile information present (Amd 1 6.8.12.1).
pub const PYMD_TILE_INFO_PRESENT: u32 = 0x000001;
/// `pymd` flag: one tile size for every layer (Amd 1 6.8.12.1).
pub const PYMD_TILE_INFO_CONSTANT: u32 = 0x000002;
/// `rgpa` flag: 32-bit area fields (Amd 1 6.8.13.1).
pub const RGPA_LARGE_FIELD_LENGTH: u32 = 0x001000;
/// `rgpa` flag: the area rectangle is present (Amd 1 6.8.13.1).
pub const RGPA_AREA_INFO_PRESENT: u32 = 0x002000;

/// Width of one `pymd` tile dimension (Amd 1 6.8.12.2).
const PYMD_TILE_FIELD_LEN: usize = core::mem::size_of::<u32>();
/// Width of one `(tile_size_x, tile_size_y)` pair.
const PYMD_TILE_ENTRY_LEN: usize = 2 * PYMD_TILE_FIELD_LEN;
/// Fields of an `rgpa` area rectangle (Amd 1 6.8.13.2).
const RGPA_AREA_FIELD_COUNT: usize = 6;

/// Image pyramid entity group body (`pymd`) — ISO/IEC 23008-12:2025 Amd 1
/// 6.8.12.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ImagePyramidEntityGroup {
    flags: u32,
    tile_sizes: Vec<(u32, u32)>,
}

impl ImagePyramidEntityGroup {
    /// A body with no tile information.
    #[must_use]
    pub const fn new(flags: u32) -> Self {
        Self {
            flags,
            tile_sizes: Vec::new(),
        }
    }

    /// A body carrying one `(tile_size_x, tile_size_y)` per layer (or one for
    /// every layer when [`PYMD_TILE_INFO_CONSTANT`] is set).
    #[must_use]
    pub fn with_tile_sizes(mut self, tile_sizes: Vec<(u32, u32)>) -> Self {
        self.tile_sizes = tile_sizes;
        self
    }

    /// The box flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The signalled tile sizes.
    #[must_use]
    pub fn tile_sizes(&self) -> &[(u32, u32)] {
        &self.tile_sizes
    }
}

/// A region partition group area rectangle (Amd 1 6.8.13.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RegionPartitionArea {
    reference_width: u32,
    reference_height: u32,
    top: u32,
    left: u32,
    width: u32,
    height: u32,
}

impl RegionPartitionArea {
    /// An area rectangle in a reference space.
    #[must_use]
    pub const fn new(
        reference_width: u32,
        reference_height: u32,
        top: u32,
        left: u32,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            reference_width,
            reference_height,
            top,
            left,
            width,
            height,
        }
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

    /// `top`.
    #[must_use]
    pub const fn top(&self) -> u32 {
        self.top
    }

    /// `left`.
    #[must_use]
    pub const fn left(&self) -> u32 {
        self.left
    }

    /// `width`.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// `height`.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
}

/// Region partition group body (`rgpa`) — ISO/IEC 23008-12:2025 Amd 1
/// 6.8.13.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RegionPartitionGroup {
    flags: u32,
    area: Option<RegionPartitionArea>,
}

impl RegionPartitionGroup {
    /// A body whose covered area is the associated image's whole area.
    #[must_use]
    pub const fn whole_image() -> Self {
        Self {
            flags: 0,
            area: None,
        }
    }

    /// A body with an explicit area rectangle.
    #[must_use]
    pub const fn with_area(area: RegionPartitionArea) -> Self {
        Self {
            flags: RGPA_AREA_INFO_PRESENT,
            area: Some(area),
        }
    }

    /// Signal 32-bit area fields (only meaningful with an area).
    #[must_use]
    pub const fn with_large_fields(mut self) -> Self {
        self.flags |= RGPA_LARGE_FIELD_LENGTH;
        self
    }

    /// The box flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The covered area rectangle, when signalled.
    #[must_use]
    pub const fn area(&self) -> Option<&RegionPartitionArea> {
        self.area.as_ref()
    }
}

/// The typed trailing body of an image entity group.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ImageGroupBody {
    /// An image pyramid (`pymd`).
    ImagePyramid(ImagePyramidEntityGroup),
    /// A region partition group (`rgpa`).
    RegionPartition(RegionPartitionGroup),
    /// A grouping type whose only body is its entity list (no trailing
    /// bytes), as clause 6.8 defines for `altr`, `ster`, `stem`, the
    /// bracketing groups, `slid`, `prgr` and the rest.
    EntitiesOnly,
    /// A grouping type this crate does not type, or a known type with
    /// trailing bytes it does not model.
    Opaque(Vec<u8>),
}

impl ImageGroupBody {
    /// Parse the trailing bytes of a group named by `grouping_type` and
    /// carrying `entity_count` entity identifiers.
    pub fn parse(
        grouping_type: EntityGroupType,
        flags: u32,
        trailing: &[u8],
        entity_count: usize,
    ) -> Result<Self> {
        match grouping_type {
            EntityGroupType::Pymd => {
                let mut tile_sizes = Vec::new();
                if flags & PYMD_TILE_INFO_PRESENT != 0 {
                    let layers = if flags & PYMD_TILE_INFO_CONSTANT != 0 {
                        1
                    } else {
                        entity_count
                    };
                    crate::box_support::check_entry_count(
                        trailing.len(),
                        PYMD_TILE_ENTRY_LEN,
                        layers,
                        "pymd tile sizes",
                    )?;
                    for index in 0..layers {
                        let at = index * PYMD_TILE_ENTRY_LEN;
                        tile_sizes.push((
                            be_u32(trailing, at, "pymd tile_size_x")?,
                            be_u32(trailing, at + PYMD_TILE_FIELD_LEN, "pymd tile_size_y")?,
                        ));
                    }
                    if trailing.len() != layers * PYMD_TILE_ENTRY_LEN {
                        return Err(Error::InvalidInput("pymd trailing bytes"));
                    }
                } else if !trailing.is_empty() {
                    return Err(Error::InvalidInput("pymd trailing bytes"));
                }
                Ok(Self::ImagePyramid(
                    ImagePyramidEntityGroup::new(flags).with_tile_sizes(tile_sizes),
                ))
            }
            EntityGroupType::Rgpa => {
                let area = if flags & RGPA_AREA_INFO_PRESENT != 0 {
                    let width = area_field_len(flags);
                    crate::box_support::check_entry_count(
                        trailing.len(),
                        width,
                        RGPA_AREA_FIELD_COUNT,
                        "rgpa area",
                    )?;
                    let mut values = [0u32; RGPA_AREA_FIELD_COUNT];
                    for (index, value) in values.iter_mut().enumerate() {
                        *value = read_field(trailing, index * width, width)?;
                    }
                    if trailing.len() != RGPA_AREA_FIELD_COUNT * width {
                        return Err(Error::InvalidInput("rgpa trailing bytes"));
                    }
                    let [reference_width, reference_height, top, left, width, height] = values;
                    Some(RegionPartitionArea::new(
                        reference_width,
                        reference_height,
                        top,
                        left,
                        width,
                        height,
                    ))
                } else {
                    if !trailing.is_empty() {
                        return Err(Error::InvalidInput("rgpa trailing bytes"));
                    }
                    None
                };
                let group = match area {
                    Some(area) => RegionPartitionGroup::with_area(area),
                    None => RegionPartitionGroup::whole_image(),
                }
                .with_flags(flags);
                Ok(Self::RegionPartition(group))
            }
            _ => {
                if is_entities_only(grouping_type) && trailing.is_empty() {
                    Ok(Self::EntitiesOnly)
                } else {
                    Ok(Self::Opaque(trailing.to_vec()))
                }
            }
        }
    }

    /// The trailing length this body writes.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        match self {
            Self::ImagePyramid(value) => value.tile_sizes.len() * PYMD_TILE_ENTRY_LEN,
            Self::RegionPartition(value) => {
                if value.area.is_some() {
                    RGPA_AREA_FIELD_COUNT * area_field_len(value.flags)
                } else {
                    0
                }
            }
            Self::EntitiesOnly => 0,
            Self::Opaque(value) => value.len(),
        }
    }

    /// Write the trailing bytes.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::ImagePyramid(value) => {
                let need = value.tile_sizes.len() * PYMD_TILE_ENTRY_LEN;
                need_out(buf, need)?;
                let mut cursor = 0usize;
                for (x, y) in &value.tile_sizes {
                    buf[cursor..cursor + PYMD_TILE_FIELD_LEN].copy_from_slice(&x.to_be_bytes());
                    cursor += PYMD_TILE_FIELD_LEN;
                    buf[cursor..cursor + PYMD_TILE_FIELD_LEN].copy_from_slice(&y.to_be_bytes());
                    cursor += PYMD_TILE_FIELD_LEN;
                }
                Ok(cursor)
            }
            Self::RegionPartition(value) => {
                let Some(area) = value.area else {
                    return Ok(0);
                };
                let width = area_field_len(value.flags);
                let need = RGPA_AREA_FIELD_COUNT * width;
                need_out(buf, need)?;
                for (index, field) in [
                    area.reference_width,
                    area.reference_height,
                    area.top,
                    area.left,
                    area.width,
                    area.height,
                ]
                .iter()
                .enumerate()
                {
                    write_field(buf, index * width, *field, width)?;
                }
                Ok(need)
            }
            Self::EntitiesOnly => Ok(0),
            Self::Opaque(value) => {
                need_out(buf, value.len())?;
                buf[..value.len()].copy_from_slice(value);
                Ok(value.len())
            }
        }
    }
}

impl RegionPartitionGroup {
    const fn with_flags(mut self, flags: u32) -> Self {
        self.flags = flags;
        self
    }
}

fn area_field_len(flags: u32) -> usize {
    if flags & RGPA_LARGE_FIELD_LENGTH != 0 {
        core::mem::size_of::<u32>()
    } else {
        core::mem::size_of::<u16>()
    }
}

fn read_field(body: &[u8], offset: usize, width: usize) -> Result<u32> {
    if width == core::mem::size_of::<u32>() {
        Ok(be_u32(body, offset, "rgpa area")?)
    } else {
        Ok(u32::from(be_u16(body, offset, "rgpa area")?))
    }
}

fn write_field(buf: &mut [u8], offset: usize, value: u32, width: usize) -> Result<()> {
    if width == core::mem::size_of::<u32>() {
        need_out(buf, offset + width)?;
        buf[offset..offset + width].copy_from_slice(&value.to_be_bytes());
    } else {
        let narrowed = u16::try_from(value).map_err(|_| Error::InvalidInput("rgpa area"))?;
        need_out(buf, offset + width)?;
        buf[offset..offset + width].copy_from_slice(&narrowed.to_be_bytes());
    }
    Ok(())
}

/// Whether clause 6.8 gives this grouping type no body beyond its entity list.
fn is_entities_only(grouping_type: EntityGroupType) -> bool {
    matches!(
        grouping_type,
        EntityGroupType::Altr
            | EntityGroupType::Prsl
            | EntityGroupType::Ster
            | EntityGroupType::Stem
            | EntityGroupType::Eqiv
            | EntityGroupType::Brst
            | EntityGroupType::Tsyn
            | EntityGroupType::Iaug
            | EntityGroupType::Aebr
            | EntityGroupType::Wbbr
            | EntityGroupType::Fobr
            | EntityGroupType::Afbr
            | EntityGroupType::Dobr
            | EntityGroupType::Albc
            | EntityGroupType::Favc
            | EntityGroupType::Pano
            | EntityGroupType::Slid
            | EntityGroupType::Prgr
    )
}
