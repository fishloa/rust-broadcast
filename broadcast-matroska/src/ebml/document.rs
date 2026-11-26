//! The EBML header: the declaration every EBML document starts with
//! (RFC 8794 §8.1, §11.2).
//!
//! The header's own elements are all defined by RFC 8794, so this module names
//! them here rather than in the Matroska registry. Every child keeps its
//! framing, so the header re-serialises byte-identically.

use alloc::vec::Vec;

use broadcast_core::{Parse, Serialize};

use crate::ebml::global::{CRC32_ID, Crc32, VOID_ID, Void};
use crate::ebml::leaf::{TextElement, Unknown, UnsignedElement};
use crate::ebml::scalar::{Text, Unsigned};
use crate::ebml::{ElementHeader, ElementIter};
use crate::error::{Error, Result};
use crate::limits::Limits;

/// Element ID of the EBML header (RFC 8794 §11.2.1).
pub const EBML_ID: u32 = 0x1A45_DFA3;
/// Element ID of `EBMLVersion` (§11.2.2).
pub const EBML_VERSION_ID: u32 = 0x4286;
/// Element ID of `EBMLReadVersion` (§11.2.3).
pub const EBML_READ_VERSION_ID: u32 = 0x42F7;
/// Element ID of `EBMLMaxIDLength` (§11.2.4).
pub const EBML_MAX_ID_LENGTH_ID: u32 = 0x42F2;
/// Element ID of `EBMLMaxSizeLength` (§11.2.5).
pub const EBML_MAX_SIZE_LENGTH_ID: u32 = 0x42F3;
/// Element ID of `DocType` (§11.2.6).
pub const DOC_TYPE_ID: u32 = 0x4282;
/// Element ID of `DocTypeVersion` (§11.2.7).
pub const DOC_TYPE_VERSION_ID: u32 = 0x4287;
/// Element ID of `DocTypeReadVersion` (§11.2.8).
pub const DOC_TYPE_READ_VERSION_ID: u32 = 0x4285;
/// Element ID of `DocTypeExtension` (§11.2.9).
pub const DOC_TYPE_EXTENSION_ID: u32 = 0x4281;
/// Element ID of `DocTypeExtensionName` (§11.2.10).
pub const DOC_TYPE_EXTENSION_NAME_ID: u32 = 0x4283;
/// Element ID of `DocTypeExtensionVersion` (§11.2.11).
pub const DOC_TYPE_EXTENSION_VERSION_ID: u32 = 0x4284;

/// The default `EBMLVersion` (RFC 8794 §11.2.2).
pub const DEFAULT_EBML_VERSION: u64 = 1;
/// The default `EBMLReadVersion` (§11.2.3).
pub const DEFAULT_EBML_READ_VERSION: u64 = 1;
/// The default `EBMLMaxIDLength` (§11.2.4).
pub const DEFAULT_EBML_MAX_ID_LENGTH: u64 = 4;
/// The default `EBMLMaxSizeLength` (§11.2.5).
pub const DEFAULT_EBML_MAX_SIZE_LENGTH: u64 = 8;
/// The default `DocTypeVersion` (§11.2.7).
pub const DEFAULT_DOC_TYPE_VERSION: u64 = 1;
/// The default `DocTypeReadVersion` (§11.2.8).
pub const DEFAULT_DOC_TYPE_READ_VERSION: u64 = 1;

/// One child of the EBML header.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EbmlHeaderChild<'a> {
    /// `EBMLVersion`.
    Version(UnsignedElement),
    /// `EBMLReadVersion`.
    ReadVersion(UnsignedElement),
    /// `EBMLMaxIDLength`.
    MaxIdLength(UnsignedElement),
    /// `EBMLMaxSizeLength`.
    MaxSizeLength(UnsignedElement),
    /// `DocType`.
    DocType(TextElement<'a>),
    /// `DocTypeVersion`.
    DocTypeVersion(UnsignedElement),
    /// `DocTypeReadVersion`.
    DocTypeReadVersion(UnsignedElement),
    /// `DocTypeExtension`.
    Extension(DocTypeExtension<'a>),
    /// A global `CRC-32`.
    Crc32(Crc32),
    /// A global `Void`.
    Void(Void<'a>),
    /// An element this crate does not type.
    Unknown(Unknown<'a>),
}

impl EbmlHeaderChild<'_> {
    /// The child's element header.
    #[must_use]
    pub const fn header(&self) -> ElementHeader {
        match self {
            Self::Version(e)
            | Self::ReadVersion(e)
            | Self::MaxIdLength(e)
            | Self::MaxSizeLength(e)
            | Self::DocTypeVersion(e)
            | Self::DocTypeReadVersion(e) => e.header(),
            Self::DocType(e) => e.header(),
            Self::Extension(e) => e.header(),
            Self::Crc32(e) => e.header(),
            Self::Void(e) => e.header(),
            Self::Unknown(e) => e.header(),
        }
    }

    /// The untyped integer payload, if this child is an integer.
    #[must_use]
    pub fn as_unsigned(&self) -> Option<u64> {
        match self {
            Self::Version(e)
            | Self::ReadVersion(e)
            | Self::MaxIdLength(e)
            | Self::MaxSizeLength(e)
            | Self::DocTypeVersion(e)
            | Self::DocTypeReadVersion(e) => Some(e.value().value()),
            _ => None,
        }
    }
}

impl<'a> Parse<'a> for EbmlHeaderChild<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (header, payload, _) = ElementHeader::split(bytes)?;
        parse_child(header, payload, 1)
    }
}

impl Serialize for EbmlHeaderChild<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Version(e)
            | Self::ReadVersion(e)
            | Self::MaxIdLength(e)
            | Self::MaxSizeLength(e)
            | Self::DocTypeVersion(e)
            | Self::DocTypeReadVersion(e) => e.serialized_len(),
            Self::DocType(e) => e.serialized_len(),
            Self::Extension(e) => e.serialized_len(),
            Self::Crc32(e) => e.serialized_len(),
            Self::Void(e) => e.serialized_len(),
            Self::Unknown(e) => e.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Version(e)
            | Self::ReadVersion(e)
            | Self::MaxIdLength(e)
            | Self::MaxSizeLength(e)
            | Self::DocTypeVersion(e)
            | Self::DocTypeReadVersion(e) => e.serialize_into(buf),
            Self::DocType(e) => e.serialize_into(buf),
            Self::Extension(e) => e.serialize_into(buf),
            Self::Crc32(e) => e.serialize_into(buf),
            Self::Void(e) => e.serialize_into(buf),
            Self::Unknown(e) => e.serialize_into(buf),
        }
    }
}

/// A `DocTypeExtension` element (RFC 8794 §11.2.9).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DocTypeExtension<'a> {
    header: ElementHeader,
    children: Vec<EbmlHeaderChild<'a>>,
}

impl<'a> DocTypeExtension<'a> {
    /// Build one from a parsed header and children.
    #[must_use]
    pub const fn from_parts(header: ElementHeader, children: Vec<EbmlHeaderChild<'a>>) -> Self {
        Self { header, children }
    }

    /// The element header.
    #[must_use]
    pub const fn header(&self) -> ElementHeader {
        self.header
    }

    /// The child elements, in file order.
    #[must_use]
    pub fn children(&self) -> &[EbmlHeaderChild<'a>] {
        &self.children
    }
}

impl<'a> Parse<'a> for DocTypeExtension<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (header, children, _) = parse_master(bytes, DOC_TYPE_EXTENSION_ID)?;
        Ok(Self::from_parts(header, children))
    }
}

impl Serialize for DocTypeExtension<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let payload: usize = self.children.iter().map(Serialize::serialized_len).sum();
        self.header.serialized_len() + payload
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_master(self.header, &self.children, buf)
    }
}

/// The EBML header (RFC 8794 §8.1, §11.2).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EbmlHeader<'a> {
    header: ElementHeader,
    children: Vec<EbmlHeaderChild<'a>>,
}

impl<'a> EbmlHeader<'a> {
    /// Build one from a parsed header and children.
    #[must_use]
    pub const fn from_parts(header: ElementHeader, children: Vec<EbmlHeaderChild<'a>>) -> Self {
        Self { header, children }
    }

    /// The element header.
    #[must_use]
    pub const fn header(&self) -> ElementHeader {
        self.header
    }

    /// The child elements, in file order.
    #[must_use]
    pub fn children(&self) -> &[EbmlHeaderChild<'a>] {
        &self.children
    }

    /// `DocType` as text, if present (RFC 8794 §11.2.6).
    #[must_use]
    pub fn doc_type(&self) -> Option<&'a str> {
        self.children.iter().find_map(|child| match child {
            EbmlHeaderChild::DocType(e) => e.value().as_str(),
            _ => None,
        })
    }

    /// `EBMLVersion` (default 1).
    #[must_use]
    pub fn version(&self) -> u64 {
        self.integer(EBML_VERSION_ID, DEFAULT_EBML_VERSION)
    }

    /// `EBMLReadVersion` (default 1).
    #[must_use]
    pub fn read_version(&self) -> u64 {
        self.integer(EBML_READ_VERSION_ID, DEFAULT_EBML_READ_VERSION)
    }

    /// `EBMLMaxIDLength` (default 4).
    #[must_use]
    pub fn max_id_length(&self) -> u64 {
        self.integer(EBML_MAX_ID_LENGTH_ID, DEFAULT_EBML_MAX_ID_LENGTH)
    }

    /// `EBMLMaxSizeLength` (default 8).
    #[must_use]
    pub fn max_size_length(&self) -> u64 {
        self.integer(EBML_MAX_SIZE_LENGTH_ID, DEFAULT_EBML_MAX_SIZE_LENGTH)
    }

    /// `DocTypeVersion` (default 1).
    #[must_use]
    pub fn doc_type_version(&self) -> u64 {
        self.integer(DOC_TYPE_VERSION_ID, DEFAULT_DOC_TYPE_VERSION)
    }

    /// `DocTypeReadVersion` (default 1).
    #[must_use]
    pub fn doc_type_read_version(&self) -> u64 {
        self.integer(DOC_TYPE_READ_VERSION_ID, DEFAULT_DOC_TYPE_READ_VERSION)
    }

    /// The integer child with `id`, or `default`.
    fn integer(&self, id: u32, default: u64) -> u64 {
        self.children
            .iter()
            .find(|child| child.header().id().value() == id)
            .and_then(EbmlHeaderChild::as_unsigned)
            .unwrap_or(default)
    }
}

impl<'a> Parse<'a> for EbmlHeader<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (header, children, _) = parse_master(bytes, EBML_ID)?;
        Ok(Self::from_parts(header, children))
    }
}

impl Serialize for EbmlHeader<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let payload: usize = self.children.iter().map(Serialize::serialized_len).sum();
        self.header.serialized_len() + payload
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        write_master(self.header, &self.children, buf)
    }
}

/// Parse one EBML-header child from its header and payload.
fn parse_child<'a>(
    header: ElementHeader,
    payload: &'a [u8],
    depth: u32,
) -> Result<EbmlHeaderChild<'a>> {
    let result = match header.id().value() {
        EBML_VERSION_ID => {
            EbmlHeaderChild::Version(UnsignedElement::new(header, Unsigned::parse(payload)?))
        }
        EBML_READ_VERSION_ID => {
            EbmlHeaderChild::ReadVersion(UnsignedElement::new(header, Unsigned::parse(payload)?))
        }
        EBML_MAX_ID_LENGTH_ID => {
            EbmlHeaderChild::MaxIdLength(UnsignedElement::new(header, Unsigned::parse(payload)?))
        }
        EBML_MAX_SIZE_LENGTH_ID => {
            EbmlHeaderChild::MaxSizeLength(UnsignedElement::new(header, Unsigned::parse(payload)?))
        }
        DOC_TYPE_ID => EbmlHeaderChild::DocType(TextElement::new(header, Text::parse(payload)?)),
        DOC_TYPE_VERSION_ID => {
            EbmlHeaderChild::DocTypeVersion(UnsignedElement::new(header, Unsigned::parse(payload)?))
        }
        DOC_TYPE_READ_VERSION_ID => EbmlHeaderChild::DocTypeReadVersion(UnsignedElement::new(
            header,
            Unsigned::parse(payload)?,
        )),
        DOC_TYPE_EXTENSION_ID => {
            let children = parse_children(payload, &Limits::new(), depth + 1)?;
            EbmlHeaderChild::Extension(DocTypeExtension::from_parts(header, children))
        }
        CRC32_ID => EbmlHeaderChild::Crc32(Crc32::from_payload(header, payload)?),
        VOID_ID => EbmlHeaderChild::Void(Void::new(header, payload)?),
        _ => EbmlHeaderChild::Unknown(Unknown::new(header, payload)),
    };
    Ok(result)
}

/// Parse a known-size EBML master's children and check its id.
fn parse_master<'a>(
    bytes: &'a [u8],
    expected_id: u32,
) -> Result<(ElementHeader, Vec<EbmlHeaderChild<'a>>, usize)> {
    let (header, payload, used) = ElementHeader::split(bytes)?;
    if header.id().value() != expected_id {
        return Err(Error::UnexpectedElement {
            expected: expected_id,
            found: header.id().value(),
        });
    }
    let children = parse_children(payload, &Limits::new(), 1)?;
    Ok((header, children, used))
}

/// Parse every child in a known-size master payload.
fn parse_children<'a>(
    payload: &'a [u8],
    limits: &Limits,
    depth: u32,
) -> Result<Vec<EbmlHeaderChild<'a>>> {
    if depth > limits.max_depth() {
        return Err(Error::LimitExceeded {
            limit: "max_depth",
            value: u64::from(depth),
        });
    }
    let mut children = Vec::new();
    let mut iterator = ElementIter::new(payload, *limits);
    while let Some(element) = iterator.next_element() {
        let element = element?;
        children.push(parse_child(element.header(), element.data(), depth)?);
        if children.len() > limits.max_children() as usize {
            return Err(Error::LimitExceeded {
                limit: "max_children",
                value: children.len() as u64,
            });
        }
    }
    Ok(children)
}

/// Write a master's header and children, returning the octets written.
fn write_master<T: Serialize<Error = Error>>(
    header: ElementHeader,
    children: &[T],
    buf: &mut [u8],
) -> Result<usize> {
    let payload: usize = children.iter().map(Serialize::serialized_len).sum();
    let payload_len = u64::try_from(payload).map_err(|_| Error::SizeOverflow)?;
    let header = header.resized(payload_len)?;
    let mut used = header.write(buf)?;
    for child in children {
        let have = buf.len();
        let rest = buf.get_mut(used..).ok_or(Error::OutputBufferTooSmall(
            broadcast_core::error::OutputBufferTooSmall {
                need: used + child.serialized_len(),
                have,
            },
        ))?;
        used += child.serialize_into(rest)?;
    }
    Ok(used)
}

/// Read the EBML header that starts `bytes`, returning it and the octets
/// consumed.
pub fn parse_document_start(bytes: &[u8]) -> Result<(EbmlHeader<'_>, usize)> {
    let (header, payload, used) = ElementHeader::split(bytes)?;
    if header.id().value() != EBML_ID {
        return Err(Error::UnexpectedElement {
            expected: EBML_ID,
            found: header.id().value(),
        });
    }
    let children = parse_children(payload, &Limits::new(), 1)?;
    Ok((EbmlHeader::from_parts(header, children), used))
}
