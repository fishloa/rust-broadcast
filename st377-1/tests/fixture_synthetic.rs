//! Parses `tests/fixtures/synthetic_minimal.mxf` — a spec-derived synthetic
//! MXF file (no real captured `.mxf` file exists anywhere in this
//! workspace; see `docs/st377-1.md`'s "Fixture provenance" section for the
//! full justification) assembled by a standalone Python script
//! (independent of this crate's own `Serialize`) directly from SMPTE
//! ST 377-1:2019's own Partition Pack / Primer Pack / Local Set / Annex A
//! tables.
//!
//! This test walks every top-level KLV item in the file, identifies it by
//! its Key, parses it with the corresponding typed `st377-1` type, and
//! asserts on the actual decoded field values — so a bug that made
//! `parse`/`serialize` agree with *each other* but disagree with the spec's
//! own tables would still be caught (unlike a plain in-memory round-trip
//! test alone).

use broadcast_common::{Parse, Serialize};
use st377_1::{
    ContentStorage, EssenceContainerData, Identification, KlvItem, LocalSet, PartitionKind,
    PartitionPack, PartitionStatus, Preface, PrimerPack, RandomIndexPack, StructuralSetKind,
    collect_klv_items, is_fill_item_key,
};

fn fixture_bytes() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/synthetic_minimal.mxf"
    ))
    .expect("read tests/fixtures/synthetic_minimal.mxf")
}

/// Reconstruct the exact KLV bytes for one already-split [`KlvItem`], so a
/// type whose `Parse` impl expects the full Key+Length+Value span (every
/// type in this crate) can be handed exactly the bytes for just that item.
fn klv_bytes(item: &KlvItem<'_>) -> Vec<u8> {
    item.to_bytes()
}

#[test]
fn synthetic_fixture_every_top_level_klv_item_identifies_and_parses() {
    let bytes = fixture_bytes();
    let items = collect_klv_items(&bytes).expect("walk top-level KLV items");

    // Header Partition, Primer Pack, 4 typed Sets, Body Partition, Fill
    // (placeholder essence), Footer Partition, Random Index Pack.
    assert_eq!(items.len(), 10, "unexpected top-level KLV item count");

    let mut seen_kinds = Vec::new();
    for (_, item) in &items {
        let full = klv_bytes(item);
        if PartitionPack::is_partition_key(&item.key) {
            let pp = PartitionPack::parse(&full).expect("parse Partition Pack");
            seen_kinds.push(format!("Partition({})", pp.kind));
        } else if PrimerPack::is_primer_key(&item.key) {
            let _pp = PrimerPack::parse(&full).expect("parse Primer Pack");
            seen_kinds.push("PrimerPack".into());
        } else if RandomIndexPack::is_rip_key(&item.key) {
            let _rip = RandomIndexPack::parse(&full).expect("parse Random Index Pack");
            seen_kinds.push("RandomIndexPack".into());
        } else if is_fill_item_key(&item.key) {
            seen_kinds.push("Fill".into());
        } else if st377_1::is_local_set_key(&item.key) {
            let set = LocalSet::parse(&full).expect("parse Local Set");
            match set.kind() {
                StructuralSetKind::Preface => {
                    Preface::parse(&full).expect("parse Preface");
                }
                StructuralSetKind::Identification => {
                    Identification::parse(&full).expect("parse Identification");
                }
                StructuralSetKind::ContentStorage => {
                    ContentStorage::parse(&full).expect("parse Content Storage");
                }
                StructuralSetKind::EssenceContainerData => {
                    EssenceContainerData::parse(&full).expect("parse Essence Container Data");
                }
                other => panic!("unexpected Set kind in fixture: {other}"),
            }
            seen_kinds.push(format!("{}", set.kind()));
        } else {
            panic!("unidentified top-level KLV key: {:02X?}", item.key);
        }
    }

    assert_eq!(
        seen_kinds,
        vec![
            "Partition(Header Partition)",
            "PrimerPack",
            "Preface",
            "Identification",
            "Content Storage",
            "Essence Container Data",
            "Partition(Body Partition)",
            "Fill",
            "Partition(Footer Partition)",
            "RandomIndexPack",
        ]
    );
}

#[test]
fn synthetic_fixture_header_partition_pack_fields() {
    let bytes = fixture_bytes();
    let (item, _) = KlvItem::parse_prefix(&bytes).unwrap();
    let pack = PartitionPack::parse(&item.to_bytes()).unwrap();

    assert_eq!(pack.kind, PartitionKind::Header);
    assert_eq!(pack.status, PartitionStatus::ClosedComplete);
    assert_eq!(pack.major_version, 1);
    assert_eq!(pack.minor_version, 3);
    assert_eq!(pack.this_partition, 0);
    assert_eq!(pack.body_sid, 0);
    assert!(pack.header_byte_count > 0);
    assert_eq!(pack.essence_containers.len(), 1);
}

#[test]
fn synthetic_fixture_preface_fields_decode_correctly() {
    let bytes = fixture_bytes();
    let items = collect_klv_items(&bytes).unwrap();
    let preface_item = items
        .iter()
        .map(|(_, i)| i)
        .find(|i| {
            st377_1::is_local_set_key(&i.key)
                && LocalSet::parse(&i.to_bytes()).unwrap().kind() == StructuralSetKind::Preface
        })
        .expect("Preface item present");
    let preface = Preface::parse(&preface_item.to_bytes()).unwrap();

    assert_eq!(preface.version, st377_1::VERSION_1_3);
    assert_eq!(preface.object_model_version, Some(1));
    assert_eq!(preface.identifications.len(), 1);
    assert_eq!(preface.essence_containers.len(), 1);
    assert!(preface.dm_schemes.is_empty());
    assert_eq!(preface.last_modified_date.year, 2026);
    assert_eq!(preface.last_modified_date.month, 7);
    assert_eq!(preface.last_modified_date.day, 12);

    // Byte-identical reserialize.
    assert_eq!(preface.to_bytes(), preface_item.to_bytes());
}

#[test]
fn synthetic_fixture_identification_fields_decode_correctly() {
    let bytes = fixture_bytes();
    let items = collect_klv_items(&bytes).unwrap();
    let ident_item = items
        .iter()
        .map(|(_, i)| i)
        .find(|i| {
            st377_1::is_local_set_key(&i.key)
                && LocalSet::parse(&i.to_bytes()).unwrap().kind()
                    == StructuralSetKind::Identification
        })
        .expect("Identification item present");
    let ident = Identification::parse(&ident_item.to_bytes()).unwrap();

    assert_eq!(ident.company_name, "Acme Broadcast Tools");
    assert_eq!(ident.product_name, "st377-1 fixture builder");
    assert_eq!(ident.version_string, "0.1.0-fixture");
    assert_eq!(ident.platform.as_deref(), Some("python3 (builder script)"));
    let pv = ident.product_version.expect("Product Version present");
    assert_eq!((pv.major, pv.minor, pv.tertiary, pv.patch), (0, 1, 0, 0));

    assert_eq!(ident.to_bytes(), ident_item.to_bytes());
}

#[test]
fn synthetic_fixture_essence_container_data_fields() {
    let bytes = fixture_bytes();
    let items = collect_klv_items(&bytes).unwrap();
    let ecd_item = items
        .iter()
        .map(|(_, i)| i)
        .find(|i| {
            st377_1::is_local_set_key(&i.key)
                && LocalSet::parse(&i.to_bytes()).unwrap().kind()
                    == StructuralSetKind::EssenceContainerData
        })
        .expect("Essence Container Data item present");
    let ecd = EssenceContainerData::parse(&ecd_item.to_bytes()).unwrap();

    assert_eq!(ecd.body_sid, 1);
    assert_eq!(ecd.index_sid, Some(0));
    assert!(!ecd.linked_package_uid.is_null());
    assert_eq!(ecd.to_bytes(), ecd_item.to_bytes());
}

#[test]
fn synthetic_fixture_content_storage_fields() {
    let bytes = fixture_bytes();
    let items = collect_klv_items(&bytes).unwrap();
    let cs_item = items
        .iter()
        .map(|(_, i)| i)
        .find(|i| {
            st377_1::is_local_set_key(&i.key)
                && LocalSet::parse(&i.to_bytes()).unwrap().kind()
                    == StructuralSetKind::ContentStorage
        })
        .expect("Content Storage item present");
    let cs = ContentStorage::parse(&cs_item.to_bytes()).unwrap();

    assert_eq!(cs.packages.len(), 1);
    assert_eq!(cs.essence_container_data.as_ref().map(Vec::len), Some(1));
    assert_eq!(cs.to_bytes(), cs_item.to_bytes());
}

#[test]
fn synthetic_fixture_random_index_pack_lists_every_partition() {
    let bytes = fixture_bytes();
    let items = collect_klv_items(&bytes).unwrap();
    let (_, rip_item) = items
        .iter()
        .find(|(_, i)| RandomIndexPack::is_rip_key(&i.key))
        .expect("Random Index Pack present");
    let rip = RandomIndexPack::parse(&rip_item.to_bytes()).unwrap();

    assert_eq!(rip.partitions.len(), 3);
    assert_eq!(rip.partitions[0].byte_offset, 0);
    assert_eq!(rip.partitions[1].body_sid, 1);
}
