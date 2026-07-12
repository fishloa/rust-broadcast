//! Essence Container Data — SMPTE ST 377-1:2019 Annex A.5
//! (`docs/st377-1.md`): links a Package to the BodySID/IndexSID pair
//! identifying its internal Essence Container / Index Table in the file's
//! Partitions.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::local_set::{LocalSet, StructuralSetKind};
use crate::sets::{
    InterchangeObjectFields, LocalSetOwnedItem, collect_dark, finish_owned_set, get_optional_fixed,
    get_required_fixed, owned_set_serialized_len, serialize_owned_set,
};
use crate::types::PackageId;

/// Local tag: Linked Package UID (A.5).
pub const TAG_LINKED_PACKAGE_UID: u16 = 0x2701;
/// Local tag: IndexSID (A.5).
pub const TAG_INDEX_SID: u16 = 0x3F06;
/// Local tag: BodySID (A.5).
pub const TAG_BODY_SID: u16 = 0x3F07;

const KNOWN_TAGS: [u16; 6] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_LINKED_PACKAGE_UID,
    TAG_INDEX_SID,
    TAG_BODY_SID,
];

/// The Essence Container Data Set — SMPTE ST 377-1:2019 Annex A.5: links a
/// `LinkedPackageUID` to the `BodySID`/`IndexSID` pair identifying its
/// Essence Container / Index Table Segments among the file's Partitions.
///
/// The four Boolean properties A.5 defines with a "dyn" (dynamically
/// allocated, no fixed static tag) local tag — `PrecedingIndexTable`,
/// `SingularPartitionUsage`, `FollowingIndexTable`, `IsSparse` — are
/// preserved byte-for-byte in `dark` rather than individually typed in this
/// first pass, per `docs/st377-1.md`'s Scope section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EssenceContainerData {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Linked Package UID (`0x2701`, Req) — the Package this Set is linked
    /// to (opaque UMID, see `docs/st377-1.md`'s Scope section).
    pub linked_package_uid: PackageId,
    /// IndexSID (`0x3F06`, Opt) — ID of the Index Table for the linked
    /// Essence Container, if any.
    pub index_sid: Option<u32>,
    /// BodySID (`0x3F07`, Req) — ID of the linked Essence Container (`0` =
    /// external to the file).
    pub body_sid: u32,
    /// Every other property found on parse, including the four "dyn"-tagged
    /// Boolean properties above and any private/dark extension — preserved
    /// byte-for-byte, not decoded.
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for EssenceContainerData {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::EssenceContainerData {
            return Err(Error::KeyPrefixMismatch {
                what: "Essence Container Data (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Essence Container Data")?;
        let linked_package_uid = PackageId(get_required_fixed::<32>(
            items,
            TAG_LINKED_PACKAGE_UID,
            "Linked Package UID",
            "Essence Container Data",
        )?);
        let index_sid =
            get_optional_fixed::<4>(items, TAG_INDEX_SID, "IndexSID")?.map(u32::from_be_bytes);
        let body_sid = u32::from_be_bytes(get_required_fixed::<4>(
            items,
            TAG_BODY_SID,
            "BodySID",
            "Essence Container Data",
        )?);
        let dark = collect_dark(items, &KNOWN_TAGS);

        Ok(EssenceContainerData {
            interchange,
            linked_package_uid,
            index_sid,
            body_sid,
            dark,
        })
    }
}

impl EssenceContainerData {
    fn owned_items(&self) -> Vec<LocalSetOwnedItem> {
        let mut out = Vec::new();
        self.interchange.encode_into(&mut out);
        out.push(LocalSetOwnedItem::fixed(
            TAG_LINKED_PACKAGE_UID,
            self.linked_package_uid.0,
        ));
        if let Some(idx) = self.index_sid {
            out.push(LocalSetOwnedItem::fixed(TAG_INDEX_SID, idx.to_be_bytes()));
        }
        out.push(LocalSetOwnedItem::fixed(
            TAG_BODY_SID,
            self.body_sid.to_be_bytes(),
        ));
        out
    }
}

impl Serialize for EssenceContainerData {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) = finish_owned_set(
            StructuralSetKind::EssenceContainerData,
            self.owned_items(),
            &self.dark,
        );
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) = finish_owned_set(
            StructuralSetKind::EssenceContainerData,
            self.owned_items(),
            &self.dark,
        );
        serialize_owned_set(key, &items, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> EssenceContainerData {
        EssenceContainerData {
            interchange: InterchangeObjectFields {
                instance_uid: [0x11; 16],
                generation_uid: None,
                object_class: None,
            },
            linked_package_uid: PackageId([0x22; 32]),
            index_sid: Some(1),
            body_sid: 1,
            dark: Vec::new(),
        }
    }

    #[test]
    fn construct_serialize_parse_round_trip() {
        let ecd = sample();
        let bytes = ecd.to_bytes();
        let parsed = EssenceContainerData::parse(&bytes).unwrap();
        assert_eq!(parsed, ecd);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn null_package_id_round_trips() {
        let mut ecd = sample();
        ecd.linked_package_uid = PackageId::NULL;
        ecd.index_sid = None;
        ecd.body_sid = 0;
        let bytes = ecd.to_bytes();
        let parsed = EssenceContainerData::parse(&bytes).unwrap();
        assert!(parsed.linked_package_uid.is_null());
        assert_eq!(parsed.index_sid, None);
    }

    #[test]
    fn dark_bool_properties_preserved() {
        let mut ecd = sample();
        // Simulate a real file's dynamically-allocated `IsSparse` tag,
        // e.g. resolved via that Partition's own Primer Pack.
        ecd.dark = alloc::vec![(0x8010, alloc::vec![0x01])];
        let bytes = ecd.to_bytes();
        let parsed = EssenceContainerData::parse(&bytes).unwrap();
        assert_eq!(parsed.dark, ecd.dark);
    }
}
