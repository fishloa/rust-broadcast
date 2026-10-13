#![no_main]

use broadcast_common::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;
use st377_1::{
    ContentStorage, EssenceContainerData, Identification, KlvItem, LocalSet, PartitionPack,
    Preface, PrimerPack, RandomIndexPack, StructuralSetKind, is_fill_item_key, is_local_set_key,
};

fuzz_target!(|data: &[u8]| {
    // SMPTE ST 377-1:2019 KLV framing: parse the leading KLV item, then
    // dispatch by Key to whichever typed parser applies, and byte-identical
    // round-trip each.
    let Ok((item, _consumed)) = KlvItem::parse_prefix(data) else {
        return;
    };
    let full = item.try_to_bytes().unwrap();

    if PartitionPack::is_partition_key(&item.key) {
        if let Ok(pp) = PartitionPack::parse(&full) {
            let reserialized = pp.try_to_bytes().unwrap();
            if let Ok(reparsed) = PartitionPack::parse(&reserialized) {
                assert_eq!(reserialized, reparsed.try_to_bytes().unwrap(), "PartitionPack roundtrip mismatch");
            }
        }
        return;
    }

    if PrimerPack::is_primer_key(&item.key) {
        if let Ok(pp) = PrimerPack::parse(&full) {
            let reserialized = pp.try_to_bytes().unwrap();
            if let Ok(reparsed) = PrimerPack::parse(&reserialized) {
                assert_eq!(reserialized, reparsed.try_to_bytes().unwrap(), "PrimerPack roundtrip mismatch");
            }
        }
        return;
    }

    if RandomIndexPack::is_rip_key(&item.key) {
        if let Ok(rip) = RandomIndexPack::parse(&full) {
            let reserialized = rip.try_to_bytes().unwrap();
            if let Ok(reparsed) = RandomIndexPack::parse(&reserialized) {
                assert_eq!(
                    reserialized,
                    reparsed.try_to_bytes().unwrap(),
                    "RandomIndexPack roundtrip mismatch"
                );
            }
        }
        return;
    }

    if is_fill_item_key(&item.key) {
        return;
    }

    if is_local_set_key(&item.key) {
        let Ok(set) = LocalSet::parse(&full) else {
            return;
        };
        let reserialized = set.try_to_bytes().unwrap();
        if let Ok(reparsed) = LocalSet::parse(&reserialized) {
            assert_eq!(reserialized, reparsed.try_to_bytes().unwrap(), "LocalSet roundtrip mismatch");
        }

        match set.kind() {
            StructuralSetKind::Preface => {
                if let Ok(p) = Preface::parse(&full) {
                    let s = p.try_to_bytes().unwrap();
                    if let Ok(rp) = Preface::parse(&s) {
                        assert_eq!(s, rp.try_to_bytes().unwrap(), "Preface roundtrip mismatch");
                    }
                }
            }
            StructuralSetKind::Identification => {
                if let Ok(p) = Identification::parse(&full) {
                    let s = p.try_to_bytes().unwrap();
                    if let Ok(rp) = Identification::parse(&s) {
                        assert_eq!(s, rp.try_to_bytes().unwrap(), "Identification roundtrip mismatch");
                    }
                }
            }
            StructuralSetKind::ContentStorage => {
                if let Ok(p) = ContentStorage::parse(&full) {
                    let s = p.try_to_bytes().unwrap();
                    if let Ok(rp) = ContentStorage::parse(&s) {
                        assert_eq!(s, rp.try_to_bytes().unwrap(), "ContentStorage roundtrip mismatch");
                    }
                }
            }
            StructuralSetKind::EssenceContainerData => {
                if let Ok(p) = EssenceContainerData::parse(&full) {
                    let s = p.try_to_bytes().unwrap();
                    if let Ok(rp) = EssenceContainerData::parse(&s) {
                        assert_eq!(s, rp.try_to_bytes().unwrap(), "EssenceContainerData roundtrip mismatch");
                    }
                }
            }
            _ => {}
        }
    }
});
