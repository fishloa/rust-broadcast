//! Equality sensitivity for every hand-written `PartialEq` in this crate
//! (audit 23 #3214). Each impl compares a conjunction of fields, so an
//! `eq -> true` or an `&& -> ||` replacement is invisible unless a test
//! asserts two values differing in exactly one field are unequal.

use broadcast_common::Parse;
use st377_1::{
    BerLength, ContentStorage, KlvItem, LocalSet, LocalTagEntry, MaterialPackage, MxfTimestamp,
    PackageId, PartitionLocation, PartitionPack, Preface, PrimerPack, RandomIndexPack, Sequence,
    SourcePackage, StrongRef, StructuralSetKind, UidBatch, UlBytes, collect_klv_items,
};

fn fixture_kvls() -> Vec<Vec<u8>> {
    let bytes = test_fixtures::read("mxf/smpte-st377/op1a-mpeg2-pcm.mxf");
    let items = collect_klv_items(&bytes).expect("collect KLV items");
    let mut out = Vec::new();
    for (offset, _) in items {
        let (_, consumed) = KlvItem::parse_prefix(&bytes[offset..]).expect("KLV prefix");
        out.push(bytes[offset..offset + consumed].to_vec());
    }
    out
}

fn first_local(kind: StructuralSetKind) -> Vec<u8> {
    for klv in fixture_kvls() {
        if let Ok(set) = LocalSet::parse(&klv) {
            if set.kind() == kind {
                return klv;
            }
        }
    }
    panic!("fixture has no {kind:?} set");
}

fn first_partition() -> Vec<u8> {
    for klv in fixture_kvls() {
        if PartitionPack::parse(&klv).is_ok() {
            return klv;
        }
    }
    panic!("fixture has no Partition Pack");
}

fn first_primer() -> Vec<u8> {
    for klv in fixture_kvls() {
        if PrimerPack::parse(&klv).is_ok() {
            return klv;
        }
    }
    panic!("fixture has no Primer Pack");
}

fn first_rip() -> Vec<u8> {
    for klv in fixture_kvls() {
        if RandomIndexPack::parse(&klv).is_ok() {
            return klv;
        }
    }
    panic!("fixture has no Random Index Pack");
}

fn ul(byte: u8) -> UlBytes {
    UlBytes([byte; 16])
}

fn different_ul(value: &UlBytes) -> UlBytes {
    let mut bytes = *value;
    bytes.0[0] ^= 0xFF;
    bytes
}

fn batch_with_other_header(batch: &UidBatch) -> UidBatch {
    let mut header = batch.header().unwrap_or([0u8; 8]);
    header[0] ^= 0xFF;
    UidBatch::with_header(batch.items().to_vec(), Some(header))
}

fn assert_other_interchange(value: &mut st377_1::InterchangeObjectFields) {
    value.instance_uid.0[0] ^= 0xFF;
}

fn assert_other_timestamp(value: &mut MxfTimestamp) {
    value.year = value.year.wrapping_add(1);
}

#[test]
fn content_storage_equality_is_field_sensitive() {
    let bytes = first_local(StructuralSetKind::ContentStorage);
    let a = ContentStorage::parse(&bytes).unwrap();
    assert_eq!(a, a.clone());

    let mut b = a.clone();
    assert_other_interchange(&mut b.interchange);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.packages.push(ul(0x55));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.packages_batch = Some(batch_with_other_header(
        a.packages_batch.as_ref().expect("parsed Batch"),
    ));
    assert_ne!(a, b);

    let mut b = a.clone();
    match b.essence_container_data.as_mut() {
        Some(items) => items.push(ul(0x66)),
        None => b.essence_container_data = Some(vec![ul(0x66)]),
    }
    assert_ne!(a, b);

    let mut b = a.clone();
    match &a.essence_container_data_batch {
        Some(batch) => {
            b.essence_container_data_batch = Some(batch_with_other_header(batch));
            assert_ne!(a, b);
        }
        None => {
            b.essence_container_data_batch = Some(UidBatch::with_header(Vec::new(), Some([9; 8])));
            assert_eq!(
                a, b,
                "a built (headerless) Batch equals a parsed one with the same items"
            );
        }
    }

    let mut b = a.clone();
    b.dark.push((0xFFFE, vec![1]));
    assert_ne!(a, b);
}

#[test]
fn material_package_equality_is_field_sensitive() {
    let bytes = first_local(StructuralSetKind::MaterialPackage);
    let a = MaterialPackage::parse(&bytes).unwrap();
    assert_eq!(a, a.clone());

    let mut b = a.clone();
    assert_other_interchange(&mut b.interchange);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.package_uid.0[0] ^= 0xFF;
    assert_ne!(a, b);

    let mut b = a.clone();
    b.name = match &a.name {
        Some(_) => None,
        None => Some(String::from("mutated")),
    };
    assert_ne!(a, b);

    let mut b = a.clone();
    assert_other_timestamp(&mut b.creation_date);
    assert_ne!(a, b);

    let mut b = a.clone();
    assert_other_timestamp(&mut b.modified_date);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.tracks.push(StrongRef([0x5A; 16]));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.tracks_batch = Some(batch_with_other_header(
        a.tracks_batch.as_ref().expect("parsed Batch"),
    ));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.dark.push((0xFFFE, vec![1]));
    assert_ne!(a, b);
}

#[test]
fn source_package_equality_is_field_sensitive() {
    let bytes = first_local(StructuralSetKind::SourcePackage);
    let a = SourcePackage::parse(&bytes).unwrap();
    assert_eq!(a, a.clone());

    let mut b = a.clone();
    assert_other_interchange(&mut b.interchange);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.package_uid = PackageId([0x5A; 32]);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.name = match &a.name {
        Some(_) => None,
        None => Some(String::from("mutated")),
    };
    assert_ne!(a, b);

    let mut b = a.clone();
    assert_other_timestamp(&mut b.creation_date);
    assert_ne!(a, b);

    let mut b = a.clone();
    assert_other_timestamp(&mut b.modified_date);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.tracks.push(StrongRef([0x5A; 16]));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.tracks_batch = Some(batch_with_other_header(
        a.tracks_batch.as_ref().expect("parsed Batch"),
    ));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.descriptor.0[0] ^= 0xFF;
    assert_ne!(a, b);

    let mut b = a.clone();
    b.dark.push((0xFFFE, vec![1]));
    assert_ne!(a, b);
}

#[test]
fn partition_pack_equality_is_field_sensitive() {
    let bytes = first_partition();
    let a = PartitionPack::parse(&bytes).unwrap();
    assert_eq!(a, a.clone());

    let mut b = a.clone();
    b.kind = match a.kind {
        st377_1::PartitionKind::Header => st377_1::PartitionKind::Body,
        _ => st377_1::PartitionKind::Header,
    };
    assert_ne!(a, b);

    let mut b = a.clone();
    b.status = match a.status {
        st377_1::PartitionStatus::OpenIncomplete => st377_1::PartitionStatus::OpenComplete,
        _ => st377_1::PartitionStatus::OpenIncomplete,
    };
    assert_ne!(a, b);

    let mut b = a.clone();
    b.major_version = a.major_version.wrapping_add(1);
    assert_ne!(a, b);
    let mut b = a.clone();
    b.minor_version = a.minor_version.wrapping_add(1);
    assert_ne!(a, b);
    let mut b = a.clone();
    b.kag_size = a.kag_size.wrapping_add(1);
    assert_ne!(a, b);
    let mut b = a.clone();
    b.this_partition = a.this_partition.wrapping_add(1);
    assert_ne!(a, b);
    let mut b = a.clone();
    b.previous_partition = a.previous_partition.wrapping_add(1);
    assert_ne!(a, b);
    let mut b = a.clone();
    b.footer_partition = a.footer_partition.wrapping_add(1);
    assert_ne!(a, b);
    let mut b = a.clone();
    b.header_byte_count = a.header_byte_count.wrapping_add(1);
    assert_ne!(a, b);
    let mut b = a.clone();
    b.index_byte_count = a.index_byte_count.wrapping_add(1);
    assert_ne!(a, b);
    let mut b = a.clone();
    b.index_sid = a.index_sid.wrapping_add(1);
    assert_ne!(a, b);
    let mut b = a.clone();
    b.body_offset = a.body_offset.wrapping_add(1);
    assert_ne!(a, b);
    let mut b = a.clone();
    b.body_sid = a.body_sid.wrapping_add(1);
    assert_ne!(a, b);
    let mut b = a.clone();
    b.operational_pattern = different_ul(&a.operational_pattern);
    assert_ne!(a, b);
    let mut b = a.clone();
    b.essence_containers.push(ul(0x5A));
    assert_ne!(a, b);
    let mut b = a.clone();
    b.essence_containers_batch = Some(batch_with_other_header(
        a.essence_containers_batch.as_ref().expect("parsed Batch"),
    ));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.len_size = BerLength::Minimal;
    assert_eq!(
        a, b,
        "len_size is a serialization-form preference, not part of the value"
    );
}

#[test]
fn preface_equality_is_field_sensitive() {
    let bytes = first_local(StructuralSetKind::Preface);
    let a = Preface::parse(&bytes).unwrap();
    assert_eq!(a, a.clone());

    let mut b = a.clone();
    assert_other_interchange(&mut b.interchange);
    assert_ne!(a, b);

    let mut b = a.clone();
    assert_other_timestamp(&mut b.last_modified_date);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.version = a.version.wrapping_add(1);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.object_model_version = match a.object_model_version {
        Some(_) => None,
        None => Some(0),
    };
    assert_ne!(a, b);

    let mut b = a.clone();
    b.primary_package = match a.primary_package {
        Some(_) => None,
        None => Some(ul(0x5A)),
    };
    assert_ne!(a, b);

    let mut b = a.clone();
    match b.identifications.as_mut() {
        Some(items) => items.push(ul(0x5A)),
        None => b.identifications = Some(vec![ul(0x5A)]),
    }
    assert_ne!(a, b);

    let mut b = a.clone();
    b.identifications_batch = Some(batch_with_other_header(
        a.identifications_batch.as_ref().expect("parsed Batch"),
    ));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.content_storage = different_ul(&a.content_storage);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.operational_pattern = different_ul(&a.operational_pattern);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.essence_containers.push(ul(0x5A));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.essence_containers_batch = Some(batch_with_other_header(
        a.essence_containers_batch.as_ref().expect("parsed Batch"),
    ));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.dm_schemes.push(ul(0x5A));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.dm_schemes_batch = Some(batch_with_other_header(
        a.dm_schemes_batch.as_ref().expect("parsed Batch"),
    ));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.dark.push((0xFFFE, vec![1]));
    assert_ne!(a, b);
}

#[test]
fn primer_pack_equality_is_field_sensitive() {
    let bytes = first_primer();
    let a = PrimerPack::parse(&bytes).unwrap();
    assert_eq!(a, a.clone());
    assert!(!a.entries.is_empty(), "fixture Primer must have entries");

    let mut b = a.clone();
    b.entries[0].tag ^= 0xFFFF;
    assert_ne!(a, b);

    let mut b = a.clone();
    b.entries[0].uid = different_ul(&a.entries[0].uid);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.entries.push(LocalTagEntry {
        tag: 0xFFFE,
        uid: ul(0x5A),
    });
    assert_ne!(a, b);

    let mut b = a.clone();
    b.len_size = BerLength::Minimal;
    assert_eq!(
        a, b,
        "len_size is a serialization-form preference, not part of the value"
    );
}

#[test]
fn random_index_pack_equality_is_field_sensitive() {
    let bytes = first_rip();
    let a = RandomIndexPack::parse(&bytes).unwrap();
    assert_eq!(a, a.clone());
    assert!(!a.partitions.is_empty(), "fixture RIP must have pairs");

    let mut b = a.clone();
    b.partitions[0].body_sid = a.partitions[0].body_sid.wrapping_add(1);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.partitions[0].byte_offset = a.partitions[0].byte_offset.wrapping_add(1);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.partitions.push(PartitionLocation {
        body_sid: 0,
        byte_offset: 0,
    });
    assert_ne!(a, b);
}

#[test]
fn sequence_equality_is_field_sensitive() {
    let bytes = first_local(StructuralSetKind::Sequence);
    let a = Sequence::parse(&bytes).unwrap();
    assert_eq!(a, a.clone());

    let mut b = a.clone();
    assert_other_interchange(&mut b.interchange);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.data_definition = different_ul(&a.data_definition);
    assert_ne!(a, b);

    let mut b = a.clone();
    b.duration = match a.duration {
        Some(_) => None,
        None => Some(0),
    };
    assert_ne!(a, b);

    let mut b = a.clone();
    b.structural_components.push(StrongRef([0x5A; 16]));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.structural_components_batch = Some(batch_with_other_header(
        a.structural_components_batch
            .as_ref()
            .expect("parsed Batch"),
    ));
    assert_ne!(a, b);

    let mut b = a.clone();
    b.dark.push((0xFFFE, vec![1]));
    assert_ne!(a, b);
}

#[test]
fn local_set_and_item_equality_are_field_sensitive() {
    let bytes = first_local(StructuralSetKind::Preface);
    let a = LocalSet::parse(&bytes).unwrap();
    assert_eq!(a, a.clone());
    assert!(!a.items.is_empty(), "fixture Preface must have items");

    let item = a.items[0];
    let mut m = item;
    m.tag ^= 0xFFFF;
    assert_ne!(item, m);
    let mut m = item;
    m.value = &[0xFF];
    assert_ne!(item, m);
    let mut m = item;
    m.len_size = BerLength::Minimal;
    assert_eq!(item, m, "len_size is a serialization-form preference");

    let mut b = a.clone();
    b.key.0[0] ^= 0xFF;
    assert_ne!(a, b);

    let mut b = a.clone();
    b.items[0].tag ^= 0xFFFF;
    assert_ne!(a, b);

    let mut b = a.clone();
    b.items.push(st377_1::LocalSetItem {
        tag: 0xFFFE,
        value: &[1],
        len_size: BerLength::default(),
    });
    assert_ne!(a, b);
}
