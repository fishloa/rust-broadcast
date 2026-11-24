//! The user data box (`udta`).
//!
//! ISO/IEC 14496-12:2026 §8.10.1 defines `udta` as a plain container; its
//! children are QuickTime user-data atoms, 3GPP boxes (`loci`, TS 26.244
//! Table 8.10) and, in iTunes files, a `meta` with an `ilst`.
//!
//! A QuickTime international-text atom (`©nam`, `©xyz`, …) is a sequence of
//! small atoms: a 16-bit text length, a 16-bit language code and that many
//! text bytes. The length is the text length alone in the fixtures
//! (`private/fixtures/mp4/quicktime/lavf-aac-tmcd.mov`,
//! `private/fixtures/mp4/camera/gopro-max-heromode.mp4`), which is what this
//! parser uses. A plain user-data text atom (`titl`, `cprt`, …) is a
//! null-terminated string instead.
//!
//! Unknown children (GoPro `FIRM`, `GPMF`, `free`, …) are an [`OpaqueBox`] at
//! the position they occupied, so a camera file round-trips byte for byte.

use alloc::vec::Vec;

use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

use crate::box_support::OpaqueBox;
use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, BoxIter, box_len, write_box_header};
use crate::error::{Error, Result};
use crate::limits::Limits;

use super::U16_LEN;
use super::chapters::{CHPL, Chpl};
use super::location::{LOCI, Loci};
use super::meta::{META, Meta};

/// The `udta` box type.
pub const UDTA: [u8; 4] = *b"udta";

/// The length and language prefix of a QuickTime international-text atom.
const TRANSLATION_PREFIX_LEN: usize = U16_LEN + U16_LEN;

/// One language-tagged string of a QuickTime international-text atom.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct UdtaTextTranslation {
    /// The 16-bit language code exactly as carried.
    pub language: u16,
    /// The untouched text bytes.
    pub value: Vec<u8>,
}

/// A QuickTime/3GPP user-data text atom.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct UdtaText {
    /// The atom type (`©nam`, `©xyz`, `titl`, …).
    pub key: [u8; BOX_TYPE_SIZE],
    /// The language-tagged strings, in wire order.
    pub translations: Vec<UdtaTextTranslation>,
}

impl UdtaText {
    /// The first translation's text as UTF-8, when it is.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        core::str::from_utf8(&self.translations.first()?.value).ok()
    }

    fn body_len(&self) -> usize {
        self.translations
            .iter()
            .map(|translation| TRANSLATION_PREFIX_LEN + translation.value.len())
            .sum()
    }
}

impl Serialize for UdtaText {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &self.key, need)?;
        for translation in &self.translations {
            let len = broadcast_common::len::fit_u16(translation.value.len(), "udta text length")?;
            buf[cursor..cursor + U16_LEN].copy_from_slice(&len.to_be_bytes());
            cursor += U16_LEN;
            buf[cursor..cursor + U16_LEN].copy_from_slice(&translation.language.to_be_bytes());
            cursor += U16_LEN;
            buf[cursor..cursor + translation.value.len()].copy_from_slice(&translation.value);
            cursor += translation.value.len();
        }
        Ok(cursor)
    }
}

/// A child of a `udta` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum UdtaChild {
    /// A text atom.
    Text(UdtaText),
    /// A Nero chapter list.
    Chapters(Chpl),
    /// A 3GPP location box.
    Location(Loci),
    /// An iTunes `meta` with an `ilst`.
    Meta(Meta),
    /// Any other child, preserved verbatim.
    Opaque(OpaqueBox),
}

impl Serialize for UdtaChild {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::Text(text) => text.serialized_len(),
            Self::Chapters(chpl) => chpl.serialized_len(),
            Self::Location(loci) => loci.serialized_len(),
            Self::Meta(meta) => meta.serialized_len(),
            Self::Opaque(opaque) => opaque.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Text(text) => text.serialize_into(buf),
            Self::Chapters(chpl) => chpl.serialize_into(buf),
            Self::Location(loci) => loci.serialize_into(buf),
            Self::Meta(meta) => meta.serialize_into(buf),
            Self::Opaque(opaque) => opaque.serialize_into(buf),
        }
    }
}

/// The user data box (`udta`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Udta {
    /// The children, in wire order.
    pub children: Vec<UdtaChild>,
}

impl Udta {
    /// An empty `udta`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a text atom.
    pub fn push_text(&mut self, text: UdtaText) {
        self.children.push(UdtaChild::Text(text));
    }

    fn body_len(&self) -> usize {
        self.children.iter().map(Serialize::serialized_len).sum()
    }
}

impl<'a> Parse<'a> for Udta {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, &Limits::default())
    }
}

impl Udta {
    /// Parse a `udta` box with the caller's hostile-input caps.
    pub fn parse_with_limits(bytes: &[u8], limits: &Limits) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        if !item.header.box_type.is(&UDTA) {
            return Err(Error::UnexpectedBox { expected: "udta" });
        }
        Self::parse_body(item.body, limits)
    }

    /// Parse a `udta` from its body.
    pub fn parse_body(body: &[u8], limits: &Limits) -> Result<Self> {
        let mut children = Vec::new();
        for child in BoxIter::new(body) {
            let child = child?;
            let fourcc = child.four_cc();
            let parsed = match fourcc {
                CHPL => UdtaChild::Chapters(Chpl::parse_body(child.body)?),
                LOCI => UdtaChild::Location(Loci::parse_body(child.body)?),
                META => UdtaChild::Meta(Meta::parse_body(child.body, limits)?),
                _ if super::is_text_key(&fourcc) => {
                    UdtaChild::Text(parse_text(fourcc, child.body)?)
                }
                _ => UdtaChild::Opaque(OpaqueBox::new(fourcc, child.payload().to_vec())),
            };
            children.push(parsed);
        }
        Ok(Self { children })
    }
}

/// Parse a text atom body: QuickTime international text for a `©`-prefixed
/// type, a null-terminated string for a plain one.
fn parse_text(key: [u8; BOX_TYPE_SIZE], body: &[u8]) -> Result<UdtaText> {
    if key[0] == super::COPYRIGHT_SIGN {
        parse_international_text(key, body)
    } else {
        let text = body
            .iter()
            .position(|byte| *byte == 0)
            .map_or(body, |end| &body[..end]);
        Ok(UdtaText {
            key,
            translations: alloc::vec![UdtaTextTranslation {
                language: 0,
                value: text.to_vec(),
            }],
        })
    }
}

fn parse_international_text(key: [u8; BOX_TYPE_SIZE], body: &[u8]) -> Result<UdtaText> {
    let mut translations = Vec::new();
    let mut cursor = 0usize;
    while cursor < body.len() {
        need_in(body, cursor + TRANSLATION_PREFIX_LEN, "udta text prefix")?;
        let len = usize::from(super::u16_at(body, cursor));
        let language = super::u16_at(body, cursor + U16_LEN);
        let start = cursor + TRANSLATION_PREFIX_LEN;
        let end = start
            .checked_add(len)
            .ok_or(Error::InvalidInput("udta text length overflows"))?;
        need_in(body, end, "udta text")?;
        translations.push(UdtaTextTranslation {
            language,
            value: body[start..end].to_vec(),
        });
        cursor = end;
    }
    if translations.is_empty() {
        return Err(Error::InvalidInput("udta text atom carries no string"));
    }
    Ok(UdtaText { key, translations })
}

impl Serialize for Udta {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &UDTA, need)?;
        for child in &self.children {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        Ok(cursor)
    }
}
