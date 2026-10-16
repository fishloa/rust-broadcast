//! Adversarial-review findings (review/signalling-captions). Each test is a
//! FAILING regression for a confirmed defect; production code is untouched.

use broadcast_common::{Parse, Serialize};
use scte35_splice::dvb_ta::{
    CompactDas, CompactSpliceInsert, CompactTimeSignal, DvbDasDescriptor,
    EquivalentSegmentationType, SI_RESERVED_DEFAULT, Scte35Carriage, StreamEventPayload,
    TimelineType,
};

fn compact_with_type(t: EquivalentSegmentationType) -> CompactSpliceInsert {
    CompactSpliceInsert {
        encrypted_packet: false,
        encryption_algorithm: 0,
        cw_index: 0,
        pts_time: 0x1234,
        splice_event_id: 1,
        duration: 0x9_0000,
        unique_program_id: 0xABCD,
        avail_num: 1,
        avails_expected: 3,
        reserved: SI_RESERVED_DEFAULT,
        das: Some(CompactDas {
            break_num: 1,
            breaks_expected: 3,
            equivalent_segmentation_type: t,
            reserved: 0x0F,
            upid: b"urn:x".to_vec(),
        }),
        e_crc_32: None,
    }
}

/// #1129/#1202 class, missed by the wave that fixed the DAS `reserved` nibble:
/// `EquivalentSegmentationType::bits()` masks a hand-set `Reserved(v)` to its
/// low nibble, so `Reserved(0x12)` is silently written as `0x2`
/// (ProviderPlacementOpportunity) - a different, valid-looking value. The
/// dvb-subtitle wave made `Reserved(v)` verbatim + range-checked; this crate
/// did not. Serialize must reject it.
#[test]
fn compact_das_reserved_equivalent_type_over_4_bits_is_rejected() {
    let si = compact_with_type(EquivalentSegmentationType::Reserved(0x12));
    let r = si.try_to_bytes();
    assert!(
        r.is_err(),
        "Reserved(0x12) serialized as {:?} (re-parses as {:?})",
        r.as_ref().ok(),
        r.as_ref()
            .ok()
            .and_then(|b| CompactSpliceInsert::parse(b).ok())
            .and_then(|p| p.das.map(|d| d.equivalent_segmentation_type))
    );
}

/// Same masking on the stand-alone `DVB_DAS_descriptor()`.
#[test]
fn das_descriptor_reserved_equivalent_type_over_4_bits_is_rejected() {
    let d = DvbDasDescriptor::new(0, 0, EquivalentSegmentationType::Reserved(0x11), b"u");
    assert!(
        d.try_to_bytes().is_err(),
        "Reserved(0x11) was silently masked to 0x1 (DPO)"
    );
}

/// `TimelineType::bits()` masks too: `Reserved(0x12)` is written as `0x2`
/// (TEMI) without any `temi_*` bytes, so the serializer emits a payload its
/// own parser rejects. Independently, the serializer writes the `temi` bytes
/// iff `temi.is_some()` without checking it against `timeline_type`, so
/// `{Temi, temi: None}` is equally unparseable and `{VideoPts, temi: Some}`
/// silently loses `temi` on reparse.
#[test]
fn stream_event_payload_serialize_output_always_reparses_equal() {
    let cases = [
        (TimelineType::Reserved(0x12), None),
        (TimelineType::Temi, None),
        (TimelineType::VideoPts, Some((7u8, 9u8))),
    ];
    for (timeline_type, temi) in cases {
        let p = StreamEventPayload {
            timeline_type,
            temi,
            private_data: None,
            carriage: Scte35Carriage::CarouselObject(b"dvb://1.2.3/x"),
        };
        match p.try_to_bytes() {
            Err(_) => {} // rejecting the inconsistent pair is fine
            Ok(bytes) => {
                let back = StreamEventPayload::parse(&bytes);
                assert_eq!(
                    back.as_ref().ok(),
                    Some(&p),
                    "{timeline_type:?}/{temi:?}: serialized {bytes:02X?} reparsed as {back:?}"
                );
            }
        }
    }
}

/// `DVB_data()` bytes after the flags byte (and optional TEMI) are parsed as
/// "skip", then dropped: serialize recomputes `DVB_data_length` = 1, so a real
/// payload carrying them is not re-serialized byte-exactly (the project's hard
/// round-trip invariant; no typed field preserves them).
#[test]
fn stream_event_payload_trailing_dvb_data_round_trips_byte_exact() {
    // DVB_data_length=2: flags (reserved 111, event_type 1, timeline 0) + 1 byte.
    let mut wire = vec![0x02, 0xF0, 0x00, 0x00];
    let name = b"dvb://1.2.3/x";
    wire.push(name.len() as u8);
    wire.extend_from_slice(name);
    let p = StreamEventPayload::parse(&wire).expect("parses");
    assert_eq!(p.try_to_bytes().unwrap(), wire);
}

/// `encrypted_packet` and `e_crc_32` are independent fields, but serialize
/// writes `E_CRC_32 = 0` for `{encrypted_packet: true, e_crc_32: None}` (and
/// drops a `Some` when not encrypted), so serialize -> parse is not equal.
#[test]
fn compact_time_signal_inconsistent_e_crc_is_rejected_or_round_trips() {
    let ts = CompactTimeSignal {
        encrypted_packet: true,
        encryption_algorithm: 1,
        cw_index: 0,
        pts_time: 0,
        segmentation_event_id: 0,
        segmentation_duration: 0,
        segmentation_type_id: 0x34,
        segmentation_upid: vec![],
        segments_num: 0,
        segments_expected: 0,
        e_crc_32: None,
    };
    if let Ok(bytes) = ts.try_to_bytes() {
        assert_eq!(CompactTimeSignal::parse(&bytes).unwrap(), ts);
    }
}
