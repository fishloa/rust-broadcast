//! The QuickTime metadata item list atom (`ilst`) and its `data`/`mean`/`name`
//! children.
//!
//! `ilst` is a container of item atoms. Each item's type is either a
//! four-character iTunes atom (`©nam`, `trkn`) or the one-based index of a key
//! in the sibling `keys` table. The item holds a `data` full box: a 32-bit
//! well-known type, a 32-bit locale indicator and the value bytes. A freeform
//! `----` item instead holds `mean` and `name` full boxes plus its values
//! (Apple `Metadata item list atom ('ilst')`, `Metadata item atom`,
//! `Value atom`, `Well-known types`).

use alloc::string::String;
use alloc::vec::Vec;

use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, BoxIter, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_OFFSET, box_len,
    write_box_header,
};
use crate::error::{Error, Result};
use crate::limits::Limits;

use super::U32_LEN;

/// The `ilst` box type.
pub const ILST: [u8; 4] = *b"ilst";
/// The `data` box type.
pub const DATA: [u8; 4] = *b"data";
/// The `mean` box type.
pub const MEAN: [u8; 4] = *b"mean";
/// The `name` box type.
pub const NAME: [u8; 4] = *b"name";
/// The freeform item type.
pub const FREEFORM: [u8; 4] = *b"----";

/// Well-known type 1: UTF-8 text without a count or terminator.
pub const DATA_TYPE_UTF8: u32 = 1;
/// Well-known type 0: no type indicated.
pub const DATA_TYPE_RESERVED: u32 = 0;
/// Well-known type 21: a big-endian signed integer in 1-4 bytes.
pub const DATA_TYPE_BE_SIGNED: u32 = 21;
/// Well-known type 22: a big-endian unsigned integer in 1-4 bytes.
pub const DATA_TYPE_BE_UNSIGNED: u32 = 22;
/// Well-known type 65: an 8-bit signed integer.
pub const DATA_TYPE_I8: u32 = 65;
/// Well-known type 66: a big-endian 16-bit signed integer.
pub const DATA_TYPE_I16: u32 = 66;
/// Well-known type 67: a big-endian 32-bit signed integer.
pub const DATA_TYPE_I32: u32 = 67;
/// Well-known type 74: a big-endian 64-bit signed integer.
pub const DATA_TYPE_I64: u32 = 74;
/// Well-known type 75: an 8-bit unsigned integer.
pub const DATA_TYPE_U8: u32 = 75;
/// Well-known type 76: a big-endian 16-bit unsigned integer.
pub const DATA_TYPE_U16: u32 = 76;
/// Well-known type 77: a big-endian 32-bit unsigned integer.
pub const DATA_TYPE_U32: u32 = 77;
/// Well-known type 78: a big-endian 64-bit unsigned integer.
pub const DATA_TYPE_U64: u32 = 78;

/// The fixed `data` prefix: type and locale.
const DATA_PREFIX_LEN: usize = U32_LEN + U32_LEN;
/// The `data` body is the prefix plus the value (no full-box header).
const DATA_MIN_BODY_LEN: usize = DATA_PREFIX_LEN;
/// The full-box header of a `mean`/`name`.
const STRING_MIN_BODY_LEN: usize = FULLBOX_EXTRA_SIZE;
/// The two reserved bytes before and after a `trkn`/`disk` pair.
pub(crate) const PAIR_RESERVED_LEN: usize = 2;
/// A `trkn`/`disk` value is two reserved bytes, two 16-bit fields and two more.
pub(crate) const PAIR_VALUE_LEN: usize = PAIR_RESERVED_LEN + PAIR_FIELD_LEN * 2 + PAIR_RESERVED_LEN;
/// The fixed number of freeform children before its values.
const FREEFORM_FIXED_CHILDREN: usize = 2;
/// The width of a `trkn`/`disk` index or total.
const PAIR_FIELD_LEN: usize = 2;
/// The offset of the index inside a `trkn`/`disk` value.
pub(crate) const PAIR_INDEX_OFFSET: usize = PAIR_RESERVED_LEN;
/// The offset of the total inside a `trkn`/`disk` value.
pub(crate) const PAIR_TOTAL_OFFSET: usize = PAIR_RESERVED_LEN + PAIR_FIELD_LEN;

/// A metadata value atom (`data`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Data {
    /// The well-known type of the value.
    pub data_type: u32,
    /// The locale indicator.
    pub locale: u32,
    /// The untouched value bytes.
    pub value: Vec<u8>,
}

impl Data {
    /// A version-0 `data` atom.
    #[must_use]
    pub fn new(data_type: u32, locale: u32, value: Vec<u8>) -> Self {
        Self {
            data_type,
            locale,
            value,
        }
    }

    /// The value as UTF-8, when it is.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        core::str::from_utf8(&self.value).ok()
    }

    /// Parse a `data` atom from its body and its declared box length.
    fn parse_body(body: &[u8], limits: &Limits) -> Result<Self> {
        need_in(body, DATA_MIN_BODY_LEN, "data body")?;
        let value_len = body.len() - DATA_MIN_BODY_LEN;
        if value_len as u64 > limits.max_metadata_item_bytes() {
            return Err(Error::LimitExceeded {
                what: "metadata item",
                limit: limits.max_metadata_item_bytes(),
            });
        }
        Ok(Self {
            data_type: super::u32_at(body, 0),
            locale: super::u32_at(body, U32_LEN),
            value: body[DATA_MIN_BODY_LEN..].to_vec(),
        })
    }
}

/// A `mean` full box: the freeform item's namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Mean {
    /// FullBox version.
    pub version: u8,
    /// FullBox flags.
    pub flags: u32,
    /// The namespace string.
    pub value: String,
}

impl Mean {
    /// A version-0 `mean` atom.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            version: 0,
            flags: 0,
            value: value.into(),
        }
    }

    fn parse_body(body: &[u8]) -> Result<Self> {
        let (version, flags, value) = parse_string_body(body, "mean")?;
        Ok(Self {
            version,
            flags,
            value,
        })
    }
}

/// A `name` full box: the freeform item's name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Name {
    /// FullBox version.
    pub version: u8,
    /// FullBox flags.
    pub flags: u32,
    /// The name string.
    pub value: String,
}

impl Name {
    /// A version-0 `name` atom.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            version: 0,
            flags: 0,
            value: value.into(),
        }
    }

    fn parse_body(body: &[u8]) -> Result<Self> {
        let (version, flags, value) = parse_string_body(body, "name")?;
        Ok(Self {
            version,
            flags,
            value,
        })
    }
}

fn parse_string_body(body: &[u8], what: &'static str) -> Result<(u8, u32, String)> {
    need_in(body, STRING_MIN_BODY_LEN, what)?;
    let version = body[0];
    let flags = broadcast_common::wire::u24_at(body, FULLBOX_FLAGS_OFFSET);
    let value = core::str::from_utf8(&body[FULLBOX_EXTRA_SIZE..])
        .map_err(|_| Error::InvalidInput("freeform string is not UTF-8"))?
        .into();
    Ok((version, flags, value))
}

/// A child of an `ilst` item atom.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum IlstItemChild {
    /// A value atom.
    Data(Data),
    /// The freeform namespace.
    Mean(Mean),
    /// The freeform name.
    Name(Name),
    /// Any other child, preserved verbatim.
    Opaque(OpaqueBox),
}

impl Serialize for IlstItemChild {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Data(data) => box_len(BOX_HEADER_MIN_SIZE + DATA_MIN_BODY_LEN + data.value.len()),
            Self::Mean(mean) => {
                box_len(BOX_HEADER_MIN_SIZE + STRING_MIN_BODY_LEN + mean.value.len())
            }
            Self::Name(name) => {
                box_len(BOX_HEADER_MIN_SIZE + STRING_MIN_BODY_LEN + name.value.len())
            }
            Self::Opaque(opaque) => opaque.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        match self {
            Self::Opaque(opaque) => opaque.serialize_into(buf),
            Self::Data(data) => {
                let mut cursor = write_box_header(buf, &DATA, need)?;
                buf[cursor..cursor + U32_LEN].copy_from_slice(&data.data_type.to_be_bytes());
                cursor += U32_LEN;
                buf[cursor..cursor + U32_LEN].copy_from_slice(&data.locale.to_be_bytes());
                cursor += U32_LEN;
                buf[cursor..cursor + data.value.len()].copy_from_slice(&data.value);
                Ok(cursor + data.value.len())
            }
            Self::Mean(mean) => {
                serialize_string_box(buf, &MEAN, need, mean.version, mean.flags, &mean.value)
            }
            Self::Name(name) => {
                serialize_string_box(buf, &NAME, need, name.version, name.flags, &name.value)
            }
        }
    }
}

fn write_full_box_head(buf: &mut [u8], cursor: usize, version: u8, flags: u32) -> Result<usize> {
    need_out(&buf[cursor..], FULLBOX_EXTRA_SIZE)?;
    buf[cursor] = version;
    let flags = flags.to_be_bytes();
    buf[cursor + 1..cursor + FULLBOX_EXTRA_SIZE].copy_from_slice(&flags[1..]);
    Ok(FULLBOX_EXTRA_SIZE)
}

fn serialize_string_box(
    buf: &mut [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    need: usize,
    version: u8,
    flags: u32,
    value: &str,
) -> Result<usize> {
    let mut cursor = write_box_header(buf, fourcc, need)?;
    cursor += write_full_box_head(buf, cursor, version, flags)?;
    buf[cursor..cursor + value.len()].copy_from_slice(value.as_bytes());
    Ok(cursor + value.len())
}

/// One item (`©nam`, `trkn`, or a keyed index) in an `ilst`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct IlstItem {
    /// The item atom type: a four-CC or a one-based key index.
    pub item_type: [u8; BOX_TYPE_SIZE],
    /// The item's children, in wire order.
    pub children: Vec<IlstItemChild>,
}

impl IlstItem {
    /// An item from its type and children.
    #[must_use]
    pub fn new(item_type: [u8; BOX_TYPE_SIZE], children: Vec<IlstItemChild>) -> Self {
        Self {
            item_type,
            children,
        }
    }

    /// An item with one `data` value.
    #[must_use]
    pub fn with_data(
        item_type: [u8; BOX_TYPE_SIZE],
        data_type: u32,
        locale: u32,
        value: Vec<u8>,
    ) -> Self {
        Self::new(
            item_type,
            alloc::vec![IlstItemChild::Data(Data::new(data_type, locale, value))],
        )
    }

    /// A freeform `----` item.
    #[must_use]
    pub fn freeform(mean: &str, name: &str, values: &[super::FreeformValue]) -> Self {
        let mut children = Vec::with_capacity(values.len() + FREEFORM_FIXED_CHILDREN);
        children.push(IlstItemChild::Mean(Mean::new(mean)));
        children.push(IlstItemChild::Name(Name::new(name)));
        for value in values {
            children.push(IlstItemChild::Data(Data::new(
                value.data_type,
                value.locale,
                value.bytes.clone(),
            )));
        }
        Self::new(FREEFORM, children)
    }

    /// A `trkn`/`disk` item.
    #[must_use]
    pub fn pair(item_type: [u8; BOX_TYPE_SIZE], index: u16, total: u16) -> Self {
        let mut value = alloc::vec![0u8; PAIR_VALUE_LEN];
        value[PAIR_INDEX_OFFSET..PAIR_INDEX_OFFSET + super::U16_LEN]
            .copy_from_slice(&index.to_be_bytes());
        value[PAIR_TOTAL_OFFSET..PAIR_TOTAL_OFFSET + super::U16_LEN]
            .copy_from_slice(&total.to_be_bytes());
        Self::with_data(item_type, DATA_TYPE_RESERVED, 0, value)
    }

    /// The item's first `data` child.
    #[must_use]
    pub fn data(&self) -> Option<&Data> {
        self.children.iter().find_map(|child| match child {
            IlstItemChild::Data(data) => Some(data),
            _ => None,
        })
    }

    /// The item's `mean` child, when it is a freeform item.
    #[must_use]
    pub fn mean(&self) -> Option<&str> {
        self.children.iter().find_map(|child| match child {
            IlstItemChild::Mean(mean) => Some(mean.value.as_str()),
            _ => None,
        })
    }

    /// The item's `name` child, when it is a freeform item.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.children.iter().find_map(|child| match child {
            IlstItemChild::Name(name) => Some(name.value.as_str()),
            _ => None,
        })
    }

    /// Every `data` child, in order.
    pub fn values(&self) -> impl Iterator<Item = &Data> {
        self.children.iter().filter_map(|child| match child {
            IlstItemChild::Data(data) => Some(data),
            _ => None,
        })
    }

    fn body_len(&self) -> usize {
        self.children.iter().map(Serialize::serialized_len).sum()
    }
}

impl Serialize for IlstItem {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &self.item_type, need)?;
        for child in &self.children {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// The metadata item list atom (`ilst`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Ilst {
    /// The items, in wire order.
    pub items: Vec<IlstItem>,
}

impl Ilst {
    /// An empty list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an item.
    pub fn push_item(&mut self, item: IlstItem) {
        self.items.push(item);
    }

    fn body_len(&self) -> usize {
        self.items.iter().map(Serialize::serialized_len).sum()
    }
}

impl<'a> Parse<'a> for Ilst {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, &Limits::default())
    }
}

impl Ilst {
    /// Parse with the caller's hostile-input caps.
    pub fn parse_with_limits(bytes: &[u8], limits: &Limits) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        if !item.header.box_type.is(&ILST) {
            return Err(Error::UnexpectedBox { expected: "ilst" });
        }
        Self::parse_body(item.body, limits)
    }

    /// Parse an `ilst` from its body.
    pub fn parse_body(body: &[u8], limits: &Limits) -> Result<Self> {
        let mut items = Vec::new();
        for child in BoxIter::new(body) {
            let child = child?;
            items.push(parse_item(&child, limits)?);
        }
        Ok(Self { items })
    }
}

fn parse_item(item: &crate::box_types::BoxItem<'_>, limits: &Limits) -> Result<IlstItem> {
    let mut children = Vec::new();
    for child in BoxIter::new(item.body) {
        let child = child?;
        match child.four_cc() {
            DATA => children.push(IlstItemChild::Data(Data::parse_body(child.body, limits)?)),
            MEAN => children.push(IlstItemChild::Mean(Mean::parse_body(child.body)?)),
            NAME => children.push(IlstItemChild::Name(Name::parse_body(child.body)?)),
            _ => children.push(IlstItemChild::Opaque(OpaqueBox::new(
                child.four_cc(),
                child.payload().to_vec(),
            ))),
        }
    }
    Ok(IlstItem::new(item.four_cc(), children))
}

impl Serialize for Ilst {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &ILST, need)?;
        for item in &self.items {
            cursor += item.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}
