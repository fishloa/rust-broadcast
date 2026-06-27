//! Known-vector tests: decode published / spec-conformant SCTE 35 messages and
//! assert the decoded field values, with provenance for each vector.
//!
//! Every vector here passes the splice_info_section CRC_32 on parse (the parser
//! rejects a bad CRC), so an intact, genuine message is structurally proven;
//! these tests add the *semantic* assertions on top.

use base64::{engine::general_purpose::STANDARD, Engine};
use dvb_common::{Parse, Serialize};
use scte35_splice::commands::AnyCommand;
use scte35_splice::descriptors::{
    AnySpliceDescriptor, DeviceRestrictions, SegmentationTypeId, SegmentationUpidType,
};
use scte35_splice::SpliceInfoSection;

fn b64(s: &str) -> Vec<u8> {
    STANDARD.decode(s).unwrap()
}

/// Provenance: this is the canonical `splice_insert()` example reproduced in
/// numerous SCTE 35 references (Comcast/Cablelabs tutorials, the AWS Elemental
/// docs, the `threefive` test corpus). `splice_event_id` 0x4800008F. It carries
/// an `avail_descriptor` with `provider_avail_id` 0x135. The CRC_32 validates.
#[test]
fn splice_insert_4800008f() {
    let bytes = b64("/DAvAAAAAAAA///wFAVIAACPf+/+c2nALv4AUsz1AAAAAAAKAAhDVUVJAAABNWLbowo=");
    let s = SpliceInfoSection::parse(&bytes).expect("CRC + parse");

    assert_eq!(bytes[0], 0xFC); // table_id
    assert_eq!(s.protocol_version, 0);
    assert!(!s.encrypted_packet);
    assert_eq!(s.pts_adjustment, 0);
    assert!(s.tier_is_ignored()); // tier == 0xFFF

    let clear = s.clear.as_ref().expect("clear payload");
    let si = match &clear.command {
        AnyCommand::SpliceInsert(si) => si,
        other => panic!("expected SpliceInsert, got {other:?}"),
    };
    assert_eq!(si.splice_event_id, 0x4800_008F);
    assert!(!si.splice_event_cancel_indicator);
    assert!(si.out_of_network_indicator);
    assert!(si.program_splice_flag);
    assert!(!si.splice_immediate_flag);
    assert_eq!(si.splice_time.unwrap().pts_time, Some(0x0_7369_C02E));
    let bd = si.break_duration.expect("break_duration present");
    assert!(bd.auto_return);
    assert_eq!(bd.duration, 0x52_CCF5);

    // One avail_descriptor with provider_avail_id 0x135.
    let descs: Vec<_> = s.descriptors().map(Result::unwrap).collect();
    assert_eq!(descs.len(), 1);
    match &descs[0] {
        AnySpliceDescriptor::Avail(a) => {
            assert_eq!(a.identifier, scte35_splice::descriptors::CUEI);
            assert_eq!(a.provider_avail_id, 0x135);
        }
        other => panic!("expected Avail, got {other:?}"),
    }

    // Round-trips byte-for-byte (recomputes the CRC).
    assert_eq!(s.to_bytes(), bytes);
}

/// Provenance: a `splice_null()` "heartbeat" section, a widely reproduced
/// minimal SCTE 35 message (e.g. the `threefive` README). 20 bytes, the legacy
/// `tier`/`cw_index` all-ones backwards-compat pattern, no descriptors. The
/// CRC_32 validates.
#[test]
fn splice_null_heartbeat() {
    let bytes = b64("/DARAAAAAAAAAP/wAAAAAHpPv/8=");
    let s = SpliceInfoSection::parse(&bytes).expect("CRC + parse");

    assert_eq!(bytes[0], 0xFC);
    assert_eq!(bytes.len(), 20);
    assert!(!s.encrypted_packet);
    assert_eq!(s.pts_adjustment, 0);
    assert!(s.tier_is_ignored());

    let clear = s.clear.as_ref().expect("clear payload");
    assert!(matches!(clear.command, AnyCommand::SpliceNull(_)));
    assert_eq!(s.descriptors().count(), 0);

    assert_eq!(s.to_bytes(), bytes);
}

/// Provenance: **ANSI/SCTE 35 2023r1 §14.1** (PDF p.113) — the spec's own worked
/// `time_signal()` + `segmentation_descriptor` sample (Placement Opportunity
/// Start). Base64 verbatim from the spec; its CRC_32 validates on parse. Expected
/// field values are read directly from the spec's §14.1 decoded breakdown. This
/// exercises the `delivery_restrictions` path the other vectors do not.
#[test]
fn time_signal_segmentation_placement_opportunity_start_s14_1() {
    let bytes = b64("/DA0AAAAAAAA///wBQb+cr0AUAAeAhxDVUVJSAAAjn/PAAGlmbAICAAAAAAsoKGKNAIAmsnRfg==");
    let s = SpliceInfoSection::parse(&bytes).expect("CRC + parse");

    assert_eq!(bytes[0], 0xFC);
    assert_eq!(s.protocol_version, 0);
    assert!(!s.encrypted_packet);
    assert!(s.tier_is_ignored()); // tier == 0xFFF

    let clear = s.clear.as_ref().expect("clear payload");
    let ts = match &clear.command {
        AnyCommand::TimeSignal(ts) => ts,
        other => panic!("expected TimeSignal, got {other:?}"),
    };
    assert_eq!(ts.splice_time.pts_time, Some(0x0_72BD_0050));

    let descs: Vec<_> = s.descriptors().map(Result::unwrap).collect();
    assert_eq!(descs.len(), 1);
    match &descs[0] {
        AnySpliceDescriptor::Segmentation(seg) => {
            assert_eq!(seg.identifier, scte35_splice::descriptors::CUEI);
            assert_eq!(seg.segmentation_event_id, 0x4800_008E);
            assert!(!seg.segmentation_event_cancel_indicator);
            assert!(seg.program_segmentation_flag);
            // §14.1: Delivery Not Restricted = 0 → delivery_restrictions present.
            let dr = seg
                .delivery_restrictions
                .expect("delivery_restrictions present");
            assert!(!dr.web_delivery_allowed);
            assert!(dr.no_regional_blackout);
            assert!(dr.archive_allowed);
            assert_eq!(dr.device_restrictions, DeviceRestrictions::None);
            assert_eq!(seg.segmentation_duration, Some(0x1A_599B0)); // 307.0 s @ 90 kHz
            assert_eq!(seg.segmentation_upid_type, SegmentationUpidType::Ti);
            assert_eq!(seg.segmentation_upid.len(), 8);
            assert_eq!(
                seg.segmentation_type_id,
                SegmentationTypeId::ProviderPlacementOpportunityStart
            );
            assert_eq!(seg.segment_num, 2);
            assert_eq!(seg.segments_expected, 0);
        }
        other => panic!("expected Segmentation, got {other:?}"),
    }

    assert_eq!(s.to_bytes(), bytes);
}

/// The parser must reject a section whose CRC_32 does not validate (corrupt one
/// byte of the splice_null vector's body).
#[test]
fn rejects_bad_crc() {
    let mut bytes = b64("/DARAAAAAAAAAP/wAAAAAHpPv/8=");
    bytes[4] ^= 0xFF; // flip the encryption/pts_adjustment byte
    assert!(matches!(
        SpliceInfoSection::parse(&bytes),
        Err(scte35_splice::Error::CrcMismatch { .. })
    ));
}
