//! splice_schedule() and splice_insert() cursor arithmetic and per-event
//! fields (ANSI/SCTE 35 2023r1 §9.7.2 Table 9, §9.7.3 Table 10), plus the two
//! empty commands' parse behaviour.

use broadcast_common::{Parse, Serialize};
use scte35_splice::Error;
use scte35_splice::commands::{
    BandwidthReservation, SpliceInsert, SpliceMode, SpliceNull, SpliceSchedule,
    SpliceScheduleComponent, SpliceScheduleEvent,
};
use scte35_splice::time::{BreakDuration, SpliceTime};

fn break_duration() -> BreakDuration {
    BreakDuration {
        auto_return: true,
        duration: 0x1_0203_0405,
    }
}

fn schedule() -> SpliceSchedule {
    SpliceSchedule {
        events: vec![
            SpliceScheduleEvent {
                splice_event_id: 0x0102_0304,
                event_id_compliance_flag: true,
                out_of_network_indicator: true,
                program_splice_flag: true,
                utc_splice_time: Some(0x0102_0304),
                break_duration: Some(break_duration()),
                unique_program_id: 0x1111,
                avail_num: 3,
                avails_expected: 4,
                ..Default::default()
            },
            SpliceScheduleEvent {
                splice_event_id: 0x0506_0708,
                splice_event_cancel_indicator: true,
                event_id_compliance_flag: false,
                ..Default::default()
            },
            SpliceScheduleEvent {
                splice_event_id: 0x0A0B_0C0D,
                event_id_compliance_flag: true,
                program_splice_flag: false,
                components: vec![
                    SpliceScheduleComponent {
                        component_tag: 0x11,
                        utc_splice_time: 0x0102_0304,
                    },
                    SpliceScheduleComponent {
                        component_tag: 0x22,
                        utc_splice_time: 0xAABB_CCDD,
                    },
                ],
                unique_program_id: 0x2222,
                avail_num: 5,
                avails_expected: 6,
                ..Default::default()
            },
        ],
    }
}

/// Hand-assembled from Table 9: count 3; event 1 program-mode with compliance
/// set, out-of-network, a break duration and the trailer; event 2 cancelled
/// with compliance clear; event 3 component-mode with two components.
#[rustfmt::skip]
const SCHEDULE_BYTES: [u8; 46] = [
    0x03,
    0x01, 0x02, 0x03, 0x04, 0x7F,
    0xFF, 0x01, 0x02, 0x03, 0x04,
    0xFF, 0x02, 0x03, 0x04, 0x05,
    0x11, 0x11, 0x03, 0x04,
    0x05, 0x06, 0x07, 0x08, 0xBF,
    0x0A, 0x0B, 0x0C, 0x0D, 0x7F,
    0x1F, 0x02,
    0x11, 0x01, 0x02, 0x03, 0x04,
    0x22, 0xAA, 0xBB, 0xCC, 0xDD,
    0x22, 0x22, 0x05, 0x06,
];

#[test]
fn splice_schedule_events_advance_the_cursor_per_event() {
    let bytes = schedule().try_to_bytes().unwrap();
    assert_eq!(bytes, SCHEDULE_BYTES);
    let parsed = SpliceSchedule::parse(&SCHEDULE_BYTES).unwrap();
    assert_eq!(parsed, schedule());

    assert_eq!(parsed.events.len(), 3);
    assert_eq!(parsed.events[0].splice_event_id, 0x0102_0304);
    assert!(parsed.events[0].event_id_compliance_flag);
    assert_eq!(parsed.events[0].utc_splice_time, Some(0x0102_0304));
    assert_eq!(parsed.events[0].break_duration, Some(break_duration()));
    assert_eq!(parsed.events[0].unique_program_id, 0x1111);
    assert_eq!(parsed.events[0].avail_num, 3);
    assert_eq!(parsed.events[0].avails_expected, 4);

    assert_eq!(parsed.events[1].splice_event_id, 0x0506_0708);
    assert!(parsed.events[1].splice_event_cancel_indicator);
    assert!(!parsed.events[1].event_id_compliance_flag);

    assert_eq!(parsed.events[2].splice_event_id, 0x0A0B_0C0D);
    assert!(!parsed.events[2].program_splice_flag);
    assert!(parsed.events[2].event_id_compliance_flag);
    assert_eq!(parsed.events[2].components.len(), 2);
    assert_eq!(parsed.events[2].components[1].component_tag, 0x22);
    assert_eq!(parsed.events[2].components[1].utc_splice_time, 0xAABB_CCDD);
    assert_eq!(parsed.events[2].avail_num, 5);
    assert_eq!(parsed.events[2].avails_expected, 6);
}

#[test]
fn a_scheduled_event_truncated_after_its_flags_reports_the_exact_need() {
    // Event header only: 4-byte id + the flags byte.
    assert_eq!(
        SpliceSchedule::parse(&[0x01, 0x01, 0x02, 0x03, 0x04, 0x7F]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 7,
                have: 6,
                what: "splice_schedule event flags",
            }
        ))
    );
}

fn insert() -> SpliceInsert {
    SpliceInsert {
        splice_event_id: 0x0102_0304,
        mode: SpliceMode::Program(scte35_splice::commands::ProgramSplice::At(SpliceTime::at(
            broadcast_common::clock33::Clock33::try_new(0x1_2345_6789).unwrap(),
        ))),
        out_of_network_indicator: true,
        event_id_compliance_flag: true,
        break_duration: Some(break_duration()),
        unique_program_id: 0x1111,
        avail_num: 2,
        avails_expected: 3,
        ..Default::default()
    }
}

#[test]
fn splice_insert_program_mode_offsets_are_pinned() {
    #[rustfmt::skip]
    let expected: [u8; 20] = [
        0x01, 0x02, 0x03, 0x04,
        0x7F,
        0xEF,
        0xFF, 0x23, 0x45, 0x67, 0x89,
        0xFF, 0x02, 0x03, 0x04, 0x05,
        0x11, 0x11, 0x02, 0x03,
    ];
    let bytes = insert().try_to_bytes().unwrap();
    assert_eq!(bytes, expected);
    let parsed = SpliceInsert::parse(&expected).unwrap();
    assert_eq!(parsed, insert());
    assert!(parsed.out_of_network_indicator);
    assert!(parsed.event_id_compliance_flag);
    assert_eq!(parsed.break_duration, Some(break_duration()));
    assert_eq!(parsed.unique_program_id, 0x1111);
    assert_eq!(parsed.avail_num, 2);
    assert_eq!(parsed.avails_expected, 3);
}

#[test]
fn splice_insert_truncated_after_the_event_id_needs_the_flags_byte() {
    assert_eq!(
        SpliceInsert::parse(&[0x01, 0x02, 0x03, 0x04, 0x00]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 6,
                have: 5,
                what: "splice_insert flags",
            }
        ))
    );
}

#[test]
fn splice_insert_component_mode_missing_the_count_needs_one_more_byte() {
    // id + cancel/compliance byte + a component-mode flags byte, nothing else.
    assert_eq!(
        SpliceInsert::parse(&[0x01, 0x02, 0x03, 0x04, 0x00, 0x08]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 7,
                have: 6,
                what: "splice_insert component_count",
            }
        ))
    );
}

#[test]
fn splice_insert_component_mode_missing_the_trailer_reports_the_exact_need() {
    // id + byte + component-mode flags + component_count 0, no trailer.
    assert_eq!(
        SpliceInsert::parse(&[0x01, 0x02, 0x03, 0x04, 0x00, 0x08, 0x00]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 11,
                have: 7,
                what: "splice_insert trailer",
            }
        ))
    );
}

#[test]
fn the_empty_commands_accept_any_body_without_reading_it() {
    assert_eq!(SpliceNull::parse(&[]), Ok(SpliceNull));
    assert_eq!(SpliceNull::parse(&[0x00]), Ok(SpliceNull));
    assert_eq!(
        SpliceNull::parse(&[0xDE, 0xAD]),
        Ok(SpliceNull),
        "splice_null has no body fields"
    );
    assert_eq!(BandwidthReservation::parse(&[]), Ok(BandwidthReservation));
    assert_eq!(
        BandwidthReservation::parse(&[0x01, 0x02, 0x03]),
        Ok(BandwidthReservation)
    );
    assert_eq!(SpliceNull.serialized_len(), 0);
    assert_eq!(BandwidthReservation.serialized_len(), 0);
}

#[test]
fn a_second_scheduled_event_truncated_in_its_header_reports_the_absolute_need() {
    let mut bytes = vec![0x02u8];
    bytes.extend_from_slice(&SCHEDULE_BYTES[1..20]);
    bytes.extend_from_slice(&[0x05, 0x06, 0x07, 0x08]);
    assert_eq!(
        SpliceSchedule::parse(&bytes),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 25,
                have: 24,
                what: "splice_schedule event header",
            }
        ))
    );
}

#[test]
fn a_second_scheduled_event_missing_its_trailer_reports_the_absolute_need() {
    let mut bytes = vec![0x02u8];
    bytes.extend_from_slice(&SCHEDULE_BYTES[1..20]);
    bytes.extend_from_slice(&[0x05, 0x06, 0x07, 0x08, 0x7F, 0xDF, 0x00, 0x00, 0x00, 0x09]);
    assert_eq!(
        SpliceSchedule::parse(&bytes),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 34,
                have: 30,
                what: "splice_schedule event trailer",
            }
        ))
    );
}
