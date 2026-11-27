//! Timed-metadata and text-track boxes — ISO/IEC 14496-12:2026 §12.3.3,
//! §8.10.2, §8.11.2, §12.8.4.
//!
//! Transcription: `private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`.
//!
//! | Box | Type | Clause | Container |
//! |-----|------|--------|-----------|
//! | [`UriBox`] | `uri ` | §12.3.3 | `urim` |
//! | [`UriInitBox`] | `uriI` | §12.3.3 | `urim` |
//! | [`TextConfigBox`] | `txtC` | §12.3.3 | `mett`/`stxt` |
//! | [`MimeBox`] | `mime` | §12.3.3 | metadata sample entry |
//! | [`XmlBox`] | `xml ` | §8.11.2 | `meta` |
//! | [`BinaryXmlBox`] | `bxml` | §8.11.2 | `meta` |
//! | [`CopyrightBox`] | `cprt` | §8.10.2 | `udta` |
//! | [`MetadataKeyDeclarationBox`] | `keyd` | §12.8.4.4 | metadata key box |
//! | [`MetadataLocaleBox`] | `loca` | §12.8.4.5 | metadata key box |
//! | [`MetadataSetupBox`] | `setu` | §12.8.4.6 | metadata key box |
//! | [`MetadataKeyBox`] | local key id | §12.8.4.3 | `keys` (track form) |
//! | [`MetadataKeyTableBox`] | `keys` | §12.8.4.2 | `mebx` |
//! | [`XmlMetaDataSampleEntry`] | `metx` | §12.3.3 | `stsd` |
//! | [`TextMetaDataSampleEntry`] | `mett`/`stxt` | §12.3.3, §12.5.3 | `stsd` |
//! | [`UriMetaDataSampleEntry`] | `urim` | §12.3.3 | `stsd` |
//! | [`BoxedMetaDataSampleEntry`] | `mebx` | §12.8.4.1 | `stsd` |
//!
//! Every type is private-field with a builder and accessors, and round-trips
//! byte-identically.
//!
//! The `keys` four-CC has two shapes in one file: [`crate::metadata::Keys`] is
//! the Apple full-box item-key table a file-level `meta` carries, while
//! [`MetadataKeyTableBox`] is the plain-box track table §12.8.4.2 defines,
//! whose children are [`MetadataKeyBox`]es typed by their local key id.

use alloc::vec::Vec;

use broadcast_core::len::{need_in, need_out};
use broadcast_core::wire::{be_u16, u24_at, u32_at};
use broadcast_core::{Parse, Serialize};

use crate::box_support::{OpaqueBox, take_c_string, write_c_string};
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK,
    box_len, parse_box, write_box_header, write_full_box_header,
};
use crate::error::{Error, Result};

/// Four-CC of the URI Box (`uri `, §12.3.3).
pub const URI: [u8; BOX_TYPE_SIZE] = *b"uri ";
/// Four-CC of the URI Initialization Box (`uriI`, §12.3.3).
pub const URI_INIT: [u8; BOX_TYPE_SIZE] = *b"uriI";
/// Four-CC of the Text Configuration Box (`txtC`, §12.3.3).
pub const TXTC: [u8; BOX_TYPE_SIZE] = *b"txtC";
/// Four-CC of the MIME Box (`mime`, §12.3.3).
pub const MIME: [u8; BOX_TYPE_SIZE] = *b"mime";
/// Four-CC of the XML Box (`xml `, §8.11.2).
pub const XML: [u8; BOX_TYPE_SIZE] = *b"xml ";
/// Four-CC of the Binary XML Box (`bxml`, §8.11.2).
pub const BXML: [u8; BOX_TYPE_SIZE] = *b"bxml";
/// Four-CC of the Copyright Box (`cprt`, §8.10.2).
pub const CPRT: [u8; BOX_TYPE_SIZE] = *b"cprt";
/// Four-CC of the Metadata Key Declaration Box (`keyd`, §12.8.4.4).
pub const KEYD: [u8; BOX_TYPE_SIZE] = *b"keyd";
/// Four-CC of the Metadata Locale Box (`loca`, §12.8.4.5).
pub const LOCA: [u8; BOX_TYPE_SIZE] = *b"loca";
/// Four-CC of the Metadata Setup Box (`setu`, §12.8.4.6).
pub const SETU: [u8; BOX_TYPE_SIZE] = *b"setu";
/// Four-CC of the Metadata Key Table Box (track form, §12.8.4.2).
pub const KEYS: [u8; BOX_TYPE_SIZE] = *b"keys";
/// Four-CC of the XML metadata sample entry (`metx`, §12.3.3).
pub const METX: [u8; BOX_TYPE_SIZE] = *b"metx";
/// Four-CC of the text metadata sample entry (`mett`, §12.3.3).
pub const METT: [u8; BOX_TYPE_SIZE] = *b"mett";
/// Four-CC of the simple-text sample entry (`stxt`, §12.5.3).
pub const STXT: [u8; BOX_TYPE_SIZE] = *b"stxt";
/// Four-CC of the URI metadata sample entry (`urim`, §12.3.3).
pub const URIM: [u8; BOX_TYPE_SIZE] = *b"urim";
/// Four-CC of the boxed metadata sample entry (`mebx`, §12.8.4.1).
pub const MEBX: [u8; BOX_TYPE_SIZE] = *b"mebx";

/// Width of a `u16` on the wire.
const U16_LEN: usize = core::mem::size_of::<u16>();
/// Width of a `u32` on the wire.
const U32_LEN: usize = core::mem::size_of::<u32>();
/// SampleEntry `reserved[6]` (§8.5.2.2).
const SAMPLE_ENTRY_RESERVED_LEN: usize = 6;
/// SampleEntry fixed part: `reserved[6]` plus `data_reference_index` (§8.5.2.2).
const SAMPLE_ENTRY_FIXED_LEN: usize = SAMPLE_ENTRY_RESERVED_LEN + U16_LEN;
/// `data_reference_index` offset inside a SampleEntry body (§8.5.2.2).
const DATA_REFERENCE_INDEX_OFFSET: usize = SAMPLE_ENTRY_RESERVED_LEN;
/// `cprt` `pad` bit plus three 5-bit language letters (§8.10.2.2).
const CPRT_LANGUAGE_BITS: u32 = 15;
/// Mask of the packed language code in `cprt` (§8.10.2.2).
const CPRT_LANGUAGE_MASK: u16 = (1 << CPRT_LANGUAGE_BITS) - 1;
/// A leading UTF-16 byte-order mark is two bytes (§4.2.1).
const BOM_LEN: usize = 2;

/// Serialize a FullBox whose payload is one `utf8string`.
fn serialize_string_full_box(
    buf: &mut [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    version: u8,
    flags: u32,
    value: &[u8],
) -> Result<usize> {
    let need = box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + value.len() + 1);
    need_out(buf, need)?;
    let mut cursor = write_full_box_header(buf, fourcc, need, version, flags, false)?;
    cursor += write_c_string(buf, cursor, value)?;
    Ok(cursor)
}

fn expect_type(bytes: &[u8], fourcc: &[u8; BOX_TYPE_SIZE], what: &'static str) -> Result<()> {
    need_in(bytes, BOX_HEADER_MIN_SIZE, what)?;
    if &bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] != fourcc {
        return Err(Error::InvalidInput(what));
    }
    Ok(())
}

/// Parse a FullBox body that is one `utf8string`, returning
/// `(version, flags, bytes)`.
fn parse_string_body(body: &[u8], what: &'static str) -> Result<(u8, u32, Vec<u8>)> {
    need_in(body, FULLBOX_EXTRA_SIZE, what)?;
    let version = body[0];
    let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
    let mut cursor = FULLBOX_EXTRA_SIZE;
    let value = take_c_string(body, &mut cursor, what)?;
    Ok((version, flags, value))
}

/// URI Box (`uri `) — ISO/IEC 14496-12:2026 §12.3.3.
///
/// The URI identifies the form of the metadata samples and of the optional
/// initialization data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct UriBox {
    version: u8,
    flags: u32,
    uri: Vec<u8>,
}

impl UriBox {
    /// Build one from a URI string.
    #[must_use]
    pub fn new(uri: &str) -> Self {
        Self {
            version: 0,
            flags: 0,
            uri: uri.as_bytes().to_vec(),
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

    /// The raw `theURI` bytes.
    #[must_use]
    pub fn uri(&self) -> &[u8] {
        &self.uri
    }

    /// The URI as UTF-8, when it is.
    #[must_use]
    pub fn uri_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.uri).ok()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (version, flags, uri) = parse_string_body(body, "uri ")?;
        Ok(Self {
            version,
            flags,
            uri,
        })
    }
}

impl<'a> Parse<'a> for UriBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &URI, "uri ")?;
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "uri ",
        )?)
    }
}

impl Serialize for UriBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.uri.len() + 1)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_string_full_box(buf, &URI, self.version, self.flags, &self.uri)
    }
}

/// Text Configuration Box (`txtC`) — ISO/IEC 14496-12:2026 §12.3.3.
///
/// The initial text of each document, prepended to the contents of each sync
/// sample.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TextConfigBox {
    version: u8,
    flags: u32,
    text_config: Vec<u8>,
}

impl TextConfigBox {
    /// Build one from the initial text.
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            version: 0,
            flags: 0,
            text_config: text.as_bytes().to_vec(),
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

    /// The raw `text_config` bytes.
    #[must_use]
    pub fn text_config(&self) -> &[u8] {
        &self.text_config
    }

    /// The initial text as UTF-8, when it is.
    #[must_use]
    pub fn text_config_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.text_config).ok()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (version, flags, text_config) = parse_string_body(body, "txtC")?;
        Ok(Self {
            version,
            flags,
            text_config,
        })
    }
}

impl<'a> Parse<'a> for TextConfigBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &TXTC, "txtC")?;
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "txtC",
        )?)
    }
}

impl Serialize for TextConfigBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.text_config.len() + 1)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_string_full_box(buf, &TXTC, self.version, self.flags, &self.text_config)
    }
}

/// MIME Box (`mime`) — ISO/IEC 14496-12:2026 §12.3.3.
///
/// The MIME type each XML document in the stream has when delivered on its
/// own.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MimeBox {
    version: u8,
    flags: u32,
    content_type: Vec<u8>,
}

impl MimeBox {
    /// Build one from a MIME type.
    #[must_use]
    pub fn new(content_type: &str) -> Self {
        Self {
            version: 0,
            flags: 0,
            content_type: content_type.as_bytes().to_vec(),
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

    /// The raw `content_type` bytes.
    #[must_use]
    pub fn content_type(&self) -> &[u8] {
        &self.content_type
    }

    /// The MIME type as UTF-8, when it is.
    #[must_use]
    pub fn content_type_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.content_type).ok()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (version, flags, content_type) = parse_string_body(body, "mime")?;
        Ok(Self {
            version,
            flags,
            content_type,
        })
    }
}

impl<'a> Parse<'a> for MimeBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &MIME, "mime")?;
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "mime",
        )?)
    }
}

impl Serialize for MimeBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.content_type.len() + 1)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_string_full_box(buf, &MIME, self.version, self.flags, &self.content_type)
    }
}

/// URI Initialization Box (`uriI`) — ISO/IEC 14496-12:2026 §12.3.3.
///
/// `uri_initialization_data` is opaque: its form is defined by the URI form in
/// the sibling [`UriBox`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct UriInitBox {
    version: u8,
    flags: u32,
    data: Vec<u8>,
}

impl UriInitBox {
    /// Build one from the initialization bytes.
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            version: 0,
            flags: 0,
            data,
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

    /// The opaque `uri_initialization_data` bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "uriI")?;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            data: body[FULLBOX_EXTRA_SIZE..].to_vec(),
        })
    }
}

impl<'a> Parse<'a> for UriInitBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &URI_INIT, "uriI")?;
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "uriI",
        )?)
    }
}

impl Serialize for UriInitBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.data.len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor =
            write_full_box_header(buf, &URI_INIT, need, self.version, self.flags, false)?;
        buf[cursor..cursor + self.data.len()].copy_from_slice(&self.data);
        cursor += self.data.len();
        Ok(cursor)
    }
}

/// XML Box (`xml `) — ISO/IEC 14496-12:2026 §8.11.2.
///
/// `xml` is a `utfstring` (§4.2.1): UTF-8 terminated by one NUL, or UTF-16
/// (with a byte-order mark) terminated by two. This edition requires the
/// terminator; a reader tolerates a box that omits it, which
/// [`XmlBox::is_null_terminated`] reports so the bytes round-trip unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct XmlBox {
    version: u8,
    flags: u32,
    xml: Vec<u8>,
    terminator: usize,
}

/// Unicode byte-order marks a `utfstring` may start with (§4.2.1).
const UTF16_BOMS: [[u8; BOM_LEN]; 2] = [[0xFF, 0xFE], [0xFE, 0xFF]];
/// Width of a single-NUL `utfstring` terminator (§4.2.1).
const UTF8_TERMINATOR_LEN: usize = 1;
/// Width of a UTF-16 `utfstring` terminator (§4.2.1).
const UTF16_TERMINATOR_LEN: usize = 2;
/// No terminator was present on the wire.
const NO_TERMINATOR: usize = 0;

/// The terminator width a `utfstring` payload carries, and the text before it.
fn split_utfstring(payload: &[u8]) -> (Vec<u8>, usize) {
    let utf16 = payload
        .get(..BOM_LEN)
        .is_some_and(|head| UTF16_BOMS.iter().any(|bom| bom == head));
    if utf16 {
        let mut end = payload.len();
        while end >= UTF16_TERMINATOR_LEN
            && payload[end - UTF16_TERMINATOR_LEN..end]
                .iter()
                .all(|&b| b == 0)
        {
            end -= UTF16_TERMINATOR_LEN;
        }
        let terminated = end != payload.len();
        return (
            payload[..end].to_vec(),
            if terminated {
                UTF16_TERMINATOR_LEN
            } else {
                NO_TERMINATOR
            },
        );
    }
    match payload.last() {
        Some(0) => (
            payload[..payload.len() - UTF8_TERMINATOR_LEN].to_vec(),
            UTF8_TERMINATOR_LEN,
        ),
        _ => (payload.to_vec(), NO_TERMINATOR),
    }
}

impl XmlBox {
    /// Build one from UTF-8 text, writing the terminator this edition requires.
    #[must_use]
    pub fn new(xml: &[u8]) -> Self {
        Self {
            version: 0,
            flags: 0,
            xml: xml.to_vec(),
            terminator: UTF8_TERMINATOR_LEN,
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

    /// The XML text, terminator excluded.
    #[must_use]
    pub fn xml(&self) -> &[u8] {
        &self.xml
    }

    /// The XML text as UTF-8, when it is.
    #[must_use]
    pub fn xml_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.xml).ok()
    }

    /// Whether the box carried a null terminator.
    #[must_use]
    pub const fn is_null_terminated(&self) -> bool {
        self.terminator != NO_TERMINATOR
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "xml ")?;
        let (xml, terminator) = split_utfstring(&body[FULLBOX_EXTRA_SIZE..]);
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            xml,
            terminator,
        })
    }
}

impl<'a> Parse<'a> for XmlBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &XML, "xml ")?;
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "xml ",
        )?)
    }
}

impl Serialize for XmlBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.xml.len() + self.terminator)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &XML, need, self.version, self.flags, false)?;
        buf[cursor..cursor + self.xml.len()].copy_from_slice(&self.xml);
        cursor += self.xml.len();
        buf[cursor..cursor + self.terminator].fill(0);
        cursor += self.terminator;
        Ok(cursor)
    }
}

/// Binary XML Box (`bxml`) — ISO/IEC 14496-12:2026 §8.11.2.
///
/// `data` is the encoded XML, opaque to this layer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct BinaryXmlBox {
    version: u8,
    flags: u32,
    data: Vec<u8>,
}

impl BinaryXmlBox {
    /// Build one from the encoded bytes.
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            version: 0,
            flags: 0,
            data,
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

    /// The encoded XML bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "bxml")?;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            data: body[FULLBOX_EXTRA_SIZE..].to_vec(),
        })
    }
}

impl<'a> Parse<'a> for BinaryXmlBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &BXML, "bxml")?;
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "bxml",
        )?)
    }
}

impl Serialize for BinaryXmlBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.data.len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &BXML, need, self.version, self.flags, false)?;
        buf[cursor..cursor + self.data.len()].copy_from_slice(&self.data);
        cursor += self.data.len();
        Ok(cursor)
    }
}

/// Copyright Box (`cprt`) — ISO/IEC 14496-12:2026 §8.10.2.
///
/// `language` is the wire's packed three-letter ISO 639-2/T code (one `pad` bit
/// then three 5-bit letters, §8.10.2.2); `notice` is a `utfstring` (§4.2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CopyrightBox {
    version: u8,
    flags: u32,
    language: u16,
    notice: Vec<u8>,
    terminator: usize,
}

impl CopyrightBox {
    /// Build one from a packed language code and a UTF-8 notice.
    #[must_use]
    pub fn new(language: u16, notice: &[u8]) -> Self {
        Self {
            version: 0,
            flags: 0,
            language: language & CPRT_LANGUAGE_MASK,
            notice: notice.to_vec(),
            terminator: UTF8_TERMINATOR_LEN,
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

    /// The packed three-letter language code (§8.10.2.2).
    #[must_use]
    pub const fn language(&self) -> u16 {
        self.language
    }

    /// The copyright notice, terminator excluded.
    #[must_use]
    pub fn notice(&self) -> &[u8] {
        &self.notice
    }

    /// The notice as UTF-8, when it is.
    #[must_use]
    pub fn notice_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.notice).ok()
    }

    /// Whether the box carried a null terminator.
    #[must_use]
    pub const fn is_null_terminated(&self) -> bool {
        self.terminator != NO_TERMINATOR
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + U16_LEN, "cprt")?;
        let offset = FULLBOX_EXTRA_SIZE;
        let (notice, terminator) = split_utfstring(&body[offset + U16_LEN..]);
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            language: be_u16(body, offset, "cprt language")? & CPRT_LANGUAGE_MASK,
            notice,
            terminator,
        })
    }
}

impl<'a> Parse<'a> for CopyrightBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &CPRT, "cprt")?;
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + U16_LEN,
            "cprt",
        )?)
    }
}

impl Serialize for CopyrightBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + U16_LEN
                + self.notice.len()
                + self.terminator,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &CPRT, need, self.version, self.flags, false)?;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.language.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + self.notice.len()].copy_from_slice(&self.notice);
        cursor += self.notice.len();
        buf[cursor..cursor + self.terminator].fill(0);
        cursor += self.terminator;
        Ok(cursor)
    }
}

/// Metadata Key Declaration Box (`keyd`) — ISO/IEC 14496-12:2026 §12.8.4.4.
///
/// `key_namespace` names the domain and structure of `key_value`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MetadataKeyDeclarationBox {
    key_namespace: u32,
    key_value: Vec<u8>,
}

impl MetadataKeyDeclarationBox {
    /// Build one from a namespace and its key bytes.
    #[must_use]
    pub fn new(key_namespace: u32, key_value: Vec<u8>) -> Self {
        Self {
            key_namespace,
            key_value,
        }
    }

    /// The 32-bit `key_namespace` (§12.8.4.4.3).
    #[must_use]
    pub const fn key_namespace(&self) -> u32 {
        self.key_namespace
    }

    /// The `key_value` bytes, whose form the namespace defines.
    #[must_use]
    pub fn key_value(&self) -> &[u8] {
        &self.key_value
    }

    /// The key value as UTF-8, when it is.
    #[must_use]
    pub fn key_value_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.key_value).ok()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, U32_LEN, "keyd")?;
        Ok(Self {
            key_namespace: u32_at(body, 0),
            key_value: body[U32_LEN..].to_vec(),
        })
    }
}

impl<'a> Parse<'a> for MetadataKeyDeclarationBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &KEYD, "keyd")?;
        Self::parse_body(crate::box_types::box_body(bytes, U32_LEN, "keyd")?)
    }
}

impl Serialize for MetadataKeyDeclarationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + U32_LEN + self.key_value.len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &KEYD, need)?;
        buf[cursor..cursor + U32_LEN].copy_from_slice(&self.key_namespace.to_be_bytes());
        cursor += U32_LEN;
        buf[cursor..cursor + self.key_value.len()].copy_from_slice(&self.key_value);
        cursor += self.key_value.len();
        Ok(cursor)
    }
}

/// Metadata Locale Box (`loca`) — ISO/IEC 14496-12:2026 §12.8.4.5.
///
/// `locale_string` is an IETF BCP 47 language tag.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MetadataLocaleBox {
    locale_string: Vec<u8>,
}

impl MetadataLocaleBox {
    /// Build one from a language tag.
    #[must_use]
    pub fn new(locale: &str) -> Self {
        Self {
            locale_string: locale.as_bytes().to_vec(),
        }
    }

    /// The raw `locale_string` bytes.
    #[must_use]
    pub fn locale_string(&self) -> &[u8] {
        &self.locale_string
    }

    /// The locale as UTF-8, when it is.
    #[must_use]
    pub fn locale_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.locale_string).ok()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut cursor = 0;
        Ok(Self {
            locale_string: take_c_string(body, &mut cursor, "loca")?,
        })
    }
}

impl<'a> Parse<'a> for MetadataLocaleBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &LOCA, "loca")?;
        Self::parse_body(crate::box_types::box_body(bytes, 0, "loca")?)
    }
}

impl Serialize for MetadataLocaleBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.locale_string.len() + 1)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let cursor = write_box_header(buf, &LOCA, need)?;
        let written = write_c_string(buf, cursor, &self.locale_string)?;
        Ok(cursor + written)
    }
}

/// Metadata Setup Box (`setu`) — ISO/IEC 14496-12:2026 §12.8.4.6.
///
/// Its contents are defined by the key namespace: leaf data, or an array of
/// boxes, kept opaque here.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MetadataSetupBox {
    data: Vec<u8>,
}

impl MetadataSetupBox {
    /// Build one from the setup bytes.
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// The setup bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        Ok(Self {
            data: body.to_vec(),
        })
    }
}

impl<'a> Parse<'a> for MetadataSetupBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &SETU, "setu")?;
        Self::parse_body(crate::box_types::box_body(bytes, 0, "setu")?)
    }
}

impl Serialize for MetadataSetupBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.data.len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &SETU, need)?;
        buf[cursor..cursor + self.data.len()].copy_from_slice(&self.data);
        cursor += self.data.len();
        Ok(cursor)
    }
}

/// One child of a [`MetadataKeyBox`] — ISO/IEC 14496-12:2026 §12.8.4.3.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MetadataKeyChild {
    /// `keyd` — [`MetadataKeyDeclarationBox`] (§12.8.4.4).
    Declaration(MetadataKeyDeclarationBox),
    /// `loca` — [`MetadataLocaleBox`] (§12.8.4.5).
    Locale(MetadataLocaleBox),
    /// `setu` — [`MetadataSetupBox`] (§12.8.4.6).
    Setup(MetadataSetupBox),
    /// Any other box the namespace allows, kept verbatim.
    Other(OpaqueBox),
}

/// Metadata Key Box (`local_key_id`) — ISO/IEC 14496-12:2026 §12.8.4.3.
///
/// The box type is the key's local identifier, so it is carried as a 32-bit
/// value rather than a four-CC. The children are kept in wire order: the
/// clause allows `keyd`, an optional `loca`, an optional `setu` and other
/// boxes of the namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MetadataKeyBox {
    local_key_id: u32,
    children: Vec<MetadataKeyChild>,
}

impl MetadataKeyBox {
    /// A key box with no children.
    #[must_use]
    pub fn new(local_key_id: u32) -> Self {
        Self {
            local_key_id,
            children: Vec::new(),
        }
    }

    /// Append one child.
    #[must_use]
    pub fn with_child(mut self, child: MetadataKeyChild) -> Self {
        self.children.push(child);
        self
    }

    /// The box type: the key's local identifier (§12.8.4.3.1).
    #[must_use]
    pub const fn local_key_id(&self) -> u32 {
        self.local_key_id
    }

    /// The children, in wire order.
    #[must_use]
    pub fn children(&self) -> &[MetadataKeyChild] {
        &self.children
    }

    /// The key declaration (`keyd`), when present.
    #[must_use]
    pub fn declaration(&self) -> Option<&MetadataKeyDeclarationBox> {
        self.children.iter().find_map(|child| match child {
            MetadataKeyChild::Declaration(b) => Some(b),
            _ => None,
        })
    }

    /// The locale (`loca`), when present.
    #[must_use]
    pub fn locale(&self) -> Option<&MetadataLocaleBox> {
        self.children.iter().find_map(|child| match child {
            MetadataKeyChild::Locale(b) => Some(b),
            _ => None,
        })
    }

    /// The setup data (`setu`), when present.
    #[must_use]
    pub fn setup(&self) -> Option<&MetadataSetupBox> {
        self.children.iter().find_map(|child| match child {
            MetadataKeyChild::Setup(b) => Some(b),
            _ => None,
        })
    }

    /// Parse the complete box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        let local_key_id = u32::from_be_bytes(item.four_cc());
        let mut children = Vec::new();
        for child in item.children() {
            let child = child?;
            children.push(match &child.four_cc() {
                b"keyd" => {
                    MetadataKeyChild::Declaration(MetadataKeyDeclarationBox::parse(child.bytes)?)
                }
                b"loca" => MetadataKeyChild::Locale(MetadataLocaleBox::parse(child.bytes)?),
                b"setu" => MetadataKeyChild::Setup(MetadataSetupBox::parse(child.bytes)?),
                _ => MetadataKeyChild::Other(OpaqueBox {
                    box_type: child.four_cc(),
                    data: child.payload().to_vec(),
                    to_end: child.to_end(),
                    largesize: child.has_largesize(),
                }),
            });
        }
        Ok(Self {
            local_key_id,
            children,
        })
    }

    fn body_len(&self) -> usize {
        self.children
            .iter()
            .map(|child| match child {
                MetadataKeyChild::Declaration(b) => b.serialized_len(),
                MetadataKeyChild::Locale(b) => b.serialized_len(),
                MetadataKeyChild::Setup(b) => b.serialized_len(),
                MetadataKeyChild::Other(b) => b.serialized_len(),
            })
            .sum()
    }
}

impl<'a> Parse<'a> for MetadataKeyBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for MetadataKeyBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &self.local_key_id.to_be_bytes(), need)?;
        for child in &self.children {
            cursor += match child {
                MetadataKeyChild::Declaration(b) => b.serialize_into(&mut buf[cursor..])?,
                MetadataKeyChild::Locale(b) => b.serialize_into(&mut buf[cursor..])?,
                MetadataKeyChild::Setup(b) => b.serialize_into(&mut buf[cursor..])?,
                MetadataKeyChild::Other(b) => b.serialize_into(&mut buf[cursor..])?,
            };
        }
        Ok(cursor)
    }
}

/// Metadata Key Table Box (`keys`, §12.8.4.2) — the track form.
///
/// A plain box whose children are [`MetadataKeyBox`]es, one per key
/// configuration that may occur in the samples. This is a different layout
/// from the Apple full-box item-key table [`crate::metadata::Keys`] a
/// file-level `meta` carries, under the same four-CC.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MetadataKeyTableBox {
    keys: Vec<MetadataKeyBox>,
}

impl MetadataKeyTableBox {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one key.
    #[must_use]
    pub fn with_key(mut self, key: MetadataKeyBox) -> Self {
        self.keys.push(key);
        self
    }

    /// The keys, in wire order.
    #[must_use]
    pub fn keys(&self) -> &[MetadataKeyBox] {
        &self.keys
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut keys = Vec::new();
        for child in crate::box_types::BoxIter::new(body) {
            keys.push(MetadataKeyBox::parse_box(child?.bytes)?);
        }
        Ok(Self { keys })
    }

    /// Parse the complete box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        expect_type(bytes, &KEYS, "keys")?;
        Self::parse_body(crate::box_types::box_body(bytes, 0, "keys")?)
    }
}

impl<'a> Parse<'a> for MetadataKeyTableBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for MetadataKeyTableBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + self
                    .keys
                    .iter()
                    .map(Serialize::serialized_len)
                    .sum::<usize>(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &KEYS, need)?;
        for key in &self.keys {
            cursor += key.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// Parse a SampleEntry's fixed part (§8.5.2.2), returning its
/// `data_reference_index` and the bytes that follow the fixed part.
fn parse_sample_entry_base<'a>(bytes: &'a [u8], what: &'static str) -> Result<(u16, &'a [u8])> {
    let body = parse_box(bytes)?.0.body;
    need_in(body, SAMPLE_ENTRY_FIXED_LEN, what)?;
    Ok((
        be_u16(body, DATA_REFERENCE_INDEX_OFFSET, what)?,
        &body[SAMPLE_ENTRY_FIXED_LEN..],
    ))
}

/// Write a SampleEntry header and its fixed part, returning the offset of the
/// entry's own fields.
fn write_sample_entry_base(
    buf: &mut [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    need: usize,
    data_reference_index: u16,
) -> Result<usize> {
    need_out(buf, need)?;
    let mut cursor = write_box_header(buf, fourcc, need)?;
    buf[cursor..cursor + SAMPLE_ENTRY_RESERVED_LEN].fill(0);
    cursor += SAMPLE_ENTRY_RESERVED_LEN;
    buf[cursor..cursor + U16_LEN].copy_from_slice(&data_reference_index.to_be_bytes());
    cursor += U16_LEN;
    Ok(cursor)
}

/// Capture one child box as an [`OpaqueBox`].
fn opaque_child(child: crate::box_types::BoxItem<'_>) -> OpaqueBox {
    OpaqueBox {
        box_type: child.four_cc(),
        data: child.payload().to_vec(),
        to_end: child.to_end(),
        largesize: child.has_largesize(),
    }
}

fn opaque_boxes_len(boxes: &[OpaqueBox]) -> usize {
    boxes.iter().map(|b| b.serialized_len()).sum()
}

/// XML Metadata Sample Entry (`metx`) — ISO/IEC 14496-12:2026 §12.3.3.
///
/// Carries the content encoding, the XML namespaces and the schema locations
/// of the samples. `content_encoding` is present but may be empty; the
/// optional fields are reported as `Option` because the clause marks them
/// optional.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct XmlMetaDataSampleEntry {
    data_reference_index: u16,
    content_encoding: Vec<u8>,
    namespace: Vec<u8>,
    schema_location: Option<Vec<u8>>,
    trailing: Vec<u8>,
}

impl XmlMetaDataSampleEntry {
    /// Build one from a content encoding, a namespace list and no schema.
    #[must_use]
    pub fn new(content_encoding: &str, namespace: &str) -> Self {
        Self {
            data_reference_index: 1,
            content_encoding: content_encoding.as_bytes().to_vec(),
            namespace: namespace.as_bytes().to_vec(),
            schema_location: None,
            trailing: Vec::new(),
        }
    }

    /// Set the schema locations.
    #[must_use]
    pub fn with_schema_location(mut self, schema_location: &str) -> Self {
        self.schema_location = Some(schema_location.as_bytes().to_vec());
        self
    }

    /// Set the data reference index.
    #[must_use]
    pub const fn with_data_reference_index(mut self, index: u16) -> Self {
        self.data_reference_index = index;
        self
    }

    /// The entry's `data_reference_index` (§8.5.2.2).
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// The raw `content_encoding` bytes.
    #[must_use]
    pub fn content_encoding(&self) -> &[u8] {
        &self.content_encoding
    }

    /// The raw `namespace` bytes (a `utf8list`, §4.2.1).
    #[must_use]
    pub fn namespace(&self) -> &[u8] {
        &self.namespace
    }

    /// The raw `schema_location` bytes, when present.
    #[must_use]
    pub fn schema_location(&self) -> Option<&[u8]> {
        self.schema_location.as_deref()
    }

    /// Bytes after the last field, kept verbatim.
    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }

    /// Parse the complete box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let (data_reference_index, rest) = parse_sample_entry_base(bytes, "metx")?;
        let mut cursor = 0;
        let content_encoding = take_c_string(rest, &mut cursor, "metx content_encoding")?;
        let namespace = take_c_string(rest, &mut cursor, "metx namespace")?;
        let schema_location = if cursor < rest.len() {
            Some(take_c_string(rest, &mut cursor, "metx schema_location")?)
        } else {
            None
        };
        Ok(Self {
            data_reference_index,
            content_encoding,
            namespace,
            schema_location,
            trailing: rest.get(cursor..).unwrap_or_default().to_vec(),
        })
    }

    fn body_len(&self) -> usize {
        self.content_encoding.len()
            + self.namespace.len()
            + 1
            + 1
            + self
                .schema_location
                .as_ref()
                .map_or(0, |value| value.len() + 1)
            + self.trailing.len()
    }
}

impl<'a> Parse<'a> for XmlMetaDataSampleEntry {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &METX, "metx")?;
        Self::parse_box(bytes)
    }
}

impl Serialize for XmlMetaDataSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + SAMPLE_ENTRY_FIXED_LEN + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        let mut cursor = write_sample_entry_base(buf, &METX, need, self.data_reference_index)?;
        cursor += write_c_string(buf, cursor, &self.content_encoding)?;
        cursor += write_c_string(buf, cursor, &self.namespace)?;
        if let Some(schema_location) = &self.schema_location {
            cursor += write_c_string(buf, cursor, schema_location)?;
        }
        buf[cursor..cursor + self.trailing.len()].copy_from_slice(&self.trailing);
        cursor += self.trailing.len();
        Ok(cursor)
    }
}

/// Text Metadata Sample Entry (`mett`) and Simple Text Sample Entry (`stxt`) —
/// ISO/IEC 14496-12:2026 §12.3.3, §12.5.3.
///
/// One type serves both: the clause gives them the same fields, and the
/// four-CC is carried so the entry writes back as the one it parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TextMetaDataSampleEntry {
    codec_type: [u8; BOX_TYPE_SIZE],
    data_reference_index: u16,
    content_encoding: Vec<u8>,
    mime_format: Vec<u8>,
    text_config: Option<TextConfigBox>,
    other_boxes: Vec<OpaqueBox>,
}

impl TextMetaDataSampleEntry {
    /// Build a `mett` entry from a content encoding and a MIME format.
    #[must_use]
    pub fn new(content_encoding: &str, mime_format: &str) -> Self {
        Self {
            codec_type: METT,
            data_reference_index: 1,
            content_encoding: content_encoding.as_bytes().to_vec(),
            mime_format: mime_format.as_bytes().to_vec(),
            text_config: None,
            other_boxes: Vec::new(),
        }
    }

    /// Set the optional `txtC` child.
    #[must_use]
    pub fn with_text_config(mut self, text: &str) -> Self {
        self.text_config = Some(TextConfigBox::new(text));
        self
    }

    /// Set the data reference index.
    #[must_use]
    pub const fn with_data_reference_index(mut self, index: u16) -> Self {
        self.data_reference_index = index;
        self
    }

    /// The entry's four-CC (`mett` or `stxt`).
    #[must_use]
    pub const fn codec_type(&self) -> [u8; BOX_TYPE_SIZE] {
        self.codec_type
    }

    /// The entry's `data_reference_index` (§8.5.2.2).
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// The raw `content_encoding` bytes.
    #[must_use]
    pub fn content_encoding(&self) -> &[u8] {
        &self.content_encoding
    }

    /// The raw `mime_format` bytes.
    #[must_use]
    pub fn mime_format(&self) -> &[u8] {
        &self.mime_format
    }

    /// The optional `txtC` child.
    #[must_use]
    pub fn text_config(&self) -> Option<&TextConfigBox> {
        self.text_config.as_ref()
    }

    /// Child boxes the model does not type, in wire order.
    #[must_use]
    pub fn other_boxes(&self) -> &[OpaqueBox] {
        &self.other_boxes
    }

    /// Parse the complete box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        let codec_type = item.four_cc();
        if codec_type != METT && codec_type != STXT {
            return Err(Error::InvalidInput("expected mett or stxt"));
        }
        let (data_reference_index, rest) = parse_sample_entry_base(bytes, "mett")?;
        let mut cursor = 0;
        let content_encoding = take_c_string(rest, &mut cursor, "mett content_encoding")?;
        let mime_format = take_c_string(rest, &mut cursor, "mett mime_format")?;
        let mut text_config = None;
        let mut other_boxes = Vec::new();
        for child in crate::box_types::BoxIter::new(&rest[cursor..]) {
            let child = child?;
            if child.four_cc() == TXTC {
                text_config = Some(TextConfigBox::parse(child.bytes)?);
            } else {
                other_boxes.push(opaque_child(child));
            }
        }
        Ok(Self {
            codec_type,
            data_reference_index,
            content_encoding,
            mime_format,
            text_config,
            other_boxes,
        })
    }
}

impl<'a> Parse<'a> for TextMetaDataSampleEntry {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for TextMetaDataSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + SAMPLE_ENTRY_FIXED_LEN
                + self.content_encoding.len()
                + 1
                + self.mime_format.len()
                + 1
                + self.text_config.as_ref().map_or(0, |b| b.serialized_len())
                + opaque_boxes_len(&self.other_boxes),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        let mut cursor =
            write_sample_entry_base(buf, &self.codec_type, need, self.data_reference_index)?;
        cursor += write_c_string(buf, cursor, &self.content_encoding)?;
        cursor += write_c_string(buf, cursor, &self.mime_format)?;
        if let Some(text_config) = &self.text_config {
            cursor += text_config.serialize_into(&mut buf[cursor..])?;
        }
        for child in &self.other_boxes {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// URI Metadata Sample Entry (`urim`) — ISO/IEC 14496-12:2026 §12.3.3.
///
/// The mandatory [`UriBox`] names the form of the samples; the optional
/// [`UriInitBox`] carries its initialization data.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct UriMetaDataSampleEntry {
    data_reference_index: u16,
    uri: UriBox,
    uri_init: Option<UriInitBox>,
    other_boxes: Vec<OpaqueBox>,
}

impl UriMetaDataSampleEntry {
    /// Build one from the metadata form's URI.
    #[must_use]
    pub fn new(uri: &str) -> Self {
        Self {
            data_reference_index: 1,
            uri: UriBox::new(uri),
            uri_init: None,
            other_boxes: Vec::new(),
        }
    }

    /// Set the optional initialization data.
    #[must_use]
    pub fn with_init_data(mut self, data: Vec<u8>) -> Self {
        self.uri_init = Some(UriInitBox::new(data));
        self
    }

    /// Set the data reference index.
    #[must_use]
    pub const fn with_data_reference_index(mut self, index: u16) -> Self {
        self.data_reference_index = index;
        self
    }

    /// The entry's `data_reference_index` (§8.5.2.2).
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// The metadata form's URI (`uri `).
    #[must_use]
    pub const fn uri(&self) -> &UriBox {
        &self.uri
    }

    /// The optional initialization data (`uriI`).
    #[must_use]
    pub fn uri_init(&self) -> Option<&UriInitBox> {
        self.uri_init.as_ref()
    }

    /// Child boxes the model does not type, in wire order.
    #[must_use]
    pub fn other_boxes(&self) -> &[OpaqueBox] {
        &self.other_boxes
    }

    /// Parse the complete box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let (data_reference_index, rest) = parse_sample_entry_base(bytes, "urim")?;
        let mut uri = None;
        let mut uri_init = None;
        let mut other_boxes = Vec::new();
        for child in crate::box_types::BoxIter::new(rest) {
            let child = child?;
            match &child.four_cc() {
                b"uri " => uri = Some(UriBox::parse(child.bytes)?),
                b"uriI" => uri_init = Some(UriInitBox::parse(child.bytes)?),
                _ => other_boxes.push(opaque_child(child)),
            }
        }
        let uri = uri.ok_or(Error::InvalidInput("urim without a uri box"))?;
        Ok(Self {
            data_reference_index,
            uri,
            uri_init,
            other_boxes,
        })
    }
}

impl<'a> Parse<'a> for UriMetaDataSampleEntry {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &URIM, "urim")?;
        Self::parse_box(bytes)
    }
}

impl Serialize for UriMetaDataSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + SAMPLE_ENTRY_FIXED_LEN
                + self.uri.serialized_len()
                + self.uri_init.as_ref().map_or(0, |b| b.serialized_len())
                + opaque_boxes_len(&self.other_boxes),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        let mut cursor = write_sample_entry_base(buf, &URIM, need, self.data_reference_index)?;
        cursor += self.uri.serialize_into(&mut buf[cursor..])?;
        if let Some(uri_init) = &self.uri_init {
            cursor += uri_init.serialize_into(&mut buf[cursor..])?;
        }
        for child in &self.other_boxes {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}

/// Boxed Metadata Sample Entry (`mebx`) — ISO/IEC 14496-12:2026 §12.8.4.1.
///
/// The mandatory [`MetadataKeyTableBox`] declares the keys the samples may
/// carry; the optional `btrt` bitrate box is kept among the other children.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct BoxedMetaDataSampleEntry {
    data_reference_index: u16,
    key_table: MetadataKeyTableBox,
    other_boxes: Vec<OpaqueBox>,
}

impl BoxedMetaDataSampleEntry {
    /// Build one from its key table.
    #[must_use]
    pub fn new(key_table: MetadataKeyTableBox) -> Self {
        Self {
            data_reference_index: 1,
            key_table,
            other_boxes: Vec::new(),
        }
    }

    /// Set the data reference index.
    #[must_use]
    pub const fn with_data_reference_index(mut self, index: u16) -> Self {
        self.data_reference_index = index;
        self
    }

    /// The entry's `data_reference_index` (§8.5.2.2).
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.data_reference_index
    }

    /// The key table (`keys`).
    #[must_use]
    pub const fn key_table(&self) -> &MetadataKeyTableBox {
        &self.key_table
    }

    /// Child boxes the model does not type, in wire order.
    #[must_use]
    pub fn other_boxes(&self) -> &[OpaqueBox] {
        &self.other_boxes
    }

    /// Parse the complete box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let (data_reference_index, rest) = parse_sample_entry_base(bytes, "mebx")?;
        let mut key_table = None;
        let mut other_boxes = Vec::new();
        for child in crate::box_types::BoxIter::new(rest) {
            let child = child?;
            if child.four_cc() == KEYS {
                key_table = Some(MetadataKeyTableBox::parse_box(child.bytes)?);
            } else {
                other_boxes.push(opaque_child(child));
            }
        }
        let key_table = key_table.ok_or(Error::InvalidInput("mebx without a keys box"))?;
        Ok(Self {
            data_reference_index,
            key_table,
            other_boxes,
        })
    }
}

impl<'a> Parse<'a> for BoxedMetaDataSampleEntry {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        expect_type(bytes, &MEBX, "mebx")?;
        Self::parse_box(bytes)
    }
}

impl Serialize for BoxedMetaDataSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + SAMPLE_ENTRY_FIXED_LEN
                + self.key_table.serialized_len()
                + opaque_boxes_len(&self.other_boxes),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        let mut cursor = write_sample_entry_base(buf, &MEBX, need, self.data_reference_index)?;
        cursor += self.key_table.serialize_into(&mut buf[cursor..])?;
        for child in &self.other_boxes {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}
