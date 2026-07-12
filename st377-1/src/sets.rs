//! Shared decode/encode helpers for the four typed Root Metadata Sets
//! (Annex A: [`crate::Preface`], [`crate::Identification`],
//! [`crate::ContentStorage`], [`crate::EssenceContainerData`]).
//!
//! Every typed Set is an **owned** struct (no borrowed lifetime — these are
//! small, KB-scale structures, unlike the essence payload [`crate::KlvItem`]
//! walks zero-copy) built by decoding known local tags out of a parsed
//! [`crate::LocalSet`] into typed fields, and preserving every other tag
//! (including "dyn"-tagged optional properties this first pass does not
//! individually type — see `docs/st377-1.md`'s Scope section) in a `dark`
//! catch-all so nothing is ever silently dropped.

extern crate alloc;

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::local_set::{ItemLengthMode, LocalSet, LocalSetItem, StructuralSetKind};
use crate::types::UlBytes;

/// The two Interchange Object (Annex A.1) properties with a static local
/// tag, common to every Root Metadata Set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InterchangeObjectFields {
    /// Instance UID (`0x3C0A`) — required on every Set.
    pub instance_uid: UlBytes,
    /// Generation UID (`0x0102`) — optional (and never encoded on
    /// `Identification` per A.3's closing note).
    pub generation_uid: Option<UlBytes>,
    /// Object Class (`0x0101`) — optional.
    pub object_class: Option<UlBytes>,
}

/// Local tag: Instance UID (A.1).
pub const TAG_INSTANCE_UID: u16 = 0x3C0A;
/// Local tag: Generation UID (A.1).
pub const TAG_GENERATION_UID: u16 = 0x0102;
/// Local tag: Object Class (A.1).
pub const TAG_OBJECT_CLASS: u16 = 0x0101;

impl InterchangeObjectFields {
    /// Decode from a parsed [`LocalSet`]'s items, collecting any of the
    /// three tags it doesn't recognize is not this function's job — the
    /// caller removes the tags it consumes from its own dark-item pass.
    pub fn decode(items: &[LocalSetItem<'_>], set_name: &'static str) -> Result<Self> {
        let instance_uid =
            get_required_fixed::<16>(items, TAG_INSTANCE_UID, "Instance UID", set_name)?;
        let generation_uid = get_optional_fixed::<16>(items, TAG_GENERATION_UID, "Generation UID")?;
        let object_class = get_optional_fixed::<16>(items, TAG_OBJECT_CLASS, "Object Class")?;
        Ok(Self {
            instance_uid,
            generation_uid,
            object_class,
        })
    }

    /// Emit this Set's Interchange Object items in canonical order.
    pub fn encode_into(&self, out: &mut Vec<LocalSetOwnedItem>) {
        out.push(LocalSetOwnedItem::fixed(
            TAG_INSTANCE_UID,
            self.instance_uid,
        ));
        if let Some(g) = self.generation_uid {
            out.push(LocalSetOwnedItem::fixed(TAG_GENERATION_UID, g));
        }
        if let Some(o) = self.object_class {
            out.push(LocalSetOwnedItem::fixed(TAG_OBJECT_CLASS, o));
        }
    }
}

/// An owned `{tag, value}` pair, used to build a fresh [`LocalSet`] for
/// serialization (the borrowed [`LocalSetItem`] can't hold bytes owned by
/// the very struct being serialized).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSetOwnedItem {
    /// The local tag.
    pub tag: u16,
    /// The value bytes.
    pub value: Vec<u8>,
}

impl LocalSetOwnedItem {
    /// Build an owned item from a fixed-size array value.
    #[must_use]
    pub fn fixed<const N: usize>(tag: u16, value: [u8; N]) -> Self {
        LocalSetOwnedItem {
            tag,
            value: value.to_vec(),
        }
    }

    /// Build an owned item from already-owned bytes.
    #[must_use]
    pub fn owned(tag: u16, value: Vec<u8>) -> Self {
        LocalSetOwnedItem { tag, value }
    }
}

/// Fetch a required property's raw bytes, checked to be exactly `N` bytes.
pub fn get_required_fixed<const N: usize>(
    items: &[LocalSetItem<'_>],
    tag: u16,
    name: &'static str,
    set_name: &'static str,
) -> Result<[u8; N]> {
    let value = items.iter().find(|i| i.tag == tag).map(|i| i.value).ok_or(
        Error::MissingRequiredProperty {
            tag,
            name,
            set: set_name,
        },
    )?;
    <[u8; N]>::try_from(value).map_err(|_| Error::InvalidPropertyLength {
        tag,
        name,
        found: value.len(),
        expected: N,
    })
}

/// Fetch an optional property's raw bytes, checked to be exactly `N` bytes
/// if present.
pub fn get_optional_fixed<const N: usize>(
    items: &[LocalSetItem<'_>],
    tag: u16,
    name: &'static str,
) -> Result<Option<[u8; N]>> {
    match items.iter().find(|i| i.tag == tag) {
        None => Ok(None),
        Some(i) => {
            <[u8; N]>::try_from(i.value)
                .map(Some)
                .map_err(|_| Error::InvalidPropertyLength {
                    tag,
                    name,
                    found: i.value.len(),
                    expected: N,
                })
        }
    }
}

/// Fetch a required property's raw bytes (variable length — e.g. a Batch,
/// Array, or UTF-16 string).
pub fn get_required_raw<'a>(
    items: &[LocalSetItem<'a>],
    tag: u16,
    name: &'static str,
    set_name: &'static str,
) -> Result<&'a [u8]> {
    items
        .iter()
        .find(|i| i.tag == tag)
        .map(|i| i.value)
        .ok_or(Error::MissingRequiredProperty {
            tag,
            name,
            set: set_name,
        })
}

/// Fetch an optional property's raw bytes (variable length), if present.
pub fn get_optional_raw<'a>(items: &[LocalSetItem<'a>], tag: u16) -> Option<&'a [u8]> {
    items.iter().find(|i| i.tag == tag).map(|i| i.value)
}

/// Every item's tag NOT in `known_tags`, copied into an owned dark list
/// (round-trip fidelity for properties this crate doesn't individually
/// decode — including every "dyn"-tagged optional property, see
/// `docs/st377-1.md`'s Scope section).
pub fn collect_dark(items: &[LocalSetItem<'_>], known_tags: &[u16]) -> Vec<(u16, Vec<u8>)> {
    items
        .iter()
        .filter(|i| !known_tags.contains(&i.tag))
        .map(|i| (i.tag, i.value.to_vec()))
        .collect()
}

/// Build the final [`LocalSet`]-shaped byte layout for a typed Set: choose
/// [`ItemLengthMode::TwoByte`] unless any item's value exceeds 65535 bytes
/// (§9.3 — BER local length encoding is required in that case), build the
/// Set Key, and hand back `(key, items)` ready for
/// [`LocalSet::serialize_into`] via a temporary borrowed [`LocalSet`].
pub fn finish_owned_set(
    kind: StructuralSetKind,
    mut owned_items: Vec<LocalSetOwnedItem>,
    dark: &[(u16, Vec<u8>)],
) -> (UlBytes, Vec<LocalSetOwnedItem>) {
    for (tag, value) in dark {
        owned_items.push(LocalSetOwnedItem {
            tag: *tag,
            value: value.clone(),
        });
    }
    let mode = if owned_items.iter().any(|i| i.value.len() > 0xFFFF) {
        ItemLengthMode::Ber
    } else {
        ItemLengthMode::TwoByte
    };
    (LocalSet::build_key(kind, mode), owned_items)
}

/// Serialize `owned_items` under `key` by borrowing each owned item's bytes
/// into a transient [`LocalSet`], then delegating to its `Serialize` impl.
pub fn serialize_owned_set(
    key: UlBytes,
    owned_items: &[LocalSetOwnedItem],
    buf: &mut [u8],
) -> Result<usize> {
    use broadcast_common::Serialize;
    let items = owned_items
        .iter()
        .map(|i| LocalSetItem {
            tag: i.tag,
            value: i.value.as_slice(),
        })
        .collect();
    let set = LocalSet { key, items };
    set.serialize_into(buf)
}

/// Length [`serialize_owned_set`] will need.
#[must_use]
pub fn owned_set_serialized_len(key: UlBytes, owned_items: &[LocalSetOwnedItem]) -> usize {
    use broadcast_common::Serialize;
    let items = owned_items
        .iter()
        .map(|i| LocalSetItem {
            tag: i.tag,
            value: i.value.as_slice(),
        })
        .collect();
    LocalSet { key, items }.serialized_len()
}
