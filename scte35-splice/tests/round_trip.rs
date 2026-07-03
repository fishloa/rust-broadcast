//! Full-section round-trip: build a `SpliceInfoSection` around each command and
//! descriptor, serialize → parse → equal, and parse → serialize → byte-identical.
//! Round-trip is the crate's primary, fabrication-proof correctness oracle.

use broadcast_common::{Parse, Serialize};
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::{
    AnyCommand, BandwidthReservation, PrivateCommand, SpliceInsert, SpliceNull, SpliceSchedule,
    SpliceScheduleEvent, TimeSignal,
};
use scte35_splice::descriptors::{
    AudioComponent, AudioDescriptor, AvailDescriptor, CUEI, DtmfDescriptor, SegmentationDescriptor,
    SegmentationTypeId, SegmentationUpidType, TimeDescriptor,
};
use scte35_splice::time::{BreakDuration, SpliceTime};

/// Serialize a section, parse it back, assert equality and byte-identity.
fn section_round_trip(section: &SpliceInfoSection) {
    let bytes = section.to_bytes();
    assert_eq!(bytes[0], 0xFC, "table_id");
    let parsed = SpliceInfoSection::parse(&bytes).expect("re-parse");
    assert_eq!(*section, parsed, "struct equality");
    assert_eq!(parsed.to_bytes(), bytes, "byte identity");
}

fn clear(command: AnyCommand<'static>) -> SpliceInfoSection<'static> {
    let mut s = SpliceInfoSection::new_clear(command, &[]);
    s.pts_adjustment = 0x1_2345_6789 & ((1 << 33) - 1);
    s.tier = 0x123;
    s.sap_type = 0x1;
    s
}

#[test]
fn splice_null_section() {
    section_round_trip(&clear(AnyCommand::SpliceNull(SpliceNull)));
}

#[test]
fn bandwidth_reservation_section() {
    section_round_trip(&clear(AnyCommand::BandwidthReservation(
        BandwidthReservation,
    )));
}

#[test]
fn time_signal_section() {
    section_round_trip(&clear(AnyCommand::TimeSignal(TimeSignal {
        splice_time: SpliceTime::with_pts(0x0_0ABC_DEF0),
    })));
    section_round_trip(&clear(AnyCommand::TimeSignal(TimeSignal::default())));
}

#[test]
fn private_command_section() {
    let cmd = PrivateCommand {
        identifier: 0x4F_4F_4F_4F,
        private_bytes: &[0x01, 0x02, 0x03],
    };
    let mut s = SpliceInfoSection::new_clear(AnyCommand::PrivateCommand(cmd), &[]);
    s.tier = 0xFFF;
    section_round_trip(&s);
}

#[test]
fn splice_insert_section() {
    let cmd = SpliceInsert {
        splice_event_id: 0x4800_00AB,
        out_of_network_indicator: true,
        program_splice_flag: true,
        splice_time: Some(SpliceTime::with_pts(0x0_1111_2222)),
        break_duration: Some(BreakDuration {
            auto_return: false,
            duration: 90_000 * 15,
        }),
        unique_program_id: 0xBEEF,
        avail_num: 2,
        avails_expected: 5,
        ..Default::default()
    };
    section_round_trip(&clear(AnyCommand::SpliceInsert(cmd)));
}

#[test]
fn splice_schedule_section() {
    let cmd = SpliceSchedule {
        events: vec![SpliceScheduleEvent {
            splice_event_id: 1,
            out_of_network_indicator: true,
            utc_splice_time: Some(0x1234_5678),
            unique_program_id: 3,
            ..Default::default()
        }],
    };
    section_round_trip(&clear(AnyCommand::SpliceSchedule(cmd)));
}

#[test]
fn unknown_command_section_round_trips() {
    // Reserved splice_command_type 0x03 with a raw body.
    let s = SpliceInfoSection::new_clear(
        AnyCommand::Unknown {
            command_type: 0x03,
            body: &[0xDE, 0xAD, 0xBE, 0xEF],
        },
        &[],
    );
    section_round_trip(&s);
}

/// A section carrying a full descriptor loop with one of every typed splice
/// descriptor plus an unknown tag.
#[test]
fn section_with_full_descriptor_loop() {
    // Build the descriptor loop bytes from each descriptor's serializer.
    let mut loop_bytes = Vec::new();
    loop_bytes.extend(
        AvailDescriptor {
            identifier: CUEI,
            provider_avail_id: 0x99,
        }
        .to_bytes(),
    );
    loop_bytes.extend(
        DtmfDescriptor {
            identifier: CUEI,
            preroll: 50,
            dtmf_chars: vec![b'1', b'2', b'#'],
        }
        .to_bytes(),
    );
    loop_bytes.extend(
        SegmentationDescriptor {
            segmentation_event_id: 0x55,
            segmentation_upid_type: SegmentationUpidType::AdId,
            segmentation_upid: b"ABCD12345678",
            segmentation_type_id: SegmentationTypeId::ProviderPlacementOpportunityStart,
            segment_num: 1,
            segments_expected: 1,
            sub_segments: Some((1, 4)),
            ..Default::default()
        }
        .to_bytes(),
    );
    loop_bytes.extend(
        TimeDescriptor {
            identifier: CUEI,
            tai_seconds: 0x1122_3344_5566 & ((1 << 48) - 1),
            tai_ns: 0xAABB_CCDD,
            utc_offset: 37,
        }
        .to_bytes(),
    );
    loop_bytes.extend(
        AudioDescriptor {
            identifier: CUEI,
            components: vec![AudioComponent {
                component_tag: 0xFF,
                iso_code: *b"eng",
                bit_stream_mode: 0b001,
                num_channels: 0b0010,
                full_srvc_audio: true,
            }],
        }
        .to_bytes(),
    );
    // An unknown splice_descriptor_tag 0xC0 with a raw body.
    loop_bytes.extend_from_slice(&[0xC0, 0x02, 0xCA, 0xFE]);

    let section = SpliceInfoSection::new_clear(
        AnyCommand::TimeSignal(TimeSignal {
            splice_time: SpliceTime::with_pts(0x1000),
        }),
        &loop_bytes,
    );
    section_round_trip(&section);

    // All six descriptors walk out typed/unknown.
    let names: Vec<_> = section.descriptors().map(|d| d.unwrap().name()).collect();
    assert_eq!(
        names,
        vec!["AVAIL", "DTMF", "SEGMENTATION", "TIME", "AUDIO", "UNKNOWN"]
    );
}

/// Encrypted sections keep the encrypted region raw and round-trip losslessly.
#[test]
fn encrypted_section_round_trips_raw() {
    // splice_command_type..E_CRC_32 kept verbatim (we do not decrypt).
    let encrypted = [0x06u8, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00, 0x11, 0x22];
    let section = SpliceInfoSection {
        sap_type: 0x3,
        protocol_version: 0,
        encrypted_packet: true,
        encryption_algorithm: 1, // DES-ECB
        pts_adjustment: 0x100,
        cw_index: 7,
        tier: 0x0AB,
        clear: None,
        encrypted_payload: Some(&encrypted),
    };
    let bytes = section.to_bytes();
    let parsed = SpliceInfoSection::parse(&bytes).unwrap();
    assert!(parsed.encrypted_packet);
    assert_eq!(parsed.encryption_algorithm, 1);
    assert_eq!(parsed.encrypted_payload, Some(&encrypted[..]));
    assert!(parsed.clear.is_none());
    assert_eq!(parsed.to_bytes(), bytes);
}

/// 33-bit `pts_adjustment` wrap boundary (§9.6.1: carry ignored on overflow).
#[test]
fn pts_adjustment_33bit_wrap_boundary() {
    use scte35_splice::time::{PTS_MAX, PTS_MODULUS, pts_add_wrapping};

    // The maximum 33-bit pts_adjustment round-trips through the section.
    let mut s = SpliceInfoSection::new_clear(AnyCommand::SpliceNull(SpliceNull), &[]);
    s.pts_adjustment = PTS_MAX;
    section_round_trip(&s);

    // A pts_adjustment above the 33-bit range is rejected by serialize.
    let mut bad = SpliceInfoSection::new_clear(AnyCommand::SpliceNull(SpliceNull), &[]);
    bad.pts_adjustment = PTS_MODULUS; // 2^33, one past the max
    let mut buf = vec![0u8; bad.serialized_len()];
    assert!(matches!(
        bad.serialize_into(&mut buf),
        Err(scte35_splice::Error::InvalidValue { .. })
    ));

    // Adding pts_adjustment to a pts_time wraps modulo 2^33, dropping the carry.
    assert_eq!(pts_add_wrapping(PTS_MAX, 1), 0);
    assert_eq!(pts_add_wrapping(PTS_MAX, 5), 4);
}

/// #216: section_length == 0 → total == 3 → crc_pos underflow panic.
/// Must return Err, not panic.
#[test]
fn section_length_zero_no_underflow() {
    // Construct bytes: table_id=0xFC, section_length=0 (<= bytes 1-2).
    let data = [0xFC, 0x00, 0x00];
    let err = SpliceInfoSection::parse(&data).unwrap_err();
    assert!(matches!(err, scte35_splice::Error::BufferTooShort { .. }));
}
