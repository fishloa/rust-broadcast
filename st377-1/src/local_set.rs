//! The "local set" KLV-lite framing used by every Header Metadata Set —
//! SMPTE ST 377-1:2019 §9.3/§9.6.1 (`docs/st377-1.md`).
//!
//! A local set's outer Key+Length is an ordinary [`crate::KlvItem`]; its
//! Value is a sequence of `{local_tag: u16, length: u16 | BER, value}`
//! items (Figure 8), with the Key's byte 6 selecting which length encoding
//! applies (§9.3 Note 1). [`LocalSet`] is the generic, identified-but-not-
//! deeply-typed fallback this crate uses for every Header Metadata Set
//! other than the four Root Metadata Sets (`Preface`/`Identification`/
//! `ContentStorage`/`EssenceContainerData`) — see `docs/st377-1.md`'s Scope
//! section.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::ber::{BerLength, ber_length_size_for, decode_ber_length, encode_ber_length_as};
use crate::error::{Error, Result};
use crate::types::{UlBytes, ul_bytes_from_prefix};

/// Byte 6 of a Local Set Key (§9.3 Note 1): which length encoding its
/// items use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ItemLengthMode {
    /// `0x53` — every item's length is a fixed 2-byte `UInt16` (default;
    /// required whenever every property's value is <= 65535 bytes).
    TwoByte,
    /// `0x13` — every item's length is BER-encoded, short or long form
    /// (required whenever any property's value exceeds 65535 bytes).
    Ber,
}

impl ItemLengthMode {
    /// The spec's own label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::TwoByte => "2-byte length",
            Self::Ber => "BER length",
        }
    }

    /// Byte 6 value for this mode.
    #[must_use]
    pub fn registry_designator_byte(self) -> u8 {
        match self {
            Self::TwoByte => 0x53,
            Self::Ber => 0x13,
        }
    }

    /// Decode from a Local Set Key's byte 6. Any value other than `0x53`/
    /// `0x13` is not a Local Set key at all (see [`is_local_set_key`]).
    #[must_use]
    pub fn from_registry_designator_byte(b: u8) -> Option<Self> {
        match b {
            0x53 => Some(Self::TwoByte),
            0x13 => Some(Self::Ber),
            _ => None,
        }
    }
}

broadcast_common::impl_spec_display!(ItemLengthMode);

/// The fixed bytes of every Structural Metadata Set Key (Table 16) other
/// than byte 6 (item length mode, [`ItemLengthMode`]), byte 8 (registry
/// version, a wildcard on parse), and bytes 14/15 (Set Kind, see
/// [`StructuralSetKind`]).
const SET_KEY_FIXED: [(usize, u8); 8] = [
    (0, 0x06),
    (1, 0x0E),
    (2, 0x2B),
    (3, 0x34),
    (4, 0x02),
    (6, 0x01),
    (9, 0x01),
    (11, 0x01),
];
// Byte 10 (Organization) and byte 13 (Structure Kind) both `0x01` too, but
// byte 10 doubles as part of the Application field family shared with
// Abstract Group ULs (Table 18) which also uses `0x01`; kept explicit
// below for clarity rather than folded into the table above.
const SET_KEY_BYTE10: u8 = 0x01;
const SET_KEY_BYTE12: u8 = 0x01;

/// True if `key` matches the common Local Set Key structure (Table 16):
/// fixed prefix bytes, byte 6 a valid [`ItemLengthMode`], and the fixed
/// `0x0D`/organization/application/structure-kind bytes. Byte 15
/// (reserved) is not checked (some dark extensions may not zero it).
#[must_use]
pub fn is_local_set_key(key: &UlBytes) -> bool {
    SET_KEY_FIXED.iter().all(|&(i, v)| key[i] == v)
        && ItemLengthMode::from_registry_designator_byte(key[5]).is_some()
        && key[8] == 0x0D
        && key[9] == SET_KEY_BYTE10
        && key[10] == 0x01
        && key[12] == SET_KEY_BYTE12
}

/// Set Kind (Table 17 — this crate's byte 14/15 identification list for
/// every Header Metadata Set the spec defines, whether or not this crate
/// types its properties). See `docs/st377-1.md`'s Table 17 reproduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum StructuralSetKind {
    /// Preface (A.2) — typed, [`crate::Preface`].
    Preface,
    /// Identification (A.3) — typed, [`crate::Identification`].
    Identification,
    /// Content Storage (A.4) — typed, [`crate::ContentStorage`].
    ContentStorage,
    /// Essence Container Data (A.5) — typed, [`crate::EssenceContainerData`].
    EssenceContainerData,
    /// Material Package (E.1) — identified only.
    MaterialPackage,
    /// Source Package, File/Physical variants (E.2-E.4) — identified only.
    SourcePackage,
    /// Timeline Track, all cases (B.12/B.15/B.18/B.21/B.24/B.27.1) —
    /// identified only.
    TimelineTrack,
    /// Event Track (DM) (B.13/B.27.2) — identified only.
    EventTrackDm,
    /// Static Track (DM) (B.14/B.27.3) — identified only.
    StaticTrackDm,
    /// Sequence, all cases (B.9/B.16/B.19/B.22/B.25/B.28) — identified only.
    Sequence,
    /// Source Clip, Picture/Sound/Data (B.10/B.20/B.23/B.26) — identified
    /// only.
    SourceClip,
    /// Timecode Component (B.17) — identified only.
    TimecodeComponent,
    /// DM Segment (B.32) — identified only.
    DmSegment,
    /// DM Source Clip (B.33) — identified only.
    DmSourceClip,
    /// Filler (B.11) — identified only.
    Filler,
    /// Package Marker Object (B.34) — identified only.
    PackageMarkerObject,
    /// File Descriptor (F.2) — identified only.
    FileDescriptor,
    /// Generic Picture Essence Descriptor (F.4.1) — identified only.
    GenericPictureEssenceDescriptor,
    /// CDCI Essence Descriptor (F.4.2) — identified only.
    CdciEssenceDescriptor,
    /// RGBA Essence Descriptor (F.4.3) — identified only.
    RgbaEssenceDescriptor,
    /// Generic Sound Essence Descriptor (F.5) — identified only.
    GenericSoundEssenceDescriptor,
    /// Generic Data Essence Descriptor (F.6) — identified only.
    GenericDataEssenceDescriptor,
    /// Multiple Descriptor (F.3) — identified only.
    MultipleDescriptor,
    /// Network Locator (B.4) — identified only.
    NetworkLocator,
    /// Text Locator (B.5) — identified only.
    TextLocator,
    /// Application Plug-In Object (C.2) — identified only.
    ApplicationPlugInObject,
    /// Application Referenced Object (C.3) — identified only.
    ApplicationReferencedObject,
    /// Any byte 14/15 pair not in Table 17 (private/dark extension, or a
    /// Set defined by another SMPTE document, e.g. an Essence Container or
    /// Operational Pattern spec — §9.6.1 Note 3).
    Unknown([u8; 2]),
}

impl StructuralSetKind {
    /// The spec's own Set name (Table 17), `"unknown"` for
    /// [`Self::Unknown`].
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Preface => "Preface",
            Self::Identification => "Identification",
            Self::ContentStorage => "Content Storage",
            Self::EssenceContainerData => "Essence Container Data",
            Self::MaterialPackage => "Material Package",
            Self::SourcePackage => "Source Package",
            Self::TimelineTrack => "Timeline Track",
            Self::EventTrackDm => "Event Track (DM)",
            Self::StaticTrackDm => "Static Track (DM)",
            Self::Sequence => "Sequence",
            Self::SourceClip => "Source Clip",
            Self::TimecodeComponent => "Timecode Component",
            Self::DmSegment => "DM Segment",
            Self::DmSourceClip => "DM Source Clip",
            Self::Filler => "Filler",
            Self::PackageMarkerObject => "Package Marker Object",
            Self::FileDescriptor => "File Descriptor",
            Self::GenericPictureEssenceDescriptor => "Generic Picture Essence Descriptor",
            Self::CdciEssenceDescriptor => "CDCI Essence Descriptor",
            Self::RgbaEssenceDescriptor => "RGBA Essence Descriptor",
            Self::GenericSoundEssenceDescriptor => "Generic Sound Essence Descriptor",
            Self::GenericDataEssenceDescriptor => "Generic Data Essence Descriptor",
            Self::MultipleDescriptor => "Multiple Descriptor",
            Self::NetworkLocator => "Network Locator",
            Self::TextLocator => "Text Locator",
            Self::ApplicationPlugInObject => "Application Plug-In Object",
            Self::ApplicationReferencedObject => "Application Referenced Object",
            Self::Unknown(_) => "unknown",
        }
    }

    /// Decode from a Set Key's bytes 14/15 (Table 17).
    #[must_use]
    pub fn from_bytes(b14: u8, b15: u8) -> Self {
        match (b14, b15) {
            (0x01, 0x2F) => Self::Preface,
            (0x01, 0x30) => Self::Identification,
            (0x01, 0x18) => Self::ContentStorage,
            (0x01, 0x23) => Self::EssenceContainerData,
            (0x01, 0x36) => Self::MaterialPackage,
            (0x01, 0x37) => Self::SourcePackage,
            (0x01, 0x3B) => Self::TimelineTrack,
            (0x01, 0x39) => Self::EventTrackDm,
            (0x01, 0x3A) => Self::StaticTrackDm,
            (0x01, 0x0F) => Self::Sequence,
            (0x01, 0x11) => Self::SourceClip,
            (0x01, 0x14) => Self::TimecodeComponent,
            (0x01, 0x41) => Self::DmSegment,
            (0x01, 0x45) => Self::DmSourceClip,
            (0x01, 0x09) => Self::Filler,
            (0x01, 0x60) => Self::PackageMarkerObject,
            (0x01, 0x25) => Self::FileDescriptor,
            (0x01, 0x27) => Self::GenericPictureEssenceDescriptor,
            (0x01, 0x28) => Self::CdciEssenceDescriptor,
            (0x01, 0x29) => Self::RgbaEssenceDescriptor,
            (0x01, 0x42) => Self::GenericSoundEssenceDescriptor,
            (0x01, 0x43) => Self::GenericDataEssenceDescriptor,
            (0x01, 0x44) => Self::MultipleDescriptor,
            (0x01, 0x32) => Self::NetworkLocator,
            (0x01, 0x33) => Self::TextLocator,
            (0x01, 0x61) => Self::ApplicationPlugInObject,
            (0x01, 0x62) => Self::ApplicationReferencedObject,
            other => Self::Unknown([other.0, other.1]),
        }
    }

    /// Encode to a Set Key's bytes 14/15.
    #[must_use]
    pub fn to_bytes(self) -> [u8; 2] {
        match self {
            Self::Preface => [0x01, 0x2F],
            Self::Identification => [0x01, 0x30],
            Self::ContentStorage => [0x01, 0x18],
            Self::EssenceContainerData => [0x01, 0x23],
            Self::MaterialPackage => [0x01, 0x36],
            Self::SourcePackage => [0x01, 0x37],
            Self::TimelineTrack => [0x01, 0x3B],
            Self::EventTrackDm => [0x01, 0x39],
            Self::StaticTrackDm => [0x01, 0x3A],
            Self::Sequence => [0x01, 0x0F],
            Self::SourceClip => [0x01, 0x11],
            Self::TimecodeComponent => [0x01, 0x14],
            Self::DmSegment => [0x01, 0x41],
            Self::DmSourceClip => [0x01, 0x45],
            Self::Filler => [0x01, 0x09],
            Self::PackageMarkerObject => [0x01, 0x60],
            Self::FileDescriptor => [0x01, 0x25],
            Self::GenericPictureEssenceDescriptor => [0x01, 0x27],
            Self::CdciEssenceDescriptor => [0x01, 0x28],
            Self::RgbaEssenceDescriptor => [0x01, 0x29],
            Self::GenericSoundEssenceDescriptor => [0x01, 0x42],
            Self::GenericDataEssenceDescriptor => [0x01, 0x43],
            Self::MultipleDescriptor => [0x01, 0x44],
            Self::NetworkLocator => [0x01, 0x32],
            Self::TextLocator => [0x01, 0x33],
            Self::ApplicationPlugInObject => [0x01, 0x61],
            Self::ApplicationReferencedObject => [0x01, 0x62],
            Self::Unknown([a, b]) => [a, b],
        }
    }
}

impl core::fmt::Display for StructuralSetKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unknown([a, b]) => write!(f, "unknown(0x{a:02X}{b:02X})"),
            other => f.write_str(other.name()),
        }
    }
}

/// One `{local_tag, value}` item inside a [`LocalSet`] (Figure 8).
///
/// `PartialEq`/`Eq` compare only `tag`/`value` — `len_size` is a
/// serialization-*form* preference, not part of the item's logical value
/// (see [`crate::KlvItem`]'s doc; issue #1047 / audit MX-C1).
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalSetItem<'a> {
    /// The item's 2-byte local tag.
    pub tag: u16,
    /// The item's value bytes (borrowed).
    pub value: &'a [u8],
    /// The on-wire BER length-field width, when the parent
    /// [`LocalSet::item_length_mode`] is [`ItemLengthMode::Ber`] —
    /// [`BerLength::Minimal`] for a freshly built item, or the exact
    /// width parsed from the wire. Meaningless (and ignored on serialize)
    /// under [`ItemLengthMode::TwoByte`], where every item length is
    /// always exactly 2 bytes with no ambiguity to preserve.
    pub len_size: BerLength,
}

impl PartialEq for LocalSetItem<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag && self.value == other.value
    }
}

impl Eq for LocalSetItem<'_> {}

/// A Header Metadata Set encoded with MXF's "local set" framing (§9.3): a
/// 16-byte Set Key identifying which Set this is (see
/// [`StructuralSetKind`]), a BER Length, and a sequence of
/// [`LocalSetItem`]s.
///
/// `PartialEq`/`Eq` compare only `key`/`items` — `len_size` is a
/// serialization-*form* preference, not part of the Set's logical value
/// (see [`crate::KlvItem`]'s doc; issue #1047 / audit MX-C1).
#[derive(Debug, Clone, Default)]
pub struct LocalSet<'a> {
    /// The Set Key (all 16 bytes, as found on the wire).
    pub key: UlBytes,
    /// The Set's items, in on-wire order.
    pub items: Vec<LocalSetItem<'a>>,
    /// The on-wire BER length-field width of the Set's own outer envelope
    /// — [`BerLength::Minimal`] for a freshly built value, or the exact
    /// width `parse` found on the wire.
    pub len_size: BerLength,
}

impl PartialEq for LocalSet<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.items == other.items
    }
}

impl Eq for LocalSet<'_> {}

impl<'a> LocalSet<'a> {
    /// This Set's Kind (Table 17), from its Key's bytes 14/15.
    #[must_use]
    pub fn kind(&self) -> StructuralSetKind {
        StructuralSetKind::from_bytes(self.key[13], self.key[14])
    }

    /// This Set's item length mode (Table 16 byte 6).
    ///
    /// `key` is a public field (callers can build a `LocalSet` directly, not
    /// only via [`LocalSet::parse_prefix`]), so this cannot assume byte 6 is
    /// one of the two valid values — falls back to [`ItemLengthMode::TwoByte`]
    /// (the spec's own documented default) rather than panicking on a
    /// self-constructed `LocalSet` with an invalid key.
    #[must_use]
    pub fn item_length_mode(&self) -> ItemLengthMode {
        ItemLengthMode::from_registry_designator_byte(self.key[5])
            .unwrap_or(ItemLengthMode::TwoByte)
    }

    /// The first item with local tag `tag`, if any.
    #[must_use]
    pub fn get(&self, tag: u16) -> Option<&'a [u8]> {
        self.items.iter().find(|i| i.tag == tag).map(|i| i.value)
    }

    /// Build a fresh Local Set Key for `kind`, using `mode` for the item
    /// length encoding and registry version `0x01`.
    #[must_use]
    pub fn build_key(kind: StructuralSetKind, mode: ItemLengthMode) -> UlBytes {
        let [b14, b15] = kind.to_bytes();
        [
            0x06,
            0x0E,
            0x2B,
            0x34,
            0x02,
            mode.registry_designator_byte(),
            0x01,
            0x01, // registry version
            0x0D,
            0x01,
            0x01,
            0x01,
            0x01,
            b14,
            b15,
            0x00,
        ]
    }

    /// Parse one Local Set (Key + BER Length + items) from the start of
    /// `bytes`, returning it with the total bytes consumed — use this to
    /// walk a sequence of Header Metadata Sets in a stream.
    pub fn parse_prefix(bytes: &'a [u8]) -> Result<(Self, usize)> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "Local Set key",
            });
        }
        let key: UlBytes = ul_bytes_from_prefix(bytes);
        if !is_local_set_key(&key) {
            return Err(Error::KeyPrefixMismatch {
                what: "Local Set (Table 16)",
            });
        }
        // is_local_set_key already confirmed key[5] is a valid mode byte;
        // TwoByte is unreachable here but kept as the same documented
        // fallback item_length_mode() uses, rather than a second code path.
        let mode = ItemLengthMode::from_registry_designator_byte(key[5])
            .unwrap_or(ItemLengthMode::TwoByte);

        let (len, len_token_size) = decode_ber_length(&bytes[16..])?;
        let value_start = 16 + len_token_size;
        let len = usize::try_from(len).map_err(|_| Error::BufferTooShort {
            need: usize::MAX,
            have: bytes.len(),
            what: "Local Set value (length exceeds platform usize)",
        })?;
        let value_end = value_start.checked_add(len).ok_or(Error::BufferTooShort {
            need: usize::MAX,
            have: bytes.len(),
            what: "Local Set value (length overflow)",
        })?;
        if bytes.len() < value_end {
            return Err(Error::BufferTooShort {
                need: value_end,
                have: bytes.len(),
                what: "Local Set value",
            });
        }

        let mut cursor = &bytes[value_start..value_end];
        let mut items = Vec::new();
        while !cursor.is_empty() {
            if cursor.len() < 2 {
                return Err(Error::BufferTooShort {
                    need: 2,
                    have: cursor.len(),
                    what: "Local Set item tag",
                });
            }
            let tag = u16::from_be_bytes([cursor[0], cursor[1]]);
            let rest = &cursor[2..];
            let (item_len, item_len_size) = match mode {
                ItemLengthMode::TwoByte => {
                    if rest.len() < 2 {
                        return Err(Error::BufferTooShort {
                            need: 2,
                            have: rest.len(),
                            what: "Local Set item 2-byte length",
                        });
                    }
                    (u64::from(u16::from_be_bytes([rest[0], rest[1]])), 2)
                }
                ItemLengthMode::Ber => decode_ber_length(rest)?,
            };
            // #1108/MX-W1: `item_len` is a 64-bit length; a bare `as usize`
            // truncates it on a 32-bit target instead of rejecting an
            // over-range value.
            let item_len = usize::try_from(item_len).map_err(|_| Error::BufferTooShort {
                need: usize::MAX,
                have: rest.len(),
                what: "Local Set item value (length exceeds platform usize)",
            })?;
            let value_start = item_len_size;
            let value_end = value_start
                .checked_add(item_len)
                .ok_or(Error::BufferTooShort {
                    need: usize::MAX,
                    have: rest.len(),
                    what: "Local Set item value (length overflow)",
                })?;
            if rest.len() < value_end {
                return Err(Error::BufferTooShort {
                    need: value_end,
                    have: rest.len(),
                    what: "Local Set item value",
                });
            }
            items.push(LocalSetItem {
                tag,
                value: &rest[value_start..value_end],
                // Only meaningful in Ber mode (TwoByte's 2-byte width has
                // no ambiguity to preserve); harmless to record either way.
                len_size: BerLength::fixed_from_consumed(item_len_size),
            });
            cursor = &rest[value_end..];
        }

        // #1108/MX-W2: §9.3 forbids duplicate local tags within a Set.
        // Detected here (once, for every caller) rather than left to each
        // typed Set's own accessors, which took the first match and folded
        // any repeat into `dark` too (its tag being "known"), silently
        // dropping the duplicate's value on every round-trip.
        let mut tags: alloc::vec::Vec<u16> = items.iter().map(|i| i.tag).collect();
        tags.sort_unstable();
        if let Some(w) = tags.windows(2).find(|w| w[0] == w[1]) {
            return Err(Error::DuplicateLocalTag(w[0]));
        }

        Ok((
            LocalSet {
                key,
                items,
                len_size: BerLength::fixed_from_consumed(len_token_size),
            },
            value_end,
        ))
    }
}

impl<'a> Parse<'a> for LocalSet<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (set, consumed) = Self::parse_prefix(bytes)?;
        if consumed != bytes.len() {
            return Err(Error::BufferTooShort {
                need: consumed,
                have: bytes.len(),
                what: "Local Set (trailing bytes after exact-fit parse)",
            });
        }
        Ok(set)
    }
}

impl Serialize for LocalSet<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mode = self.item_length_mode();
        let items_len: usize = self
            .items
            .iter()
            .map(|i| {
                2 + match mode {
                    ItemLengthMode::TwoByte => 2,
                    ItemLengthMode::Ber => ber_length_size_for(i.value.len() as u64, i.len_size),
                } + i.value.len()
            })
            .sum();
        16 + ber_length_size_for(items_len as u64, self.len_size) + items_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        // #1108/MX-W6: `item_length_mode()` falls back to `TwoByte` for an
        // invalid byte 6 (registry designator), so a hand-built `LocalSet`
        // with a bad key used to serialize successfully in that mode and
        // then fail `is_local_set_key` on re-parse — serialize accepted
        // what parse rejects. Reject the bad key here instead.
        if !is_local_set_key(&self.key) {
            return Err(Error::KeyPrefixMismatch {
                what: "Local Set (Table 16)",
            });
        }
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: buf.len(),
                what: "Local Set",
            });
        }
        buf[..16].copy_from_slice(&self.key);
        let mode = self.item_length_mode();
        let items_len: usize = self
            .items
            .iter()
            .map(|i| {
                2 + match mode {
                    ItemLengthMode::TwoByte => 2,
                    ItemLengthMode::Ber => ber_length_size_for(i.value.len() as u64, i.len_size),
                } + i.value.len()
            })
            .sum();
        let len_size = encode_ber_length_as(items_len as u64, self.len_size, &mut buf[16..])?;
        let mut pos = 16 + len_size;
        for item in &self.items {
            buf[pos..pos + 2].copy_from_slice(&item.tag.to_be_bytes());
            pos += 2;
            match mode {
                ItemLengthMode::TwoByte => {
                    let len = u16::try_from(item.value.len()).map_err(|_| {
                        Error::InvalidPropertyLength {
                            tag: item.tag,
                            name: "Local Set item",
                            found: item.value.len(),
                            expected: usize::from(u16::MAX),
                        }
                    })?;
                    buf[pos..pos + 2].copy_from_slice(&len.to_be_bytes());
                    pos += 2;
                }
                ItemLengthMode::Ber => {
                    pos += encode_ber_length_as(
                        item.value.len() as u64,
                        item.len_size,
                        &mut buf[pos..],
                    )?;
                }
            }
            buf[pos..pos + item.value.len()].copy_from_slice(item.value);
            pos += item.value.len();
        }
        Ok(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_set_kind_round_trips_every_named_variant() {
        let variants = [
            StructuralSetKind::Preface,
            StructuralSetKind::Identification,
            StructuralSetKind::ContentStorage,
            StructuralSetKind::EssenceContainerData,
            StructuralSetKind::MaterialPackage,
            StructuralSetKind::SourcePackage,
            StructuralSetKind::TimelineTrack,
            StructuralSetKind::EventTrackDm,
            StructuralSetKind::StaticTrackDm,
            StructuralSetKind::Sequence,
            StructuralSetKind::SourceClip,
            StructuralSetKind::TimecodeComponent,
            StructuralSetKind::DmSegment,
            StructuralSetKind::DmSourceClip,
            StructuralSetKind::Filler,
            StructuralSetKind::PackageMarkerObject,
            StructuralSetKind::FileDescriptor,
            StructuralSetKind::GenericPictureEssenceDescriptor,
            StructuralSetKind::CdciEssenceDescriptor,
            StructuralSetKind::RgbaEssenceDescriptor,
            StructuralSetKind::GenericSoundEssenceDescriptor,
            StructuralSetKind::GenericDataEssenceDescriptor,
            StructuralSetKind::MultipleDescriptor,
            StructuralSetKind::NetworkLocator,
            StructuralSetKind::TextLocator,
            StructuralSetKind::ApplicationPlugInObject,
            StructuralSetKind::ApplicationReferencedObject,
        ];
        for v in variants {
            let bytes = v.to_bytes();
            assert_eq!(StructuralSetKind::from_bytes(bytes[0], bytes[1]), v);
        }
        assert_eq!(
            StructuralSetKind::from_bytes(0xFE, 0xFD),
            StructuralSetKind::Unknown([0xFE, 0xFD])
        );
    }

    #[test]
    fn local_set_round_trip_two_byte_mode() {
        let key = LocalSet::build_key(StructuralSetKind::Preface, ItemLengthMode::TwoByte);
        let set = LocalSet {
            key,
            items: alloc::vec![
                LocalSetItem {
                    tag: 0x3B02,
                    value: &[1, 2, 3, 4, 5, 6, 7, 8],
                    ..Default::default()
                },
                LocalSetItem {
                    tag: 0x3B05,
                    value: &[0x01, 0x03],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut buf = alloc::vec![0u8; set.serialized_len()];
        set.serialize_into(&mut buf).unwrap();
        let parsed = LocalSet::parse(&buf).unwrap();
        assert_eq!(parsed, set);
        assert_eq!(parsed.kind(), StructuralSetKind::Preface);
        assert_eq!(parsed.get(0x3B02), Some(&[1, 2, 3, 4, 5, 6, 7, 8][..]));
    }

    #[test]
    fn local_set_round_trip_ber_mode() {
        let key = LocalSet::build_key(StructuralSetKind::Identification, ItemLengthMode::Ber);
        let big_value = alloc::vec![0x42u8; 70000];
        let set = LocalSet {
            key,
            items: alloc::vec![LocalSetItem {
                tag: 0x3C01,
                value: &big_value,
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut buf = alloc::vec![0u8; set.serialized_len()];
        set.serialize_into(&mut buf).unwrap();
        let parsed = LocalSet::parse(&buf).unwrap();
        assert_eq!(parsed, set);
        assert_eq!(parsed.item_length_mode(), ItemLengthMode::Ber);
    }

    /// MX-W2 (#1108): §9.3 forbids duplicate local tags within a Set;
    /// pre-fix, the second occurrence was silently accepted and dropped by
    /// every typed Set's `get_*`/`collect_dark` accessors (the first match
    /// wins, and the tag being "known" excluded the extra from `dark` too).
    #[test]
    fn rejects_duplicate_local_tag() {
        let key = LocalSet::build_key(StructuralSetKind::Preface, ItemLengthMode::TwoByte);
        let set = LocalSet {
            key,
            items: alloc::vec![
                LocalSetItem {
                    tag: 0x3C0A,
                    value: &[0xAA; 16],
                    ..Default::default()
                },
                LocalSetItem {
                    tag: 0x3C0A,
                    value: &[0xBB; 16],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut buf = alloc::vec![0u8; set.serialized_len()];
        set.serialize_into(&mut buf).unwrap();
        assert!(matches!(
            LocalSet::parse(&buf),
            Err(Error::DuplicateLocalTag(0x3C0A))
        ));
    }

    #[test]
    fn non_local_set_key_rejected() {
        let bytes = [0u8; 20];
        assert!(matches!(
            LocalSet::parse_prefix(&bytes),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }

    /// MX-W6 (#1108): `serialize_into` used to accept a bad key (invalid
    /// byte 6/registry-designator) by falling back to `TwoByte` mode via
    /// `item_length_mode()`, producing bytes that `is_local_set_key` (and
    /// so `parse`) would then reject — serialize accepted what parse
    /// couldn't read back.
    #[test]
    fn serialize_rejects_bad_key() {
        let mut key = LocalSet::build_key(StructuralSetKind::Preface, ItemLengthMode::TwoByte);
        key[5] = 0xFF; // not a valid registry designator byte
        let set = LocalSet {
            key,
            items: alloc::vec::Vec::new(),
            ..Default::default()
        };
        let mut buf = alloc::vec![0u8; set.serialized_len()];
        assert!(matches!(
            set.serialize_into(&mut buf),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }

    /// Issue #1047 (audit MX-C1): a non-minimal outer-envelope BER length
    /// token must round-trip byte-identically.
    #[test]
    fn non_minimal_outer_length_token_round_trips_byte_identical() {
        let key = LocalSet::build_key(StructuralSetKind::Preface, ItemLengthMode::TwoByte);
        // One item: tag(2) + len(2) + value(4) = 8 bytes, well within
        // short form; force a non-minimal 2-byte long-form token instead.
        let mut original = alloc::vec::Vec::new();
        original.extend_from_slice(&key);
        original.push(0x81);
        original.push(8);
        original.extend_from_slice(&0x3B02u16.to_be_bytes());
        original.extend_from_slice(&4u16.to_be_bytes());
        original.extend_from_slice(&[1, 2, 3, 4]);

        let parsed = LocalSet::parse(&original).unwrap();
        assert_eq!(
            parsed.len_size,
            BerLength::Fixed(core::num::NonZeroU8::new(2).unwrap())
        );

        let mut out = alloc::vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut out).unwrap();
        assert_eq!(out, original);
    }

    /// Same as above, but the non-minimal length is on an ITEM'S own
    /// length token under [`ItemLengthMode::Ber`], not the outer envelope.
    #[test]
    fn non_minimal_item_length_token_round_trips_byte_identical() {
        let key = LocalSet::build_key(StructuralSetKind::Identification, ItemLengthMode::Ber);
        // Outer items_len = tag(2) + item-len-token(2, non-minimal) +
        // value(4) = 8.
        let mut original = alloc::vec::Vec::new();
        original.extend_from_slice(&key);
        original.push(8); // outer: minimal short form
        original.extend_from_slice(&0x3C01u16.to_be_bytes());
        original.push(0x81); // item length: long form, 1 following byte
        original.push(4);
        original.extend_from_slice(&[1, 2, 3, 4]);

        let parsed = LocalSet::parse(&original).unwrap();
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(
            parsed.items[0].len_size,
            BerLength::Fixed(core::num::NonZeroU8::new(2).unwrap())
        );

        let mut out = alloc::vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut out).unwrap();
        assert_eq!(out, original);
    }
}
