//! Content Storage — SMPTE ST 377-1:2019 Annex A.4 (`docs/st377-1.md`):
//! the file's inventory of every Package and Essence Container Data Set.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::local_set::{LocalSet, StructuralSetKind};
use crate::sets::{
    InterchangeObjectFields, LocalSetOwnedItem, collect_dark, finish_owned_set, get_optional_raw,
    get_required_raw, owned_set_serialized_len, serialize_owned_set,
};
use crate::types::{UlBytes, parse_uid_batch, serialize_uid_batch};

/// Local tag: Packages (A.4).
pub const TAG_PACKAGES: u16 = 0x1901;
/// Local tag: Essence Container Data (A.4).
pub const TAG_ESSENCE_CONTAINER_DATA: u16 = 0x1902;

const KNOWN_TAGS: [u16; 5] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_PACKAGES,
    TAG_ESSENCE_CONTAINER_DATA,
];

/// The Content Storage Set — SMPTE ST 377-1:2019 Annex A.4: strong
/// references to every Package (`Packages`) and every Essence Container
/// Data Set (`EssenceContainerData`) in the file, referenced from the
/// Preface's `ContentStorage` property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentStorage {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Packages (`0x1901`, Req) — Batch of strong references to every
    /// Package used in this file.
    pub packages: Vec<UlBytes>,
    /// Essence Container Data (`0x1902`, Opt) — Batch of strong references
    /// to every Essence Container Data Set used in this file.
    pub essence_container_data: Option<Vec<UlBytes>>,
    /// Every other property found on parse (private/dark extension).
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for ContentStorage {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::ContentStorage {
            return Err(Error::KeyPrefixMismatch {
                what: "Content Storage (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Content Storage")?;
        let packages = parse_uid_batch(get_required_raw(
            items,
            TAG_PACKAGES,
            "Packages",
            "Content Storage",
        )?)?;
        let essence_container_data = get_optional_raw(items, TAG_ESSENCE_CONTAINER_DATA)
            .map(parse_uid_batch)
            .transpose()?;
        let dark = collect_dark(items, &KNOWN_TAGS);

        Ok(ContentStorage {
            interchange,
            packages,
            essence_container_data,
            dark,
        })
    }
}

impl ContentStorage {
    fn owned_items(&self) -> Vec<LocalSetOwnedItem> {
        let mut out = Vec::new();
        self.interchange.encode_into(&mut out);
        out.push(LocalSetOwnedItem::owned(
            TAG_PACKAGES,
            serialize_uid_batch(&self.packages),
        ));
        if let Some(ecd) = &self.essence_container_data {
            out.push(LocalSetOwnedItem::owned(
                TAG_ESSENCE_CONTAINER_DATA,
                serialize_uid_batch(ecd),
            ));
        }
        out
    }
}

impl Serialize for ContentStorage {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) = finish_owned_set(
            StructuralSetKind::ContentStorage,
            self.owned_items(),
            &self.dark,
        );
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) = finish_owned_set(
            StructuralSetKind::ContentStorage,
            self.owned_items(),
            &self.dark,
        );
        serialize_owned_set(key, &items, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ContentStorage {
        ContentStorage {
            interchange: InterchangeObjectFields {
                instance_uid: [0x11; 16],
                generation_uid: None,
                object_class: None,
            },
            packages: alloc::vec![[0x22; 16], [0x33; 16]],
            essence_container_data: Some(alloc::vec![[0x44; 16]]),
            dark: Vec::new(),
        }
    }

    #[test]
    fn construct_serialize_parse_round_trip() {
        let cs = sample();
        let bytes = cs.to_bytes();
        let parsed = ContentStorage::parse(&bytes).unwrap();
        assert_eq!(parsed, cs);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn optional_essence_container_data_omitted_round_trips() {
        let mut cs = sample();
        cs.essence_container_data = None;
        let bytes = cs.to_bytes();
        let parsed = ContentStorage::parse(&bytes).unwrap();
        assert_eq!(parsed.essence_container_data, None);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn mutation_changes_serialized_bytes() {
        let mut cs = sample();
        let before = cs.to_bytes();
        cs.packages.push([0x55; 16]);
        let after = cs.to_bytes();
        assert_ne!(before, after);
    }
}
