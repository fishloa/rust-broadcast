//! Item boxes of a file-level `MetaBox` — ISO/IEC 14496-12:2026 §8.11.
//!
//! A still image file has no movie: its content is *items* listed by the
//! file-level `meta` box (§8.11.1). This module types the boxes that describe
//! them: [`ItemLocationBox`] (`iloc`, §8.11.3), [`ItemInfoBox`] (`iinf`,
//! §8.11.6), [`PrimaryItemBox`] (`pitm`, §8.11.4), [`ItemReferenceBox`]
//! (`iref`, §8.11.12), [`ItemPropertiesBox`] (`iprp`/`ipco`/`ipma`, §8.11.14),
//! [`ItemDataBox`] (`idat`, §8.11.11), [`ItemProtectionBox`] (`ipro`,
//! §8.11.5), [`GroupsListBox`] (`grpl`, §8.15.2) with its
//! [`EntityToGroupBox`] (§8.15.3), and the two properties §8.11 itself defines:
//! [`BrandProperty`] (`brnd`, §8.11.15) and [`HandlerProperty`] (`hdlp`,
//! §8.11.16).
//!
//! Image item properties and derived image item bodies are in
//! [`crate::image`]; the streaming reader and the writer are
//! [`crate::item_reader`] and [`crate::item_writer`].
//!
//! Sources: `private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`
//! §8.11 and §8.15. Every type is private-field with a builder and accessors,
//! and a parse → serialize round trip is byte-identical.

use alloc::vec::Vec;

use broadcast_core::error::{BufferTooShort, InvalidValue};
use broadcast_core::len::{fit_u32, need_in, need_out};
use broadcast_core::wire::{be_u16, be_u32, be_u64, u24_at, u32_at};
use broadcast_core::{Parse, Serialize};

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK,
    box_len, write_box_header, write_full_box_header,
};
use crate::error::{Error, Result};
use crate::segments::FileTypeBox;

/// Four-CC of the Item Location Box (§8.11.3).
pub const ILOC: [u8; BOX_TYPE_SIZE] = *b"iloc";
/// Four-CC of the Item Information Box (§8.11.6).
pub const IINF: [u8; BOX_TYPE_SIZE] = *b"iinf";
/// Four-CC of an Item Information Entry (§8.11.6).
pub const INFE: [u8; BOX_TYPE_SIZE] = *b"infe";
/// Four-CC of the Primary Item Box (§8.11.4).
pub const PITM: [u8; BOX_TYPE_SIZE] = *b"pitm";
/// Four-CC of the Item Reference Box (§8.11.12).
pub const IREF: [u8; BOX_TYPE_SIZE] = *b"iref";
/// Four-CC of the Item Properties Box (§8.11.14).
pub const IPRP: [u8; BOX_TYPE_SIZE] = *b"iprp";
/// Four-CC of the Item Property Container Box (§8.11.14).
pub const IPCO: [u8; BOX_TYPE_SIZE] = *b"ipco";
/// Four-CC of the Item Property Association Box (§8.11.14).
pub const IPMA: [u8; BOX_TYPE_SIZE] = *b"ipma";
/// Four-CC of the Item Data Box (§8.11.11).
pub const IDAT: [u8; BOX_TYPE_SIZE] = *b"idat";
/// Four-CC of the Item Protection Box (§8.11.5).
pub const IPRO: [u8; BOX_TYPE_SIZE] = *b"ipro";
/// Four-CC of the Groups List Box (§8.15.2).
pub const GRPL: [u8; BOX_TYPE_SIZE] = *b"grpl";
/// Four-CC of the Brand Property (§8.11.15).
pub const BRND: [u8; BOX_TYPE_SIZE] = *b"brnd";
/// Four-CC of the Handler Property (§8.11.16).
pub const HDLP: [u8; BOX_TYPE_SIZE] = *b"hdlp";

/// Bits in a wire byte.
const BITS_PER_BYTE: u32 = 8;
/// Width of one 4-bit field-size nibble.
const NIBBLE_BITS: u32 = 4;
/// Mask of one 4-bit field-size nibble.
const NIBBLE_MASK: u8 = 0x0f;
/// Width of `construction_method` (§8.11.3.3).
const CONSTRUCTION_METHOD_BITS: u32 = 4;
/// The only field sizes §8.11.3.2.1 allows.
const FIELD_SIZES: [u8; 3] = [0, 4, 8];
/// Width of the `offset_size` byte pair.
const FIELD_SIZE_NIBBLES_LEN: usize = 2;
/// Width of a 16-bit `item_ID` / count.
const U16_LEN: usize = core::mem::size_of::<u16>();
/// Width of a 32-bit `item_ID` / count.
const U32_LEN: usize = core::mem::size_of::<u32>();
/// Width of a 64-bit field.
const U64_LEN: usize = core::mem::size_of::<u64>();
/// Value of `construction_method` naming an absolute file offset (§8.11.3.2.2).
const CONSTRUCTION_FILE: u8 = 0;
/// Value of `construction_method` naming an `idat` offset (§8.11.3.2.2).
const CONSTRUCTION_IDAT: u8 = 1;
/// Value of `construction_method` naming another item's data (§8.11.3.2.2).
const CONSTRUCTION_ITEM: u8 = 2;
/// The `(flags & 1)` bit that marks an item hidden (§8.11.6.1).
const ITEM_FLAG_HIDDEN: u32 = 1;
/// The `(flags & 1)` bit that widens an `ipma` property index to 15 bits.
const IPMA_WIDE_INDEX: u32 = 1;
/// Bits of the `ipma` `essential` flag.
const IPMA_ESSENTIAL_BITS: u32 = 1;
/// Bits of the narrow `ipma` `property_index`.
const IPMA_NARROW_INDEX_BITS: u32 = 7;
/// Bits of the wide `ipma` `property_index`.
const IPMA_WIDE_INDEX_BITS: u32 = 15;
/// The `iloc` version that may widen `item_ID` to 32 bits (§8.11.3.3).
pub const ILOC_VERSION_WIDE_ID: u8 = 2;
/// `iloc` versions 1 and 2 carry `construction_method` and `index_size`.
const ILOC_VERSION_CONSTRUCTION: u8 = 1;
/// `iinf`/`infe` version from which `item_ID` may be 32 bits (§8.11.6.2).
const INFE_VERSION_WIDE_ID: u8 = 3;
/// `iinf`/`infe` version from which `item_type` is present (§8.11.6.2).
const INFE_VERSION_TYPED: u8 = 2;
/// `iref` version carrying 32-bit item IDs (§8.11.12.2).
const IREF_VERSION_WIDE_ID: u8 = 1;
/// `ipma` version carrying 32-bit item IDs (§8.11.14.2).
const IPMA_VERSION_WIDE_ID: u8 = 1;
/// Bytes of an entity group's `group_id` and `num_entities_in_group`.
const ENTITY_GROUP_FIXED_LEN: usize = U32_LEN * 2;
/// Four-CC of the `mime` item type (§8.11.6.3).
pub const ITEM_TYPE_MIME: [u8; BOX_TYPE_SIZE] = *b"mime";
/// Four-CC of the `uri ` item type (§8.11.6.3).
pub const ITEM_TYPE_URI: [u8; BOX_TYPE_SIZE] = *b"uri ";

/// Parse a whole box's body after its header, borrowing nothing.
fn body_of<'a>(bytes: &'a [u8], min_body: usize, what: &'static str) -> Result<&'a [u8]> {
    crate::box_types::box_body(bytes, min_body, what)
}

/// Check that a whole box carries `fourcc` at `[4:8]`.
fn expect_type(bytes: &[u8], fourcc: &[u8; BOX_TYPE_SIZE], what: &'static str) -> Result<()> {
    need_in(bytes, BOX_HEADER_MIN_SIZE, what)?;
    if u32_at(bytes, BOX_TYPE_OFFSET) != u32::from_be_bytes(*fourcc) {
        return Err(Error::InvalidInput(what));
    }
    Ok(())
}

/// Bytes a field of the declared `size` occupies, refusing anything but the
/// §8.11.3.2.1 set.
fn field_width(size: u8, what: &'static str) -> Result<usize> {
    if FIELD_SIZES.contains(&size) {
        Ok(usize::from(size))
    } else {
        Err(Error::InvalidValue(InvalidValue {
            field: what,
            value: u64::from(size),
            reason: "field size shall be 0, 4 or 8",
        }))
    }
}

/// Read one `size`-byte big-endian field at `*cursor` (0 bytes reads 0).
fn read_sized(bytes: &[u8], cursor: &mut usize, size: usize, what: &'static str) -> Result<u64> {
    let width = field_width(size_byte(size), what)?;
    let value = match width {
        0 => 0,
        U32_LEN => u64::from(be_u32(bytes, *cursor, what)?),
        U64_LEN => be_u64(bytes, *cursor, what)?,
        _ => return Err(Error::InvalidInput(what)),
    };
    *cursor += width;
    Ok(value)
}

/// Narrow a validated width back to the nibble it came from.
fn size_byte(size: usize) -> u8 {
    match size {
        0 => 0,
        4 => 4,
        8 => 8,
        _ => u8::MAX,
    }
}

/// Write one `size`-byte big-endian field at `*cursor`.
fn write_sized(
    buf: &mut [u8],
    cursor: &mut usize,
    size: usize,
    value: u64,
    what: &'static str,
) -> Result<()> {
    let width = field_width(size_byte(size), what)?;
    if width == 0 {
        if value != 0 {
            return Err(Error::InvalidInput(what));
        }
        return Ok(());
    }
    broadcast_core::len::fit_bits(value, (width as u32) * BITS_PER_BYTE, what)?;
    need_out(buf, *cursor + width)?;
    match width {
        U32_LEN => {
            let narrow = u32::try_from(value).map_err(|_| Error::InvalidInput(what))?;
            buf[*cursor..*cursor + U32_LEN].copy_from_slice(&narrow.to_be_bytes());
        }
        U64_LEN => buf[*cursor..*cursor + U64_LEN].copy_from_slice(&value.to_be_bytes()),
        _ => {}
    }
    *cursor += width;
    Ok(())
}

/// The identifier of an item inside one `MetaBox` scope (§8.11.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ItemId(u32);

impl ItemId {
    /// An item identifier.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// The identifier.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Whether this identifier needs the 32-bit wire form (it does not fit 16
    /// bits).
    #[must_use]
    pub const fn needs_wide(self) -> bool {
        self.0 > u16::MAX as u32
    }
}

/// A 1-based index into the `ItemPropertyContainerBox` (§8.11.14.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PropertyIndex(u16);

impl PropertyIndex {
    /// Index `value` (1-based; 0 means "no property").
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// The index.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Whether the index fits the narrow (7-bit) `ipma` form.
    #[must_use]
    pub const fn fits_narrow(self) -> bool {
        (self.0 as u32) < (1u32 << IPMA_NARROW_INDEX_BITS)
    }
}

/// The identifier of an entity group (§8.15.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GroupId(u32);

impl GroupId {
    /// A group identifier.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// The identifier.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// How an item's extents address its data — §8.11.3.2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ConstructionMethod {
    /// `0`: absolute byte offsets into the file (or the referenced file).
    #[default]
    File,
    /// `1`: offsets into the `ItemDataBox` in the same `MetaBox`.
    Idat,
    /// `2`: offsets into the concatenated data of another item.
    Item,
}

impl ConstructionMethod {
    /// The `construction_method` value.
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::File => CONSTRUCTION_FILE,
            Self::Idat => CONSTRUCTION_IDAT,
            Self::Item => CONSTRUCTION_ITEM,
        }
    }

    /// The value named by `value`, or `None` when it is reserved.
    #[must_use]
    pub const fn from_value(value: u8) -> Option<Self> {
        match value {
            CONSTRUCTION_FILE => Some(Self::File),
            CONSTRUCTION_IDAT => Some(Self::Idat),
            CONSTRUCTION_ITEM => Some(Self::Item),
            _ => None,
        }
    }

    /// Label for the method, per the #204 convention.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Idat => "idat",
            Self::Item => "item",
        }
    }
}

broadcast_core::impl_spec_display!(ConstructionMethod);

/// One item extent: a contiguous run of bytes of an item's data (§8.11.3.2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemExtent {
    /// The 1-based `item_reference_index` of an `iloc` reference, when the
    /// version and `index_size` carry one.
    pub(crate) item_reference_index: Option<u32>,
    /// `extent_offset` (0 when the field is absent).
    pub(crate) offset: u64,
    /// `extent_length` (0 when the field is absent).
    pub(crate) length: u64,
}

impl ItemExtent {
    /// An extent at `offset` of `length` bytes.
    #[must_use]
    pub const fn new(offset: u64, length: u64) -> Self {
        Self {
            item_reference_index: None,
            offset,
            length,
        }
    }

    /// An extent carrying an `item_reference_index`.
    #[must_use]
    pub const fn with_item_reference(offset: u64, length: u64, index: u32) -> Self {
        Self {
            item_reference_index: Some(index),
            offset,
            length,
        }
    }

    /// The `item_reference_index`, when present.
    #[must_use]
    pub const fn item_reference_index(&self) -> Option<u32> {
        self.item_reference_index
    }

    /// `extent_offset`.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// `extent_length`.
    #[must_use]
    pub const fn length(&self) -> u64 {
        self.length
    }
}

/// One item's entry in the [`ItemLocationBox`] (§8.11.3.3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemLocationEntry {
    /// `item_ID`.
    pub(crate) item_id: ItemId,
    /// `construction_method` (0 for versions 0).
    pub(crate) construction_method: ConstructionMethod,
    /// `data_reference_index` (0 addresses the same file).
    pub(crate) data_reference_index: u16,
    /// `base_offset`.
    pub(crate) base_offset: u64,
    /// The extents, in order.
    pub(crate) extents: Vec<ItemExtent>,
}

impl ItemLocationEntry {
    /// An entry with one extent under the given construction method.
    #[must_use]
    pub fn new(
        item_id: ItemId,
        construction_method: ConstructionMethod,
        extents: Vec<ItemExtent>,
    ) -> Self {
        Self {
            item_id,
            construction_method,
            data_reference_index: 0,
            base_offset: 0,
            extents,
        }
    }

    /// Set `data_reference_index`.
    #[must_use]
    pub fn with_data_reference_index(mut self, index: u16) -> Self {
        self.data_reference_index = index;
        self
    }

    /// Set `base_offset`.
    #[must_use]
    pub fn with_base_offset(mut self, base_offset: u64) -> Self {
        self.base_offset = base_offset;
        self
    }

    /// `item_ID`.
    #[must_use]
    pub const fn item_id(&self) -> ItemId {
        self.item_id
    }

    /// The construction method.
    #[must_use]
    pub const fn construction_method(&self) -> ConstructionMethod {
        self.construction_method
    }

    /// `data_reference_index`.
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// `base_offset`.
    #[must_use]
    pub const fn base_offset(&self) -> u64 {
        self.base_offset
    }

    /// The extents, in order.
    #[must_use]
    pub fn extents(&self) -> &[ItemExtent] {
        &self.extents
    }

    /// Sum of the extent lengths.
    #[must_use]
    pub fn data_len(&self) -> u64 {
        self.extents
            .iter()
            .fold(0, |acc, e| acc.saturating_add(e.length))
    }
}

/// Item Location Box (`iloc`) — ISO/IEC 14496-12:2026 §8.11.3.
///
/// The directory of where each item's bytes live. The field sizes are kept as
/// parsed so the serializer re-emits exactly the same width choices, and a
/// zero-width field is written as absent (the spec's implied value).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemLocationBox {
    version: u8,
    flags: u32,
    offset_size: u8,
    length_size: u8,
    base_offset_size: u8,
    index_size: u8,
    items: Vec<ItemLocationEntry>,
}

impl ItemLocationBox {
    /// A version-0 box with 32-bit offsets and lengths, no base offset and no
    /// item-reference index — the layout `mif1` prefers (§8.11.3.2.5).
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: 0,
            flags: 0,
            offset_size: 4,
            length_size: 4,
            base_offset_size: 0,
            index_size: 0,
            items: Vec::new(),
        }
    }

    /// A box of the given version with this crate's default field sizes.
    #[must_use]
    pub fn with_version(version: u8) -> Self {
        let mut this = Self::new();
        this.version = version;
        this
    }

    /// Replace the field widths (`{0, 4, 8}` each).
    pub fn set_field_sizes(
        &mut self,
        offset_size: u8,
        length_size: u8,
        base_offset_size: u8,
        index_size: u8,
    ) -> Result<()> {
        for (size, what) in [
            (offset_size, "iloc offset_size"),
            (length_size, "iloc length_size"),
            (base_offset_size, "iloc base_offset_size"),
            (index_size, "iloc index_size"),
        ] {
            field_width(size, what)?;
        }
        self.offset_size = offset_size;
        self.length_size = length_size;
        self.base_offset_size = base_offset_size;
        self.index_size = index_size;
        Ok(())
    }

    /// Append an item entry.
    pub fn push(&mut self, entry: ItemLocationEntry) {
        self.items.push(entry);
    }

    /// The FullBox `version`.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox `flags`.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The `offset_size` field width.
    #[must_use]
    pub const fn offset_size(&self) -> u8 {
        self.offset_size
    }

    /// The `length_size` field width.
    #[must_use]
    pub const fn length_size(&self) -> u8 {
        self.length_size
    }

    /// The `base_offset_size` field width.
    #[must_use]
    pub const fn base_offset_size(&self) -> u8 {
        self.base_offset_size
    }

    /// The `index_size` field width.
    #[must_use]
    pub const fn index_size(&self) -> u8 {
        self.index_size
    }

    /// The item entries, in wire order.
    #[must_use]
    pub fn items(&self) -> &[ItemLocationEntry] {
        &self.items
    }

    /// One item's entry.
    #[must_use]
    pub fn item(&self, id: ItemId) -> Option<&ItemLocationEntry> {
        self.items.iter().find(|entry| entry.item_id == id)
    }

    fn super_type(&self) -> u8 {
        if self.version >= ILOC_VERSION_WIDE_ID {
            ILOC_VERSION_WIDE_ID
        } else {
            0
        }
    }

    fn id_width(&self) -> usize {
        match self.super_type() {
            ILOC_VERSION_WIDE_ID => U32_LEN,
            _ => U16_LEN,
        }
    }

    fn has_construction(&self) -> bool {
        self.version >= ILOC_VERSION_CONSTRUCTION
    }

    fn entry_min_len(&self) -> usize {
        self.id_width()
            + if self.has_construction() { U16_LEN } else { 0 }
            + U16_LEN
            + usize::from(self.base_offset_size)
            + U16_LEN
    }

    fn extent_len(&self) -> usize {
        usize::from(self.offset_size)
            + usize::from(self.length_size)
            + if self.has_construction() {
                usize::from(self.index_size)
            } else {
                0
            }
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + FIELD_SIZE_NIBBLES_LEN,
            "iloc body",
        )?;
        let version = body[0];
        let flags = u24_at(body, crate::box_types::FULLBOX_FLAGS_OFFSET) & FULLBOX_FLAGS_MASK;
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let offset_size = body[cursor] >> NIBBLE_BITS;
        let length_size = body[cursor] & NIBBLE_MASK;
        cursor += 1;
        let base_offset_size = body[cursor] >> NIBBLE_BITS;
        let fourth = body[cursor] & NIBBLE_MASK;
        cursor += 1;
        let index_size = if version >= ILOC_VERSION_CONSTRUCTION {
            fourth
        } else {
            0
        };
        for (size, what) in [
            (offset_size, "iloc offset_size"),
            (length_size, "iloc length_size"),
            (base_offset_size, "iloc base_offset_size"),
            (index_size, "iloc index_size"),
        ] {
            field_width(size, what)?;
        }
        let count_width = if version >= ILOC_VERSION_WIDE_ID {
            U32_LEN
        } else {
            U16_LEN
        };
        need_in(body, cursor + count_width, "iloc item_count")?;
        let item_count = if version >= ILOC_VERSION_WIDE_ID {
            let value = be_u32(body, cursor, "iloc item_count")? as usize;
            cursor += U32_LEN;
            value
        } else {
            let value = usize::from(be_u16(body, cursor, "iloc item_count")?);
            cursor += U16_LEN;
            value
        };
        let mut this = Self {
            version,
            flags,
            offset_size,
            length_size,
            base_offset_size,
            index_size,
            items: Vec::new(),
        };
        let remaining = body.len().saturating_sub(cursor);
        crate::box_support::check_entry_count(
            remaining,
            this.entry_min_len(),
            item_count,
            "iloc item_count",
        )?;
        this.items.reserve(crate::box_support::bounded_entry_count(
            remaining,
            this.entry_min_len(),
            item_count,
        ));
        for _ in 0..item_count {
            let item_id = this.read_id(body, &mut cursor)?;
            let construction_method = if this.has_construction() {
                let packed = be_u16(body, cursor, "iloc construction_method")?;
                cursor += U16_LEN;
                let reserved = packed >> CONSTRUCTION_METHOD_BITS;
                if reserved != 0 {
                    return Err(Error::InvalidValue(InvalidValue {
                        field: "iloc reserved",
                        value: u64::from(reserved),
                        reason: "reserved bits shall be zero",
                    }));
                }
                let value = u8::try_from(packed & u16::from(NIBBLE_MASK))
                    .map_err(|_| Error::InvalidInput("iloc construction_method"))?;
                ConstructionMethod::from_value(value)
                    .ok_or(Error::InvalidInput("iloc construction_method"))?
            } else {
                ConstructionMethod::File
            };
            let data_reference_index = be_u16(body, cursor, "iloc data_reference_index")?;
            cursor += U16_LEN;
            let base_offset = read_sized(
                body,
                &mut cursor,
                usize::from(base_offset_size),
                "iloc base_offset",
            )?;
            let extent_count = usize::from(be_u16(body, cursor, "iloc extent_count")?);
            cursor += U16_LEN;
            if extent_count == 0 {
                return Err(Error::InvalidValue(InvalidValue {
                    field: "iloc extent_count",
                    value: 0,
                    reason: "an item shall have at least one extent",
                }));
            }
            if (offset_size == 0 || length_size == 0) && extent_count > 1 {
                return Err(Error::InvalidValue(InvalidValue {
                    field: "iloc field size",
                    value: extent_count as u64,
                    reason: "a zero-width offset or length is only meaningful for one extent",
                }));
            }
            crate::box_support::check_entry_count(
                body.len().saturating_sub(cursor),
                this.extent_len(),
                extent_count,
                "iloc extent_count",
            )?;
            let mut extents = Vec::with_capacity(crate::box_support::bounded_entry_count(
                body.len().saturating_sub(cursor),
                this.extent_len(),
                extent_count,
            ));
            for _ in 0..extent_count {
                let item_reference_index = if this.has_construction() && index_size > 0 {
                    let index = read_sized(
                        body,
                        &mut cursor,
                        usize::from(index_size),
                        "iloc item_reference_index",
                    )?;
                    Some(u32::try_from(index).map_err(|_| {
                        Error::InvalidInput("iloc item_reference_index exceeds u32")
                    })?)
                } else {
                    None
                };
                let offset = read_sized(
                    body,
                    &mut cursor,
                    usize::from(offset_size),
                    "iloc extent_offset",
                )?;
                let length = read_sized(
                    body,
                    &mut cursor,
                    usize::from(length_size),
                    "iloc extent_length",
                )?;
                extents.push(ItemExtent {
                    item_reference_index,
                    offset,
                    length,
                });
            }
            this.items.push(ItemLocationEntry {
                item_id,
                construction_method,
                data_reference_index,
                base_offset,
                extents,
            });
        }
        Ok(this)
    }

    fn read_id(&self, body: &[u8], cursor: &mut usize) -> Result<ItemId> {
        if self.super_type() == ILOC_VERSION_WIDE_ID {
            let value = be_u32(body, *cursor, "iloc item_ID")?;
            *cursor += U32_LEN;
            Ok(ItemId::new(value))
        } else {
            let value = be_u16(body, *cursor, "iloc item_ID")?;
            *cursor += U16_LEN;
            Ok(ItemId::new(u32::from(value)))
        }
    }

    fn body_len(&self) -> usize {
        let mut len = FULLBOX_EXTRA_SIZE + FIELD_SIZE_NIBBLES_LEN + self.id_width();
        len += self.items.len() * self.entry_min_len();
        for entry in &self.items {
            len += entry.extents.len() * self.extent_len();
        }
        len
    }
}

impl<'a> Parse<'a> for ItemLocationBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &ILOC, "expected iloc")?;
        Self::parse_body(body_of(bytes, FULLBOX_EXTRA_SIZE, "iloc")?)
    }
}

impl Serialize for ItemLocationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &ILOC, need, self.version, self.flags, false)?;
        buf[cursor] = (self.offset_size << NIBBLE_BITS) | (self.length_size & NIBBLE_MASK);
        cursor += 1;
        let fourth = if self.has_construction() {
            self.index_size
        } else {
            0
        };
        buf[cursor] = (self.base_offset_size << NIBBLE_BITS) | (fourth & NIBBLE_MASK);
        cursor += 1;
        if self.super_type() == ILOC_VERSION_WIDE_ID {
            buf[cursor..cursor + U32_LEN]
                .copy_from_slice(&fit_u32(self.items.len(), "iloc item_count")?.to_be_bytes());
            cursor += U32_LEN;
        } else {
            let count = u16::try_from(self.items.len())
                .map_err(|_| Error::InvalidInput("iloc item_count exceeds u16"))?;
            buf[cursor..cursor + U16_LEN].copy_from_slice(&count.to_be_bytes());
            cursor += U16_LEN;
        }
        for entry in &self.items {
            if self.super_type() == ILOC_VERSION_WIDE_ID {
                buf[cursor..cursor + U32_LEN].copy_from_slice(&entry.item_id.get().to_be_bytes());
                cursor += U32_LEN;
            } else {
                let id = u16::try_from(entry.item_id.get())
                    .map_err(|_| Error::InvalidInput("iloc item_ID exceeds u16"))?;
                buf[cursor..cursor + U16_LEN].copy_from_slice(&id.to_be_bytes());
                cursor += U16_LEN;
            }
            if self.has_construction() {
                let packed = u16::from(entry.construction_method.value());
                buf[cursor..cursor + U16_LEN].copy_from_slice(&packed.to_be_bytes());
                cursor += U16_LEN;
            }
            buf[cursor..cursor + U16_LEN]
                .copy_from_slice(&entry.data_reference_index.to_be_bytes());
            cursor += U16_LEN;
            write_sized(
                buf,
                &mut cursor,
                usize::from(self.base_offset_size),
                entry.base_offset,
                "iloc base_offset",
            )?;
            let extents = u16::try_from(entry.extents.len())
                .map_err(|_| Error::InvalidInput("iloc extent_count exceeds u16"))?;
            buf[cursor..cursor + U16_LEN].copy_from_slice(&extents.to_be_bytes());
            cursor += U16_LEN;
            for extent in &entry.extents {
                if self.has_construction() && self.index_size > 0 {
                    write_sized(
                        buf,
                        &mut cursor,
                        usize::from(self.index_size),
                        u64::from(extent.item_reference_index.unwrap_or(0)),
                        "iloc item_reference_index",
                    )?;
                }
                write_sized(
                    buf,
                    &mut cursor,
                    usize::from(self.offset_size),
                    extent.offset,
                    "iloc extent_offset",
                )?;
                write_sized(
                    buf,
                    &mut cursor,
                    usize::from(self.length_size),
                    extent.length,
                    "iloc extent_length",
                )?;
            }
        }
        Ok(cursor)
    }
}

/// The `item_type` four-CC of an [`ItemInfoEntry`] (§8.11.6.3).
///
/// The coded-image, derived-image and metadata types are the ones ISO/IEC
/// 23008-12:2025, Amendment 1 and the AVIF specification define; any other type is kept as
/// [`ItemType::Other`] so it round-trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ItemType {
    /// `hvc1`: a HEVC coded image item (ISO/IEC 23008-12:2025 B.2).
    Hvc1,
    /// `avc1`: an AVC coded image item (ISO/IEC 23008-12:2025 E.2).
    Avc1,
    /// `av01`: an AV1 coded image item (AVIF §2.1).
    Av01,
    /// `grid`: an image grid derived image item (ISO/IEC 23008-12:2025 6.6.2.3).
    Grid,
    /// `iovl`: an image overlay derived image item (6.6.2.2).
    Iovl,
    /// `iden`: an identity derived image item (6.6.2.1).
    Iden,
    /// `tmap`: a tone-map derived image item (Amd 1 6.6.2.4).
    Tmap,
    /// `cfen`: a colour format enhancement derived image item (Amd 1 6.6.2.5).
    Cfen,
    /// `Exif`: an Exif metadata item (ISO/IEC 23008-12:2025 A.2).
    Exif,
    /// `mime`: an item whose type is its MIME type.
    Mime,
    /// `uri `: an item whose type is a URI.
    Uri,
    /// `fdel`: FD item information.
    Fdel,
    /// `it35`: an ITU-T T.35 message item (§8.11.17).
    It35,
    /// `mint`: a data-integrity metadata item (ISO/IEC 23008-12:2025 9.8).
    Mint,
    /// `mski`: a mask item (23008-12 11.2.2).
    Mski,
    /// `rgan`: a region item (11.3.2).
    Rgan,
    /// Any other registered or private type.
    Other([u8; BOX_TYPE_SIZE]),
}

impl Default for ItemType {
    fn default() -> Self {
        Self::Other([0; BOX_TYPE_SIZE])
    }
}

impl ItemType {
    /// The four-CC.
    #[must_use]
    pub const fn fourcc(self) -> [u8; BOX_TYPE_SIZE] {
        match self {
            Self::Hvc1 => *b"hvc1",
            Self::Avc1 => *b"avc1",
            Self::Av01 => *b"av01",
            Self::Grid => *b"grid",
            Self::Iovl => *b"iovl",
            Self::Iden => *b"iden",
            Self::Tmap => *b"tmap",
            Self::Cfen => *b"cfen",
            Self::Exif => *b"Exif",
            Self::Mime => ITEM_TYPE_MIME,
            Self::Uri => ITEM_TYPE_URI,
            Self::Fdel => *b"fdel",
            Self::It35 => *b"it35",
            Self::Mint => *b"mint",
            Self::Mski => *b"mski",
            Self::Rgan => *b"rgan",
            Self::Other(fourcc) => fourcc,
        }
    }

    /// The type named by a four-CC.
    #[must_use]
    pub const fn from_fourcc(fourcc: [u8; BOX_TYPE_SIZE]) -> Self {
        match &fourcc {
            b"hvc1" => Self::Hvc1,
            b"avc1" => Self::Avc1,
            b"av01" => Self::Av01,
            b"grid" => Self::Grid,
            b"iovl" => Self::Iovl,
            b"iden" => Self::Iden,
            b"tmap" => Self::Tmap,
            b"cfen" => Self::Cfen,
            b"Exif" => Self::Exif,
            b"mime" => Self::Mime,
            b"uri " => Self::Uri,
            b"fdel" => Self::Fdel,
            b"it35" => Self::It35,
            b"mint" => Self::Mint,
            b"mski" => Self::Mski,
            b"rgan" => Self::Rgan,
            _ => Self::Other(fourcc),
        }
    }

    /// Whether this is a coded image item.
    #[must_use]
    pub const fn is_coded_image(self) -> bool {
        matches!(self, Self::Hvc1 | Self::Avc1 | Self::Av01)
    }

    /// Whether this is a derived image item.
    #[must_use]
    pub const fn is_derived_image(self) -> bool {
        matches!(
            self,
            Self::Grid | Self::Iovl | Self::Iden | Self::Tmap | Self::Cfen
        )
    }

    /// Label for the type, per the #204 convention.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Hvc1 => "hvc1",
            Self::Avc1 => "avc1",
            Self::Av01 => "av01",
            Self::Grid => "grid",
            Self::Iovl => "iovl",
            Self::Iden => "iden",
            Self::Tmap => "tmap",
            Self::Cfen => "cfen",
            Self::Exif => "Exif",
            Self::Mime => "mime",
            Self::Uri => "uri",
            Self::Fdel => "fdel",
            Self::It35 => "it35",
            Self::Mint => "mint",
            Self::Mski => "mski",
            Self::Rgan => "rgan",
            Self::Other(_) => "other",
        }
    }
}

broadcast_core::impl_spec_display!(ItemType);

/// The `reference_type` of a [`SingleItemTypeReferenceBox`] (§8.11.12.3).
///
/// The image reference types come from ISO/IEC 23008-12:2025 6.4/6.6; `iloc`
/// is §8.11.3.2.3. Any other type is kept verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ReferenceType {
    /// `thmb`: the from item is a thumbnail of the to item (23008-12 6.4.4).
    Thmb,
    /// `auxl`: the from item is an auxiliary image of the to item (6.4.5).
    Auxl,
    /// `dimg`: the to item is an input of the derived from item (6.6.1).
    Dimg,
    /// `cdsc`: the from item describes the to item (6.7).
    Cdsc,
    /// `base`: the from item was derived from the to item (6.4.7).
    Base,
    /// `tbas`: the from item's pixels are positioned relative to the to item
    /// (6.5.7: the `rloc` property).
    Tbas,
    /// `exbl`: the to item is a decoded reference for the from item (6.4.8).
    Exbl,
    /// `iloc`: an item-reference index used by `construction_method` 2
    /// (§8.11.3.2.3).
    Iloc,
    /// `font`: the from item uses fonts carried by the to item (§8.11.12.1).
    Font,
    /// `fdel`: FD item information (§8.11.6.1).
    Fdel,
    /// `mint`: the from item is a data-integrity item for the to item
    /// (ISO/IEC 23008-12:2025 9.8).
    Mint,
    /// `pred`: the to item is a predictive reference for the from item
    /// (23008-12 10.2.4).
    Pred,
    /// `prem`: the from item is a premultiplied-alpha variant of the to item
    /// (10.2.3).
    Prem,
    /// `dpnd`: the from deductive item depends on the to deductive items
    /// (8.2.2.2).
    Dpnd,
    /// `mask`: the to item is the mask a region item references (11.2.1.1).
    Mask,
    /// `eroi`: the from region item's annotation is the to item (11.3.1).
    Eroi,
    /// `drgn`: the to item is an input of the derived region from item
    /// (11.3.3).
    Drgn,
    /// `rpds`: the from image item associates a region partition group
    /// (Amd 1 6.8.13.1).
    Rpds,
    /// `anot`: the to item or track carries a region annotation (11.4.1).
    Anot,
    /// Any other registered or private type.
    Other([u8; BOX_TYPE_SIZE]),
}

impl Default for ReferenceType {
    fn default() -> Self {
        Self::Other([0; BOX_TYPE_SIZE])
    }
}

impl ReferenceType {
    /// The four-CC.
    #[must_use]
    pub const fn fourcc(self) -> [u8; BOX_TYPE_SIZE] {
        match self {
            Self::Thmb => *b"thmb",
            Self::Auxl => *b"auxl",
            Self::Dimg => *b"dimg",
            Self::Cdsc => *b"cdsc",
            Self::Base => *b"base",
            Self::Tbas => *b"tbas",
            Self::Exbl => *b"exbl",
            Self::Iloc => *b"iloc",
            Self::Font => *b"font",
            Self::Pred => *b"pred",
            Self::Prem => *b"prem",
            Self::Dpnd => *b"dpnd",
            Self::Mask => *b"mask",
            Self::Eroi => *b"eroi",
            Self::Drgn => *b"drgn",
            Self::Rpds => *b"rpds",
            Self::Anot => *b"anot",
            Self::Fdel => *b"fdel",
            Self::Mint => *b"mint",
            Self::Other(fourcc) => fourcc,
        }
    }

    /// The reference type named by a four-CC.
    #[must_use]
    pub const fn from_fourcc(fourcc: [u8; BOX_TYPE_SIZE]) -> Self {
        match &fourcc {
            b"thmb" => Self::Thmb,
            b"auxl" => Self::Auxl,
            b"dimg" => Self::Dimg,
            b"cdsc" => Self::Cdsc,
            b"base" => Self::Base,
            b"tbas" => Self::Tbas,
            b"exbl" => Self::Exbl,
            b"iloc" => Self::Iloc,
            b"font" => Self::Font,
            b"pred" => Self::Pred,
            b"prem" => Self::Prem,
            b"dpnd" => Self::Dpnd,
            b"mask" => Self::Mask,
            b"eroi" => Self::Eroi,
            b"drgn" => Self::Drgn,
            b"rpds" => Self::Rpds,
            b"anot" => Self::Anot,
            b"fdel" => Self::Fdel,
            b"mint" => Self::Mint,
            _ => Self::Other(fourcc),
        }
    }

    /// Label for the reference type, per the #204 convention.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Thmb => "thmb",
            Self::Auxl => "auxl",
            Self::Dimg => "dimg",
            Self::Cdsc => "cdsc",
            Self::Base => "base",
            Self::Tbas => "tbas",
            Self::Exbl => "exbl",
            Self::Iloc => "iloc",
            Self::Font => "font",
            Self::Pred => "pred",
            Self::Prem => "prem",
            Self::Dpnd => "dpnd",
            Self::Mask => "mask",
            Self::Eroi => "eroi",
            Self::Drgn => "drgn",
            Self::Rpds => "rpds",
            Self::Anot => "anot",
            Self::Fdel => "fdel",
            Self::Mint => "mint",
            Self::Other(_) => "other",
        }
    }
}

broadcast_core::impl_spec_display!(ReferenceType);

/// One entry of the [`ItemInfoBox`] (`infe`) — ISO/IEC 14496-12:2026 §8.11.6.
///
/// Versions 0 and 1 carry symbolic names and MIME types; versions 2 and 3
/// carry a typed `item_type` and a 16- or 32-bit identifier. The version and
/// the shape it implies are both kept so the entry re-serializes byte-exactly,
/// including any trailing extension bytes the parser did not interpret.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemInfoEntry {
    version: u8,
    flags: u32,
    item_id: ItemId,
    item_protection_index: u16,
    item_type: ItemType,
    item_name: Vec<u8>,
    content_type: Option<Vec<u8>>,
    content_encoding: Option<Vec<u8>>,
    item_uri_type: Option<Vec<u8>>,
    extension_type: Option<[u8; BOX_TYPE_SIZE]>,
    trailing: Vec<u8>,
}

impl ItemInfoEntry {
    /// A version-2 entry with no protection and an empty name.
    #[must_use]
    pub fn new(item_id: ItemId, item_type: ItemType, item_name: &[u8]) -> Self {
        Self {
            version: INFE_VERSION_TYPED,
            flags: 0,
            item_id,
            item_protection_index: 0,
            item_type,
            item_name: item_name.to_vec(),
            content_type: None,
            content_encoding: None,
            item_uri_type: None,
            extension_type: None,
            trailing: Vec::new(),
        }
    }

    /// Set the FullBox version (0, 1, 2 or 3).
    #[must_use]
    pub fn with_version(mut self, version: u8) -> Self {
        self.version = version;
        self
    }

    /// Choose the typed-entry version this item's identifier needs: 2 when it
    /// fits 16 bits, 3 when it does not (§8.11.6.2).
    #[must_use]
    pub fn with_selected_version(mut self) -> Self {
        self.version = if self.item_id.needs_wide() {
            INFE_VERSION_WIDE_ID
        } else {
            INFE_VERSION_TYPED
        };
        self
    }

    /// Set the hidden flag (§8.11.6.1) for a typed entry.
    #[must_use]
    pub fn with_hidden(mut self, hidden: bool) -> Self {
        if hidden {
            self.flags |= ITEM_FLAG_HIDDEN;
        } else {
            self.flags &= !ITEM_FLAG_HIDDEN;
        }
        self
    }

    /// Set `item_protection_index`.
    #[must_use]
    pub fn with_protection_index(mut self, index: u16) -> Self {
        self.item_protection_index = index;
        self
    }

    /// Set the MIME `content_type`.
    #[must_use]
    pub fn with_content_type(mut self, content_type: &[u8]) -> Self {
        self.content_type = Some(content_type.to_vec());
        self
    }

    /// Set the `content_encoding`.
    #[must_use]
    pub fn with_content_encoding(mut self, content_encoding: &[u8]) -> Self {
        self.content_encoding = Some(content_encoding.to_vec());
        self
    }

    /// Set the `item_uri_type`.
    #[must_use]
    pub fn with_item_uri_type(mut self, uri: &[u8]) -> Self {
        self.item_uri_type = Some(uri.to_vec());
        self
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

    /// `item_ID`.
    #[must_use]
    pub const fn item_id(&self) -> ItemId {
        self.item_id
    }

    /// `item_protection_index`.
    #[must_use]
    pub const fn item_protection_index(&self) -> u16 {
        self.item_protection_index
    }

    /// `item_type`.
    #[must_use]
    pub const fn item_type(&self) -> ItemType {
        self.item_type
    }

    /// `item_name` (an empty slice when the entry carries none).
    #[must_use]
    pub fn item_name(&self) -> &[u8] {
        &self.item_name
    }

    /// `item_name` as UTF-8, when it is.
    #[must_use]
    pub fn item_name_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.item_name).ok()
    }

    /// `content_type`, when the entry carries one.
    #[must_use]
    pub fn content_type(&self) -> Option<&[u8]> {
        self.content_type.as_deref()
    }

    /// `content_encoding`, when the entry carries one.
    #[must_use]
    pub fn content_encoding(&self) -> Option<&[u8]> {
        self.content_encoding.as_deref()
    }

    /// `item_uri_type`, when the entry carries one.
    #[must_use]
    pub fn item_uri_type(&self) -> Option<&[u8]> {
        self.item_uri_type.as_deref()
    }

    /// Whether `(flags & 1)` marks the item hidden (versions 2 and above).
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.flags & ITEM_FLAG_HIDDEN != 0
    }

    fn is_typed(&self) -> bool {
        self.version >= INFE_VERSION_TYPED
    }

    fn id_width(&self) -> usize {
        if self.version == INFE_VERSION_WIDE_ID {
            U32_LEN
        } else {
            U16_LEN
        }
    }

    fn parse_payload(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "infe body")?;
        let version = body[0];
        let id_width = if version == INFE_VERSION_WIDE_ID {
            U32_LEN
        } else {
            U16_LEN
        };
        let fixed = FULLBOX_EXTRA_SIZE
            + id_width
            + U16_LEN
            + if version >= INFE_VERSION_TYPED {
                BOX_TYPE_SIZE
            } else {
                0
            };
        need_in(body, fixed, "infe entry header")?;
        let flags = u24_at(body, crate::box_types::FULLBOX_FLAGS_OFFSET) & FULLBOX_FLAGS_MASK;
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let item_id = if version == INFE_VERSION_WIDE_ID {
            let value = be_u32(body, cursor, "infe item_ID")?;
            cursor += U32_LEN;
            ItemId::new(value)
        } else {
            let value = be_u16(body, cursor, "infe item_ID")?;
            cursor += U16_LEN;
            ItemId::new(u32::from(value))
        };
        let item_protection_index = be_u16(body, cursor, "infe item_protection_index")?;
        cursor += U16_LEN;
        if version < INFE_VERSION_TYPED {
            let item_name = crate::box_support::take_c_string(body, &mut cursor, "infe item_name")?;
            let content_type =
                crate::box_support::take_c_string(body, &mut cursor, "infe content_type")?;
            let content_encoding = if cursor < body.len() {
                Some(crate::box_support::take_c_string(
                    body,
                    &mut cursor,
                    "infe content_encoding",
                )?)
            } else {
                None
            };
            let (extension_type, trailing) = if version == 1 && cursor + U32_LEN <= body.len() {
                let mut fourcc = [0u8; BOX_TYPE_SIZE];
                fourcc.copy_from_slice(&body[cursor..cursor + BOX_TYPE_SIZE]);
                cursor += U32_LEN;
                (Some(fourcc), body[cursor..].to_vec())
            } else {
                (None, body[cursor..].to_vec())
            };
            return Ok(Self {
                version,
                flags,
                item_id,
                item_protection_index,
                item_type: ItemType::Other([0; BOX_TYPE_SIZE]),
                item_name,
                content_type: Some(content_type),
                content_encoding,
                item_uri_type: None,
                extension_type,
                trailing,
            });
        }
        let mut item_type_fourcc = [0u8; BOX_TYPE_SIZE];
        item_type_fourcc.copy_from_slice(&body[cursor..cursor + BOX_TYPE_SIZE]);
        let item_type = ItemType::from_fourcc(item_type_fourcc);
        cursor += BOX_TYPE_SIZE;
        let item_name = crate::box_support::take_c_string(body, &mut cursor, "infe item_name")?;
        let mut content_type = None;
        let mut content_encoding = None;
        let mut item_uri_type = None;
        if item_type.fourcc() == ITEM_TYPE_MIME {
            content_type = Some(crate::box_support::take_c_string(
                body,
                &mut cursor,
                "infe content_type",
            )?);
            if cursor < body.len() {
                content_encoding = Some(crate::box_support::take_c_string(
                    body,
                    &mut cursor,
                    "infe content_encoding",
                )?);
            }
        } else if item_type.fourcc() == ITEM_TYPE_URI {
            item_uri_type = Some(crate::box_support::take_c_string(
                body,
                &mut cursor,
                "infe item_uri_type",
            )?);
        }
        Ok(Self {
            version,
            flags,
            item_id,
            item_protection_index,
            item_type,
            item_name,
            content_type,
            content_encoding,
            item_uri_type,
            extension_type: None,
            trailing: body[cursor..].to_vec(),
        })
    }

    /// The file-delivery extension (`fdel`, §8.11.6.1) this version-1 entry
    /// carries, when its `extension_type` names one.
    pub fn fd_extension(&self) -> Result<Option<FdItemInfoExtension>> {
        match self.extension_type {
            Some(fourcc) if &fourcc == b"fdel" => {
                Ok(Some(FdItemInfoExtension::parse(&self.trailing)?))
            }
            _ => Ok(None),
        }
    }

    fn payload_len(&self) -> usize {
        let mut len = FULLBOX_EXTRA_SIZE + self.id_width() + U16_LEN;
        if self.is_typed() {
            len += BOX_TYPE_SIZE;
        }
        len += self.item_name.len() + 1;
        if self.is_typed() {
            if self.item_type.fourcc() == ITEM_TYPE_MIME {
                len += self
                    .content_type
                    .as_ref()
                    .map_or(1, |value| value.len() + 1);
                if let Some(encoding) = &self.content_encoding {
                    len += encoding.len() + 1;
                }
            } else if self.item_type.fourcc() == ITEM_TYPE_URI {
                len += self
                    .item_uri_type
                    .as_ref()
                    .map_or(1, |value| value.len() + 1);
            }
        } else {
            len += self
                .content_type
                .as_ref()
                .map_or(1, |value| value.len() + 1);
            if let Some(encoding) = &self.content_encoding {
                len += encoding.len() + 1;
            }
            if self.extension_type.is_some() {
                len += U32_LEN;
            }
        }
        len + self.trailing.len()
    }

    fn write_payload(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.payload_len();
        need_out(buf, need)?;
        buf[0] = self.version;
        let flags = self.flags.to_be_bytes();
        buf[1..FULLBOX_EXTRA_SIZE].copy_from_slice(&flags[1..]);
        let mut cursor = FULLBOX_EXTRA_SIZE;
        if self.version == INFE_VERSION_WIDE_ID {
            buf[cursor..cursor + U32_LEN].copy_from_slice(&self.item_id.get().to_be_bytes());
            cursor += U32_LEN;
        } else {
            let id = u16::try_from(self.item_id.get())
                .map_err(|_| Error::InvalidInput("infe item_ID exceeds u16"))?;
            buf[cursor..cursor + U16_LEN].copy_from_slice(&id.to_be_bytes());
            cursor += U16_LEN;
        }
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.item_protection_index.to_be_bytes());
        cursor += U16_LEN;
        if self.is_typed() {
            buf[cursor..cursor + BOX_TYPE_SIZE].copy_from_slice(&self.item_type.fourcc());
            cursor += BOX_TYPE_SIZE;
        }
        cursor += crate::box_support::write_c_string(buf, cursor, &self.item_name)?;
        if self.is_typed() {
            if self.item_type.fourcc() == ITEM_TYPE_MIME {
                let content_type = self.content_type.as_deref().unwrap_or(&[]);
                cursor += crate::box_support::write_c_string(buf, cursor, content_type)?;
                if let Some(encoding) = &self.content_encoding {
                    cursor += crate::box_support::write_c_string(buf, cursor, encoding)?;
                }
            } else if self.item_type.fourcc() == ITEM_TYPE_URI {
                let uri = self.item_uri_type.as_deref().unwrap_or(&[]);
                cursor += crate::box_support::write_c_string(buf, cursor, uri)?;
            }
        } else {
            let content_type = self.content_type.as_deref().unwrap_or(&[]);
            cursor += crate::box_support::write_c_string(buf, cursor, content_type)?;
            if let Some(encoding) = &self.content_encoding {
                cursor += crate::box_support::write_c_string(buf, cursor, encoding)?;
            }
            if let Some(extension) = self.extension_type {
                buf[cursor..cursor + BOX_TYPE_SIZE].copy_from_slice(&extension);
                cursor += U32_LEN;
            }
        }
        buf[cursor..cursor + self.trailing.len()].copy_from_slice(&self.trailing);
        Ok(cursor + self.trailing.len())
    }
}

impl<'a> Parse<'a> for ItemInfoEntry {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &INFE, "expected infe")?;
        Self::parse_payload(body_of(bytes, FULLBOX_EXTRA_SIZE, "infe")?)
    }
}

impl Serialize for ItemInfoEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.payload_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &INFE, need)?;
        cursor += self.write_payload(&mut buf[cursor..])?;
        Ok(cursor)
    }
}

/// Item Information Box (`iinf`) — ISO/IEC 14496-12:2026 §8.11.6.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemInfoBox {
    version: u8,
    flags: u32,
    entries: Vec<ItemInfoEntry>,
}

impl ItemInfoBox {
    /// An empty version-0 box.
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: 0,
            flags: 0,
            entries: Vec::new(),
        }
    }

    /// Append an entry.
    pub fn push(&mut self, entry: ItemInfoEntry) {
        self.entries.push(entry);
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

    /// The entries, in wire order.
    #[must_use]
    pub fn entries(&self) -> &[ItemInfoEntry] {
        &self.entries
    }

    /// One item's entry.
    #[must_use]
    pub fn entry(&self, id: ItemId) -> Option<&ItemInfoEntry> {
        self.entries.iter().find(|entry| entry.item_id == id)
    }

    fn count_width(&self) -> usize {
        if self.version == 0 { U16_LEN } else { U32_LEN }
    }

    fn body_len(&self) -> usize {
        let mut len = FULLBOX_EXTRA_SIZE + self.count_width();
        for entry in &self.entries {
            len += entry.serialized_len();
        }
        len
    }
}

impl<'a> Parse<'a> for ItemInfoBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &IINF, "expected iinf")?;
        let body = body_of(bytes, FULLBOX_EXTRA_SIZE + U16_LEN, "iinf")?;
        let version = body[0];
        let flags = u24_at(body, crate::box_types::FULLBOX_FLAGS_OFFSET) & FULLBOX_FLAGS_MASK;
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let count_width = if version == 0 { U16_LEN } else { U32_LEN };
        need_in(body, cursor + count_width, "iinf entry_count")?;
        let count = if version == 0 {
            let value = usize::from(be_u16(body, cursor, "iinf entry_count")?);
            cursor += U16_LEN;
            value
        } else {
            let value = be_u32(body, cursor, "iinf entry_count")? as usize;
            cursor += U32_LEN;
            value
        };
        crate::box_support::check_entry_count(
            body.len() - cursor,
            BOX_HEADER_MIN_SIZE,
            count,
            "iinf entry_count",
        )?;
        let mut entries = Vec::with_capacity(crate::box_support::bounded_entry_count(
            body.len() - cursor,
            BOX_HEADER_MIN_SIZE,
            count,
        ));
        let mut rest = &body[cursor..];
        for _ in 0..count {
            let (item, used) = crate::box_types::parse_box(rest)?;
            entries.push(ItemInfoEntry::parse(item.bytes)?);
            rest = &rest[used..];
        }
        Ok(Self {
            version,
            flags,
            entries,
        })
    }
}

impl Serialize for ItemInfoBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &IINF, need, self.version, self.flags, false)?;
        if self.version == 0 {
            let count = u16::try_from(self.entries.len())
                .map_err(|_| Error::InvalidInput("iinf entry_count exceeds u16"))?;
            buf[cursor..cursor + U16_LEN].copy_from_slice(&count.to_be_bytes());
            cursor += U16_LEN;
        } else {
            buf[cursor..cursor + U32_LEN]
                .copy_from_slice(&fit_u32(self.entries.len(), "iinf entry_count")?.to_be_bytes());
            cursor += U32_LEN;
        }
        for entry in &self.entries {
            cursor += entry.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// Primary Item Box (`pitm`) — ISO/IEC 14496-12:2026 §8.11.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PrimaryItemBox {
    version: u8,
    flags: u32,
    item_id: ItemId,
}

impl PrimaryItemBox {
    /// A version-0 box naming `item_id`.
    #[must_use]
    pub fn new(item_id: ItemId) -> Self {
        Self {
            version: if item_id.needs_wide() { 1 } else { 0 },
            flags: 0,
            item_id,
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

    /// The primary `item_ID`.
    #[must_use]
    pub const fn item_id(&self) -> ItemId {
        self.item_id
    }
}

impl<'a> Parse<'a> for PrimaryItemBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &PITM, "expected pitm")?;
        let body = body_of(bytes, FULLBOX_EXTRA_SIZE + U16_LEN, "pitm")?;
        let version = body[0];
        let id_width = if version == 0 { U16_LEN } else { U32_LEN };
        need_in(body, FULLBOX_EXTRA_SIZE + id_width, "pitm item_ID")?;
        let flags = u24_at(body, crate::box_types::FULLBOX_FLAGS_OFFSET) & FULLBOX_FLAGS_MASK;
        let item_id = if version == 0 {
            ItemId::new(u32::from(be_u16(body, FULLBOX_EXTRA_SIZE, "pitm item_ID")?))
        } else {
            ItemId::new(be_u32(body, FULLBOX_EXTRA_SIZE, "pitm item_ID")?)
        };
        Ok(Self {
            version,
            flags,
            item_id,
        })
    }
}

impl Serialize for PrimaryItemBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let id_width = if self.version == 0 { U16_LEN } else { U32_LEN };
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + id_width)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &PITM, need, self.version, self.flags, false)?;
        if self.version == 0 {
            let id = u16::try_from(self.item_id.get())
                .map_err(|_| Error::InvalidInput("pitm item_ID exceeds u16"))?;
            buf[cursor..cursor + U16_LEN].copy_from_slice(&id.to_be_bytes());
            cursor += U16_LEN;
        } else {
            buf[cursor..cursor + U32_LEN].copy_from_slice(&self.item_id.get().to_be_bytes());
            cursor += U32_LEN;
        }
        Ok(cursor)
    }
}

/// Item Reference Box (`iref`) — ISO/IEC 14496-12:2026 §8.11.12.
///
/// One child box per reference type and from-item; each child lists the items
/// it refers to, in wire order (position is significant, §8.11.12.1 NOTE 2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemReferenceBox {
    version: u8,
    flags: u32,
    references: Vec<SingleItemTypeReferenceBox>,
}

impl ItemReferenceBox {
    /// An empty version-0 box.
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: 0,
            flags: 0,
            references: Vec::new(),
        }
    }

    /// Append one typed reference.
    pub fn push(&mut self, reference: SingleItemTypeReferenceBox) {
        self.references.push(reference);
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

    /// Use the 32-bit item-ID form (version 1).
    pub fn set_wide_id(&mut self) -> Result<()> {
        self.version = IREF_VERSION_WIDE_ID;
        Ok(())
    }

    /// Every typed reference child, in wire order.
    #[must_use]
    pub fn references(&self) -> &[SingleItemTypeReferenceBox] {
        &self.references
    }

    fn id_width(&self) -> usize {
        if self.version >= IREF_VERSION_WIDE_ID {
            U32_LEN
        } else {
            U16_LEN
        }
    }
}

impl<'a> Parse<'a> for ItemReferenceBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &IREF, "expected iref")?;
        let body = body_of(bytes, FULLBOX_EXTRA_SIZE, "iref")?;
        let version = body[0];
        let flags = u24_at(body, crate::box_types::FULLBOX_FLAGS_OFFSET) & FULLBOX_FLAGS_MASK;
        let mut this = Self {
            version,
            flags,
            references: Vec::new(),
        };
        if version > IREF_VERSION_WIDE_ID {
            return Err(Error::UnsupportedVersion(
                broadcast_core::error::UnsupportedVersion {
                    what: "iref",
                    found: version,
                },
            ));
        }
        let mut rest = &body[FULLBOX_EXTRA_SIZE..];
        while !rest.is_empty() {
            if rest.len() < BOX_HEADER_MIN_SIZE {
                return Err(Error::BufferTooShort(BufferTooShort {
                    need: BOX_HEADER_MIN_SIZE,
                    have: rest.len(),
                    what: "iref child header",
                }));
            }
            let (item, used) = crate::box_types::parse_box(rest)?;
            let usertype = item.header.usertype;
            let mut reference = this.parse_reference(item.four_cc(), item.body)?;
            reference.usertype = usertype;
            this.references.push(reference);
            rest = &rest[used..];
        }
        Ok(this)
    }
}

impl ItemReferenceBox {
    fn parse_reference(
        &self,
        reference_type: [u8; BOX_TYPE_SIZE],
        body: &[u8],
    ) -> Result<SingleItemTypeReferenceBox> {
        let width = self.id_width();
        need_in(body, width + U16_LEN, "iref child")?;
        let mut cursor = 0;
        let from = self.read_id(body, &mut cursor)?;
        let count = usize::from(be_u16(body, cursor, "iref reference_count")?);
        cursor += U16_LEN;
        crate::box_support::check_entry_count(
            body.len() - cursor,
            width,
            count,
            "iref reference_count",
        )?;
        let mut to = Vec::with_capacity(crate::box_support::bounded_entry_count(
            body.len() - cursor,
            width,
            count,
        ));
        for _ in 0..count {
            to.push(self.read_id(body, &mut cursor)?);
        }
        Ok(SingleItemTypeReferenceBox {
            reference_type: ReferenceType::from_fourcc(reference_type),
            from,
            to,
            usertype: None,
        })
    }

    fn read_id(&self, body: &[u8], cursor: &mut usize) -> Result<ItemId> {
        if self.version >= IREF_VERSION_WIDE_ID {
            let value = be_u32(body, *cursor, "iref item_ID")?;
            *cursor += U32_LEN;
            Ok(ItemId::new(value))
        } else {
            let value = be_u16(body, *cursor, "iref item_ID")?;
            *cursor += U16_LEN;
            Ok(ItemId::new(u32::from(value)))
        }
    }

    fn write_id(&self, buf: &mut [u8], cursor: &mut usize, id: ItemId) -> Result<()> {
        if self.version >= IREF_VERSION_WIDE_ID {
            buf[*cursor..*cursor + U32_LEN].copy_from_slice(&id.get().to_be_bytes());
            *cursor += U32_LEN;
        } else {
            let value = u16::try_from(id.get())
                .map_err(|_| Error::InvalidInput("iref item_ID exceeds u16"))?;
            buf[*cursor..*cursor + U16_LEN].copy_from_slice(&value.to_be_bytes());
            *cursor += U16_LEN;
        }
        Ok(())
    }

    fn child_len(&self, reference: &SingleItemTypeReferenceBox) -> usize {
        BOX_HEADER_MIN_SIZE
            + if reference.usertype.is_some() {
                crate::box_types::UUID_TYPE_SIZE
            } else {
                0
            }
            + self.id_width()
            + U16_LEN
            + reference.to.len() * self.id_width()
    }
}

impl Serialize for ItemReferenceBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut len = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE;
        for reference in &self.references {
            len += self.child_len(reference);
        }
        box_len(len)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &IREF, need, self.version, self.flags, false)?;
        for reference in &self.references {
            let child_need = self.child_len(reference);
            let header = write_box_header(&mut buf[cursor..], &reference.box_type(), child_need)?;
            let mut at = cursor + header;
            if let Some(usertype) = &reference.usertype {
                buf[at..at + crate::box_types::UUID_TYPE_SIZE].copy_from_slice(usertype);
                at += crate::box_types::UUID_TYPE_SIZE;
            }
            self.write_id(buf, &mut at, reference.from)?;
            let count = u16::try_from(reference.to.len())
                .map_err(|_| Error::InvalidInput("iref reference_count exceeds u16"))?;
            buf[at..at + U16_LEN].copy_from_slice(&count.to_be_bytes());
            at += U16_LEN;
            for id in &reference.to {
                self.write_id(buf, &mut at, *id)?;
            }
            cursor += child_need;
        }
        Ok(cursor)
    }
}

/// One `SingleItemTypeReferenceBox` of an [`ItemReferenceBox`] (§8.11.12.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SingleItemTypeReferenceBox {
    reference_type: ReferenceType,
    from: ItemId,
    to: Vec<ItemId>,
    /// The `uuid` box's 16-byte extended type (§4.2), when the reference is a
    /// user extension rather than a registered reference type.
    usertype: Option<[u8; crate::box_types::UUID_TYPE_SIZE]>,
}

impl SingleItemTypeReferenceBox {
    /// A reference from `from` to every item in `to`.
    #[must_use]
    pub fn new(reference_type: ReferenceType, from: ItemId, to: Vec<ItemId>) -> Self {
        Self {
            reference_type,
            from,
            to,
            usertype: None,
        }
    }

    /// A `uuid` reference from `from` to every item in `to`, keyed by its
    /// 16-byte extended type (§4.2).
    #[must_use]
    pub fn uuid(
        usertype: [u8; crate::box_types::UUID_TYPE_SIZE],
        from: ItemId,
        to: Vec<ItemId>,
    ) -> Self {
        Self {
            reference_type: ReferenceType::Other(crate::box_types::UUID),
            from,
            to,
            usertype: Some(usertype),
        }
    }

    /// The box's `uuid` extended type, when it is a `uuid` box.
    #[must_use]
    pub const fn usertype(&self) -> Option<[u8; crate::box_types::UUID_TYPE_SIZE]> {
        self.usertype
    }

    /// The box type this reference writes: `uuid` for a user extension, else
    /// the reference type's own four-CC.
    fn box_type(&self) -> [u8; BOX_TYPE_SIZE] {
        if self.usertype.is_some() {
            crate::box_types::UUID
        } else {
            self.reference_type.fourcc()
        }
    }

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

    /// The referred items, in wire order.
    #[must_use]
    pub fn to(&self) -> &[ItemId] {
        &self.to
    }
}

/// Item Data Box (`idat`) — ISO/IEC 14496-12:2026 §8.11.11.
///
/// The bytes items address with `construction_method` 1. Kept verbatim: the
/// box has no structure beyond its payload.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemDataBox {
    data: Vec<u8>,
}

impl ItemDataBox {
    /// A box holding `data`.
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// The payload.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The payload, mutably (a writer may fill it in place).
    #[must_use]
    pub fn data_mut(&mut self) -> &mut Vec<u8> {
        &mut self.data
    }
}

impl<'a> Parse<'a> for ItemDataBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &IDAT, "expected idat")?;
        Ok(Self {
            data: body_of(bytes, 0, "idat")?.to_vec(),
        })
    }
}

impl Serialize for ItemDataBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.data.len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &IDAT, need)?;
        buf[cursor..cursor + self.data.len()].copy_from_slice(&self.data);
        cursor += self.data.len();
        Ok(cursor)
    }
}

/// Item Protection Box (`ipro`) — ISO/IEC 14496-12:2026 §8.11.5.
///
/// Each entry is a `ProtectionSchemeInfoBox` (a `sinf`) without an
/// `OriginalFormatBox`; the entries are kept as whole child boxes because
/// this crate's typed `sinf` requires the `frma` the clause forbids here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemProtectionBox {
    version: u8,
    flags: u32,
    protections: Vec<OpaqueBox>,
}

impl ItemProtectionBox {
    /// An empty version-0 box.
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: 0,
            flags: 0,
            protections: Vec::new(),
        }
    }

    /// Append one protection entry (a whole `sinf` child).
    pub fn push(&mut self, protection: OpaqueBox) {
        self.protections.push(protection);
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

    /// The protection entries, in wire order.
    #[must_use]
    pub fn protections(&self) -> &[OpaqueBox] {
        &self.protections
    }

    fn body_len(&self) -> usize {
        let mut len = FULLBOX_EXTRA_SIZE + U16_LEN;
        for protection in &self.protections {
            len += protection.serialized_len();
        }
        len
    }
}

impl<'a> Parse<'a> for ItemProtectionBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &IPRO, "expected ipro")?;
        let body = body_of(bytes, FULLBOX_EXTRA_SIZE + U16_LEN, "ipro")?;
        let version = body[0];
        let flags = u24_at(body, crate::box_types::FULLBOX_FLAGS_OFFSET) & FULLBOX_FLAGS_MASK;
        let count = usize::from(be_u16(body, FULLBOX_EXTRA_SIZE, "ipro protection_count")?);
        let cursor = FULLBOX_EXTRA_SIZE + U16_LEN;
        crate::box_support::check_entry_count(
            body.len() - cursor,
            BOX_HEADER_MIN_SIZE,
            count,
            "ipro protection_count",
        )?;
        let mut protections = Vec::with_capacity(crate::box_support::bounded_entry_count(
            body.len() - cursor,
            BOX_HEADER_MIN_SIZE,
            count,
        ));
        let mut rest = &body[cursor..];
        for _ in 0..count {
            let (item, used) = crate::box_types::parse_box(rest)?;
            let mut opaque = OpaqueBox::new(item.four_cc(), item.payload().to_vec());
            opaque.to_end = item.to_end();
            opaque.largesize = item.has_largesize();
            protections.push(opaque);
            rest = &rest[used..];
        }
        Ok(Self {
            version,
            flags,
            protections,
        })
    }
}

impl Serialize for ItemProtectionBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &IPRO, need, self.version, self.flags, false)?;
        let count = u16::try_from(self.protections.len())
            .map_err(|_| Error::InvalidInput("ipro protection_count exceeds u16"))?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&count.to_be_bytes());
        cursor += U16_LEN;
        for protection in &self.protections {
            cursor += protection.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// A grouping type of an [`EntityToGroupBox`] (§8.15.3.1, ISO/IEC 23008-12:2025
/// 9.4.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EntityGroupType {
    /// `altr`: the members are alternatives of each other.
    Altr,
    /// `prsl`: a preselection group (§8.15.4.1).
    Prsl,
    /// `ster`: a stereo pair (ISO/IEC 23008-12:2025 9.4.3.1).
    Ster,
    /// `eqiv`: equivalent items and samples (ISO/IEC 23008-12:2025 6.8.1).
    Eqiv,
    /// `brst`: burst images (23008-12 6.8.2).
    Brst,
    /// `tsyn`: time-synchronized capture (6.8.3).
    Tsyn,
    /// `iaug`: audio-to-image association (6.8.4).
    Iaug,
    /// `stem`: stereo pair with a monoscopic fallback (Amd 1 6.8.11).
    Stem,
    /// `aebr`: auto exposure bracketing (6.8.6.2).
    Aebr,
    /// `wbbr`: white balance bracketing (6.8.6.3).
    Wbbr,
    /// `fobr`: focus bracketing (6.8.6.4).
    Fobr,
    /// `afbr`: flash exposure bracketing (6.8.6.5).
    Afbr,
    /// `dobr`: depth of field bracketing (6.8.6.6).
    Dobr,
    /// `albc`: album collection (6.8.7.1).
    Albc,
    /// `favc`: favorites collection (6.8.7.2).
    Favc,
    /// `pano`: panorama (6.8.8).
    Pano,
    /// `slid`: slideshow (6.8.9).
    Slid,
    /// `prgr`: progressive rendering (6.8.10).
    Prgr,
    /// `pymd`: image pyramid (Amd 1 6.8.12).
    Pymd,
    /// `rgpa`: region partition group (Amd 1 6.8.13).
    Rgpa,
    /// Any other grouping type.
    Other([u8; BOX_TYPE_SIZE]),
}

impl Default for EntityGroupType {
    fn default() -> Self {
        Self::Other([0; BOX_TYPE_SIZE])
    }
}

impl EntityGroupType {
    /// The four-CC.
    #[must_use]
    pub const fn fourcc(self) -> [u8; BOX_TYPE_SIZE] {
        match self {
            Self::Altr => *b"altr",
            Self::Prsl => *b"prsl",
            Self::Ster => *b"ster",
            Self::Eqiv => *b"eqiv",
            Self::Brst => *b"brst",
            Self::Tsyn => *b"tsyn",
            Self::Iaug => *b"iaug",
            Self::Stem => *b"stem",
            Self::Aebr => *b"aebr",
            Self::Wbbr => *b"wbbr",
            Self::Fobr => *b"fobr",
            Self::Afbr => *b"afbr",
            Self::Dobr => *b"dobr",
            Self::Albc => *b"albc",
            Self::Favc => *b"favc",
            Self::Pano => *b"pano",
            Self::Slid => *b"slid",
            Self::Prgr => *b"prgr",
            Self::Pymd => *b"pymd",
            Self::Rgpa => *b"rgpa",
            Self::Other(fourcc) => fourcc,
        }
    }

    /// The grouping type named by a four-CC.
    #[must_use]
    pub const fn from_fourcc(fourcc: [u8; BOX_TYPE_SIZE]) -> Self {
        match &fourcc {
            b"altr" => Self::Altr,
            b"prsl" => Self::Prsl,
            b"ster" => Self::Ster,
            b"eqiv" => Self::Eqiv,
            b"brst" => Self::Brst,
            b"tsyn" => Self::Tsyn,
            b"iaug" => Self::Iaug,
            b"stem" => Self::Stem,
            b"aebr" => Self::Aebr,
            b"wbbr" => Self::Wbbr,
            b"fobr" => Self::Fobr,
            b"afbr" => Self::Afbr,
            b"dobr" => Self::Dobr,
            b"albc" => Self::Albc,
            b"favc" => Self::Favc,
            b"pano" => Self::Pano,
            b"slid" => Self::Slid,
            b"prgr" => Self::Prgr,
            b"pymd" => Self::Pymd,
            b"rgpa" => Self::Rgpa,
            _ => Self::Other(fourcc),
        }
    }

    /// Label for the grouping type, per the #204 convention.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Altr => "altr",
            Self::Prsl => "prsl",
            Self::Ster => "ster",
            Self::Eqiv => "eqiv",
            Self::Brst => "brst",
            Self::Tsyn => "tsyn",
            Self::Iaug => "iaug",
            Self::Stem => "stem",
            Self::Aebr => "aebr",
            Self::Wbbr => "wbbr",
            Self::Fobr => "fobr",
            Self::Afbr => "afbr",
            Self::Dobr => "dobr",
            Self::Albc => "albc",
            Self::Favc => "favc",
            Self::Pano => "pano",
            Self::Slid => "slid",
            Self::Prgr => "prgr",
            Self::Pymd => "pymd",
            Self::Rgpa => "rgpa",
            Self::Other(_) => "other",
        }
    }
}

broadcast_core::impl_spec_display!(EntityGroupType);

/// Entity to Group Box (`grpl` child) — ISO/IEC 14496-12:2026 §8.15.3.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct EntityToGroupBox {
    grouping_type: EntityGroupType,
    usertype: Option<[u8; crate::box_types::UUID_TYPE_SIZE]>,
    version: u8,
    flags: u32,
    group_id: GroupId,
    entity_ids: Vec<u32>,
    trailing: Vec<u8>,
}

impl EntityToGroupBox {
    /// A version-0 group of `entity_ids`.
    #[must_use]
    pub fn new(grouping_type: EntityGroupType, group_id: GroupId, entity_ids: Vec<u32>) -> Self {
        Self {
            grouping_type,
            usertype: None,
            version: 0,
            flags: 0,
            group_id,
            entity_ids,
            trailing: Vec::new(),
        }
    }

    /// A `uuid` entity group, keyed by its 16-byte extended type (§4.2).
    #[must_use]
    pub fn uuid(
        usertype: [u8; crate::box_types::UUID_TYPE_SIZE],
        group_id: GroupId,
        entity_ids: Vec<u32>,
    ) -> Self {
        Self {
            grouping_type: EntityGroupType::Other(crate::box_types::UUID),
            usertype: Some(usertype),
            version: 0,
            flags: 0,
            group_id,
            entity_ids,
            trailing: Vec::new(),
        }
    }

    /// The box's `uuid` extended type, when it is a `uuid` box.
    #[must_use]
    pub const fn usertype(&self) -> Option<[u8; crate::box_types::UUID_TYPE_SIZE]> {
        self.usertype
    }

    /// Keep grouping-type-specific bytes after the entity list.
    #[must_use]
    pub fn with_trailing(mut self, trailing: Vec<u8>) -> Self {
        self.trailing = trailing;
        self
    }

    /// Set the FullBox version and flags (a typed body reads its flags).
    #[must_use]
    pub const fn with_version_flags(mut self, version: u8, flags: u32) -> Self {
        self.version = version;
        self.flags = flags;
        self
    }

    /// Keep a typed body after the entity list.
    pub fn with_body(mut self, body: &crate::image_groups::ImageGroupBody) -> Result<Self> {
        let mut trailing = alloc::vec![0u8; body.serialized_len()];
        body.serialize_into(&mut trailing)?;
        self.trailing = trailing;
        Ok(self)
    }

    /// The grouping type.
    #[must_use]
    pub const fn grouping_type(&self) -> EntityGroupType {
        self.grouping_type
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

    /// `group_id`.
    #[must_use]
    pub const fn group_id(&self) -> GroupId {
        self.group_id
    }

    /// The `entity_id` values, in wire order.
    #[must_use]
    pub fn entity_ids(&self) -> &[u32] {
        &self.entity_ids
    }

    /// Grouping-type-specific bytes after the entity list.
    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }

    /// The typed trailing body, for a grouping type this crate models.
    pub fn image_body(&self) -> Result<crate::image_groups::ImageGroupBody> {
        crate::image_groups::ImageGroupBody::parse(
            self.grouping_type,
            self.flags,
            &self.trailing,
            self.entity_ids.len(),
        )
    }

    fn payload_len(&self) -> usize {
        FULLBOX_EXTRA_SIZE
            + ENTITY_GROUP_FIXED_LEN
            + self.entity_ids.len() * U32_LEN
            + self.trailing.len()
            + if self.usertype.is_some() {
                crate::box_types::UUID_TYPE_SIZE
            } else {
                0
            }
    }
}

impl<'a> Parse<'a> for EntityToGroupBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE, "entity group")?;
        let item = crate::box_types::parse_box(bytes)?.0;
        let body = item.body;
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + ENTITY_GROUP_FIXED_LEN,
            "entity group",
        )?;
        let version = body[0];
        let flags = u24_at(body, crate::box_types::FULLBOX_FLAGS_OFFSET) & FULLBOX_FLAGS_MASK;
        let group_id = GroupId::new(be_u32(body, FULLBOX_EXTRA_SIZE, "group_id")?);
        let count = be_u32(body, FULLBOX_EXTRA_SIZE + U32_LEN, "num_entities_in_group")? as usize;
        let mut cursor = FULLBOX_EXTRA_SIZE + ENTITY_GROUP_FIXED_LEN;
        crate::box_support::check_entry_count(
            body.len() - cursor,
            U32_LEN,
            count,
            "num_entities_in_group",
        )?;
        let mut entity_ids = Vec::with_capacity(crate::box_support::bounded_entry_count(
            body.len() - cursor,
            U32_LEN,
            count,
        ));
        for _ in 0..count {
            entity_ids.push(be_u32(body, cursor, "entity_id")?);
            cursor += U32_LEN;
        }
        Ok(Self {
            grouping_type: EntityGroupType::from_fourcc(item.four_cc()),
            usertype: item.header.usertype,
            version,
            flags,
            group_id,
            entity_ids,
            trailing: body[cursor..].to_vec(),
        })
    }
}

impl Serialize for EntityToGroupBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.payload_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = match &self.usertype {
            Some(usertype) => {
                // §4.2 puts the `uuid` extended type immediately after the
                // box header, ahead of the FullBox fields.
                let mut at = write_box_header(buf, &crate::box_types::UUID, need)?;
                buf[at..at + crate::box_types::UUID_TYPE_SIZE].copy_from_slice(usertype);
                at += crate::box_types::UUID_TYPE_SIZE;
                buf[at] = self.version;
                let flags = self.flags.to_be_bytes();
                let flags_len = FULLBOX_EXTRA_SIZE - 1;
                buf[at + 1..at + 1 + flags_len].copy_from_slice(&flags[1..]);
                at + FULLBOX_EXTRA_SIZE
            }
            None => write_full_box_header(
                buf,
                &self.grouping_type.fourcc(),
                need,
                self.version,
                self.flags,
                false,
            )?,
        };
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.group_id.get().to_be_bytes());
        cursor += U32_LEN;
        buf[cursor..cursor + U32_LEN].copy_from_slice(
            &fit_u32(self.entity_ids.len(), "num_entities_in_group")?.to_be_bytes(),
        );
        cursor += U32_LEN;
        for entity in &self.entity_ids {
            buf[cursor..cursor + U32_LEN].copy_from_slice(&entity.to_be_bytes());
            cursor += U32_LEN;
        }
        buf[cursor..cursor + self.trailing.len()].copy_from_slice(&self.trailing);
        cursor += self.trailing.len();
        Ok(cursor)
    }
}

/// Groups List Box (`grpl`) — ISO/IEC 14496-12:2026 §8.15.2.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct GroupsListBox {
    groups: Vec<EntityToGroupBox>,
}

impl GroupsListBox {
    /// An empty box.
    #[must_use]
    pub fn new() -> Self {
        Self { groups: Vec::new() }
    }

    /// Append one entity group.
    pub fn push(&mut self, group: EntityToGroupBox) {
        self.groups.push(group);
    }

    /// The entity groups, in wire order.
    #[must_use]
    pub fn groups(&self) -> &[EntityToGroupBox] {
        &self.groups
    }

    /// The group an item belongs to, when any.
    #[must_use]
    pub fn group_of(&self, entity: u32) -> Option<&EntityToGroupBox> {
        self.groups
            .iter()
            .find(|group| group.entity_ids.contains(&entity))
    }
}

impl<'a> Parse<'a> for GroupsListBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &GRPL, "expected grpl")?;
        let body = body_of(bytes, 0, "grpl")?;
        let mut groups = Vec::new();
        let mut rest = body;
        while !rest.is_empty() {
            if rest.len() < BOX_HEADER_MIN_SIZE {
                return Err(Error::BufferTooShort(BufferTooShort {
                    need: BOX_HEADER_MIN_SIZE,
                    have: rest.len(),
                    what: "grpl child header",
                }));
            }
            let (item, used) = crate::box_types::parse_box(rest)?;
            groups.push(EntityToGroupBox::parse(item.bytes)?);
            rest = &rest[used..];
        }
        Ok(Self { groups })
    }
}

impl Serialize for GroupsListBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut len = BOX_HEADER_MIN_SIZE;
        for group in &self.groups {
            len += group.serialized_len();
        }
        box_len(len)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &GRPL, need)?;
        for group in &self.groups {
            cursor += group.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// Brand Property (`brnd`) — ISO/IEC 14496-12:2026 §8.11.15.
///
/// A descriptive item property whose payload is a `GeneralTypeBox`, the same
/// structure as `ftyp`; the value is the crate's [`FileTypeBox`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct BrandProperty {
    brands: FileTypeBox,
}

impl BrandProperty {
    /// A brand property carrying `brands`.
    #[must_use]
    pub fn new(brands: FileTypeBox) -> Self {
        Self { brands }
    }

    /// The brand list.
    #[must_use]
    pub fn brands(&self) -> &FileTypeBox {
        &self.brands
    }
}

impl<'a> Parse<'a> for BrandProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &BRND, "expected brnd")?;
        Ok(Self {
            brands: FileTypeBox::parse_payload(body_of(bytes, 0, "brnd")?)?,
        })
    }
}

impl Serialize for BrandProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.brands.payload_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &BRND, need)?;
        cursor += self.brands.serialize_payload_into(&mut buf[cursor..])?;
        Ok(cursor)
    }
}

/// Handler Property (`hdlp`) — ISO/IEC 14496-12:2026 §8.11.16.
///
/// A descriptive item property mapping an item to a media handler type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HandlerProperty {
    version: u8,
    flags: u32,
    handler_type: [u8; BOX_TYPE_SIZE],
}

impl HandlerProperty {
    /// A version-0 property with the given handler type.
    #[must_use]
    pub fn new(handler_type: [u8; BOX_TYPE_SIZE]) -> Self {
        Self {
            version: 0,
            flags: 0,
            handler_type,
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

    /// The `handler_type` four-CC.
    #[must_use]
    pub const fn handler_type(&self) -> [u8; BOX_TYPE_SIZE] {
        self.handler_type
    }
}

impl<'a> Parse<'a> for HandlerProperty {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &HDLP, "expected hdlp")?;
        let body = body_of(bytes, FULLBOX_EXTRA_SIZE + BOX_TYPE_SIZE, "hdlp")?;
        let mut handler_type = [0u8; BOX_TYPE_SIZE];
        handler_type.copy_from_slice(&body[FULLBOX_EXTRA_SIZE..FULLBOX_EXTRA_SIZE + BOX_TYPE_SIZE]);
        Ok(Self {
            version: body[0],
            flags: u24_at(body, crate::box_types::FULLBOX_FLAGS_OFFSET) & FULLBOX_FLAGS_MASK,
            handler_type,
        })
    }
}

impl Serialize for HandlerProperty {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + BOX_TYPE_SIZE)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &HDLP, need, self.version, self.flags, false)?;
        buf[cursor..cursor + BOX_TYPE_SIZE].copy_from_slice(&self.handler_type);
        cursor += BOX_TYPE_SIZE;
        Ok(cursor)
    }
}

/// One property association of an [`ItemPropertyAssociationBox`] (§8.11.14.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PropertyAssociation {
    essential: bool,
    property_index: PropertyIndex,
}

impl PropertyAssociation {
    /// An association of `property_index`, essential or not.
    #[must_use]
    pub const fn new(property_index: PropertyIndex, essential: bool) -> Self {
        Self {
            essential,
            property_index,
        }
    }

    /// The `essential` flag.
    #[must_use]
    pub const fn essential(&self) -> bool {
        self.essential
    }

    /// The 1-based property index (0 means "no property").
    #[must_use]
    pub const fn property_index(&self) -> PropertyIndex {
        self.property_index
    }
}

/// The associations of one item in an [`ItemPropertyAssociationBox`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemPropertyEntry {
    item_id: ItemId,
    associations: Vec<PropertyAssociation>,
}

impl ItemPropertyEntry {
    /// An entry with these associations.
    #[must_use]
    pub fn new(item_id: ItemId, associations: Vec<PropertyAssociation>) -> Self {
        Self {
            item_id,
            associations,
        }
    }

    /// The item.
    #[must_use]
    pub const fn item_id(&self) -> ItemId {
        self.item_id
    }

    /// The associations, in property-application order.
    #[must_use]
    pub fn associations(&self) -> &[PropertyAssociation] {
        &self.associations
    }
}

/// Item Property Association Box (`ipma`) — ISO/IEC 14496-12:2026 §8.11.14.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemPropertyAssociationBox {
    version: u8,
    flags: u32,
    entries: Vec<ItemPropertyEntry>,
}

impl ItemPropertyAssociationBox {
    /// An empty version-0 box with narrow (7-bit) property indices.
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: 0,
            flags: 0,
            entries: Vec::new(),
        }
    }

    /// Append one item's associations.
    pub fn push(&mut self, entry: ItemPropertyEntry) {
        self.entries.push(entry);
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags (`(flags & 1)` widens the property index to 15 bits).
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// Use the 15-bit property-index form.
    pub fn set_wide_index(&mut self) -> Result<()> {
        self.flags |= IPMA_WIDE_INDEX;
        Ok(())
    }

    /// Use the 32-bit item-ID form (version 1).
    pub fn set_wide_id(&mut self) -> Result<()> {
        self.version = IPMA_VERSION_WIDE_ID;
        Ok(())
    }

    /// Whether property indices use the 15-bit form.
    #[must_use]
    pub const fn wide_index(&self) -> bool {
        self.flags & IPMA_WIDE_INDEX != 0
    }

    /// The per-item associations, in wire order.
    #[must_use]
    pub fn entries(&self) -> &[ItemPropertyEntry] {
        &self.entries
    }

    /// One item's associations.
    #[must_use]
    pub fn entry(&self, id: ItemId) -> Option<&ItemPropertyEntry> {
        self.entries.iter().find(|entry| entry.item_id == id)
    }

    /// Resolve every association against a property container, failing when an
    /// index is past it (including index 0 with nothing to point at).
    pub fn validate(&self, properties: usize) -> Result<()> {
        for entry in &self.entries {
            for association in &entry.associations {
                let index = usize::from(association.property_index.get());
                if index > properties {
                    return Err(Error::PropertyIndexOutOfRange {
                        index: association.property_index.get(),
                        available: properties,
                    });
                }
            }
        }
        Ok(())
    }

    fn id_width(&self) -> usize {
        if self.version >= IPMA_VERSION_WIDE_ID {
            U32_LEN
        } else {
            U16_LEN
        }
    }

    fn index_width(&self) -> usize {
        if self.wide_index() { U16_LEN } else { 1 }
    }

    fn entry_min_len(&self) -> usize {
        self.id_width() + 1
    }

    fn body_len(&self) -> usize {
        let mut len = FULLBOX_EXTRA_SIZE + U32_LEN;
        for entry in &self.entries {
            len += self.id_width() + 1 + entry.associations.len() * self.index_width();
        }
        len
    }
}

impl<'a> Parse<'a> for ItemPropertyAssociationBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &IPMA, "expected ipma")?;
        let body = body_of(bytes, FULLBOX_EXTRA_SIZE + U32_LEN, "ipma")?;
        let version = body[0];
        let flags = u24_at(body, crate::box_types::FULLBOX_FLAGS_OFFSET) & FULLBOX_FLAGS_MASK;
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let count = be_u32(body, cursor, "ipma entry_count")? as usize;
        cursor += U32_LEN;
        let mut this = Self {
            version,
            flags,
            entries: Vec::new(),
        };
        crate::box_support::check_entry_count(
            body.len() - cursor,
            this.entry_min_len(),
            count,
            "ipma entry_count",
        )?;
        this.entries
            .reserve(crate::box_support::bounded_entry_count(
                body.len() - cursor,
                this.entry_min_len(),
                count,
            ));
        for _ in 0..count {
            let item_id = if this.id_width() == U32_LEN {
                let value = be_u32(body, cursor, "ipma item_ID")?;
                cursor += U32_LEN;
                ItemId::new(value)
            } else {
                let value = be_u16(body, cursor, "ipma item_ID")?;
                cursor += U16_LEN;
                ItemId::new(u32::from(value))
            };
            let association_count = usize::from(body[cursor]);
            cursor += 1;
            let width = this.index_width();
            crate::box_support::check_entry_count(
                body.len() - cursor,
                width,
                association_count,
                "ipma association_count",
            )?;
            let mut associations = Vec::with_capacity(crate::box_support::bounded_entry_count(
                body.len() - cursor,
                width,
                association_count,
            ));
            for _ in 0..association_count {
                let (essential, index) = if this.wide_index() {
                    let packed = be_u16(body, cursor, "ipma association")?;
                    cursor += U16_LEN;
                    (
                        packed >> IPMA_WIDE_INDEX_BITS != 0,
                        packed & ((1 << IPMA_WIDE_INDEX_BITS) - 1),
                    )
                } else {
                    let packed = u16::from(body[cursor]);
                    cursor += 1;
                    (
                        packed >> IPMA_NARROW_INDEX_BITS != 0,
                        packed & ((1 << IPMA_NARROW_INDEX_BITS) - 1),
                    )
                };
                associations.push(PropertyAssociation::new(
                    PropertyIndex::new(index),
                    essential,
                ));
            }
            this.entries.push(ItemPropertyEntry {
                item_id,
                associations,
            });
        }
        Ok(this)
    }
}

impl Serialize for ItemPropertyAssociationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &IPMA, need, self.version, self.flags, false)?;
        buf[cursor..cursor + U32_LEN]
            .copy_from_slice(&fit_u32(self.entries.len(), "ipma entry_count")?.to_be_bytes());
        cursor += U32_LEN;
        for entry in &self.entries {
            if self.id_width() == U32_LEN {
                buf[cursor..cursor + U32_LEN].copy_from_slice(&entry.item_id.get().to_be_bytes());
                cursor += U32_LEN;
            } else {
                let id = u16::try_from(entry.item_id.get())
                    .map_err(|_| Error::InvalidInput("ipma item_ID exceeds u16"))?;
                buf[cursor..cursor + U16_LEN].copy_from_slice(&id.to_be_bytes());
                cursor += U16_LEN;
            }
            let count = u8::try_from(entry.associations.len())
                .map_err(|_| Error::InvalidInput("ipma association_count exceeds u8"))?;
            buf[cursor] = count;
            cursor += 1;
            for association in &entry.associations {
                let index = association.property_index.get();
                if self.wide_index() {
                    let bits = (u16::from(association.essential) << IPMA_WIDE_INDEX_BITS) | index;
                    broadcast_core::len::fit_bits(
                        u64::from(bits),
                        IPMA_WIDE_INDEX_BITS + IPMA_ESSENTIAL_BITS,
                        "ipma association",
                    )?;
                    buf[cursor..cursor + U16_LEN].copy_from_slice(&bits.to_be_bytes());
                    cursor += U16_LEN;
                } else {
                    if index >= (1u16 << IPMA_NARROW_INDEX_BITS) {
                        return Err(Error::PropertyIndexOutOfRange {
                            index,
                            available: usize::from((1u16 << IPMA_NARROW_INDEX_BITS) - 1),
                        });
                    }
                    let bits = (u8::from(association.essential) << IPMA_NARROW_INDEX_BITS)
                        | u8::try_from(index)
                            .map_err(|_| Error::InvalidInput("ipma property_index"))?;
                    buf[cursor] = bits;
                    cursor += 1;
                }
            }
        }
        Ok(cursor)
    }
}

/// Item Property Container Box (`ipco`) — ISO/IEC 14496-12:2026 §8.11.14.
///
/// The implicitly indexed list of properties. A property this crate does not
/// type is kept as [`crate::image::ItemProperty::Opaque`] so the index order —
/// which every `ipma` entry depends on — is preserved exactly.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemPropertyContainerBox {
    properties: Vec<crate::image::ItemProperty>,
}

impl ItemPropertyContainerBox {
    /// An empty container.
    #[must_use]
    pub fn new() -> Self {
        Self {
            properties: Vec::new(),
        }
    }

    /// Append one property (its position is its 1-based index).
    pub fn push(&mut self, property: crate::image::ItemProperty) {
        self.properties.push(property);
    }

    /// The properties, in index order.
    #[must_use]
    pub fn properties(&self) -> &[crate::image::ItemProperty] {
        &self.properties
    }

    /// The property at a 1-based index.
    #[must_use]
    pub fn property(&self, index: PropertyIndex) -> Option<&crate::image::ItemProperty> {
        index
            .get()
            .checked_sub(1)
            .and_then(|zero_based| self.properties.get(usize::from(zero_based)))
    }

    /// Parse the container's children from a whole `ipco` box.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        expect_type(bytes, &IPCO, "expected ipco")?;
        Self::parse_body(body_of(bytes, 0, "ipco")?)
    }

    /// Parse the container's children from the payload after its header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut properties = Vec::new();
        let mut rest = body;
        while !rest.is_empty() {
            if rest.len() < BOX_HEADER_MIN_SIZE {
                return Err(Error::BufferTooShort(BufferTooShort {
                    need: BOX_HEADER_MIN_SIZE,
                    have: rest.len(),
                    what: "ipco child header",
                }));
            }
            let (item, used) = crate::box_types::parse_box(rest)?;
            properties.push(crate::image::ItemProperty::parse_box(item.bytes)?);
            rest = &rest[used..];
        }
        Ok(Self { properties })
    }
}

impl Serialize for ItemPropertyContainerBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut len = BOX_HEADER_MIN_SIZE;
        for property in &self.properties {
            len += property.serialized_len();
        }
        box_len(len)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &IPCO, need)?;
        for property in &self.properties {
            cursor += property.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// Item Properties Box (`iprp`) — ISO/IEC 14496-12:2026 §8.11.14.
///
/// The container of properties plus one or more association boxes. Every
/// association is resolved against the container on parse, so an index past
/// it is a typed error rather than an out-of-bounds lookup later.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ItemPropertiesBox {
    container: ItemPropertyContainerBox,
    associations: Vec<ItemPropertyAssociationBox>,
}

impl ItemPropertiesBox {
    /// A box with an empty container and no associations.
    #[must_use]
    pub fn new() -> Self {
        Self {
            container: ItemPropertyContainerBox::new(),
            associations: Vec::new(),
        }
    }

    /// A box with `container` and no associations yet.
    #[must_use]
    pub fn with_container(container: ItemPropertyContainerBox) -> Self {
        Self {
            container,
            associations: Vec::new(),
        }
    }

    /// Append an association box.
    pub fn push_associations(&mut self, associations: ItemPropertyAssociationBox) {
        self.associations.push(associations);
    }

    /// The property container.
    #[must_use]
    pub fn container(&self) -> &ItemPropertyContainerBox {
        &self.container
    }

    /// The property container, mutably.
    #[must_use]
    pub fn container_mut(&mut self) -> &mut ItemPropertyContainerBox {
        &mut self.container
    }

    /// The association boxes, in wire order.
    #[must_use]
    pub fn associations(&self) -> &[ItemPropertyAssociationBox] {
        &self.associations
    }

    /// One item's associations across every association box.
    #[must_use]
    pub fn associations_for(&self, item: ItemId) -> Vec<PropertyAssociation> {
        let mut out = Vec::new();
        for associations in &self.associations {
            if let Some(entry) = associations.entry(item) {
                out.extend_from_slice(entry.associations());
            }
        }
        out
    }

    /// The properties associated with `item`, in association order.
    #[must_use]
    pub fn properties_for(&self, item: ItemId) -> Vec<(&crate::image::ItemProperty, bool)> {
        let mut out = Vec::new();
        for association in self.associations_for(item) {
            if let Some(property) = self.container.property(association.property_index()) {
                out.push((property, association.essential()));
            }
        }
        out
    }

    /// Check every association against the container.
    pub fn validate(&self) -> Result<()> {
        for associations in &self.associations {
            associations.validate(self.container.properties().len())?;
        }
        Ok(())
    }
}

impl<'a> Parse<'a> for ItemPropertiesBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &IPRP, "expected iprp")?;
        let body = body_of(bytes, BOX_HEADER_MIN_SIZE, "iprp")?;
        let (first, used) = crate::box_types::parse_box(body)?;
        let container = ItemPropertyContainerBox::parse_box(first.bytes)?;
        let mut associations = Vec::new();
        let mut rest = &body[used..];
        while !rest.is_empty() {
            if rest.len() < BOX_HEADER_MIN_SIZE {
                return Err(Error::BufferTooShort(BufferTooShort {
                    need: BOX_HEADER_MIN_SIZE,
                    have: rest.len(),
                    what: "iprp child header",
                }));
            }
            let (item, used) = crate::box_types::parse_box(rest)?;
            associations.push(ItemPropertyAssociationBox::parse(item.bytes)?);
            rest = &rest[used..];
        }
        let this = Self {
            container,
            associations,
        };
        this.validate()?;
        Ok(this)
    }
}

impl Serialize for ItemPropertiesBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut len = BOX_HEADER_MIN_SIZE + self.container.serialized_len();
        for associations in &self.associations {
            len += associations.serialized_len();
        }
        box_len(len)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &IPRP, need)?;
        cursor += self.container.serialize_into(&mut buf[cursor..])?;
        for associations in &self.associations {
            cursor += associations.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// The FD item information extension (`fdel`) of a version-1
/// [`ItemInfoEntry`] — ISO/IEC 14496-12:2026 §8.11.6.1.
///
/// It is not a box: it follows the entry's `extension_type` field, so its
/// bytes are carried by [`ItemInfoEntry`] and typed on demand through
/// [`ItemInfoEntry::fd_extension`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FdItemInfoExtension {
    content_location: Vec<u8>,
    content_md5: Vec<u8>,
    content_length: u64,
    transfer_length: u64,
    group_ids: Vec<u32>,
}

impl FdItemInfoExtension {
    /// Build one from the file's identity and size.
    #[must_use]
    pub fn new(
        content_location: &[u8],
        content_md5: &[u8],
        content_length: u64,
        transfer_length: u64,
    ) -> Self {
        Self {
            content_location: content_location.to_vec(),
            content_md5: content_md5.to_vec(),
            content_length,
            transfer_length,
            group_ids: Vec::new(),
        }
    }

    /// `content_location`, the item's URI.
    #[must_use]
    pub fn content_location(&self) -> &[u8] {
        &self.content_location
    }

    /// `content_MD5`, the file's MD5 digest.
    #[must_use]
    pub fn content_md5(&self) -> &[u8] {
        &self.content_md5
    }

    /// `content_length`, the un-encoded file length.
    #[must_use]
    pub const fn content_length(&self) -> u64 {
        self.content_length
    }

    /// `transfer_length`, the encoded file length.
    #[must_use]
    pub const fn transfer_length(&self) -> u64 {
        self.transfer_length
    }

    /// The file group identifiers.
    #[must_use]
    pub fn group_ids(&self) -> &[u32] {
        &self.group_ids
    }

    /// Append one file group identifier.
    pub fn push_group_id(&mut self, group_id: u32) {
        self.group_ids.push(group_id);
    }

    /// Parse the extension bytes that follow the `extension_type` field.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut cursor = 0usize;
        let content_location =
            crate::box_support::take_c_string(bytes, &mut cursor, "fdel content_location")?;
        let content_md5 =
            crate::box_support::take_c_string(bytes, &mut cursor, "fdel content_MD5")?;
        let content_length = be_u64(bytes, cursor, "fdel content_length")?;
        cursor += core::mem::size_of::<u64>();
        let transfer_length = be_u64(bytes, cursor, "fdel transfer_length")?;
        cursor += core::mem::size_of::<u64>();
        need_in(bytes, cursor + 1, "fdel entry_count")?;
        let entry_count = usize::from(bytes[cursor]);
        cursor += 1;
        crate::box_support::check_entry_count(
            bytes.len().saturating_sub(cursor),
            core::mem::size_of::<u32>(),
            entry_count,
            "fdel group_ids",
        )?;
        let mut group_ids = Vec::with_capacity(crate::box_support::bounded_entry_count(
            bytes.len().saturating_sub(cursor),
            core::mem::size_of::<u32>(),
            entry_count,
        ));
        for _ in 0..entry_count {
            group_ids.push(be_u32(bytes, cursor, "fdel group_id")?);
            cursor += core::mem::size_of::<u32>();
        }
        Ok(Self {
            content_location,
            content_md5,
            content_length,
            transfer_length,
            group_ids,
        })
    }
}

impl Serialize for FdItemInfoExtension {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.content_location.len()
            + 1
            + self.content_md5.len()
            + 1
            + 2 * core::mem::size_of::<u64>()
            + 1
            + self.group_ids.len() * core::mem::size_of::<u32>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = 0usize;
        cursor += crate::box_support::write_c_string(buf, cursor, &self.content_location)?;
        cursor += crate::box_support::write_c_string(buf, cursor, &self.content_md5)?;
        buf[cursor..cursor + core::mem::size_of::<u64>()]
            .copy_from_slice(&self.content_length.to_be_bytes());
        cursor += core::mem::size_of::<u64>();
        buf[cursor..cursor + core::mem::size_of::<u64>()]
            .copy_from_slice(&self.transfer_length.to_be_bytes());
        cursor += core::mem::size_of::<u64>();
        buf[cursor] = broadcast_core::len::fit_u8(self.group_ids.len(), "fdel entry_count")?;
        cursor += 1;
        for group_id in &self.group_ids {
            buf[cursor..cursor + core::mem::size_of::<u32>()]
                .copy_from_slice(&group_id.to_be_bytes());
            cursor += core::mem::size_of::<u32>();
        }
        Ok(cursor)
    }
}
