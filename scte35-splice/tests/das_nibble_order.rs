//! Pins BOTH readings of the DVB DAS `equivalent_segmentation_type` / `reserved`
//! nibble order (ETSI TS 103 752-1 V1.2.1 Table 1 p.17 vs Table 8 p.28), so a
//! future real DAS-carrying capture decides by flipping one constant
//! ([`DasNibbleOrder`]) rather than by rewriting the codecs.
//!
//! AMBIGUITY (see `DasNibbleOrder` for the full evidence): Table 1 prints
//! `reserved(4)`, `equivalent_segmentation_type(4)` (type in the LOW nibble);
//! Table 8 prints `equivalent_segmentation_type(4)`, `reserved(4)` (type in the
//! HIGH nibble). Nothing in the text, the other published parts, independent
//! implementations or any capture settles it. These tests therefore assert the
//! *behaviour of each explicit order* and that the defaults are each table's own
//! order - they do not claim either reading is the true one.

use broadcast_common::{Parse, Serialize};
use scte35_splice::dvb_ta::{
    CompactDas, CompactSpliceInsert, DasNibbleOrder, DvbDasDescriptor, EquivalentSegmentationType,
    SI_RESERVED_DEFAULT,
};

const UPID: &[u8] = b"urn:tv.acme:B637643";

/// `F0 len "DVB_" break_num breaks_expected <type/reserved byte> upid`.
fn descriptor_bytes(nibbles: u8) -> Vec<u8> {
    let mut v = vec![
        0xF0,
        (4 + 3 + UPID.len()) as u8,
        0x44,
        0x56,
        0x42,
        0x5F,
        1,
        3,
        nibbles,
    ];
    v.extend_from_slice(UPID);
    v
}

fn compact(das_type: EquivalentSegmentationType, reserved: u8) -> CompactSpliceInsert {
    CompactSpliceInsert {
        encrypted_packet: false,
        encryption_algorithm: 0,
        cw_index: 0,
        pts_time: 0x1_2345_6789 & 0x1_FFFF_FFFF,
        splice_event_id: 0x0102_0304,
        duration: 0x9_0000,
        unique_program_id: 0xABCD,
        avail_num: 1,
        avails_expected: 3,
        reserved: SI_RESERVED_DEFAULT,
        das: Some(CompactDas {
            break_num: 1,
            breaks_expected: 3,
            equivalent_segmentation_type: das_type,
            reserved,
            upid: UPID.to_vec(),
        }),
        e_crc_32: None,
    }
}

/// Offset of the type/reserved byte inside `compact_splice_insert()`:
/// 152 fixed bits (19 bytes) + `descriptor_length`(1) + break_num(1) + breaks_expected(1).
const COMPACT_NIBBLE_BYTE: usize = 19 + 1 + 2;

#[test]
fn defaults_follow_each_tables_own_order() {
    assert_eq!(DasNibbleOrder::TABLE_1, DasNibbleOrder::TypeLow);
    assert_eq!(DasNibbleOrder::TABLE_8, DasNibbleOrder::TypeHigh);
    // The same byte 0x2F reads as PPO + reserved 0xF under Table 8 and as
    // reserved-valued type 0xF + reserved 0x2 under Table 1.
    assert_eq!(DasNibbleOrder::TypeHigh.split(0x2F), (0x2, 0xF));
    assert_eq!(DasNibbleOrder::TypeLow.split(0x2F), (0xF, 0x2));
    assert_eq!(DasNibbleOrder::TypeHigh.join(0x2, 0xF), 0x2F);
    assert_eq!(DasNibbleOrder::TypeLow.join(0x2, 0xF), 0xF2);
}

#[test]
fn standalone_descriptor_each_interpretation() {
    // Default (Table 1): reserved HIGH, type LOW => 0xF2 is PPO.
    let t1 = descriptor_bytes(0xF2);
    let d = DvbDasDescriptor::parse(&t1).unwrap();
    assert_eq!(
        d.equivalent_segmentation_type,
        EquivalentSegmentationType::ProviderPlacementOpportunity
    );
    assert_eq!(
        d,
        DvbDasDescriptor::parse_with_order(&t1, DasNibbleOrder::TypeLow).unwrap()
    );
    // The same bytes under the Table 8 reading are the reserved type 0xF.
    let other = DvbDasDescriptor::parse_with_order(&t1, DasNibbleOrder::TypeHigh).unwrap();
    assert_eq!(
        other.equivalent_segmentation_type,
        EquivalentSegmentationType::Reserved(0xF)
    );
    // Serialising PPO: default (Table 1) -> 0xF2; Table 8 order -> 0x2F.
    let ppo = DvbDasDescriptor::new(
        1,
        3,
        EquivalentSegmentationType::ProviderPlacementOpportunity,
        UPID,
    );
    assert_eq!(ppo.try_to_bytes().unwrap(), t1);
    let mut buf = vec![0u8; ppo.serialized_len()];
    ppo.serialize_into_with_order(&mut buf, DasNibbleOrder::TypeHigh)
        .unwrap();
    assert_eq!(buf, descriptor_bytes(0x2F));
    // Round trip under each order.
    for order in [DasNibbleOrder::TypeLow, DasNibbleOrder::TypeHigh] {
        let mut b = vec![0u8; ppo.serialized_len()];
        ppo.serialize_into_with_order(&mut b, order).unwrap();
        assert_eq!(DvbDasDescriptor::parse_with_order(&b, order).unwrap(), ppo);
    }
}

#[test]
fn compact_splice_insert_each_interpretation() {
    let si = compact(
        EquivalentSegmentationType::ProviderPlacementOpportunity,
        0xF,
    );
    // Default (Table 8): type HIGH => the byte is 0x2F.
    let bytes = si.try_to_bytes().unwrap();
    assert_eq!(bytes[COMPACT_NIBBLE_BYTE], 0x2F);
    assert_eq!(CompactSpliceInsert::parse(&bytes).unwrap(), si);
    // The same bytes under the Table 1 reading: type 0xF, reserved 0x2.
    let alt = CompactSpliceInsert::parse_with_order(&bytes, DasNibbleOrder::TypeLow).unwrap();
    let das = alt.das.unwrap();
    assert_eq!(
        das.equivalent_segmentation_type,
        EquivalentSegmentationType::Reserved(0xF)
    );
    assert_eq!(das.reserved, 0x2);
    // Serialising the type-low order writes 0xF2 and round-trips under that order.
    let mut b = vec![0u8; si.serialized_len()];
    si.serialize_into_with_order(&mut b, DasNibbleOrder::TypeLow)
        .unwrap();
    assert_eq!(b[COMPACT_NIBBLE_BYTE], 0xF2);
    assert_eq!(
        CompactSpliceInsert::parse_with_order(&b, DasNibbleOrder::TypeLow).unwrap(),
        si
    );
    // Bytes outside the nibble byte are order-independent.
    let mut masked = bytes.clone();
    masked[COMPACT_NIBBLE_BYTE] = 0;
    let mut b2 = b.clone();
    b2[COMPACT_NIBBLE_BYTE] = 0;
    assert_eq!(masked, b2);
}

/// A caller that picks ONE order for both forms gets the same typed DAS body
/// from the stand-alone descriptor and the compact message - the consistency a
/// future capture would let a decoder rely on.
#[test]
fn one_order_gives_one_meaning_across_both_forms() {
    for order in [DasNibbleOrder::TypeHigh, DasNibbleOrder::TypeLow] {
        let si = compact(EquivalentSegmentationType::DistributorAdvertisement, 0x5);
        let mut cb = vec![0u8; si.serialized_len()];
        si.serialize_into_with_order(&mut cb, order).unwrap();
        let from_compact = CompactSpliceInsert::parse_with_order(&cb, order)
            .unwrap()
            .das
            .unwrap();

        let d = DvbDasDescriptor::new(
            1,
            3,
            EquivalentSegmentationType::DistributorAdvertisement,
            UPID,
        );
        let mut db = vec![0u8; d.serialized_len()];
        d.serialize_into_with_order(&mut db, order).unwrap();
        let from_desc = DvbDasDescriptor::parse_with_order(&db, order).unwrap();

        assert_eq!(
            from_compact.equivalent_segmentation_type,
            from_desc.equivalent_segmentation_type
        );
        assert_eq!(from_compact.upid.as_slice(), from_desc.upid);
    }
}
