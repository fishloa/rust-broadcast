//! Material Package and Source Package — SMPTE ST 377-1:2019 Annex B §B.1/
//! E.1-E.4 (`docs/st377-1.md`): the two concrete Package kinds every MXF
//! file uses.
//!
//! Both inherit the Generic Package properties (B.1): Package UID, Name,
//! Creation Date, Modified Date, and a Tracks batch.  `SourcePackage` adds a
//! `Descriptor` strong reference (§E.2).
//!
//! `MaterialPackage` — byte 14/15 = `0x01`/`0x36`.
//! `SourcePackage`   — byte 14/15 = `0x01`/`0x37`.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::local_set::{LocalSet, StructuralSetKind};
use crate::sets::{
    InterchangeObjectFields, LocalSetOwnedItem, collect_dark, finish_owned_set, get_optional_raw,
    get_required_fixed, get_required_raw, owned_set_serialized_len, serialize_owned_set,
};
use crate::types::{
    MxfTimestamp, PackageId, StrongRef, TIMESTAMP_LEN, decode_utf16_be, encode_utf16_be,
    parse_uid_batch, serialize_uid_batch,
};

// ── Generic Package local tags (B.1) ────────────────────────────────────

/// Local tag: Package UID (B.1) — 32-byte UMID.
pub const TAG_PACKAGE_UID: u16 = 0x4401;
/// Local tag: Name (B.1) — UTF-16 string, optional.
pub const TAG_NAME: u16 = 0x4402;
/// Local tag: Tracks (B.1) — Array of StrongRef.
pub const TAG_TRACKS: u16 = 0x4403;
/// Local tag: Package Modified Date (B.1).
pub const TAG_MODIFIED_DATE: u16 = 0x4404;
/// Local tag: Package Creation Date (B.1).
pub const TAG_CREATION_DATE: u16 = 0x4405;

// ── Source Package additional tag (E.2) ─────────────────────────────────

/// Local tag: Descriptor (E.2) — StrongRef to the top-level Descriptor.
pub const TAG_DESCRIPTOR: u16 = 0x4701;

// ── Known-tag tables ────────────────────────────────────────────────────

const MATERIAL_KNOWN_TAGS: [u16; 8] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_PACKAGE_UID,
    TAG_NAME,
    TAG_TRACKS,
    TAG_MODIFIED_DATE,
    TAG_CREATION_DATE,
];

const SOURCE_KNOWN_TAGS: [u16; 9] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_PACKAGE_UID,
    TAG_NAME,
    TAG_TRACKS,
    TAG_MODIFIED_DATE,
    TAG_CREATION_DATE,
    TAG_DESCRIPTOR,
];

// ═══════════════════════════════════════════════════════════════════════
// MaterialPackage
// ═══════════════════════════════════════════════════════════════════════

/// The Material Package Set — SMPTE ST 377-1:2019 Annex E §E.1 (byte
/// 14/15 = `0x01`/`0x36`): the top-level composition that describes the
/// final timeline of the file.  Carries only the Generic Package
/// properties (B.1) — no additional fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialPackage {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Package UID (`0x4401`, Req) — 32-byte UMID.
    pub package_uid: PackageId,
    /// Name (`0x4402`, Opt) — human-readable package name.
    pub name: Option<String>,
    /// Package Creation Date (`0x4405`, Req).
    pub creation_date: MxfTimestamp,
    /// Package Modified Date (`0x4404`, Req).
    pub modified_date: MxfTimestamp,
    /// Tracks (`0x4403`, Req) — strong references to this package's Tracks.
    pub tracks: Vec<StrongRef>,
    /// Unrecognized properties preserved for round-trip fidelity.
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for MaterialPackage {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::MaterialPackage {
            return Err(Error::KeyPrefixMismatch {
                what: "Material Package (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Material Package")?;
        let package_uid = PackageId(get_required_fixed::<32>(
            items,
            TAG_PACKAGE_UID,
            "Package UID",
            "Material Package",
        )?);
        let name = get_optional_raw(items, TAG_NAME)
            .map(decode_utf16_be)
            .transpose()
            .map_err(|_| Error::InvalidUtf16 {
                tag: TAG_NAME,
                name: "Name",
            })?;
        let creation_date = MxfTimestamp::parse(get_required_raw(
            items,
            TAG_CREATION_DATE,
            "Package Creation Date",
            "Material Package",
        )?)?;
        let modified_date = MxfTimestamp::parse(get_required_raw(
            items,
            TAG_MODIFIED_DATE,
            "Package Modified Date",
            "Material Package",
        )?)?;
        let tracks = parse_uid_batch(get_required_raw(
            items,
            TAG_TRACKS,
            "Tracks",
            "Material Package",
        )?)?;
        let dark = collect_dark(items, &MATERIAL_KNOWN_TAGS);

        Ok(MaterialPackage {
            interchange,
            package_uid,
            name,
            creation_date,
            modified_date,
            tracks,
            dark,
        })
    }
}

impl MaterialPackage {
    fn owned_items(&self) -> Vec<LocalSetOwnedItem> {
        let mut out = Vec::new();
        self.interchange.encode_into(&mut out);
        out.push(LocalSetOwnedItem::fixed(
            TAG_PACKAGE_UID,
            self.package_uid.0,
        ));
        if let Some(n) = &self.name {
            out.push(LocalSetOwnedItem::owned(TAG_NAME, encode_utf16_be(n)));
        }
        encode_timestamp_item(&mut out, TAG_CREATION_DATE, &self.creation_date);
        encode_timestamp_item(&mut out, TAG_MODIFIED_DATE, &self.modified_date);
        out.push(LocalSetOwnedItem::owned(
            TAG_TRACKS,
            serialize_uid_batch(&self.tracks),
        ));
        out
    }
}

impl Serialize for MaterialPackage {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) = finish_owned_set(
            StructuralSetKind::MaterialPackage,
            self.owned_items(),
            &self.dark,
        );
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) = finish_owned_set(
            StructuralSetKind::MaterialPackage,
            self.owned_items(),
            &self.dark,
        );
        serialize_owned_set(key, &items, buf)
    }
}

// ═══════════════════════════════════════════════════════════════════════
// SourcePackage
// ═══════════════════════════════════════════════════════════════════════

/// The Source Package Set — SMPTE ST 377-1:2019 Annex E §E.2 (byte
/// 14/15 = `0x01`/`0x37`): references the file's actual essence via a
/// `Descriptor` strong reference, plus the Generic Package properties
/// (B.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePackage {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Package UID (`0x4401`, Req) — 32-byte UMID.
    pub package_uid: PackageId,
    /// Name (`0x4402`, Opt) — human-readable package name.
    pub name: Option<String>,
    /// Package Creation Date (`0x4405`, Req).
    pub creation_date: MxfTimestamp,
    /// Package Modified Date (`0x4404`, Req).
    pub modified_date: MxfTimestamp,
    /// Tracks (`0x4403`, Req) — strong references to this package's Tracks.
    pub tracks: Vec<StrongRef>,
    /// Descriptor (`0x4701`, Req) — strong reference to the top-level
    /// Descriptor (or Multiple Descriptor) for this package's essence.
    ///
    /// This crate has no typed `EssenceDescriptor` (see the crate root
    /// docs' "OP1a support is structural-metadata-only" section): this is
    /// the raw 16-byte Instance UID, opaque here, not a value this crate
    /// can dereference to the target Descriptor Set.
    pub descriptor: StrongRef,
    /// Unrecognized properties preserved for round-trip fidelity.
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for SourcePackage {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::SourcePackage {
            return Err(Error::KeyPrefixMismatch {
                what: "Source Package (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Source Package")?;
        let package_uid = PackageId(get_required_fixed::<32>(
            items,
            TAG_PACKAGE_UID,
            "Package UID",
            "Source Package",
        )?);
        let name = get_optional_raw(items, TAG_NAME)
            .map(decode_utf16_be)
            .transpose()
            .map_err(|_| Error::InvalidUtf16 {
                tag: TAG_NAME,
                name: "Name",
            })?;
        let creation_date = MxfTimestamp::parse(get_required_raw(
            items,
            TAG_CREATION_DATE,
            "Package Creation Date",
            "Source Package",
        )?)?;
        let modified_date = MxfTimestamp::parse(get_required_raw(
            items,
            TAG_MODIFIED_DATE,
            "Package Modified Date",
            "Source Package",
        )?)?;
        let tracks = parse_uid_batch(get_required_raw(
            items,
            TAG_TRACKS,
            "Tracks",
            "Source Package",
        )?)?;
        let descriptor =
            get_required_fixed::<16>(items, TAG_DESCRIPTOR, "Descriptor", "Source Package")?;
        let dark = collect_dark(items, &SOURCE_KNOWN_TAGS);

        Ok(SourcePackage {
            interchange,
            package_uid,
            name,
            creation_date,
            modified_date,
            tracks,
            descriptor,
            dark,
        })
    }
}

impl SourcePackage {
    fn owned_items(&self) -> Vec<LocalSetOwnedItem> {
        let mut out = Vec::new();
        self.interchange.encode_into(&mut out);
        out.push(LocalSetOwnedItem::fixed(
            TAG_PACKAGE_UID,
            self.package_uid.0,
        ));
        if let Some(n) = &self.name {
            out.push(LocalSetOwnedItem::owned(TAG_NAME, encode_utf16_be(n)));
        }
        encode_timestamp_item(&mut out, TAG_CREATION_DATE, &self.creation_date);
        encode_timestamp_item(&mut out, TAG_MODIFIED_DATE, &self.modified_date);
        out.push(LocalSetOwnedItem::owned(
            TAG_TRACKS,
            serialize_uid_batch(&self.tracks),
        ));
        out.push(LocalSetOwnedItem::fixed(TAG_DESCRIPTOR, self.descriptor));
        out
    }
}

impl Serialize for SourcePackage {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) = finish_owned_set(
            StructuralSetKind::SourcePackage,
            self.owned_items(),
            &self.dark,
        );
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) = finish_owned_set(
            StructuralSetKind::SourcePackage,
            self.owned_items(),
            &self.dark,
        );
        serialize_owned_set(key, &items, buf)
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────

/// Encode a [`MxfTimestamp`] into an owned local-set item.
fn encode_timestamp_item(out: &mut Vec<LocalSetOwnedItem>, tag: u16, ts: &MxfTimestamp) {
    let mut buf = [0u8; TIMESTAMP_LEN];
    ts.serialize_into(&mut buf).expect("fixed-size buffer");
    out.push(LocalSetOwnedItem::owned(tag, buf.to_vec()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_timestamp() -> MxfTimestamp {
        MxfTimestamp {
            year: 2026,
            month: 7,
            day: 12,
            hour: 10,
            minute: 0,
            second: 0,
            msec_div4: 0,
        }
    }

    fn sample_material_package() -> MaterialPackage {
        MaterialPackage {
            interchange: InterchangeObjectFields {
                instance_uid: [0x11; 16],
                generation_uid: Some([0x22; 16]),
                object_class: None,
            },
            package_uid: PackageId([0x33; 32]),
            name: Some(String::from("Main Timeline")),
            creation_date: sample_timestamp(),
            modified_date: sample_timestamp(),
            tracks: alloc::vec![[0x44; 16], [0x55; 16]],
            dark: Vec::new(),
        }
    }

    fn sample_source_package() -> SourcePackage {
        SourcePackage {
            interchange: InterchangeObjectFields {
                instance_uid: [0xAA; 16],
                generation_uid: None,
                object_class: None,
            },
            package_uid: PackageId([0xBB; 32]),
            name: None,
            creation_date: sample_timestamp(),
            modified_date: sample_timestamp(),
            tracks: alloc::vec![[0xCC; 16]],
            descriptor: [0xDD; 16],
            dark: Vec::new(),
        }
    }

    #[test]
    fn material_package_round_trip() {
        let mp = sample_material_package();
        let bytes = mp.to_bytes();
        let parsed = MaterialPackage::parse(&bytes).unwrap();
        assert_eq!(parsed, mp);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn material_package_no_name_round_trip() {
        let mut mp = sample_material_package();
        mp.name = None;
        let bytes = mp.to_bytes();
        let parsed = MaterialPackage::parse(&bytes).unwrap();
        assert_eq!(parsed.name, None);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn material_package_dark_preserved() {
        let mut mp = sample_material_package();
        mp.dark = alloc::vec![(0x9001, alloc::vec![1, 2, 3])];
        let bytes = mp.to_bytes();
        let parsed = MaterialPackage::parse(&bytes).unwrap();
        assert_eq!(parsed.dark, mp.dark);
    }

    #[test]
    fn material_package_wrong_kind_rejected() {
        let sp = sample_source_package();
        let bytes = sp.to_bytes();
        assert!(matches!(
            MaterialPackage::parse(&bytes),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }

    #[test]
    fn source_package_round_trip() {
        let sp = sample_source_package();
        let bytes = sp.to_bytes();
        let parsed = SourcePackage::parse(&bytes).unwrap();
        assert_eq!(parsed, sp);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn source_package_with_name_round_trip() {
        let mut sp = sample_source_package();
        sp.name = Some(String::from("File Source"));
        let bytes = sp.to_bytes();
        let parsed = SourcePackage::parse(&bytes).unwrap();
        assert_eq!(parsed.name.as_deref(), Some("File Source"));
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn source_package_wrong_kind_rejected() {
        let mp = sample_material_package();
        let bytes = mp.to_bytes();
        assert!(matches!(
            SourcePackage::parse(&bytes),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }

    #[test]
    fn mutation_changes_serialized_bytes() {
        let mut mp = sample_material_package();
        let before = mp.to_bytes();
        mp.tracks.push([0x66; 16]);
        let after = mp.to_bytes();
        assert_ne!(before, after);
        assert_eq!(MaterialPackage::parse(&after).unwrap().tracks.len(), 3);
    }
}
