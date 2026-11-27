//! The sans-IO still-image reader: it asks a caller for the top-level box
//! headers, the file-level `meta` box, and the tiny bodies of derived image
//! items, then yields the item model. Coded item data never passes through it:
//! an item's data is an ordered list of [`ByteRange`]s the caller fetches
//! itself.
//!
//! The model is [`StillImage`]: the brands, the primary item, every item (id,
//! type, name, hidden flag, protection, properties in association order with
//! the essential flag, references in both directions, resolved data ranges),
//! and the entity groups. Convenience on top is derived only from those facts:
//! [`StillImage::image_size`], [`StillImage::rotation`],
//! [`StillImage::mirror`], [`StillImage::thumbnails_of`],
//! [`StillImage::auxiliary_images_of`], [`StillImage::grid_tiles`],
//! [`StillImage::overlay_layers`], [`StillImage::exif_items`] and
//! [`StillImage::xmp_items`].
//!
//! [`StillImage::reemit_plan`] re-emits a parsed file through the preserved
//! structure: every top-level box keeps its exact header bytes, the `meta`
//! body is rebuilt from its typed children in wire order, and a box this crate
//! did not need to read is a [`ReemitOp::Copy`] of the caller's own bytes.
//!
//! Hostile input: every count that drives an allocation or a loop is checked
//! against [`Limits`], a reference cycle is [`Error::ReferenceCycle`], an
//! extent past the end of the file is [`Error::ExtentOutOfRange`], and an
//! `ipma` index past its container is [`Error::PropertyIndexOutOfRange`].
//!
//! Sources: `private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`
//! §8.11 and §8.15; `private/specs-verified/mpeg/heif/iso-iec-23008-12-2017.md`
//! 6.3-6.7; `private/specs/aom_av1_avif_image_file_format.html` (AVIF v1.2.0).

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use broadcast_core::wire::u32_at;
use broadcast_core::{Parse, Serialize};

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, LARGESIZE_SIZE,
    SIZE_INDICATES_LARGESIZE, SIZE_TO_EOF, UUID, UUID_TYPE_SIZE, parse_box,
};
use crate::error::{Error, Result};
use crate::image::{DerivedImage, ImageMirror, ImageRotation, ItemProperty, is_still_image_brand};
use crate::item::{
    GroupsListBox, ItemDataBox, ItemId, ItemInfoBox, ItemInfoEntry, ItemLocationBox,
    ItemPropertiesBox, ItemReferenceBox, ItemType, PrimaryItemBox, PropertyIndex, ReferenceType,
};
use crate::limits::Limits;
use crate::metadata::Hdlr;
use crate::reader::ByteRange;
use crate::segments::FileTypeBox;

/// Bytes of the `size` + `type` box header a read starts with.
const TOP_HEADER_LEN: usize = BOX_HEADER_MIN_SIZE;
/// Offset of the `size` field (ISO/IEC 14496-12:2026 §4.2).
const SIZE_OFFSET: usize = 0;
/// Offset of the four-CC `type` field (§4.2).
const TYPE_OFFSET: usize = BOX_TYPE_SIZE;
/// Offset of the `largesize` field when `size == 1` (§4.2).
const LARGESIZE_OFFSET: usize = BOX_HEADER_MIN_SIZE;
/// Four-CC of the File Type Box (§5.2).
pub const FTYP: [u8; BOX_TYPE_SIZE] = *b"ftyp";
/// Four-CC of the Meta Box (§8.11.1).
pub const META: [u8; BOX_TYPE_SIZE] = *b"meta";
/// Four-CC of a handler reference box (§8.4.3).
pub const HDLR: [u8; BOX_TYPE_SIZE] = *b"hdlr";
/// Four-CC of the Item Data Box (§8.11.11).
pub const IDAT: [u8; BOX_TYPE_SIZE] = *b"idat";
/// Four-CC of the Groups List Box (§8.15.2).
pub const GRPL: [u8; BOX_TYPE_SIZE] = *b"grpl";
/// The `pict` handler type ISO/IEC 23008-12:2017 6.2 requires.
pub const PICT_HANDLER: [u8; BOX_TYPE_SIZE] = *b"pict";
/// The XMP MIME type ISO/IEC 23008-12:2017 A.3 requires.
pub const XMP_CONTENT_TYPE: &[u8] = b"application/rdf+xml";
/// One past the last byte of a range.
fn end(offset: u64, length: u64) -> u64 {
    offset.saturating_add(length)
}

/// Configuration for [`ItemReader`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ItemReaderConfig {
    limits: Limits,
    total_len: Option<u64>,
}

impl ItemReaderConfig {
    /// The caps every count read from the file is checked against.
    #[must_use]
    pub const fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The file length, when the caller knows it. A size-0 top-level box and
    /// an `ExtentOutOfRange` check need it; without it the reader asks the
    /// caller to declare the end of file.
    #[must_use]
    pub const fn with_total_len(mut self, total_len: Option<u64>) -> Self {
        self.total_len = total_len;
        self
    }

    /// The caps.
    #[must_use]
    pub const fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The file length, when known.
    #[must_use]
    pub const fn total_len(&self) -> Option<u64> {
        self.total_len
    }
}

/// One step of a re-emit: write these bytes, or copy this range of the source.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReemitOp {
    /// Write these bytes (a header, a rebuilt `meta` body, a small box).
    Bytes(Vec<u8>),
    /// Copy this range from the source file (a payload the reader never read).
    Copy(ByteRange),
}

/// Where a top-level box's payload came from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TopLevelPayload {
    Meta,
    Bytes(Vec<u8>),
    Copy(ByteRange),
}

/// One top-level box: its exact header bytes and where its payload is.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TopLevelBox {
    fourcc: [u8; BOX_TYPE_SIZE],
    header: Vec<u8>,
    payload: TopLevelPayload,
}

impl TopLevelBox {
    /// The box's four-CC.
    #[must_use]
    pub const fn fourcc(&self) -> [u8; BOX_TYPE_SIZE] {
        self.fourcc
    }

    /// The exact header bytes the file carried.
    #[must_use]
    pub fn header(&self) -> &[u8] {
        &self.header
    }
}

/// A typed child of the file-level `MetaBox` (§8.11.1), in wire order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MetaChild {
    /// The handler reference.
    Hdlr(Hdlr),
    /// The primary item.
    Pitm(PrimaryItemBox),
    /// The item locations.
    Iloc(ItemLocationBox),
    /// The item information.
    Iinf(ItemInfoBox),
    /// The item references.
    Iref(ItemReferenceBox),
    /// The item properties.
    Iprp(ItemPropertiesBox),
    /// The item data.
    Idat(ItemDataBox),
    /// The item protections.
    Ipro(crate::item::ItemProtectionBox),
    /// The entity groups.
    Grpl(GroupsListBox),
    /// Any other child, preserved verbatim.
    Opaque(OpaqueBox),
}

impl Serialize for MetaChild {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Hdlr(value) => value.serialized_len(),
            Self::Pitm(value) => value.serialized_len(),
            Self::Iloc(value) => value.serialized_len(),
            Self::Iinf(value) => value.serialized_len(),
            Self::Iref(value) => value.serialized_len(),
            Self::Iprp(value) => value.serialized_len(),
            Self::Idat(value) => value.serialized_len(),
            Self::Ipro(value) => value.serialized_len(),
            Self::Grpl(value) => value.serialized_len(),
            Self::Opaque(value) => value.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Hdlr(value) => value.serialize_into(buf),
            Self::Pitm(value) => value.serialize_into(buf),
            Self::Iloc(value) => value.serialize_into(buf),
            Self::Iinf(value) => value.serialize_into(buf),
            Self::Iref(value) => value.serialize_into(buf),
            Self::Iprp(value) => value.serialize_into(buf),
            Self::Idat(value) => value.serialize_into(buf),
            Self::Ipro(value) => value.serialize_into(buf),
            Self::Grpl(value) => value.serialize_into(buf),
            Self::Opaque(value) => value.serialize_into(buf),
        }
    }
}

/// The file-level `MetaBox` body: its FullBox version/flags and typed children
/// in wire order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MetaRecord {
    version: u8,
    flags: u32,
    children: Vec<MetaChild>,
}

impl MetaRecord {
    /// A record with these FullBox fields and children.
    #[must_use]
    pub fn new(version: u8, flags: u32, children: Vec<MetaChild>) -> Self {
        Self {
            version,
            flags,
            children,
        }
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The children, in wire order.
    #[must_use]
    pub fn children(&self) -> &[MetaChild] {
        &self.children
    }

    /// The whole `meta` box length (header, FullBox word and children).
    pub fn serialized_len(&self) -> Result<usize> {
        Ok(BOX_HEADER_MIN_SIZE + self.body_len()?)
    }

    fn body_len(&self) -> Result<usize> {
        let mut len = FULLBOX_EXTRA_SIZE;
        for child in &self.children {
            len = len
                .checked_add(child.serialized_len())
                .ok_or(Error::InvalidInput("meta body length overflow"))?;
        }
        Ok(len)
    }

    /// Rebuild the `meta` body (FullBox word plus children).
    pub fn to_body(&self) -> Result<Vec<u8>> {
        let need = self.body_len()?;
        let mut out = alloc::vec![0u8; need];
        out[0] = self.version;
        let flags = self.flags.to_be_bytes();
        out[1..FULLBOX_EXTRA_SIZE].copy_from_slice(&flags[1..]);
        let mut cursor = FULLBOX_EXTRA_SIZE;
        for child in &self.children {
            cursor += child.serialize_into(&mut out[cursor..])?;
        }
        if cursor != need {
            return Err(Error::InvalidInput("meta body length changed"));
        }
        Ok(out)
    }
}

/// One item property associated with an item, in association order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AssociatedProperty {
    index: PropertyIndex,
    essential: bool,
    property: ItemProperty,
}

impl AssociatedProperty {
    /// The 1-based index in the `ipco`.
    #[must_use]
    pub const fn index(&self) -> PropertyIndex {
        self.index
    }

    /// Whether the property is essential to the item.
    #[must_use]
    pub const fn essential(&self) -> bool {
        self.essential
    }

    /// The property itself.
    #[must_use]
    pub fn property(&self) -> &ItemProperty {
        &self.property
    }
}

/// One outgoing item reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemReferenceOut {
    reference_type: ReferenceType,
    to: ItemId,
}

impl ItemReferenceOut {
    /// The reference type.
    #[must_use]
    pub const fn reference_type(&self) -> ReferenceType {
        self.reference_type
    }

    /// The referred item.
    #[must_use]
    pub const fn to(&self) -> ItemId {
        self.to
    }
}

/// One incoming item reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemReferenceIn {
    reference_type: ReferenceType,
    from: ItemId,
}

impl ItemReferenceIn {
    /// The reference type.
    #[must_use]
    pub const fn reference_type(&self) -> ReferenceType {
        self.reference_type
    }

    /// The referring item.
    #[must_use]
    pub const fn from(&self) -> ItemId {
        self.from
    }
}

/// One item of a [`StillImage`]: its declared facts and its resolved data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Item {
    entry: ItemInfoEntry,
    id: ItemId,
    item_type: ItemType,
    name: Vec<u8>,
    hidden: bool,
    protection_index: u16,
    content_type: Option<Vec<u8>>,
    properties: Vec<AssociatedProperty>,
    references_out: Vec<ItemReferenceOut>,
    references_in: Vec<ItemReferenceIn>,
    data: Vec<ByteRange>,
    location: Option<crate::item::ItemLocationEntry>,
    derived: Option<DerivedImage>,
}

impl Item {
    /// The item's `infe` entry (a synthesized one for an item the file left
    /// untermed).
    #[must_use]
    pub const fn entry(&self) -> &ItemInfoEntry {
        &self.entry
    }

    /// The item identifier.
    #[must_use]
    pub const fn id(&self) -> ItemId {
        self.id
    }

    /// The declared item type.
    #[must_use]
    pub const fn item_type(&self) -> ItemType {
        self.item_type
    }

    /// The symbolic `item_name` (empty when absent).
    #[must_use]
    pub fn item_name(&self) -> &[u8] {
        &self.name
    }

    /// The name as UTF-8, when it is.
    #[must_use]
    pub fn name_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.name).ok()
    }

    /// Whether `(flags & 1)` marks the item hidden.
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// The `item_protection_index` (0 when unprotected).
    #[must_use]
    pub const fn protection_index(&self) -> u16 {
        self.protection_index
    }

    /// The MIME `content_type`, when the entry declares one.
    #[must_use]
    pub fn content_type(&self) -> Option<&[u8]> {
        self.content_type.as_deref()
    }

    /// The properties, in association order.
    #[must_use]
    pub fn properties(&self) -> &[AssociatedProperty] {
        &self.properties
    }

    /// The outgoing references, in reference-box order.
    #[must_use]
    pub fn references_out(&self) -> &[ItemReferenceOut] {
        &self.references_out
    }

    /// The incoming references, in reference-box order.
    #[must_use]
    pub fn references_in(&self) -> &[ItemReferenceIn] {
        &self.references_in
    }

    /// The item's data as ordered byte ranges (concatenate them in order).
    #[must_use]
    pub fn data_ranges(&self) -> &[ByteRange] {
        &self.data
    }

    /// The `iloc` entry, when the file carried one.
    #[must_use]
    pub fn location(&self) -> Option<&crate::item::ItemLocationEntry> {
        self.location.as_ref()
    }

    /// The typed derived-image body, for a derived image item the reader could
    /// fetch.
    #[must_use]
    pub fn derived(&self) -> Option<&DerivedImage> {
        self.derived.as_ref()
    }

    /// The total data length.
    #[must_use]
    pub fn data_len(&self) -> u64 {
        self.data.iter().fold(0, |acc, r| acc.saturating_add(r.len))
    }

    fn property(&self, fourcc: &[u8; BOX_TYPE_SIZE]) -> Option<&ItemProperty> {
        self.properties
            .iter()
            .map(|associated| associated.property())
            .find(|property| property.fourcc() == *fourcc)
    }
}

/// The item model of one still-image file, as read by [`ItemReader`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StillImage {
    brands: Option<FileTypeBox>,
    meta: MetaRecord,
    top_level: Vec<TopLevelBox>,
    items: Vec<Item>,
    primary: Option<ItemId>,
    groups: GroupsListBox,
    handler_type: Option<[u8; BOX_TYPE_SIZE]>,
    total_len: Option<u64>,
}

impl StillImage {
    /// The `ftyp` brand list, when the file has one.
    #[must_use]
    pub fn brands(&self) -> Option<&FileTypeBox> {
        self.brands.as_ref()
    }

    /// Whether `brand` is the major brand or a compatible brand.
    #[must_use]
    pub fn has_brand(&self, brand: [u8; BOX_TYPE_SIZE]) -> bool {
        self.brands.as_ref().is_some_and(|brands| {
            brands.major_brand == brand || brands.compatible_brands.contains(&brand)
        })
    }

    /// Whether the brands name a still-image file this crate models.
    #[must_use]
    pub fn is_still_image(&self) -> bool {
        self.brands.as_ref().is_some_and(|brands| {
            is_still_image_brand(brands.major_brand)
                || brands
                    .compatible_brands
                    .iter()
                    .any(|brand| is_still_image_brand(*brand))
        })
    }

    /// The file-level `meta` record.
    #[must_use]
    pub const fn meta(&self) -> &MetaRecord {
        &self.meta
    }

    /// The top-level boxes, in file order.
    #[must_use]
    pub fn top_level(&self) -> &[TopLevelBox] {
        &self.top_level
    }

    /// The items, in `iinf` wire order with any item the file left untermed
    /// appended.
    #[must_use]
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// One item by identifier.
    #[must_use]
    pub fn item(&self, id: ItemId) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }

    /// The primary item, when the file names one.
    #[must_use]
    pub const fn primary(&self) -> Option<ItemId> {
        self.primary
    }

    /// The entity groups.
    #[must_use]
    pub const fn groups(&self) -> &GroupsListBox {
        &self.groups
    }

    /// The file-level `hdlr` handler type, when present.
    #[must_use]
    pub const fn handler_type(&self) -> Option<[u8; BOX_TYPE_SIZE]> {
        self.handler_type
    }

    /// Whether the file-level handler is `pict` (ISO/IEC 23008-12:2017 6.2).
    #[must_use]
    pub fn is_picture_handler(&self) -> bool {
        self.handler_type == Some(PICT_HANDLER)
    }

    /// The width and height of an item's reconstructed image, from its `ispe`
    /// property (ISO/IEC 23008-12:2017 6.5.3).
    #[must_use]
    pub fn image_size(&self, id: ItemId) -> Option<(u32, u32)> {
        self.item(id)?
            .property(&crate::image::ISPE)?
            .as_spatial_extents()
            .map(|ispe| (ispe.image_width(), ispe.image_height()))
    }

    /// The bit depths per channel of an item, from its `pixi` property.
    #[must_use]
    pub fn pixel_bits(&self, id: ItemId) -> Option<&[u8]> {
        self.item(id)?
            .property(&crate::image::PIXI)?
            .as_pixel_information()
            .map(crate::image::PixelInformationProperty::bits_per_channel)
    }

    /// The rotation of an item, from its `irot` property.
    #[must_use]
    pub fn rotation(&self, id: ItemId) -> Option<ImageRotation> {
        self.item(id)?
            .property(&crate::image::IROT)?
            .as_rotation()
            .copied()
    }

    /// The mirroring of an item, from its `imir` property.
    #[must_use]
    pub fn mirror(&self, id: ItemId) -> Option<ImageMirror> {
        self.item(id)?
            .property(&crate::image::IMIR)?
            .as_mirror()
            .copied()
    }

    /// The auxiliary-image type of an item, from its `auxC` property.
    #[must_use]
    pub fn auxiliary_type(&self, id: ItemId) -> Option<&crate::image::AuxiliaryTypeProperty> {
        self.item(id)?.property(&crate::image::AUXC)?.as_aux_type()
    }

    /// The thumbnails of `master`, in reference order.
    #[must_use]
    pub fn thumbnails_of(&self, master: ItemId) -> Vec<&Item> {
        self.items
            .iter()
            .filter(|item| {
                item.references_out.iter().any(|reference| {
                    reference.reference_type == ReferenceType::Thmb && reference.to == master
                })
            })
            .collect()
    }

    /// The auxiliary images of `master`, in reference order.
    #[must_use]
    pub fn auxiliary_images_of(&self, master: ItemId) -> Vec<&Item> {
        self.items
            .iter()
            .filter(|item| {
                item.references_out.iter().any(|reference| {
                    reference.reference_type == ReferenceType::Auxl && reference.to == master
                })
            })
            .collect()
    }

    /// The items a derived image item takes as inputs, in `dimg` order.
    #[must_use]
    pub fn derived_inputs(&self, derived: ItemId) -> Vec<&Item> {
        let Some(item) = self.item(derived) else {
            return Vec::new();
        };
        item.references_out
            .iter()
            .filter(|reference| reference.reference_type == ReferenceType::Dimg)
            .filter_map(|reference| self.item(reference.to))
            .collect()
    }

    /// A grid item's tiles, in row-major `dimg` order
    /// (ISO/IEC 23008-12:2017 6.6.2.3).
    #[must_use]
    pub fn grid_tiles(&self, grid: ItemId) -> Vec<&Item> {
        match self.item(grid).and_then(Item::derived) {
            Some(DerivedImage::Grid(_)) => self.derived_inputs(grid),
            _ => Vec::new(),
        }
    }

    /// An overlay item's layers with their offsets, bottom-most first
    /// (ISO/IEC 23008-12:2017 6.6.2.2).
    #[must_use]
    pub fn overlay_layers(&self, overlay: ItemId) -> Vec<(&Item, (i32, i32))> {
        let Some(DerivedImage::Overlay(image_overlay)) = self.item(overlay).and_then(Item::derived)
        else {
            return Vec::new();
        };
        self.derived_inputs(overlay)
            .into_iter()
            .enumerate()
            .filter_map(|(index, item)| {
                image_overlay
                    .offsets()
                    .get(index)
                    .map(|offset| (item, *offset))
            })
            .collect()
    }

    /// Every Exif metadata item (`item_type` `Exif`, ISO/IEC 23008-12 A.2).
    #[must_use]
    pub fn exif_items(&self) -> Vec<&Item> {
        self.items
            .iter()
            .filter(|item| item.item_type == ItemType::Exif)
            .collect()
    }

    /// Every XMP metadata item (`mime` with the RDF/XML content type,
    /// ISO/IEC 23008-12 A.3).
    #[must_use]
    pub fn xmp_items(&self) -> Vec<&Item> {
        self.items
            .iter()
            .filter(|item| {
                item.item_type == ItemType::Mime
                    && item.content_type.as_deref() == Some(XMP_CONTENT_TYPE)
            })
            .collect()
    }

    /// The item data bytes, copied out of `source`.
    pub fn data_bytes(&self, id: ItemId, source: &[u8]) -> Result<Vec<u8>> {
        let item = self.item(id).ok_or(Error::InvalidInput("unknown item"))?;
        let mut out = Vec::with_capacity(usize::try_from(item.data_len()).unwrap_or(0));
        for range in item.data_ranges() {
            out.extend_from_slice(source_range(source, *range)?);
        }
        Ok(out)
    }

    /// Re-emit the file as an ordered plan: every parsed box's exact header,
    /// the rebuilt `meta` body, and a [`ReemitOp::Copy`] for each payload the
    /// reader never read.
    pub fn reemit_plan(&self) -> Result<Vec<ReemitOp>> {
        let meta_body = self.meta.to_body()?;
        let mut ops = Vec::new();
        for top in &self.top_level {
            match &top.payload {
                TopLevelPayload::Meta => {
                    let declared = declared_size(&top.header)?;
                    let actual = top.header.len().saturating_add(meta_body.len());
                    if declared.is_some_and(|size| usize::try_from(size).ok() != Some(actual)) {
                        return Err(Error::InvalidInput("meta body length changed"));
                    }
                    ops.push(ReemitOp::Bytes(top.header.clone()));
                    ops.push(ReemitOp::Bytes(meta_body.clone()));
                }
                TopLevelPayload::Bytes(payload) => {
                    let mut bytes = top.header.clone();
                    bytes.extend_from_slice(payload);
                    ops.push(ReemitOp::Bytes(bytes));
                }
                TopLevelPayload::Copy(range) => {
                    ops.push(ReemitOp::Bytes(top.header.clone()));
                    ops.push(ReemitOp::Copy(*range));
                }
            }
        }
        Ok(ops)
    }

    /// Re-emit the file by applying [`StillImage::reemit_plan`] to `source`.
    pub fn reemit_into(&self, source: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for op in self.reemit_plan()? {
            match op {
                ReemitOp::Bytes(bytes) => out.extend_from_slice(&bytes),
                ReemitOp::Copy(range) => out.extend_from_slice(source_range(source, range)?),
            }
        }
        Ok(out)
    }

    /// The total file length, when the caller supplied it.
    #[must_use]
    pub const fn total_len(&self) -> Option<u64> {
        self.total_len
    }
}

fn source_range(source: &[u8], range: ByteRange) -> Result<&[u8]> {
    let start = usize::try_from(range.offset)
        .map_err(|_| Error::InvalidInput("byte range offset exceeds usize"))?;
    let len = usize::try_from(range.len)
        .map_err(|_| Error::InvalidInput("byte range length exceeds usize"))?;
    source
        .get(start..start.saturating_add(len))
        .ok_or(Error::InvalidInput("byte range outside the source"))
}

fn declared_size(header: &[u8]) -> Result<Option<u64>> {
    if header.len() < TOP_HEADER_LEN {
        return Err(Error::InvalidInput("short box header"));
    }
    match u32_at(header, SIZE_OFFSET) {
        SIZE_TO_EOF => Ok(None),
        SIZE_INDICATES_LARGESIZE => {
            Ok(Some(broadcast_core::wire::u64_at(header, LARGESIZE_OFFSET)))
        }
        size => Ok(Some(u64::from(size))),
    }
}

impl StillImage {
    fn assemble(
        brands: Option<FileTypeBox>,
        meta: MetaRecord,
        top_level: Vec<TopLevelBox>,
        idat: Option<(u64, usize)>,
        total_len: Option<u64>,
        limits: &Limits,
    ) -> Result<Self> {
        let mut iloc: Option<ItemLocationBox> = None;
        let mut iinf: Option<ItemInfoBox> = None;
        let mut iref: Option<ItemReferenceBox> = None;
        let mut iprp: Option<ItemPropertiesBox> = None;
        let mut grpl: Option<GroupsListBox> = None;
        let mut pitm: Option<PrimaryItemBox> = None;
        let mut handler_type = None;
        for child in &meta.children {
            match child {
                MetaChild::Hdlr(hdlr) => handler_type = Some(hdlr.handler_type),
                MetaChild::Pitm(value) => pitm = Some(*value),
                MetaChild::Iloc(value) => iloc = Some(value.clone()),
                MetaChild::Iinf(value) => iinf = Some(value.clone()),
                MetaChild::Iref(value) => iref = Some(value.clone()),
                MetaChild::Iprp(value) => iprp = Some(value.clone()),
                MetaChild::Grpl(value) => grpl = Some(value.clone()),
                _ => {}
            }
        }
        let empty_iloc = ItemLocationBox::new();
        let iloc = iloc.unwrap_or(empty_iloc);
        let empty_iinf = ItemInfoBox::new();
        let iinf = iinf.unwrap_or(empty_iinf);
        let empty_iref = ItemReferenceBox::new();
        let iref = iref.unwrap_or(empty_iref);
        let groups = grpl.unwrap_or_default();
        check_count("item count", iinf.entries().len(), limits.max_items())?;
        check_count("iloc item count", iloc.items().len(), limits.max_items())?;
        check_count(
            "item reference count",
            iref.references().len(),
            limits.max_references(),
        )?;
        if let Some(properties) = &iprp {
            check_count(
                "item property count",
                properties.container().properties().len(),
                limits.max_properties(),
            )?;
        }
        let mut items: Vec<Item> = Vec::new();
        for entry in iinf.entries() {
            items.push(item_from_entry(entry, &iloc, iprp.as_ref()));
        }
        for location in iloc.items() {
            ensure_item(&mut items, location.item_id(), &iloc);
        }
        for reference in iref.references() {
            ensure_item(&mut items, reference.from(), &iloc);
            for to in reference.to() {
                ensure_item(&mut items, *to, &iloc);
            }
        }
        for reference in iref.references() {
            for to in reference.to() {
                if let Some(from_item) = items.iter_mut().find(|item| item.id == reference.from()) {
                    from_item.references_out.push(ItemReferenceOut {
                        reference_type: reference.reference_type(),
                        to: *to,
                    });
                }
                if let Some(to_item) = items.iter_mut().find(|item| item.id == *to) {
                    to_item.references_in.push(ItemReferenceIn {
                        reference_type: reference.reference_type(),
                        from: reference.from(),
                    });
                }
            }
        }
        let (idat_offset, idat_len) = idat.unwrap_or((0, 0));
        let mut resolved: Vec<Vec<ByteRange>> = Vec::with_capacity(items.len());
        for item in &items {
            resolved.push(resolve_item_data(
                item.id,
                &iloc,
                &iref,
                idat_offset,
                idat_len,
                total_len,
                &mut Vec::new(),
            )?);
        }
        for (item, data) in items.iter_mut().zip(resolved) {
            item.data = data;
            if item.item_type == ItemType::Iden {
                item.derived = Some(DerivedImage::parse(ItemType::Iden, &[], 0)?);
            }
        }
        detect_derived_cycle(&items)?;
        Ok(Self {
            brands,
            meta,
            top_level,
            items,
            primary: pitm.map(|box_value| box_value.item_id()),
            groups,
            handler_type,
            total_len,
        })
    }
}

fn check_count(what: &'static str, count: usize, limit: u32) -> Result<()> {
    if u64::try_from(count).unwrap_or(u64::MAX) > u64::from(limit) {
        return Err(Error::LimitExceeded {
            what,
            limit: u64::from(limit),
        });
    }
    Ok(())
}

fn item_from_entry(
    entry: &ItemInfoEntry,
    iloc: &ItemLocationBox,
    iprp: Option<&ItemPropertiesBox>,
) -> Item {
    let mut properties = Vec::new();
    if let Some(iprp) = iprp {
        for association in iprp.associations_for(entry.item_id()) {
            if let Some(property) = iprp.container().property(association.property_index()) {
                properties.push(AssociatedProperty {
                    index: association.property_index(),
                    essential: association.essential(),
                    property: property.clone(),
                });
            }
        }
    }
    Item {
        entry: entry.clone(),
        id: entry.item_id(),
        item_type: entry.item_type(),
        name: entry.item_name().to_vec(),
        hidden: entry.is_hidden(),
        protection_index: entry.item_protection_index(),
        content_type: entry.content_type().map(|value| value.to_vec()),
        properties,
        references_out: Vec::new(),
        references_in: Vec::new(),
        data: Vec::new(),
        location: iloc.item(entry.item_id()).cloned(),
        derived: None,
    }
}

fn ensure_item(items: &mut Vec<Item>, id: ItemId, iloc: &ItemLocationBox) {
    if items.iter().any(|item| item.id == id) {
        return;
    }
    items.push(Item {
        entry: ItemInfoEntry::new(id, ItemType::Other([0; BOX_TYPE_SIZE]), &[]),
        id,
        item_type: ItemType::Other([0; BOX_TYPE_SIZE]),
        name: Vec::new(),
        hidden: false,
        protection_index: 0,
        content_type: None,
        properties: Vec::new(),
        references_out: Vec::new(),
        references_in: Vec::new(),
        data: Vec::new(),
        location: iloc.item(id).cloned(),
        derived: None,
    });
}

fn iloc_target(iref: &ItemReferenceBox, item: ItemId, index: u32) -> Option<ItemId> {
    for reference in iref.references() {
        if reference.from() == item && reference.reference_type() == ReferenceType::Iloc {
            let zero_based = usize::try_from(index).ok()?.checked_sub(1)?;
            return reference.to().get(zero_based).copied();
        }
    }
    None
}

fn resolve_item_data(
    id: ItemId,
    iloc: &ItemLocationBox,
    iref: &ItemReferenceBox,
    idat_offset: u64,
    idat_len: usize,
    total_len: Option<u64>,
    stack: &mut Vec<ItemId>,
) -> Result<Vec<ByteRange>> {
    if stack.contains(&id) {
        return Err(Error::ReferenceCycle { item: id.get() });
    }
    let Some(location) = iloc.item(id) else {
        return Ok(Vec::new());
    };
    stack.push(id);
    let mut out = Vec::with_capacity(location.extents().len());
    for extent in location.extents() {
        let range = match location.construction_method() {
            crate::item::ConstructionMethod::File => {
                if location.data_reference_index() != 0 {
                    return Err(Error::DataReferenceUnsupported {
                        item: id.get(),
                        index: location.data_reference_index(),
                    });
                }
                let offset = location.base_offset().saturating_add(extent.offset());
                let length = if extent.length() == 0 {
                    match total_len {
                        Some(total) => total.saturating_sub(offset),
                        None => 0,
                    }
                } else {
                    extent.length()
                };
                ByteRange {
                    offset,
                    len: length,
                }
            }
            crate::item::ConstructionMethod::Idat => {
                let data_len =
                    u64::try_from(idat_len).map_err(|_| Error::InvalidInput("idat length"))?;
                let offset = idat_offset
                    .saturating_add(location.base_offset())
                    .saturating_add(extent.offset());
                let length = if extent.length() == 0 {
                    data_len.saturating_sub(offset.saturating_sub(idat_offset))
                } else {
                    extent.length()
                };
                ByteRange {
                    offset,
                    len: length,
                }
            }
            crate::item::ConstructionMethod::Item => {
                let index = extent.item_reference_index().unwrap_or(1);
                let target = iloc_target(iref, id, index)
                    .ok_or(Error::InvalidInput("iloc item reference is missing"))?;
                let target_ranges =
                    resolve_item_data(target, iloc, iref, idat_offset, idat_len, total_len, stack)?;
                let target_len = target_ranges
                    .iter()
                    .fold(0u64, |acc, r| acc.saturating_add(r.len));
                let offset = location.base_offset().saturating_add(extent.offset());
                let length = if extent.length() == 0 {
                    target_len.saturating_sub(offset)
                } else {
                    extent.length()
                };
                let sliced = slice_ranges(&target_ranges, offset, length)
                    .ok_or(Error::InvalidInput("iloc extent is past its source item"))?;
                out.extend_from_slice(&sliced);
                continue;
            }
        };
        if let Some(total) = total_len
            && end(range.offset, range.len) > total
        {
            return Err(Error::ExtentOutOfRange {
                item: id.get(),
                offset: range.offset,
                length: range.len,
                file_len: total,
            });
        }
        out.push(range);
    }
    stack.pop();
    Ok(out)
}

fn slice_ranges(ranges: &[ByteRange], offset: u64, length: u64) -> Option<Vec<ByteRange>> {
    let total = ranges.iter().fold(0u64, |acc, r| acc.saturating_add(r.len));
    if offset > total {
        return None;
    }
    let wanted = if length == 0 { total - offset } else { length };
    if end(offset, wanted) > total {
        return None;
    }
    let mut out = Vec::new();
    let mut skip = offset;
    let mut left = wanted;
    for range in ranges {
        if left == 0 {
            break;
        }
        if skip >= range.len {
            skip -= range.len;
            continue;
        }
        let take = core::cmp::min(range.len - skip, left);
        out.push(ByteRange {
            offset: range.offset.saturating_add(skip),
            len: take,
        });
        left -= take;
        skip = 0;
    }
    Some(out)
}

fn detect_derived_cycle(items: &[Item]) -> Result<()> {
    let index_of = |id: ItemId| items.iter().position(|item| item.id == id);
    let mut indegree = alloc::vec![0usize; items.len()];
    let mut edges: Vec<Vec<usize>> = alloc::vec![Vec::new(); items.len()];
    for (from, item) in items.iter().enumerate() {
        for reference in &item.references_out {
            if reference.reference_type != ReferenceType::Dimg {
                continue;
            }
            if let Some(to) = index_of(reference.to) {
                edges[from].push(to);
                indegree[to] += 1;
            }
        }
    }
    let mut queue: VecDeque<usize> = indegree
        .iter()
        .enumerate()
        .filter_map(|(index, degree)| (*degree == 0).then_some(index))
        .collect();
    let mut removed = 0usize;
    while let Some(index) = queue.pop_front() {
        removed += 1;
        for to in &edges[index] {
            indegree[*to] = indegree[*to].saturating_sub(1);
            if indegree[*to] == 0 {
                queue.push_back(*to);
            }
        }
    }
    if removed < items.len()
        && let Some(index) = indegree.iter().position(|degree| *degree > 0)
    {
        return Err(Error::ReferenceCycle {
            item: items[index].id.get(),
        });
    }
    Ok(())
}

/// What [`ItemReader::step`] yields.
#[derive(Debug)]
#[non_exhaustive]
pub enum ItemStep<'a> {
    /// Supply exactly this byte range, then call `step` again.
    Need(ByteRange),
    /// The item model, once every structure the reader needs has been read.
    Loaded(&'a StillImage),
    /// The file has been fully reported.
    Done,
}

/// Which body a [`Phase::Body`] demand reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    Ftyp,
    Meta,
}

#[derive(Debug)]
enum Phase {
    Header {
        offset: u64,
        prefix: Vec<u8>,
        target: usize,
    },
    Body {
        box_start: u64,
        header: Vec<u8>,
        size: u64,
        kind: BodyKind,
    },
    Derived,
    Loaded,
    Done,
}

#[derive(Debug)]
struct DerivedFetch {
    item_index: usize,
    ranges: Vec<ByteRange>,
    next: usize,
    buffer: Vec<u8>,
}

/// The sans-IO still-image reader. Drive it with [`ItemReader::step`] and
/// [`ItemReader::supply`], exactly like [`crate::reader::Reader`].
#[derive(Debug)]
pub struct ItemReader {
    config: ItemReaderConfig,
    phase: Phase,
    pending: Option<ByteRange>,
    top_level: Vec<TopLevelBox>,
    brands: Option<FileTypeBox>,
    meta: Option<MetaRecord>,
    idat: Option<(u64, usize)>,
    image: Option<StillImage>,
    reported_loaded: bool,
    derived_queue: VecDeque<usize>,
    derived_current: Option<DerivedFetch>,
}

impl ItemReader {
    /// A reader at the start of the file.
    #[must_use]
    pub fn new(config: ItemReaderConfig) -> Self {
        Self {
            config,
            phase: Phase::Header {
                offset: 0,
                prefix: Vec::new(),
                target: TOP_HEADER_LEN,
            },
            pending: None,
            top_level: Vec::new(),
            brands: None,
            meta: None,
            idat: None,
            image: None,
            reported_loaded: false,
            derived_queue: VecDeque::new(),
            derived_current: None,
        }
    }

    /// The configuration.
    #[must_use]
    pub const fn config(&self) -> &ItemReaderConfig {
        &self.config
    }

    /// Ask for the next range, or yield the model.
    pub fn step(&mut self) -> Result<ItemStep<'_>> {
        loop {
            if self.pending.is_none() {
                match &self.phase {
                    Phase::Header {
                        offset,
                        prefix,
                        target,
                    } => {
                        self.pending = Some(ByteRange {
                            offset: offset.saturating_add(prefix.len() as u64),
                            len: (target.saturating_sub(prefix.len())) as u64,
                        });
                    }
                    Phase::Body {
                        box_start,
                        header,
                        size,
                        ..
                    } => {
                        let body_len = size.saturating_sub(header.len() as u64);
                        if body_len == 0 {
                            self.process_body(&[])?;
                            continue;
                        }
                        self.pending = Some(ByteRange {
                            offset: box_start.saturating_add(header.len() as u64),
                            len: body_len,
                        });
                    }
                    Phase::Derived => {
                        if let Some(range) = self.next_derived_need()? {
                            self.pending = Some(range);
                        } else {
                            self.phase = Phase::Loaded;
                            continue;
                        }
                    }
                    Phase::Loaded => {
                        if self.reported_loaded {
                            self.phase = Phase::Done;
                            return Ok(ItemStep::Done);
                        }
                        self.reported_loaded = true;
                        let image = self
                            .image
                            .as_ref()
                            .ok_or(Error::InvalidInput("item reader has no model"))?;
                        return Ok(ItemStep::Loaded(image));
                    }
                    Phase::Done => return Ok(ItemStep::Done),
                }
            }
            if let Some(range) = self.pending {
                return Ok(ItemStep::Need(range));
            }
        }
    }

    /// Accept exactly the bytes the outstanding [`ItemStep::Need`] asked for.
    pub fn supply(&mut self, offset: u64, bytes: &[u8]) -> Result<()> {
        let got = ByteRange {
            offset,
            len: bytes.len() as u64,
        };
        let Some(expected) = self.pending.take() else {
            return Err(Error::SupplyMismatch {
                expected: ByteRange { offset: 0, len: 0 },
                got,
            });
        };
        if expected.offset != offset || expected.len != got.len {
            return Err(Error::SupplyMismatch { expected, got });
        }
        let filled = match &mut self.phase {
            Phase::Header { prefix, .. } => {
                prefix.extend_from_slice(bytes);
                Some(prefix.len())
            }
            _ => None,
        };
        if let Some(filled) = filled {
            let target = self.header_target(filled);
            if let Phase::Header { target: slot, .. } = &mut self.phase {
                *slot = target;
            }
            if filled >= target {
                self.finalize_header()?;
            }
            return Ok(());
        }
        match &mut self.phase {
            Phase::Body { .. } => self.process_body(bytes),
            Phase::Derived => {
                let Some(fetch) = self.derived_current.as_mut() else {
                    return Err(Error::InvalidInput("derived demand without a fetch"));
                };
                fetch.buffer.extend_from_slice(bytes);
                fetch.next += 1;
                if fetch.next >= fetch.ranges.len() {
                    self.finish_derived_item()?;
                }
                Ok(())
            }
            Phase::Header { .. } => Err(Error::InvalidInput("header supply already consumed")),
            Phase::Loaded | Phase::Done => Err(Error::InvalidInput("supply after the model")),
        }
    }

    /// Declare the input complete when the caller does not know the file
    /// length in advance. A no-op once the reader has every top-level box.
    pub fn end_of_file(&mut self) -> Result<()> {
        match &self.phase {
            Phase::Header { offset, prefix, .. } if prefix.is_empty() && *offset > 0 => {
                self.finalize()
            }
            Phase::Loaded | Phase::Done => Ok(()),
            _ => Err(Error::InvalidInput("end of file outside a box boundary")),
        }
    }

    /// The model, once loaded.
    #[must_use]
    pub fn image(&self) -> Option<&StillImage> {
        self.image.as_ref()
    }

    fn header_target(&self, filled: usize) -> usize {
        if filled < TOP_HEADER_LEN {
            return TOP_HEADER_LEN;
        }
        match &self.phase {
            Phase::Header { prefix, .. } => {
                let size32 = u32_at(prefix, SIZE_OFFSET);
                let fourcc = fourcc_of(prefix);
                let mut target = TOP_HEADER_LEN;
                if size32 == SIZE_INDICATES_LARGESIZE {
                    target += LARGESIZE_SIZE;
                }
                if fourcc == UUID {
                    target += UUID_TYPE_SIZE;
                }
                target
            }
            _ => TOP_HEADER_LEN,
        }
    }

    fn finalize_header(&mut self) -> Result<()> {
        let Phase::Header {
            offset,
            prefix,
            target,
        } = &mut self.phase
        else {
            return Err(Error::InvalidInput("not at a box header"));
        };
        if prefix.len() < *target {
            return Ok(());
        }
        let size32 = u32_at(prefix, SIZE_OFFSET);
        let fourcc = fourcc_of(prefix);
        let header_len = prefix.len();
        let size = match size32 {
            SIZE_TO_EOF => match self.config.total_len {
                Some(total) => total.saturating_sub(*offset),
                None => {
                    return Err(Error::InvalidInput(
                        "a size-0 box needs the file length; call with_total_len",
                    ));
                }
            },
            SIZE_INDICATES_LARGESIZE => broadcast_core::wire::u64_at(prefix, LARGESIZE_OFFSET),
            size => u64::from(size),
        };
        if size != 0 && size < header_len as u64 {
            return Err(Error::BoxSizeUnderflow {
                size,
                header_size: header_len,
            });
        }
        let body_len = size.saturating_sub(header_len as u64);
        let header = core::mem::take(prefix);
        let next_offset = offset.saturating_add(size);
        let kind = if fourcc == META {
            Some(BodyKind::Meta)
        } else if fourcc == FTYP {
            Some(BodyKind::Ftyp)
        } else {
            None
        };
        match kind {
            Some(kind) => {
                self.phase = Phase::Body {
                    box_start: *offset,
                    header,
                    size,
                    kind,
                };
            }
            None => {
                self.top_level.push(TopLevelBox {
                    fourcc,
                    header,
                    payload: TopLevelPayload::Copy(ByteRange {
                        offset: offset.saturating_add(header_len as u64),
                        len: body_len,
                    }),
                });
                self.phase = Phase::Header {
                    offset: next_offset,
                    prefix: Vec::new(),
                    target: TOP_HEADER_LEN,
                };
                if self.config.total_len == Some(next_offset) || size == 0 {
                    self.finalize()?;
                }
            }
        }
        Ok(())
    }

    fn process_body(&mut self, bytes: &[u8]) -> Result<()> {
        let Phase::Body {
            box_start,
            header,
            size,
            kind,
        } = &self.phase
        else {
            return Err(Error::InvalidInput("not at a box body"));
        };
        let box_start = *box_start;
        let size = *size;
        let kind = *kind;
        let header = header.clone();
        let next_offset = box_start.saturating_add(size);
        match kind {
            BodyKind::Ftyp => {
                let brands = FileTypeBox::parse_payload(bytes)?;
                self.brands = Some(brands);
                self.top_level.push(TopLevelBox {
                    fourcc: FTYP,
                    header,
                    payload: TopLevelPayload::Bytes(bytes.to_vec()),
                });
            }
            BodyKind::Meta => {
                if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                    > self.config.limits.max_moov_bytes()
                {
                    return Err(Error::LimitExceeded {
                        what: "meta box size",
                        limit: self.config.limits.max_moov_bytes(),
                    });
                }
                let payload_abs = box_start.saturating_add(header.len() as u64);
                let (record, idat) = parse_meta(bytes, payload_abs)?;
                if let Some(idat) = idat {
                    self.idat = Some(idat);
                }
                self.meta = Some(record);
                self.top_level.push(TopLevelBox {
                    fourcc: META,
                    header,
                    payload: TopLevelPayload::Meta,
                });
            }
        }
        self.phase = Phase::Header {
            offset: next_offset,
            prefix: Vec::new(),
            target: TOP_HEADER_LEN,
        };
        if self.config.total_len == Some(next_offset) || size == 0 {
            self.finalize()?;
        }
        Ok(())
    }

    fn finalize(&mut self) -> Result<()> {
        if self.image.is_some() {
            self.phase = Phase::Derived;
            return Ok(());
        }
        let Some(meta) = self.meta.take() else {
            return Err(Error::InvalidInput("file-level meta box is missing"));
        };
        let image = StillImage::assemble(
            self.brands.clone(),
            meta,
            core::mem::take(&mut self.top_level),
            self.idat,
            self.config.total_len,
            &self.config.limits,
        )?;
        for (index, item) in image.items.iter().enumerate() {
            if item.derived().is_none() && item.item_type().is_derived_image() {
                self.derived_queue.push_back(index);
            }
        }
        self.image = Some(image);
        self.phase = Phase::Derived;
        Ok(())
    }

    fn next_derived_need(&mut self) -> Result<Option<ByteRange>> {
        loop {
            if self.derived_current.is_none() {
                let Some(item_index) = self.derived_queue.pop_front() else {
                    return Ok(None);
                };
                let Some(image) = self.image.as_ref() else {
                    return Err(Error::InvalidInput("derived body without a model"));
                };
                let ranges = image.items[item_index].data_ranges().to_vec();
                let total = ranges.iter().fold(0u64, |acc, r| acc.saturating_add(r.len));
                if total > self.config.limits.max_metadata_item_bytes() {
                    return Err(Error::LimitExceeded {
                        what: "derived image item body",
                        limit: self.config.limits.max_metadata_item_bytes(),
                    });
                }
                if ranges.is_empty() {
                    self.parse_derived_body(item_index, &[])?;
                    continue;
                }
                self.derived_current = Some(DerivedFetch {
                    item_index,
                    ranges,
                    next: 0,
                    buffer: Vec::new(),
                });
            }
            let Some(fetch) = self.derived_current.as_ref() else {
                continue;
            };
            if fetch.next < fetch.ranges.len() {
                return Ok(Some(fetch.ranges[fetch.next]));
            }
            self.finish_derived_item()?;
        }
    }

    fn finish_derived_item(&mut self) -> Result<()> {
        let Some(fetch) = self.derived_current.take() else {
            return Ok(());
        };
        self.parse_derived_body(fetch.item_index, &fetch.buffer)
    }

    fn parse_derived_body(&mut self, item_index: usize, body: &[u8]) -> Result<()> {
        let Some(image) = self.image.as_mut() else {
            return Err(Error::InvalidInput("derived body without a model"));
        };
        let Some(item) = image.items.get_mut(item_index) else {
            return Err(Error::InvalidInput("derived item index"));
        };
        let reference_count = item
            .references_out
            .iter()
            .filter(|reference| reference.reference_type == ReferenceType::Dimg)
            .count();
        item.derived = Some(DerivedImage::parse(item.item_type, body, reference_count)?);
        Ok(())
    }
}

fn fourcc_of(prefix: &[u8]) -> [u8; BOX_TYPE_SIZE] {
    let mut fourcc = [0u8; BOX_TYPE_SIZE];
    if prefix.len() >= TYPE_OFFSET + BOX_TYPE_SIZE {
        fourcc.copy_from_slice(&prefix[TYPE_OFFSET..TYPE_OFFSET + BOX_TYPE_SIZE]);
    }
    fourcc
}

fn parse_meta(body: &[u8], payload_abs: u64) -> Result<(MetaRecord, Option<(u64, usize)>)> {
    if body.len() < FULLBOX_EXTRA_SIZE {
        return Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: FULLBOX_EXTRA_SIZE,
                have: body.len(),
                what: "meta body",
            },
        ));
    }
    let version = body[0];
    let flags = broadcast_core::wire::u24_at(body, crate::box_types::FULLBOX_FLAGS_OFFSET)
        & crate::box_types::FULLBOX_FLAGS_MASK;
    let mut children = Vec::new();
    let mut idat = None;
    let mut rest = &body[FULLBOX_EXTRA_SIZE..];
    let children_start = payload_abs.saturating_add(FULLBOX_EXTRA_SIZE as u64);
    while !rest.is_empty() {
        if rest.len() < BOX_HEADER_MIN_SIZE {
            return Err(Error::BufferTooShort(
                broadcast_core::error::BufferTooShort {
                    need: BOX_HEADER_MIN_SIZE,
                    have: rest.len(),
                    what: "meta child header",
                },
            ));
        }
        let (child, used) = parse_box(rest)?;
        let absolute = children_start.saturating_add(child.start as u64);
        let fourcc = child.four_cc();
        let parsed = match &fourcc {
            b"hdlr" => MetaChild::Hdlr(Hdlr::parse_box(child.bytes)?),
            b"pitm" => MetaChild::Pitm(PrimaryItemBox::parse(child.bytes)?),
            b"iloc" => MetaChild::Iloc(ItemLocationBox::parse(child.bytes)?),
            b"iinf" => MetaChild::Iinf(ItemInfoBox::parse(child.bytes)?),
            b"iref" => MetaChild::Iref(ItemReferenceBox::parse(child.bytes)?),
            b"iprp" => MetaChild::Iprp(ItemPropertiesBox::parse(child.bytes)?),
            b"idat" => {
                let box_value = ItemDataBox::parse(child.bytes)?;
                idat = Some((
                    absolute.saturating_add(child.header.header_size() as u64),
                    box_value.data().len(),
                ));
                MetaChild::Idat(box_value)
            }
            b"ipro" => MetaChild::Ipro(crate::item::ItemProtectionBox::parse(child.bytes)?),
            b"grpl" => MetaChild::Grpl(GroupsListBox::parse(child.bytes)?),
            _ => {
                let mut opaque = OpaqueBox::new(fourcc, child.payload().to_vec());
                opaque.to_end = child.to_end();
                opaque.largesize = child.has_largesize();
                MetaChild::Opaque(opaque)
            }
        };
        children.push(parsed);
        rest = &rest[used..];
    }
    Ok((
        MetaRecord {
            version,
            flags,
            children,
        },
        idat,
    ))
}
