//! The metadata box (`meta`) and its handler reference (`hdlr`).
//!
//! ISO/IEC 14496-12:2026 defines `meta` as a full box (§8.11.1); QuickTime
//! writes it without the version/flags word. The two forms are told apart by
//! the bytes at the start of the body: when a `hdlr` box header follows
//! immediately, the box is the QuickTime form, so the body has no full-box
//! header. The form read is kept and written back unchanged, which is what
//! makes a `.mov` round-trip byte for byte.
//!
//! Children this crate types are `hdlr`, `keys` and `ilst`; any other child
//! (`free` padding included) is an [`OpaqueBox`] at the position it occupied.

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
use super::{Ilst, Keys};

/// The `meta` box type.
pub const META: [u8; 4] = *b"meta";
/// The `hdlr` box type.
pub const HDLR: [u8; 4] = *b"hdlr";

/// The `hdlr` fixed body after the full-box header: `pre_defined`,
/// `handler_type` and three reserved words.
const HDLR_FIXED_BODY_LEN: usize = U32_LEN + BOX_TYPE_SIZE + U32_LEN * 3;
/// Offset of the handler type inside the `hdlr` body.
const HDLR_HANDLER_OFFSET: usize = FULLBOX_EXTRA_SIZE + U32_LEN;
/// Offset of the reserved words inside the `hdlr` body.
const HDLR_RESERVED_OFFSET: usize = HDLR_HANDLER_OFFSET + BOX_TYPE_SIZE;
/// Number of reserved words in an `hdlr`.
const HDLR_RESERVED_WORDS: usize = 3;
/// The full-box header plus the `hdlr` fixed body.
const HDLR_MIN_BODY_LEN: usize = FULLBOX_EXTRA_SIZE + HDLR_FIXED_BODY_LEN;
/// The body offset a QuickTime `meta`'s first child header sits at.
const QUICKTIME_HDLR_TYPE_OFFSET: usize = BOX_TYPE_SIZE;

/// Which wire form a `meta` box uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MetaFlavor {
    /// ISO/IEC 14496-12:2026 §8.11.1: a full box.
    #[default]
    Iso,
    /// QuickTime: no version/flags word.
    QuickTime,
}

/// A handler reference box (`hdlr`, ISO/IEC 14496-12:2026 §8.4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Hdlr {
    /// FullBox version.
    pub version: u8,
    /// FullBox flags.
    pub flags: u32,
    /// The `pre_defined` field.
    pub pre_defined: u32,
    /// The `handler_type` (`mdir`, `mdta`, …).
    pub handler_type: [u8; BOX_TYPE_SIZE],
    /// The reserved words.
    pub reserved: [u32; HDLR_RESERVED_WORDS],
    /// The name bytes after the fixed body, terminator included when present.
    pub name: Vec<u8>,
}

impl Hdlr {
    /// A version-0 handler box with an empty name.
    #[must_use]
    pub fn new(handler_type: [u8; BOX_TYPE_SIZE]) -> Self {
        Self {
            version: 0,
            flags: 0,
            pre_defined: 0,
            handler_type,
            reserved: [0; HDLR_RESERVED_WORDS],
            name: Vec::new(),
        }
    }

    fn body_len(&self) -> usize {
        HDLR_MIN_BODY_LEN + self.name.len()
    }

    /// Parse a whole `hdlr` box (header included).
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        if !item.four_cc().eq(&HDLR) {
            return Err(Error::InvalidInput("expected hdlr"));
        }
        Self::parse_body(item.body)
    }

    fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, HDLR_MIN_BODY_LEN, "hdlr body")?;
        let reserved_slice = &body[HDLR_RESERVED_OFFSET..HDLR_MIN_BODY_LEN];
        let mut reserved = [0u32; HDLR_RESERVED_WORDS];
        for (slot, chunk) in reserved
            .iter_mut()
            .zip(reserved_slice.chunks_exact(U32_LEN))
        {
            *slot = super::u32_at(chunk, 0);
        }
        let handler_type = body[HDLR_HANDLER_OFFSET..HDLR_RESERVED_OFFSET]
            .try_into()
            .map_err(|_| Error::InvalidInput("hdlr handler_type"))?;
        Ok(Self {
            version: body[0],
            flags: broadcast_core::wire::u24_at(body, FULLBOX_FLAGS_OFFSET),
            pre_defined: super::u32_at(body, FULLBOX_EXTRA_SIZE),
            handler_type,
            reserved,
            name: body[HDLR_MIN_BODY_LEN..].to_vec(),
        })
    }
}

impl Serialize for Hdlr {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &HDLR, need)?;
        buf[cursor] = self.version;
        cursor += 1;
        let flags = self.flags.to_be_bytes();
        buf[cursor..cursor + (FULLBOX_EXTRA_SIZE - 1)].copy_from_slice(&flags[1..]);
        cursor += FULLBOX_EXTRA_SIZE - 1;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.pre_defined.to_be_bytes());
        cursor += U32_LEN;
        buf[cursor..cursor + BOX_TYPE_SIZE].copy_from_slice(&self.handler_type);
        cursor += BOX_TYPE_SIZE;
        for word in self.reserved {
            buf[cursor..cursor + U32_LEN].copy_from_slice(&word.to_be_bytes());
            cursor += U32_LEN;
        }
        buf[cursor..cursor + self.name.len()].copy_from_slice(&self.name);
        Ok(cursor + self.name.len())
    }
}

/// A child of a `meta` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MetaChild {
    /// The handler reference.
    Hdlr(Hdlr),
    /// The metadata item keys table.
    Keys(Keys),
    /// The metadata item list.
    Ilst(Ilst),
    /// The primary XML document of the meta box (`xml `, §8.11.2).
    Xml(crate::meta_track::XmlBox),
    /// The binarized XML document of the meta box (`bxml`, §8.11.2).
    BinaryXml(crate::meta_track::BinaryXmlBox),
    /// Any other child, preserved verbatim.
    Opaque(OpaqueBox),
}

impl Serialize for MetaChild {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Hdlr(hdlr) => hdlr.serialized_len(),
            Self::Keys(keys) => keys.serialized_len(),
            Self::Ilst(ilst) => ilst.serialized_len(),
            Self::Xml(xml) => xml.serialized_len(),
            Self::BinaryXml(bxml) => bxml.serialized_len(),
            Self::Opaque(opaque) => opaque.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Hdlr(hdlr) => hdlr.serialize_into(buf),
            Self::Keys(keys) => keys.serialize_into(buf),
            Self::Ilst(ilst) => ilst.serialize_into(buf),
            Self::Xml(xml) => xml.serialize_into(buf),
            Self::BinaryXml(bxml) => bxml.serialize_into(buf),
            Self::Opaque(opaque) => opaque.serialize_into(buf),
        }
    }
}

/// The metadata box (`meta`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Meta {
    /// Which wire form the box uses.
    pub flavor: MetaFlavor,
    /// FullBox version (zero in the QuickTime form).
    pub version: u8,
    /// FullBox flags (zero in the QuickTime form).
    pub flags: u32,
    /// The children, in wire order.
    pub children: Vec<MetaChild>,
}

impl Meta {
    /// An empty `meta` in the given form.
    #[must_use]
    pub fn new(flavor: MetaFlavor) -> Self {
        Self {
            flavor,
            version: 0,
            flags: 0,
            children: Vec::new(),
        }
    }

    /// The `hdlr.handler_type`, when a handler is present.
    #[must_use]
    pub fn handler(&self) -> Option<[u8; BOX_TYPE_SIZE]> {
        self.children.iter().find_map(|child| match child {
            MetaChild::Hdlr(hdlr) => Some(hdlr.handler_type),
            _ => None,
        })
    }

    /// The `keys` table, when present.
    #[must_use]
    pub fn keys(&self) -> Option<&Keys> {
        self.children.iter().find_map(|child| match child {
            MetaChild::Keys(keys) => Some(keys),
            _ => None,
        })
    }

    /// The `ilst` item list, when present.
    #[must_use]
    pub fn ilst(&self) -> Option<&Ilst> {
        self.children.iter().find_map(|child| match child {
            MetaChild::Ilst(ilst) => Some(ilst),
            _ => None,
        })
    }

    fn body_len(&self) -> usize {
        let full_box = if self.flavor == MetaFlavor::Iso {
            FULLBOX_EXTRA_SIZE
        } else {
            0
        };
        full_box
            + self
                .children
                .iter()
                .map(Serialize::serialized_len)
                .sum::<usize>()
    }
}

impl<'a> Parse<'a> for Meta {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, &Limits::default())
    }
}

impl Meta {
    /// Parse a `meta` box with the caller's hostile-input caps.
    pub fn parse_with_limits(bytes: &[u8], limits: &Limits) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        if !item.header.box_type.is(&META) {
            return Err(Error::UnexpectedBox { expected: "meta" });
        }
        Self::parse_body(item.body, limits)
    }

    /// Parse a `meta` from its body.
    pub fn parse_body(body: &[u8], limits: &Limits) -> Result<Self> {
        let (flavor, version, flags, children_body) = if body.len() >= BOX_HEADER_MIN_SIZE
            && &body[QUICKTIME_HDLR_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] == b"hdlr"
        {
            (MetaFlavor::QuickTime, 0, 0, body)
        } else {
            need_in(body, FULLBOX_EXTRA_SIZE, "meta full-box header")?;
            (
                MetaFlavor::Iso,
                body[0],
                broadcast_core::wire::u24_at(body, FULLBOX_FLAGS_OFFSET),
                &body[FULLBOX_EXTRA_SIZE..],
            )
        };
        let mut children = Vec::new();
        for child in BoxIter::new(children_body) {
            let child = child?;
            let parsed = match child.four_cc() {
                HDLR => MetaChild::Hdlr(Hdlr::parse_body(child.body)?),
                super::keys::KEYS => MetaChild::Keys(Keys::parse_body(child.body)?),
                crate::meta_track::XML => {
                    MetaChild::Xml(crate::meta_track::XmlBox::parse_body(child.body)?)
                }
                crate::meta_track::BXML => {
                    MetaChild::BinaryXml(crate::meta_track::BinaryXmlBox::parse_body(child.body)?)
                }
                super::ilst::ILST => MetaChild::Ilst(Ilst::parse_body(child.body, limits)?),
                _ => MetaChild::Opaque(OpaqueBox::new(child.four_cc(), child.payload().to_vec())),
            };
            children.push(parsed);
        }
        Ok(Self {
            flavor,
            version,
            flags,
            children,
        })
    }
}

impl Serialize for Meta {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &META, need)?;
        if self.flavor == MetaFlavor::Iso {
            buf[cursor] = self.version;
            cursor += 1;
            let flags = self.flags.to_be_bytes();
            buf[cursor..cursor + (FULLBOX_EXTRA_SIZE - 1)].copy_from_slice(&flags[1..]);
            cursor += FULLBOX_EXTRA_SIZE - 1;
        }
        for child in &self.children {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// The four-CC a handler uses for iTunes-style keyed metadata.
pub const MDTA_HANDLER: [u8; BOX_TYPE_SIZE] = *b"mdta";
/// The four-CC a handler uses for iTunes four-character metadata.
pub const MDIR_HANDLER: [u8; BOX_TYPE_SIZE] = *b"mdir";
