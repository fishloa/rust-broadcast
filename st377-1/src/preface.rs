//! Preface — SMPTE ST 377-1:2019 Annex A.2 (`docs/st377-1.md`): the file's
//! root Header Metadata Set, always the first Set after the Primer Pack
//! (§9.5.1).

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::local_set::{LocalSet, StructuralSetKind};
use crate::sets::{
    InterchangeObjectFields, LocalSetOwnedItem, collect_dark, finish_owned_set, get_optional_fixed,
    get_optional_raw, get_required_fixed, get_required_raw, owned_set_serialized_len,
    serialize_owned_set,
};
use crate::types::{MxfTimestamp, UlBytes, parse_uid_batch, serialize_uid_batch};

/// Local tag: Last Modified Date (A.2).
pub const TAG_LAST_MODIFIED_DATE: u16 = 0x3B02;
/// Local tag: Version (A.2).
pub const TAG_VERSION: u16 = 0x3B05;
/// Local tag: Object Model Version (A.2).
pub const TAG_OBJECT_MODEL_VERSION: u16 = 0x3B07;
/// Local tag: Primary Package (A.2).
pub const TAG_PRIMARY_PACKAGE: u16 = 0x3B08;
/// Local tag: Identifications (A.2).
pub const TAG_IDENTIFICATIONS: u16 = 0x3B06;
/// Local tag: Content Storage (A.2).
pub const TAG_CONTENT_STORAGE: u16 = 0x3B03;
/// Local tag: Operational Pattern (A.2).
pub const TAG_OPERATIONAL_PATTERN: u16 = 0x3B09;
/// Local tag: EssenceContainers (A.2).
pub const TAG_ESSENCE_CONTAINERS: u16 = 0x3B0A;
/// Local tag: DM Schemes (A.2).
pub const TAG_DM_SCHEMES: u16 = 0x3B0B;

const KNOWN_TAGS: [u16; 12] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_LAST_MODIFIED_DATE,
    TAG_VERSION,
    TAG_OBJECT_MODEL_VERSION,
    TAG_PRIMARY_PACKAGE,
    TAG_IDENTIFICATIONS,
    TAG_CONTENT_STORAGE,
    TAG_OPERATIONAL_PATTERN,
    TAG_ESSENCE_CONTAINERS,
    TAG_DM_SCHEMES,
];

/// This revision's fixed `Version` property value (`0x0103` = v1.3, A.2).
pub const VERSION_1_3: u16 = 0x0103;

/// The Preface Set — SMPTE ST 377-1:2019 Annex A.2: the file's overall
/// metadata (modification time, Operational Pattern, Essence Container /
/// Descriptive Metadata scheme inventory) and strong references to the
/// `Identification` history and the `ContentStorage` Set.
///
/// Properties marked "dyn" in Annex A.2 (`ApplicationSchemes`,
/// `IsRIPPresent` — no fixed static local tag; a real Primer Pack must be
/// consulted to learn which tag a given file used) are preserved
/// byte-for-byte in `dark` rather than individually typed in this first
/// pass — see `docs/st377-1.md`'s Scope section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preface {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Last Modified Date (`0x3B02`, Req).
    pub last_modified_date: MxfTimestamp,
    /// Version (`0x3B05`, Req) — `major*256 + minor`; [`VERSION_1_3`] for
    /// this revision.
    pub version: u16,
    /// Object Model Version (`0x3B07`, Opt).
    pub object_model_version: Option<u32>,
    /// Primary Package (`0x3B08`, Opt) — weak reference (Instance UID) to
    /// this file's primary Material Package, if any.
    pub primary_package: Option<UlBytes>,
    /// Identifications (`0x3B06`, **E/req** — encoder-required, but a
    /// decoder must not fail if it's absent, per Annex A.2's "Req?" column)
    /// — strong references to every `Identification` Set recording a
    /// modification to this file. Empty if the property was absent on
    /// parse.
    pub identifications: Vec<UlBytes>,
    /// Content Storage (`0x3B03`, Req) — strong reference to the
    /// `ContentStorage` Set.
    pub content_storage: UlBytes,
    /// Operational Pattern (`0x3B09`, Req) — copy of the Partition Pack's
    /// own value.
    pub operational_pattern: UlBytes,
    /// EssenceContainers (`0x3B0A`, Req) — Batch of Essence Container ULs
    /// used in/referenced by this file.
    pub essence_containers: Vec<UlBytes>,
    /// DM Schemes (`0x3B0B`, Req) — Batch of Descriptive Metadata Scheme
    /// ULs used in this file.
    pub dm_schemes: Vec<UlBytes>,
    /// Every other property found on parse, including any "dyn"-tagged
    /// optional property (`ApplicationSchemes`/`IsRIPPresent`) and any
    /// private/dark extension — preserved byte-for-byte, not decoded.
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for Preface {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::Preface {
            return Err(Error::KeyPrefixMismatch {
                what: "Preface (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Preface")?;
        let last_modified_date = MxfTimestamp::parse(get_required_raw(
            items,
            TAG_LAST_MODIFIED_DATE,
            "Last Modified Date",
            "Preface",
        )?)?;
        let version = u16::from_be_bytes(get_required_fixed::<2>(
            items,
            TAG_VERSION,
            "Version",
            "Preface",
        )?);
        let object_model_version =
            get_optional_fixed::<4>(items, TAG_OBJECT_MODEL_VERSION, "Object Model Version")?
                .map(u32::from_be_bytes);
        let primary_package =
            get_optional_fixed::<16>(items, TAG_PRIMARY_PACKAGE, "Primary Package")?;
        let identifications = match get_optional_raw(items, TAG_IDENTIFICATIONS) {
            Some(raw) => parse_uid_batch(raw)?,
            None => Vec::new(),
        };
        let content_storage =
            get_required_fixed::<16>(items, TAG_CONTENT_STORAGE, "Content Storage", "Preface")?;
        let operational_pattern = get_required_fixed::<16>(
            items,
            TAG_OPERATIONAL_PATTERN,
            "Operational Pattern",
            "Preface",
        )?;
        let essence_containers = parse_uid_batch(get_required_raw(
            items,
            TAG_ESSENCE_CONTAINERS,
            "EssenceContainers",
            "Preface",
        )?)?;
        let dm_schemes = parse_uid_batch(get_required_raw(
            items,
            TAG_DM_SCHEMES,
            "DM Schemes",
            "Preface",
        )?)?;
        let dark = collect_dark(items, &KNOWN_TAGS);

        Ok(Preface {
            interchange,
            last_modified_date,
            version,
            object_model_version,
            primary_package,
            identifications,
            content_storage,
            operational_pattern,
            essence_containers,
            dm_schemes,
            dark,
        })
    }
}

impl Preface {
    fn owned_items(&self) -> Vec<LocalSetOwnedItem> {
        let mut out = Vec::new();
        self.interchange.encode_into(&mut out);
        {
            let mut buf = [0u8; crate::types::TIMESTAMP_LEN];
            self.last_modified_date
                .serialize_into(&mut buf)
                .expect("fixed-size buffer");
            out.push(LocalSetOwnedItem::owned(
                TAG_LAST_MODIFIED_DATE,
                buf.to_vec(),
            ));
        }
        out.push(LocalSetOwnedItem::fixed(
            TAG_VERSION,
            self.version.to_be_bytes(),
        ));
        if let Some(v) = self.object_model_version {
            out.push(LocalSetOwnedItem::fixed(
                TAG_OBJECT_MODEL_VERSION,
                v.to_be_bytes(),
            ));
        }
        if let Some(p) = self.primary_package {
            out.push(LocalSetOwnedItem::fixed(TAG_PRIMARY_PACKAGE, p));
        }
        out.push(LocalSetOwnedItem::owned(
            TAG_IDENTIFICATIONS,
            serialize_uid_batch(&self.identifications),
        ));
        out.push(LocalSetOwnedItem::fixed(
            TAG_CONTENT_STORAGE,
            self.content_storage,
        ));
        out.push(LocalSetOwnedItem::fixed(
            TAG_OPERATIONAL_PATTERN,
            self.operational_pattern,
        ));
        out.push(LocalSetOwnedItem::owned(
            TAG_ESSENCE_CONTAINERS,
            serialize_uid_batch(&self.essence_containers),
        ));
        out.push(LocalSetOwnedItem::owned(
            TAG_DM_SCHEMES,
            serialize_uid_batch(&self.dm_schemes),
        ));
        out
    }
}

impl Serialize for Preface {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) =
            finish_owned_set(StructuralSetKind::Preface, self.owned_items(), &self.dark);
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) =
            finish_owned_set(StructuralSetKind::Preface, self.owned_items(), &self.dark);
        serialize_owned_set(key, &items, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Preface {
        Preface {
            interchange: InterchangeObjectFields {
                instance_uid: [0x11; 16],
                generation_uid: Some([0x22; 16]),
                object_class: None,
            },
            last_modified_date: MxfTimestamp {
                year: 2019,
                month: 11,
                day: 28,
                hour: 10,
                minute: 0,
                second: 0,
                msec_div4: 0,
            },
            version: VERSION_1_3,
            object_model_version: Some(1),
            primary_package: Some([0x33; 16]),
            identifications: alloc::vec![[0x44; 16]],
            content_storage: [0x55; 16],
            operational_pattern: [0x66; 16],
            essence_containers: alloc::vec![[0x77; 16]],
            dm_schemes: Vec::new(),
            dark: Vec::new(),
        }
    }

    #[test]
    fn construct_serialize_parse_round_trip() {
        let preface = sample();
        let mut buf = alloc::vec![0u8; preface.serialized_len()];
        preface.serialize_into(&mut buf).unwrap();
        let parsed = Preface::parse(&buf).unwrap();
        assert_eq!(parsed, preface);

        // Reserialize the parsed value and confirm byte-identical output.
        let mut buf2 = alloc::vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2);
    }

    #[test]
    fn mutation_changes_serialized_bytes() {
        let mut preface = sample();
        let original = preface.to_bytes();
        preface.version = 0x0200;
        let mutated = preface.to_bytes();
        assert_ne!(original, mutated);
        assert_eq!(Preface::parse(&mutated).unwrap().version, 0x0200);
    }

    #[test]
    fn dark_tags_preserved_round_trip() {
        let mut preface = sample();
        preface.dark = alloc::vec![(0x8001, alloc::vec![9, 9, 9])];
        let bytes = preface.to_bytes();
        let parsed = Preface::parse(&bytes).unwrap();
        assert_eq!(parsed.dark, preface.dark);
    }

    #[test]
    fn identifications_absent_tolerated_e_req_not_hard_required() {
        // Annex A.2's "Req?" column marks Identifications "E/req": an
        // encoder must write it, but a decoder must not fail if it's
        // missing. Build a Preface local set with every other required
        // property present but Identifications omitted entirely.
        let mut preface = sample();
        preface.identifications = Vec::new();
        let owned = preface.owned_items();
        let items: Vec<LocalSetOwnedItem> = owned
            .into_iter()
            .filter(|item| item.tag != TAG_IDENTIFICATIONS)
            .collect();
        let (key, encoded) = finish_owned_set(StructuralSetKind::Preface, items, &Vec::new());
        let mut buf = alloc::vec![0u8; owned_set_serialized_len(key, &encoded)];
        serialize_owned_set(key, &encoded, &mut buf).unwrap();

        let parsed = Preface::parse(&buf).expect("absent Identifications must not error");
        assert_eq!(parsed.identifications, Vec::<UlBytes>::new());
    }

    #[test]
    fn wrong_kind_rejected() {
        let key = LocalSet::build_key(
            StructuralSetKind::Identification,
            crate::local_set::ItemLengthMode::TwoByte,
        );
        let set = LocalSet {
            key,
            items: Vec::new(),
        };
        let bytes = set.to_bytes();
        assert!(matches!(
            Preface::parse(&bytes),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }
}
