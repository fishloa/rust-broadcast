//! Content Storage — SMPTE ST 377-1:2019 Annex A.4 (`private/specs-verified/smpte/st377-1/st377-1.md`):
//! the file's inventory of every Package and Essence Container Data Set.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::Parse;

use crate::error::{Error, Result};
use crate::local_set::{LocalSet, StructuralSetKind};
use crate::sets::batch_bytes;
use crate::sets::{
    InterchangeObjectFields, LocalSetOwnedItem, collect_dark, get_optional_raw, get_required_raw,
};
use crate::types::{UidBatch, UlBytes};

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
#[derive(Debug, Clone)]
pub struct ContentStorage {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Packages (`0x1901`, Req) — Batch of strong references to every
    /// Package used in this file.
    pub packages: Vec<UlBytes>,
    /// The Packages Batch exactly as parsed (`None` for a freshly built
    /// value), so its header reproduces byte-for-byte on serialize.
    pub packages_batch: Option<UidBatch>,
    /// Essence Container Data (`0x1902`, Opt) — Batch of strong references
    /// to every Essence Container Data Set used in this file.
    pub essence_container_data: Option<Vec<UlBytes>>,
    /// The EssenceContainerData Batch exactly as parsed (`None` when absent
    /// or freshly built).
    pub essence_container_data_batch: Option<UidBatch>,
    /// Every other property found on parse (private/dark extension).
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl PartialEq for ContentStorage {
    fn eq(&self, other: &Self) -> bool {
        self.interchange == other.interchange
            && self.packages == other.packages
            && match (&self.packages_batch, &other.packages_batch) {
                (Some(a), Some(b)) => a.header() == b.header(),
                _ => true,
            }
            && self.essence_container_data == other.essence_container_data
            && match (
                &self.essence_container_data_batch,
                &other.essence_container_data_batch,
            ) {
                (Some(a), Some(b)) => a.header() == b.header(),
                _ => true,
            }
            && self.dark == other.dark
    }
}

impl Eq for ContentStorage {}

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
        let packages_batch = UidBatch::parse(get_required_raw(
            items,
            TAG_PACKAGES,
            "Packages",
            "Content Storage",
        )?)?;
        let packages = packages_batch.items().to_vec();
        let essence_container_data_batch = get_optional_raw(items, TAG_ESSENCE_CONTAINER_DATA)
            .map(UidBatch::parse)
            .transpose()?;
        let essence_container_data = essence_container_data_batch
            .as_ref()
            .map(|b| b.items().to_vec());
        let dark = collect_dark(items, &KNOWN_TAGS);

        Ok(ContentStorage {
            interchange,
            packages,
            packages_batch: Some(packages_batch),
            essence_container_data,
            essence_container_data_batch,
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
            batch_bytes(self.packages_batch.as_ref(), &self.packages),
        ));
        if let Some(ecd) = &self.essence_container_data {
            out.push(LocalSetOwnedItem::owned(
                TAG_ESSENCE_CONTAINER_DATA,
                batch_bytes(self.essence_container_data_batch.as_ref(), ecd),
            ));
        }
        out
    }
}

crate::sets::declare_set_serialize!(ContentStorage, ContentStorage);

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Serialize;

    fn sample() -> ContentStorage {
        ContentStorage {
            interchange: InterchangeObjectFields {
                instance_uid: [0x11; 16],
                generation_uid: None,
                object_class: None,
            },
            packages: alloc::vec![[0x22; 16], [0x33; 16]],
            packages_batch: None,
            essence_container_data: Some(alloc::vec![[0x44; 16]]),
            essence_container_data_batch: None,
            dark: Vec::new(),
        }
    }

    #[test]
    fn construct_serialize_parse_round_trip() {
        let cs = sample();
        let bytes = cs.try_to_bytes().unwrap();
        let parsed = ContentStorage::parse(&bytes).unwrap();
        assert_eq!(parsed, cs);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn optional_essence_container_data_omitted_round_trips() {
        let mut cs = sample();
        cs.essence_container_data = None;
        let bytes = cs.try_to_bytes().unwrap();
        let parsed = ContentStorage::parse(&bytes).unwrap();
        assert_eq!(parsed.essence_container_data, None);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn mutation_changes_serialized_bytes() {
        let mut cs = sample();
        let before = cs.try_to_bytes().unwrap();
        cs.packages.push([0x55; 16]);
        let after = cs.try_to_bytes().unwrap();
        assert_ne!(before, after);
    }
}
