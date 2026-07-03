//! DVB Targeted Advertising (ETSI TS 103 752-1) integration tests.
//!
//! End-to-end: the DVB_DAS_descriptor() inside a real base SCTE 35
//! splice_info_section() (parsed back via the generic descriptor loop), the
//! compact encoding round-trip, and the DSM-CC stream-event wrap + base-64.

use broadcast_common::{Parse, Serialize};
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::{AnyCommand, SpliceInsert};
use scte35_splice::descriptors::AnySpliceDescriptor;
use scte35_splice::dvb_ta::{
    CompactDas, CompactScte35, CompactSpliceInsert, DvbDasDescriptor, EquivalentSegmentationType,
    PlacementOpportunity, Scte35Carriage, StreamEventPayload, TimelineType, base64_encode,
};

/// The §5.3.5.11 example UPID URI.
const EXAMPLE_UPID: &[u8] = b"urn:com.broadcaster:112210F47DE98115";

#[test]
fn das_descriptor_rides_in_a_real_splice_info_section() {
    // Build a DVB_DAS_descriptor and place it in the splice descriptor loop of a
    // base splice_insert() section, then parse the whole section back and walk
    // the loop with the generic dispatcher — it must surface as DvbDas (0xF0).
    let das = DvbDasDescriptor::new(
        1,
        2,
        EquivalentSegmentationType::ProviderPlacementOpportunity,
        EXAMPLE_UPID,
    );
    let loop_bytes = das.to_bytes();

    let cmd = AnyCommand::SpliceInsert(SpliceInsert::default());
    let section = SpliceInfoSection::new_clear(cmd, &loop_bytes);
    let on_wire = section.to_bytes();

    // table_id 0xFC, parses back byte-exact.
    assert_eq!(on_wire[0], 0xFC);
    let parsed = SpliceInfoSection::parse(&on_wire).unwrap();
    assert_eq!(parsed.to_bytes(), on_wire);

    // The descriptor loop dispatches the DVB DAS descriptor by tag 0xF0.
    let descs: Vec<_> = parsed.descriptors().collect::<Result<_, _>>().unwrap();
    assert_eq!(descs.len(), 1);
    match &descs[0] {
        AnySpliceDescriptor::DvbDas(d) => {
            assert_eq!(d.identifier, scte35_splice::dvb_ta::DVB_IDENTIFIER);
            assert_eq!(d.break_num, 1);
            assert_eq!(d.breaks_expected, 2);
            assert_eq!(
                d.equivalent_segmentation_type,
                EquivalentSegmentationType::ProviderPlacementOpportunity
            );
            assert_eq!(d.upid, EXAMPLE_UPID);
        }
        other => panic!("expected DvbDas, got {other:?}"),
    }
    assert_eq!(descs[0].name(), "DVB_DAS");
}

#[test]
fn compact_splice_insert_with_das_round_trips() {
    let si = CompactSpliceInsert {
        encrypted_packet: false,
        encryption_algorithm: 0,
        cw_index: 0,
        pts_time: 0x0_1234_5678,
        splice_event_id: 0x4800_008F,
        duration: 0x0_0009_0000,
        unique_program_id: 0x1234,
        avail_num: 1,
        avails_expected: 3,
        das: Some(CompactDas {
            break_num: 1,
            breaks_expected: 2,
            equivalent_segmentation_type: EquivalentSegmentationType::ProviderPlacementOpportunity,
            upid: EXAMPLE_UPID.to_vec(),
        }),
        e_crc_32: None,
    };
    let compact = CompactScte35::SpliceInsert(si.clone());
    let bytes = compact.to_bytes();
    let back = CompactScte35::parse(&bytes).unwrap();
    assert_eq!(compact, back);
    assert_eq!(back.to_bytes(), bytes);
}

#[test]
fn stream_event_wraps_section_and_base64_inflates_by_4_3() {
    let inner =
        SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(SpliceInsert::default()), &[]);
    let inner_bytes = inner.to_bytes();
    let inner = SpliceInfoSection::parse(&inner_bytes).unwrap();

    let payload = StreamEventPayload {
        timeline_type: TimelineType::VideoPts,
        temi: None,
        private_data: None,
        carriage: Scte35Carriage::Inline(inner),
    };
    let bin = payload.to_bytes();
    let back = StreamEventPayload::parse(&bin).unwrap();
    assert_eq!(payload, back);
    assert_eq!(back.to_bytes(), bin);

    // base-64 (RFC 4648) inflates 3 bytes → 4 chars.
    let b64 = base64_encode(&bin);
    assert_eq!(b64.len(), bin.len().div_ceil(3) * 4);
    // Output is valid base-64 (cross-check against the `base64` dev-dep).
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&b64)
        .unwrap();
    assert_eq!(decoded, bin);
}

#[test]
fn placement_opportunity_classifies_the_spec_block() {
    // §5.3.5.4 fixes the four PO segmentation_type_ids 0x34..=0x37.
    for (id, is_provider, is_start) in [
        (0x34u8, true, true),
        (0x35, true, false),
        (0x36, false, true),
        (0x37, false, false),
    ] {
        let stid = scte35_splice::descriptors::SegmentationTypeId::from_u8(id);
        let po = PlacementOpportunity::from_segmentation_type_id(stid).unwrap();
        assert_eq!(po.is_provider(), is_provider);
        assert_eq!(po.is_start(), is_start);
        assert_eq!(po.segmentation_type_id().to_u8(), id);
    }
    // A non-PO type is rejected.
    assert!(
        PlacementOpportunity::from_segmentation_type_id(
            scte35_splice::descriptors::SegmentationTypeId::ProgramStart
        )
        .is_none()
    );
}
