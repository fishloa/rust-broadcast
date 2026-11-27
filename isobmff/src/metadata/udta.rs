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

use alloc::boxed::Box;
use alloc::vec::Vec;

use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

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
    /// Whether a plain (non-`©`) atom's text carried its NUL terminator.
    ///
    /// The international form's translations each carry their own length, so
    /// this is unread there; a plain atom is a single NUL-terminated string
    /// (the module doc), and a file that omitted the NUL must not gain one on
    /// a rewrite.
    pub plain_nul_terminated: bool,
}

impl UdtaText {
    /// The first translation's text as UTF-8, when it is.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        core::str::from_utf8(&self.translations.first()?.value).ok()
    }

    fn body_len(&self) -> usize {
        if self.key[0] == super::COPYRIGHT_SIGN {
            self.translations
                .iter()
                .map(|translation| TRANSLATION_PREFIX_LEN + translation.value.len())
                .sum()
        } else {
            self.translations
                .first()
                .map_or(0, |translation| translation.value.len())
                + usize::from(self.plain_nul_terminated)
        }
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
        if self.key[0] == super::COPYRIGHT_SIGN {
            for translation in &self.translations {
                let len =
                    broadcast_core::len::fit_u16(translation.value.len(), "udta text length")?;
                buf[cursor..cursor + U16_LEN].copy_from_slice(&len.to_be_bytes());
                cursor += U16_LEN;
                buf[cursor..cursor + U16_LEN].copy_from_slice(&translation.language.to_be_bytes());
                cursor += U16_LEN;
                buf[cursor..cursor + translation.value.len()].copy_from_slice(&translation.value);
                cursor += translation.value.len();
            }
        } else if let Some(translation) = self.translations.first() {
            buf[cursor..cursor + translation.value.len()].copy_from_slice(&translation.value);
            cursor += translation.value.len();
            if self.plain_nul_terminated {
                buf[cursor] = 0;
                cursor += 1;
            }
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
    /// A track kind (§8.10.4).
    Kind(crate::track_desc::KindBox),
    /// A label (§8.10.5).
    Label(crate::audio_boxes::LabelBox),
    /// Audio stream loudness (§12.2.7).
    Loudness(crate::audio_boxes::LoudnessBox),
    /// A track selection (§8.10.3).
    TrackSelection(crate::track_desc::TrackSelectionBox),
    /// A copyright declaration (§8.10.2).
    Copyright(crate::meta_track::CopyrightBox),
    /// A sub-track's information and definition (§8.13.3).
    SubTrack(crate::track_desc::SubTrackBox),
    /// Hint information (`hnti`, §9.1.4).
    HintInformation(Box<crate::hint::HintInformationBox>),
    /// Hint statistics (`hinf`, §9.1.5).
    HintStatistics(Box<crate::hint::HintStatisticsBox>),
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
            Self::Kind(kind) => kind.serialized_len(),
            Self::Label(label) => label.serialized_len(),
            Self::Loudness(loudness) => loudness.serialized_len(),
            Self::TrackSelection(selection) => selection.serialized_len(),
            Self::Copyright(copyright) => copyright.serialized_len(),
            Self::SubTrack(subtrack) => subtrack.serialized_len(),
            Self::HintInformation(hint) => hint.serialized_len(),
            Self::HintStatistics(statistics) => statistics.serialized_len(),
            Self::Opaque(opaque) => opaque.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Text(text) => text.serialize_into(buf),
            Self::Chapters(chpl) => chpl.serialize_into(buf),
            Self::Location(loci) => loci.serialize_into(buf),
            Self::Meta(meta) => meta.serialize_into(buf),
            Self::Kind(kind) => kind.serialize_into(buf),
            Self::Label(label) => label.serialize_into(buf),
            Self::Loudness(loudness) => loudness.serialize_into(buf),
            Self::TrackSelection(selection) => selection.serialize_into(buf),
            Self::Copyright(copyright) => copyright.serialize_into(buf),
            Self::SubTrack(subtrack) => subtrack.serialize_into(buf),
            Self::HintInformation(hint) => hint.serialize_into(buf),
            Self::HintStatistics(statistics) => statistics.serialize_into(buf),
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
        Self::parse_body_notes(body, limits, None)
    }

    /// [`Udta::parse_body`], keeping a child whose typed parse fails as an
    /// opaque child of its own kind and reporting it through `notes`, so one
    /// unrepresentable atom costs that atom rather than the whole metadata
    /// tree. With no sink the parse is the strict one.
    pub(crate) fn parse_body_notes(
        body: &[u8],
        limits: &Limits,
        mut notes: Option<&mut crate::box_support::Notes<'_>>,
    ) -> Result<Self> {
        let mut children = Vec::new();
        for child in BoxIter::new(body) {
            let child = child?;
            let fourcc = child.four_cc();
            if child.to_end() || child.has_largesize() {
                let mut opaque = OpaqueBox::new(fourcc, child.payload().to_vec());
                opaque.to_end = child.to_end();
                opaque.largesize = child.has_largesize();
                children.push(UdtaChild::Opaque(opaque));
                continue;
            }
            let typed = match fourcc {
                CHPL => {
                    crate::box_support::keep_or_note(child.bytes, notes.as_deref_mut(), |_| {
                        Chpl::parse_body(child.body).map(UdtaChild::Chapters)
                    })?
                }
                LOCI => {
                    crate::box_support::keep_or_note(child.bytes, notes.as_deref_mut(), |_| {
                        Loci::parse_body(child.body).map(UdtaChild::Location)
                    })?
                }
                META => {
                    crate::box_support::keep_or_note(child.bytes, notes.as_deref_mut(), |_| {
                        Meta::parse_body(child.body, limits).map(UdtaChild::Meta)
                    })?
                }
                crate::track_desc::KIND => {
                    crate::box_support::keep_or_note(child.bytes, notes.as_deref_mut(), |_| {
                        crate::track_desc::KindBox::parse_body(child.body).map(UdtaChild::Kind)
                    })?
                }
                crate::audio_boxes::LABL => {
                    crate::box_support::keep_or_note(child.bytes, notes.as_deref_mut(), |_| {
                        crate::audio_boxes::LabelBox::parse_body(child.body).map(UdtaChild::Label)
                    })?
                }
                crate::audio_boxes::LUDT => {
                    crate::box_support::keep_or_note(child.bytes, notes.as_deref_mut(), |_| {
                        crate::audio_boxes::LoudnessBox::parse_body(child.body)
                            .map(UdtaChild::Loudness)
                    })?
                }
                crate::track_desc::STRK => {
                    crate::box_support::keep_or_note(child.bytes, notes.as_deref_mut(), |_| {
                        crate::track_desc::SubTrackBox::parse_body(child.body)
                            .map(UdtaChild::SubTrack)
                    })?
                }
                crate::meta_track::CPRT => {
                    crate::box_support::keep_or_note(child.bytes, notes.as_deref_mut(), |_| {
                        crate::meta_track::CopyrightBox::parse_body(child.body)
                            .map(UdtaChild::Copyright)
                    })?
                }
                crate::track_desc::TSEL => {
                    crate::box_support::keep_or_note(child.bytes, notes.as_deref_mut(), |_| {
                        crate::track_desc::TrackSelectionBox::parse_body(child.body)
                            .map(UdtaChild::TrackSelection)
                    })?
                }
                crate::hint::HNTI => {
                    crate::box_support::keep_or_note(child.bytes, notes.as_deref_mut(), |_| {
                        crate::hint::HintInformationBox::parse_body(child.body)
                            .map(|boxed| UdtaChild::HintInformation(Box::new(boxed)))
                    })?
                }
                crate::hint::HINF => {
                    crate::box_support::keep_or_note(child.bytes, notes.as_deref_mut(), |_| {
                        crate::hint::HintStatisticsBox::parse_body(child.body)
                            .map(|boxed| UdtaChild::HintStatistics(Box::new(boxed)))
                    })?
                }
                _ if super::is_text_key(&fourcc) => {
                    crate::box_support::keep_or_note(child.bytes, notes.as_deref_mut(), |_| {
                        parse_text(fourcc, child.body).map(UdtaChild::Text)
                    })?
                }
                _ => None,
            };
            // A typed child that would not write its own bytes back — a
            // dropped tail, a re-derived count — is kept opaque instead, so a
            // rewrite stays exact.
            let typed = match typed {
                Some(value) if reproduces(&value, &child) => Some(value),
                Some(_) => {
                    if let Some(notes) = notes.as_deref_mut() {
                        notes.not_reproducible(child.bytes);
                    }
                    None
                }
                None => None,
            };
            let parsed = typed.unwrap_or_else(|| {
                let mut opaque = OpaqueBox::new(fourcc, child.payload().to_vec());
                opaque.to_end = child.to_end();
                opaque.largesize = child.has_largesize();
                UdtaChild::Opaque(opaque)
            });
            children.push(parsed);
        }
        Ok(Self { children })
    }
}

/// Whether a typed `udta` child writes back the type and body it parsed. The
/// header form (`size == 0`, `size == 1` + `largesize`) is carried by the
/// opaque child instead, so only the four-CC and the bytes after the header
/// are compared.
fn reproduces(parsed: &UdtaChild, child: &crate::box_types::BoxItem<'_>) -> bool {
    if matches!(parsed, UdtaChild::Opaque(_)) {
        return true;
    }
    let Ok(bytes) = parsed.try_to_bytes() else {
        return false;
    };
    let Some(body) = bytes.get(BOX_HEADER_MIN_SIZE..) else {
        return false;
    };
    if bytes.get(crate::box_types::BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE)
        != child
            .bytes
            .get(crate::box_types::BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE)
    {
        return false;
    }
    child.bytes.get(child.header.header_size()..) == Some(body)
}

/// Parse a text atom body: QuickTime international text for a `©`-prefixed
/// type, a null-terminated string for a plain one.
fn parse_text(key: [u8; BOX_TYPE_SIZE], body: &[u8]) -> Result<UdtaText> {
    if key[0] == super::COPYRIGHT_SIGN {
        parse_international_text(key, body)
    } else {
        let terminator = body.iter().position(|byte| *byte == 0);
        let text = terminator.map_or(body, |end| &body[..end]);
        Ok(UdtaText {
            key,
            translations: alloc::vec![UdtaTextTranslation {
                language: 0,
                value: text.to_vec(),
            }],
            plain_nul_terminated: terminator.is_some(),
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
    Ok(UdtaText {
        key,
        translations,
        plain_nul_terminated: false,
    })
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
