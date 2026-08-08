//! Parses `tests/fixtures/op1a_mpeg2_pcm.mxf` — a real OP1a MXF file
//! (MPEG-2 video + PCM audio) — and round-trips every top-level KLV item
//! to verify byte-identical reserialize.  This catches property-order
//! asymmetry: if the serializer emits local-set properties in a different
//! order than a real encoder wrote them, this test fails.

use broadcast_common::{Parse, Serialize};
use st377_1::{
    LocalSet, PartitionPack, PrimerPack, RandomIndexPack, collect_klv_items, is_fill_item_key,
};

fn fixture_bytes() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/op1a_mpeg2_pcm.mxf"
    ))
    .expect("read tests/fixtures/op1a_mpeg2_pcm.mxf")
}

#[test]
fn real_fixture_every_klv_item_round_trips_byte_identical() {
    let bytes = fixture_bytes();
    let items = collect_klv_items(&bytes).expect("collect KLV items from real fixture");
    assert!(
        !items.is_empty(),
        "fixture should contain at least one KLV item"
    );

    for (i, (_offset, item)) in items.iter().enumerate() {
        if is_fill_item_key(&item.key) {
            continue;
        }

        let klv = item.to_bytes();

        if PartitionPack::is_partition_key(&item.key) {
            let pp = PartitionPack::parse(&klv)
                .unwrap_or_else(|e| panic!("item {i}: PartitionPack parse failed: {e}"));
            let mut out = vec![0u8; pp.serialized_len()];
            pp.serialize_into(&mut out)
                .unwrap_or_else(|e| panic!("item {i}: PartitionPack serialize failed: {e}"));
            assert_eq!(
                out, klv,
                "item {i}: PartitionPack not byte-identical after round-trip"
            );
        } else if PrimerPack::is_primer_key(&item.key) {
            let primer = PrimerPack::parse(&klv)
                .unwrap_or_else(|e| panic!("item {i}: PrimerPack parse failed: {e}"));
            let mut out = vec![0u8; primer.serialized_len()];
            primer
                .serialize_into(&mut out)
                .unwrap_or_else(|e| panic!("item {i}: PrimerPack serialize failed: {e}"));
            assert_eq!(
                out, klv,
                "item {i}: PrimerPack not byte-identical after round-trip"
            );
        } else if RandomIndexPack::is_rip_key(&item.key) {
            let rip = RandomIndexPack::parse(&klv)
                .unwrap_or_else(|e| panic!("item {i}: RandomIndexPack parse failed: {e}"));
            let mut out = vec![0u8; rip.serialized_len()];
            rip.serialize_into(&mut out)
                .unwrap_or_else(|e| panic!("item {i}: RandomIndexPack serialize failed: {e}"));
            assert_eq!(
                out, klv,
                "item {i}: RandomIndexPack not byte-identical after round-trip"
            );
        } else {
            // Try as LocalSet — all structural metadata is encoded as local sets.
            if let Ok(set) = LocalSet::parse(&klv) {
                let mut out = vec![0u8; set.serialized_len()];
                set.serialize_into(&mut out)
                    .unwrap_or_else(|e| panic!("item {i}: LocalSet serialize failed: {e}"));
                assert_eq!(
                    out, klv,
                    "item {i}: LocalSet not byte-identical after round-trip (key {:02x?})",
                    &item.key[..4]
                );
            }
        }
    }
}
