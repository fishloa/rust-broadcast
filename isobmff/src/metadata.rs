//! Typed MP4 metadata: the `udta`, `meta`, `keys`, `ilst` and `data` boxes,
//! QuickTime user-data text, 3GPP `loci`, Apple's keyed metadata and Nero
//! chapter lists.
//!
//! The wire types are `Udta`, `Meta`, `Keys`, `Ilst`, `Data`, `Mean`, `Name`,
//! `Chpl` and `Loci`; each implements the symmetric
//! [`Parse`](broadcast_core::Parse)/[`Serialize`] pair, so an unknown atom (a GoPro `FIRM`, a `free`, an item with a data type
//! this crate does not name) round-trips byte for byte as an [`OpaqueBox`].
//! [`Metadata`] is the logical view: one item list gathered from a `udta`
//! and/or a `meta`, with `©xyz` as an [`Iso6709`] and a Nero chapter list as
//! [`Chapter`] values.
//!
//! Sources:
//!
//! - `private/specs-verified/apple/isobmff/quicktime-file-format-online.md`
//!   (`User data atom 'udta'`, `User data text strings and language codes`,
//!   `Metadata item keys atom ('keys')`, `Metadata item list atom ('ilst')`,
//!   `Metadata item atom`, `Value atom`, `Well-known types`, `Chapter lists`).
//! - `private/specs-verified/3gpp/isobmff/ts-126-244-v19.md` (Table 8.10,
//!   `Location Information box`).
//! - `private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md` (§8.10.1
//!   `udta`, §8.11 `meta`).
//!
//! The iTunes four-character `ilst` atoms (`©nam`, `©ART`, `trkn`, `disk`,
//! `covr`, `----` with `mean`/`name`/`data`) and Nero's `chpl` are a de-facto
//! convention rather than an Apple/ISO definition; the atoms typed here are
//! exactly the ones observed in the fixtures named in
//! `private/fixtures/mp4/PROVENANCE.md` (written by AudioToolbox, Lavf and
//! GPAC), and every other atom stays an opaque item.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use broadcast_core::Serialize;

use crate::box_support::OpaqueBox;
use crate::box_types::BOX_TYPE_SIZE;
use crate::error::{Error, Result};
use crate::track_id::TrackId;

pub mod chapters;
pub mod ilst;
pub mod itunsmpb;
pub mod keys;
pub mod location;
pub mod meta;
pub mod udta;

pub use chapters::{Chapter, Chpl};
pub use ilst::{Data, Ilst, IlstItem, IlstItemChild, Mean, Name};
pub use itunsmpb::{ITUNSMPB_MEAN, ITUNSMPB_MIN_FIELDS, ITUNSMPB_NAME, Itunsmpb};
pub use keys::{KeyEntry, Keys};
pub use location::{Iso6709, Loci};
pub use meta::{Hdlr, MDIR_HANDLER, MDTA_HANDLER, Meta, MetaChild, MetaFlavor};
pub use udta::{Udta, UdtaChild, UdtaText, UdtaTextTranslation};

/// Width of a 16-bit wire field.
pub(crate) const U16_LEN: usize = 2;
/// Width of a 32-bit wire field.
pub(crate) const U32_LEN: usize = 4;
/// Width of a 64-bit wire field.
pub(crate) const U64_LEN: usize = 8;
/// Width of a 16.16 fixed-point field.
pub(crate) const FIXED_16_16_LEN: usize = 4;
/// The `©` byte that starts a QuickTime international-text atom type.
pub(crate) const COPYRIGHT_SIGN: u8 = 0xA9;
/// Start times in Nero chapters are counted in 100 ns units.
pub(crate) const CHAPTER_TIME_UNITS_PER_SECOND: u64 = 10_000_000;
/// The low half of an `ilst` locale indicator holds the language code.
const LOCALE_LANGUAGE_BITS: u32 = 16;
/// The mask of the language code in an `ilst` locale indicator.
const LOCALE_LANGUAGE_MASK: u32 = 0xffff;
/// The packed `©xyz` atom type.
pub const XYZ_KEY: [u8; BOX_TYPE_SIZE] = [COPYRIGHT_SIGN, b'x', b'y', b'z'];

/// The key an [`MetadataItem`] is stored under.
///
/// [`Display`](core::fmt::Display) and [`Debug`](core::fmt::Debug) print a
/// four-character key as its text (`©nam`, `trkn`; a byte that is not printable
/// as `\xNN`) and a keyed item as its reverse-DNS string, so a tag never has to
/// be spelled as the bytes of `©nam`.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ItemKey {
    /// A four-character iTunes/user-data atom type (`©nam`, `trkn`, `covr`).
    FourCc([u8; BOX_TYPE_SIZE]),
    /// A reverse-DNS key from a `keys` table (`com.apple.quicktime.make`).
    Mdta(String),
}

impl ItemKey {
    /// `©nam`, the title (`User data list entry types`).
    pub const TITLE: Self = Self::FourCc([COPYRIGHT_SIGN, b'n', b'a', b'm']);
    /// `©ART`, the artist: the de-facto iTunes `ilst` atom carried by
    /// `private/fixtures/mp4/tags/lavf-tagged.m4a`.
    pub const ARTIST: Self = Self::FourCc([COPYRIGHT_SIGN, b'A', b'R', b'T']);
    /// `©alb`, the album: the de-facto iTunes `ilst` atom carried by
    /// `private/fixtures/mp4/tags/lavf-tagged.m4a`.
    pub const ALBUM: Self = Self::FourCc([COPYRIGHT_SIGN, b'a', b'l', b'b']);
    /// `©day`, the date the content was created (`User data list entry
    /// types`).
    pub const DATE: Self = Self::FourCc([COPYRIGHT_SIGN, b'd', b'a', b'y']);
    /// `©gen`, the genre: the de-facto iTunes `ilst` atom carried by
    /// `private/fixtures/mp4/tags/lavf-tagged.m4a`.
    pub const GENRE: Self = Self::FourCc([COPYRIGHT_SIGN, b'g', b'e', b'n']);
    /// `©cmt`, the comment: the de-facto iTunes `ilst` atom carried by
    /// `private/fixtures/mp4/tags/lavf-tagged.m4a`.
    pub const COMMENT: Self = Self::FourCc([COPYRIGHT_SIGN, b'c', b'm', b't']);
    /// `©too`, the encoder: the de-facto iTunes `ilst` atom carried by
    /// `private/fixtures/mp4/tags/lavf-tagged.m4a`.
    pub const ENCODER: Self = Self::FourCc([COPYRIGHT_SIGN, b't', b'o', b'o']);
    /// `©wrt`, the name of the movie's writer (`User data list entry types`).
    pub const COMPOSER: Self = Self::FourCc([COPYRIGHT_SIGN, b'w', b'r', b't']);
    /// `trkn`, the track index and total: the de-facto iTunes `ilst` atom
    /// carried by `private/fixtures/mp4/tags/lavf-tagged.m4a`.
    pub const TRACK_NUMBER: Self = Self::FourCc(*b"trkn");
    /// `disk`, the disc index and total: the de-facto iTunes `ilst` atom
    /// carried by `private/fixtures/mp4/tags/lavf-tagged.m4a`.
    pub const DISC_NUMBER: Self = Self::FourCc(*b"disk");
    /// `covr`, cover art: the de-facto iTunes `ilst` atom, decoded as a binary
    /// item.
    pub const COVER_ART: Self = Self::FourCc(*b"covr");

    /// Apple's `com.apple.quicktime.make` key
    /// (`private/fixtures/mp4/quicktime/iphone-x-live-photo.mov`).
    pub const APPLE_MAKE: &'static str = "com.apple.quicktime.make";
    /// Apple's `com.apple.quicktime.model` key (same fixture).
    pub const APPLE_MODEL: &'static str = "com.apple.quicktime.model";
    /// Apple's `com.apple.quicktime.software` key (`QuickTime metadata keys`).
    pub const APPLE_SOFTWARE: &'static str = "com.apple.quicktime.software";
    /// Apple's `com.apple.quicktime.creationdate` key (`QuickTime metadata
    /// keys`).
    pub const APPLE_CREATION_DATE: &'static str = "com.apple.quicktime.creationdate";
    /// Apple's `com.apple.quicktime.location.ISO6709` key (`QuickTime metadata
    /// keys`).
    pub const APPLE_LOCATION_ISO6709: &'static str = "com.apple.quicktime.location.ISO6709";

    /// The four-CC form, when this key is one.
    #[must_use]
    pub fn as_four_cc(&self) -> Option<&[u8; BOX_TYPE_SIZE]> {
        match self {
            Self::FourCc(fourcc) => Some(fourcc),
            Self::Mdta(_) => None,
        }
    }

    /// The reverse-DNS form, when this key is one.
    #[must_use]
    pub fn as_mdta(&self) -> Option<&str> {
        match self {
            Self::FourCc(_) => None,
            Self::Mdta(key) => Some(key),
        }
    }
}

impl core::fmt::Display for ItemKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FourCc(fourcc) => write_four_cc(f, fourcc),
            Self::Mdta(key) => f.write_str(key),
        }
    }
}

impl core::fmt::Debug for ItemKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::FourCc(fourcc) => {
                f.write_str("FourCc(")?;
                write_four_cc(f, fourcc)?;
                f.write_str(")")
            }
            Self::Mdta(key) => f.debug_tuple("Mdta").field(key).finish(),
        }
    }
}

/// Write a four-character atom type as text: the `©` byte and printable
/// ASCII as themselves, every other byte as `\xNN`.
fn write_four_cc(
    f: &mut core::fmt::Formatter<'_>,
    fourcc: &[u8; BOX_TYPE_SIZE],
) -> core::fmt::Result {
    for byte in fourcc {
        match *byte {
            COPYRIGHT_SIGN => f.write_char('©')?,
            b' '..=b'~' => f.write_char(char::from(*byte))?,
            other => write!(f, "\\x{other:02x}")?,
        }
    }
    Ok(())
}

/// Where a text value's language/locale was carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TextLocale {
    /// A QuickTime international-text atom's 16-bit language code.
    QuickTime(u16),
    /// An `ilst` `data` atom's type, country and language.
    Itunes {
        /// The well-known `data` type (1 is UTF-8).
        data_type: u32,
        /// The country code in the locale indicator.
        country: u16,
        /// The language code in the locale indicator.
        language: u16,
    },
}

impl TextLocale {
    /// The language code, whichever form carried it.
    #[must_use]
    pub const fn language(&self) -> u16 {
        match self {
            Self::QuickTime(language) => *language,
            Self::Itunes { language, .. } => *language,
        }
    }
}

/// One value of a freeform (`----`) item.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FreeformValue {
    /// The well-known `data` type.
    pub data_type: u32,
    /// The locale indicator.
    pub locale: u32,
    /// The untouched value bytes.
    pub bytes: Vec<u8>,
}

impl FreeformValue {
    /// The value as UTF-8, when it is.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        core::str::from_utf8(&self.bytes).ok()
    }
}

/// One logical metadata item.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MetadataItem {
    /// A text value, in the form its container carried it.
    Text {
        /// The item's key.
        key: ItemKey,
        /// The decoded UTF-8 value.
        value: String,
        /// Where the language/locale came from.
        locale: TextLocale,
    },
    /// An integer value.
    Number {
        /// The item's key.
        key: ItemKey,
        /// The decoded value.
        value: i64,
        /// The well-known `data` type.
        data_type: u32,
        /// The locale indicator.
        locale: u32,
    },
    /// A `trkn`/`disk` index and total.
    Pair {
        /// The item's key (`trkn` or `disk`).
        key: ItemKey,
        /// The one-based index.
        index: u16,
        /// The total, zero when unknown.
        total: u16,
    },
    /// Any value whose type this crate does not interpret (cover art,
    /// unknown integer widths, a vendor item).
    Binary {
        /// The item's key.
        key: ItemKey,
        /// The well-known `data` type.
        data_type: u32,
        /// The locale indicator.
        locale: u32,
        /// The untouched value bytes.
        bytes: Vec<u8>,
    },
    /// A freeform `----` item: its `mean` namespace, `name` and values.
    Freeform {
        /// The `mean` namespace.
        mean: String,
        /// The `name` field.
        name: String,
        /// Every `data` value, in order.
        values: Vec<FreeformValue>,
    },
    /// A QuickTime `©xyz` ISO 6709 location string.
    Location(Iso6709),
    /// A 3GPP `loci` location box.
    Loci(Loci),
    /// A Nero `chpl` chapter list.
    Chapters(Vec<Chapter>),
    /// An atom this crate does not type.
    Opaque(OpaqueBox),
}

impl MetadataItem {
    /// The item's key, when it has one (a chapter list and an opaque atom do
    /// not).
    #[must_use]
    pub fn key(&self) -> Option<&ItemKey> {
        match self {
            Self::Text { key, .. }
            | Self::Number { key, .. }
            | Self::Pair { key, .. }
            | Self::Binary { key, .. } => Some(key),
            Self::Freeform { .. }
            | Self::Location(_)
            | Self::Loci(_)
            | Self::Chapters(_)
            | Self::Opaque(_) => None,
        }
    }
}

/// One cover-art value: its well-known `data` type and its untouched bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CoverArt<'a> {
    /// The well-known `data` type (`Value atom`): 13 for JPEG, 14 for PNG, 27
    /// for BMP.
    pub data_type: u32,
    /// The image bytes.
    pub bytes: &'a [u8],
}

/// The logical metadata of one movie or track.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Metadata {
    /// Every item, in the order the containers carried them.
    pub items: Vec<MetadataItem>,
}

/// The movie's metadata and each track's metadata.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MovieMetadata {
    /// Metadata on the `moov`.
    pub movie: Metadata,
    /// Metadata on each `trak`, in `moov` order.
    pub tracks: Vec<(TrackId, Metadata)>,
}

impl Metadata {
    /// Gather the item list of a `udta` box.
    #[must_use]
    pub fn from_udta(udta: &Udta) -> Self {
        let mut items = Vec::new();
        for child in &udta.children {
            push_udta_child(child, &mut items);
        }
        Self { items }
    }

    /// Gather the item list of a `meta` box.
    #[must_use]
    pub fn from_meta(meta: &Meta) -> Self {
        let mut items = Vec::new();
        push_meta(meta, &mut items);
        Self { items }
    }

    /// The first text item stored under `key`.
    #[must_use]
    pub fn text(&self, key: &ItemKey) -> Option<&str> {
        self.items.iter().find_map(|item| match item {
            MetadataItem::Text {
                key: item_key,
                value,
                ..
            } if item_key == key => Some(value.as_str()),
            _ => None,
        })
    }

    /// The first item stored under `key`.
    #[must_use]
    pub fn get(&self, key: &ItemKey) -> Option<&MetadataItem> {
        self.items
            .iter()
            .find(|item| item.key().is_some_and(|k| k == key))
    }

    /// The title ([`ItemKey::TITLE`]).
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.text(&ItemKey::TITLE)
    }

    /// The artist ([`ItemKey::ARTIST`]).
    #[must_use]
    pub fn artist(&self) -> Option<&str> {
        self.text(&ItemKey::ARTIST)
    }

    /// The album ([`ItemKey::ALBUM`]).
    #[must_use]
    pub fn album(&self) -> Option<&str> {
        self.text(&ItemKey::ALBUM)
    }

    /// The date the content was created ([`ItemKey::DATE`]).
    #[must_use]
    pub fn date(&self) -> Option<&str> {
        self.text(&ItemKey::DATE)
    }

    /// The genre ([`ItemKey::GENRE`]).
    #[must_use]
    pub fn genre(&self) -> Option<&str> {
        self.text(&ItemKey::GENRE)
    }

    /// The comment ([`ItemKey::COMMENT`]).
    #[must_use]
    pub fn comment(&self) -> Option<&str> {
        self.text(&ItemKey::COMMENT)
    }

    /// The encoder ([`ItemKey::ENCODER`]).
    #[must_use]
    pub fn encoder(&self) -> Option<&str> {
        self.text(&ItemKey::ENCODER)
    }

    /// The track index and total ([`ItemKey::TRACK_NUMBER`]).
    #[must_use]
    pub fn track(&self) -> Option<(u16, u16)> {
        self.pair(&ItemKey::TRACK_NUMBER)
    }

    /// The disc index and total ([`ItemKey::DISC_NUMBER`]).
    #[must_use]
    pub fn disc(&self) -> Option<(u16, u16)> {
        self.pair(&ItemKey::DISC_NUMBER)
    }

    /// The cover art ([`ItemKey::COVER_ART`]).
    #[must_use]
    pub fn cover_art(&self) -> Option<CoverArt<'_>> {
        self.items.iter().find_map(|item| match item {
            MetadataItem::Binary {
                key,
                data_type,
                bytes,
                ..
            } if *key == ItemKey::COVER_ART => Some(CoverArt {
                data_type: *data_type,
                bytes,
            }),
            _ => None,
        })
    }

    /// The location: a `©xyz` item or, when the file carries no such item, the
    /// `com.apple.quicktime.location.ISO6709` keyed item.
    ///
    /// The keyed form is a text value, so it is parsed here instead of the two
    /// forms sharing one borrowed value.
    #[must_use]
    pub fn location(&self) -> Option<Iso6709> {
        let mut keyed = None;
        for item in &self.items {
            match item {
                MetadataItem::Location(location) => return Some(location.clone()),
                MetadataItem::Text { key, value, .. }
                    if key.as_mdta() == Some(ItemKey::APPLE_LOCATION_ISO6709) =>
                {
                    keyed.get_or_insert_with(|| Iso6709::new(value.clone()));
                }
                _ => {}
            }
        }
        keyed
    }

    /// The Nero chapter list, empty when the file carries none.
    #[must_use]
    pub fn chapters(&self) -> &[Chapter] {
        self.items
            .iter()
            .find_map(|item| match item {
                MetadataItem::Chapters(chapters) => Some(chapters.as_slice()),
                _ => None,
            })
            .unwrap_or(&[])
    }

    /// Replace the text stored under `key`, or append it when the key is not
    /// present.
    ///
    /// Every other item and the item order are left untouched. A new item goes
    /// into the `ilst` form with an empty locale; a replaced one keeps its own.
    pub fn set_text(&mut self, key: &ItemKey, value: impl Into<String>) {
        let value = value.into();
        for item in &mut self.items {
            if let MetadataItem::Text {
                key: item_key,
                value: slot,
                ..
            } = item
                && item_key == key
            {
                *slot = value;
                return;
            }
        }
        self.items.push(MetadataItem::Text {
            key: key.clone(),
            value,
            locale: TextLocale::Itunes {
                data_type: ilst::DATA_TYPE_UTF8,
                country: 0,
                language: 0,
            },
        });
    }

    /// Replace the track index and total ([`ItemKey::TRACK_NUMBER`]), or append
    /// them.
    pub fn set_track(&mut self, index: u16, total: u16) {
        self.set_pair(&ItemKey::TRACK_NUMBER, index, total);
    }

    /// Replace the disc index and total ([`ItemKey::DISC_NUMBER`]), or append
    /// them.
    pub fn set_disc(&mut self, index: u16, total: u16) {
        self.set_pair(&ItemKey::DISC_NUMBER, index, total);
    }

    /// Remove every item stored under `key`, leaving the others in order.
    pub fn remove(&mut self, key: &ItemKey) {
        self.items
            .retain(|item| item.key().is_none_or(|item_key| item_key != key));
    }

    /// The first `trkn`/`disk` pair stored under `key`.
    fn pair(&self, key: &ItemKey) -> Option<(u16, u16)> {
        self.items.iter().find_map(|item| match item {
            MetadataItem::Pair {
                key: item_key,
                index,
                total,
            } if item_key == key => Some((*index, *total)),
            _ => None,
        })
    }

    /// Replace the first pair stored under `key`, or append it.
    fn set_pair(&mut self, key: &ItemKey, index: u16, total: u16) {
        for item in &mut self.items {
            if let MetadataItem::Pair {
                key: item_key,
                index: slot_index,
                total: slot_total,
            } = item
                && item_key == key
            {
                *slot_index = index;
                *slot_total = total;
                return;
            }
        }
        self.items.push(MetadataItem::Pair {
            key: key.clone(),
            index,
            total,
        });
    }

    /// Build the `udta` box that carries this item list.
    pub fn to_udta(&self) -> Result<Udta> {
        let mut udta = Udta::new();
        let mut list = Ilst::new();
        let mut table = Keys::new();
        for item in &self.items {
            push_item(item, &mut udta, &mut list, &mut table)?;
        }
        if !list.items.is_empty() {
            let handler = if table.entries.is_empty() {
                MDIR_HANDLER
            } else {
                MDTA_HANDLER
            };
            let mut meta = Meta::new(MetaFlavor::Iso);
            meta.children.push(MetaChild::Hdlr(Hdlr::new(handler)));
            if !table.entries.is_empty() {
                meta.children.push(MetaChild::Keys(table));
            }
            meta.children.push(MetaChild::Ilst(list));
            udta.children.push(UdtaChild::Meta(meta));
        }
        Ok(udta)
    }

    /// The serialized `udta` box.
    pub fn to_udta_bytes(&self) -> Result<Vec<u8>> {
        self.to_udta()?.try_to_bytes()
    }

    /// Build the `meta` box that carries this item list, empty when the list
    /// has no `ilst` items.
    pub fn to_meta(&self) -> Result<Meta> {
        let udta = self.to_udta()?;
        for child in udta.children {
            if let UdtaChild::Meta(meta) = child {
                return Ok(meta);
            }
        }
        Ok(Meta::new(MetaFlavor::Iso))
    }

    /// The serialized `meta` box.
    pub fn to_meta_bytes(&self) -> Result<Vec<u8>> {
        self.to_meta()?.try_to_bytes()
    }
}

fn push_udta_child(child: &UdtaChild, items: &mut Vec<MetadataItem>) {
    match child {
        UdtaChild::Text(text) => {
            for translation in &text.translations {
                let Some(value) = core::str::from_utf8(&translation.value).ok() else {
                    continue;
                };
                if text.key == XYZ_KEY {
                    items.push(MetadataItem::Location(Iso6709::new(value)));
                } else {
                    items.push(MetadataItem::Text {
                        key: ItemKey::FourCc(text.key),
                        value: String::from(value),
                        locale: TextLocale::QuickTime(translation.language),
                    });
                }
            }
        }
        UdtaChild::Chapters(chpl) => items.push(MetadataItem::Chapters(chpl.chapters.clone())),
        UdtaChild::Location(loci) => items.push(MetadataItem::Loci(loci.clone())),
        UdtaChild::Meta(meta) => push_meta(meta, items),
        UdtaChild::Kind(_)
        | UdtaChild::Label(_)
        | UdtaChild::Loudness(_)
        | UdtaChild::TrackSelection(_)
        | UdtaChild::Copyright(_)
        | UdtaChild::SubTrack(_) => {}
        UdtaChild::Opaque(opaque) => items.push(MetadataItem::Opaque(opaque.clone())),
    }
}

fn push_meta(meta: &Meta, items: &mut Vec<MetadataItem>) {
    let keys = meta.keys();
    if let Some(list) = meta.ilst() {
        for item in &list.items {
            push_ilst_item(item, keys, items);
        }
    }
    for child in &meta.children {
        if let MetaChild::Opaque(opaque) = child {
            items.push(MetadataItem::Opaque(opaque.clone()));
        }
    }
}

fn push_ilst_item(item: &IlstItem, keys: Option<&Keys>, items: &mut Vec<MetadataItem>) {
    if item.item_type == ilst::FREEFORM {
        let values = item
            .values()
            .map(|data| FreeformValue {
                data_type: data.data_type,
                locale: data.locale,
                bytes: data.value.clone(),
            })
            .collect();
        items.push(MetadataItem::Freeform {
            mean: item.mean().unwrap_or_default().into(),
            name: item.name().unwrap_or_default().into(),
            values,
        });
        return;
    }
    let key = ilst_key(item.item_type, keys);
    let mut pushed_data = false;
    for data in item.values() {
        pushed_data = true;
        push_data(&key, data, items);
    }
    if !pushed_data
        && let Ok(bytes) = item.try_to_bytes()
        && let Ok((boxed, _)) = crate::box_types::parse_box(&bytes)
    {
        items.push(MetadataItem::Opaque(OpaqueBox::new(
            item.item_type,
            boxed.payload().to_vec(),
        )));
    }
}

fn ilst_key(item_type: [u8; BOX_TYPE_SIZE], keys: Option<&Keys>) -> ItemKey {
    if let Some(keys) = keys
        && let Some(value) = keys.key(u32::from_be_bytes(item_type))
    {
        return ItemKey::Mdta(String::from(value));
    }
    ItemKey::FourCc(item_type)
}

fn push_data(key: &ItemKey, data: &Data, items: &mut Vec<MetadataItem>) {
    if let ItemKey::FourCc(fourcc) = key
        && (fourcc == b"trkn" || fourcc == b"disk")
        && data.data_type == ilst::DATA_TYPE_RESERVED
        && data.value.len() == ilst::PAIR_VALUE_LEN
    {
        items.push(MetadataItem::Pair {
            key: key.clone(),
            index: u16_at(&data.value, ilst::PAIR_INDEX_OFFSET),
            total: u16_at(&data.value, ilst::PAIR_TOTAL_OFFSET),
        });
        return;
    }
    if data.data_type == ilst::DATA_TYPE_UTF8
        && let Ok(value) = core::str::from_utf8(&data.value)
    {
        let country = (data.locale >> LOCALE_LANGUAGE_BITS) as u16;
        let language = (data.locale & LOCALE_LANGUAGE_MASK) as u16;
        items.push(MetadataItem::Text {
            key: key.clone(),
            value: String::from(value),
            locale: TextLocale::Itunes {
                data_type: data.data_type,
                country,
                language,
            },
        });
        return;
    }
    if let Some(value) = decode_number(&data.value, data.data_type) {
        items.push(MetadataItem::Number {
            key: key.clone(),
            value,
            data_type: data.data_type,
            locale: data.locale,
        });
        return;
    }
    items.push(MetadataItem::Binary {
        key: key.clone(),
        data_type: data.data_type,
        locale: data.locale,
        bytes: data.value.clone(),
    });
}

fn decode_number(bytes: &[u8], data_type: u32) -> Option<i64> {
    let value = match data_type {
        ilst::DATA_TYPE_I8 => i64::from(*bytes.first()? as i8),
        ilst::DATA_TYPE_U8 => i64::from(*bytes.first()?),
        ilst::DATA_TYPE_I16 => i64::from(i16::from_be_bytes(bytes.try_into().ok()?)),
        ilst::DATA_TYPE_U16 => i64::from(u16::from_be_bytes(bytes.try_into().ok()?)),
        ilst::DATA_TYPE_I32 | ilst::DATA_TYPE_BE_SIGNED => {
            i64::from(i32::from_be_bytes(bytes.try_into().ok()?))
        }
        ilst::DATA_TYPE_U32 | ilst::DATA_TYPE_BE_UNSIGNED => {
            i64::from(u32::from_be_bytes(bytes.try_into().ok()?))
        }
        ilst::DATA_TYPE_I64 => i64::from_be_bytes(bytes.try_into().ok()?),
        ilst::DATA_TYPE_U64 => i64::try_from(u64::from_be_bytes(bytes.try_into().ok()?)).ok()?,
        _ => return None,
    };
    (number_bytes(value, data_type).as_deref() == Some(bytes)).then_some(value)
}

fn number_bytes(value: i64, data_type: u32) -> Option<Vec<u8>> {
    Some(match data_type {
        ilst::DATA_TYPE_I8 => alloc::vec![i8::try_from(value).ok()? as u8],
        ilst::DATA_TYPE_U8 => alloc::vec![u8::try_from(value).ok()?],
        ilst::DATA_TYPE_I16 => i16::try_from(value).ok()?.to_be_bytes().to_vec(),
        ilst::DATA_TYPE_U16 => u16::try_from(value).ok()?.to_be_bytes().to_vec(),
        ilst::DATA_TYPE_I32 | ilst::DATA_TYPE_BE_SIGNED => {
            i32::try_from(value).ok()?.to_be_bytes().to_vec()
        }
        ilst::DATA_TYPE_U32 | ilst::DATA_TYPE_BE_UNSIGNED => {
            u32::try_from(value).ok()?.to_be_bytes().to_vec()
        }
        ilst::DATA_TYPE_I64 => value.to_be_bytes().to_vec(),
        ilst::DATA_TYPE_U64 => u64::try_from(value).ok()?.to_be_bytes().to_vec(),
        _ => return None,
    })
}

fn push_item(
    item: &MetadataItem,
    udta: &mut Udta,
    list: &mut Ilst,
    table: &mut Keys,
) -> Result<()> {
    match item {
        MetadataItem::Text { key, value, locale } => match locale {
            TextLocale::QuickTime(language) => udta.push_text(UdtaText {
                key: four_cc(key)?,
                translations: alloc::vec![UdtaTextTranslation {
                    language: *language,
                    value: value.as_bytes().to_vec(),
                }],
                plain_nul_terminated: true,
            }),
            TextLocale::Itunes {
                data_type,
                country,
                language,
            } => {
                let locale = (u32::from(*country) << LOCALE_LANGUAGE_BITS) | u32::from(*language);
                list.push_item(IlstItem::with_data(
                    item_type(key, table)?,
                    *data_type,
                    locale,
                    value.as_bytes().to_vec(),
                ));
            }
        },
        MetadataItem::Location(location) => udta.push_text(UdtaText {
            key: XYZ_KEY,
            translations: alloc::vec![UdtaTextTranslation {
                language: 0,
                value: location.as_str().as_bytes().to_vec(),
            }],
            plain_nul_terminated: true,
        }),
        MetadataItem::Loci(loci) => udta.children.push(UdtaChild::Location(loci.clone())),
        MetadataItem::Chapters(chapters) => udta
            .children
            .push(UdtaChild::Chapters(Chpl::new(chapters.clone()))),
        MetadataItem::Opaque(opaque) => udta.children.push(UdtaChild::Opaque(opaque.clone())),
        MetadataItem::Pair { key, index, total } => {
            list.push_item(IlstItem::pair(item_type(key, table)?, *index, *total))
        }
        MetadataItem::Number {
            key,
            value,
            data_type,
            locale,
        } => {
            let bytes = number_bytes(*value, *data_type).ok_or(Error::InvalidInput(
                "a Number item's value does not fit its data type",
            ))?;
            list.push_item(IlstItem::with_data(
                item_type(key, table)?,
                *data_type,
                *locale,
                bytes,
            ));
        }
        MetadataItem::Binary {
            key,
            data_type,
            locale,
            bytes,
        } => list.push_item(IlstItem::with_data(
            item_type(key, table)?,
            *data_type,
            *locale,
            bytes.clone(),
        )),
        MetadataItem::Freeform { mean, name, values } => {
            list.push_item(IlstItem::freeform(mean, name, values))
        }
    }
    Ok(())
}

fn four_cc(key: &ItemKey) -> Result<[u8; BOX_TYPE_SIZE]> {
    match key {
        ItemKey::FourCc(fourcc) => Ok(*fourcc),
        ItemKey::Mdta(_) => Err(Error::InvalidInput(
            "a reverse-DNS key cannot be a udta text atom",
        )),
    }
}

fn item_type(key: &ItemKey, table: &mut Keys) -> Result<[u8; BOX_TYPE_SIZE]> {
    match key {
        ItemKey::FourCc(fourcc) => Ok(*fourcc),
        ItemKey::Mdta(value) => table.push(value.clone()),
    }
}

/// Whether `fourcc` names a QuickTime/3GPP text atom.
pub(crate) fn is_text_key(fourcc: &[u8; BOX_TYPE_SIZE]) -> bool {
    fourcc[0] == COPYRIGHT_SIGN
        || matches!(
            fourcc,
            b"titl" | b"dscp" | b"cprt" | b"auth" | b"perf" | b"albm" | b"gnre" | b"yrrc" | b"name"
        )
}

/// Parse a big-endian u16 at `off`, the caller having proved two bytes there.
pub(crate) fn u16_at(bytes: &[u8], off: usize) -> u16 {
    broadcast_core::wire::u16_at(bytes, off)
}

/// Parse a big-endian u32 at `off`, the caller having proved four bytes there.
pub(crate) fn u32_at(bytes: &[u8], off: usize) -> u32 {
    broadcast_core::wire::u32_at(bytes, off)
}

/// Parse a big-endian u64 at `off`, the caller having proved eight bytes there.
pub(crate) fn u64_at(bytes: &[u8], off: usize) -> u64 {
    broadcast_core::wire::u64_at(bytes, off)
}
