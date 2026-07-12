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
    let full = item.to_bytes();

    if PartitionPack::is_partition_key(&item.key) {
        if let Ok(pp) = PartitionPack::parse(&full) {
            let reserialized = pp.to_bytes();
            if let Ok(reparsed) = PartitionPack::parse(&reserialized) {
                assert_eq!(reserialized, reparsed.to_bytes(), "PartitionPack roundtrip mismatch");
            }
        }
        return;
    }

    if PrimerPack::is_primer_key(&item.key) {
        if let Ok(pp) = PrimerPack::parse(&full) {
            let reserialized = pp.to_bytes();
            if let Ok(reparsed) = PrimerPack::parse(&reserialized) {
                assert_eq!(reserialized, reparsed.to_bytes(), "PrimerPack roundtrip mismatch");
            }
        }
        return;
    }

    if RandomIndexPack::is_rip_key(&item.key) {
        if let Ok(rip) = RandomIndexPack::parse(&full) {
            let reserialized = rip.to_bytes();
            if let Ok(reparsed) = RandomIndexPack::parse(&reserialized) {
                assert_eq!(
                    reserialized,
                    reparsed.to_bytes(),
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
        let reserialized = set.to_bytes();
        if let Ok(reparsed) = LocalSet::parse(&reserialized) {
            assert_eq!(reserialized, reparsed.to_bytes(), "LocalSet roundtrip mismatch");
        }

        match set.kind() {
            StructuralSetKind::Preface => {
                if let Ok(p) = Preface::parse(&full) {
                    let s = p.to_bytes();
                    if let Ok(rp) = Preface::parse(&s) {
                        assert_eq!(s, rp.to_bytes(), "Preface roundtrip mismatch");
                    }
                }
            }
            StructuralSetKind::Identification => {
                if let Ok(p) = Identification::parse(&full) {
                    let s = p.to_bytes();
                    if let Ok(rp) = Identification::parse(&s) {
                        assert_eq!(s, rp.to_bytes(), "Identification roundtrip mismatch");
                    }
                }
            }
            StructuralSetKind::ContentStorage => {
                if let Ok(p) = ContentStorage::parse(&full) {
                    let s = p.to_bytes();
                    if let Ok(rp) = ContentStorage::parse(&s) {
                        assert_eq!(s, rp.to_bytes(), "ContentStorage roundtrip mismatch");
                    }
                }
            }
            StructuralSetKind::EssenceContainerData => {
                if let Ok(p) = EssenceContainerData::parse(&full) {
                    let s = p.to_bytes();
                    if let Ok(rp) = EssenceContainerData::parse(&s) {
                        assert_eq!(s, rp.to_bytes(), "EssenceContainerData roundtrip mismatch");
                    }
                }
            }
            _ => {}
        }
    }
});
