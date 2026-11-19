//! Variable-length operation bodies: offset arithmetic, exact `need` values,
//! fixed lengths and message-size accounting (ANSI/SCTE 104 2023 §9.4/§9.5/
//! §9.7/§9.8/§10.5, `private/specs-verified/scte/scte104/`).

use broadcast_common::{Parse, Serialize};
use scte104::operations::component_mode_dpi::{ComponentModeDpi, ComponentModeEntry};
use scte104::operations::control_word::{
    DeleteControlWord, UpdateControlWord, DELETE_LEN, UPDATE_LEN,
};
use scte104::operations::insert_segmentation_descriptor::InsertSegmentationDescriptor;
use scte104::operations::provisioning_request::{
    InjectorComponentList, ProvisioningRequest, ProvisioningService,
};
use scte104::operations::schedule_component_mode::{
    ScheduleComponentEntry, ScheduleComponentMode,
};
use scte104::operations::{AliveRequest, AnySingleOperation};
use scte104::time::{SpliceScheduleTime, Time};
use scte104::SingleOperationMessage;

#[test]
fn injector_component_list_offsets_and_length_are_exact() {
    let request = ProvisioningRequest {
        services: vec![ProvisioningService {
            injector_ip_address: 0,
            injector_socket_number: 0,
            service_name: [0; 32],
            dpi_pids: Vec::new(),
            component_mode: 1,
            injector_component_list: Some(InjectorComponentList {
                video_component_tag: 0x10,
                audio_component_tags: vec![0x20, 0x21, 0x22],
                data_component_tags: vec![0x30, 0x31],
            }),
        }],
    };
    let mut out = vec![0u8; request.serialized_len()];
    let n = request.serialize_into(&mut out).unwrap();
    assert_eq!(n, 49);
    assert_eq!(out.len(), 49);
    assert_eq!(out[0], 1, "service_count");
    assert_eq!(out[39], 0, "number_of_DPI_PIDs");
    assert_eq!(out[40], 1, "component_mode");
    assert_eq!(&out[41..], &[0x10, 3, 0x20, 0x21, 0x22, 2, 0x30, 0x31]);

    assert_eq!(ProvisioningRequest::parse(&out).unwrap(), request);

    for n in 0..out.len() {
        assert!(
            ProvisioningRequest::parse(&out[..n]).is_err(),
            "a {n}-byte prefix must not parse"
        );
    }
}

#[test]
fn component_mode_dpi_entries_are_exactly_three_bytes_each() {
    let dpi = ComponentModeDpi {
        components: vec![
            ComponentModeEntry {
                component_tag: 0x10,
                component_preroll: 0x0102,
            },
            ComponentModeEntry {
                component_tag: 0x20,
                component_preroll: 0x0304,
            },
            ComponentModeEntry {
                component_tag: 0x30,
                component_preroll: 0x0506,
            },
        ],
    };
    let mut out = vec![0u8; dpi.serialized_len()];
    assert_eq!(dpi.serialize_into(&mut out).unwrap(), 9);
    assert_eq!(
        &out,
        &[0x10, 0x01, 0x02, 0x20, 0x03, 0x04, 0x30, 0x05, 0x06]
    );
    assert_eq!(ComponentModeDpi::parse(&out).unwrap(), dpi);

    // 2.5 entries: the reported need is the next whole entry boundary.
    assert_eq!(
        ComponentModeDpi::parse(&[0u8; 8]),
        Err(scte104::Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 9,
                have: 8,
                what: "component_mode_DPI data",
            }
        ))
    );
}

#[test]
fn schedule_component_mode_entries_are_exactly_five_bytes_each() {
    let scheduled = ScheduleComponentMode {
        components: vec![
            ScheduleComponentEntry {
                component_tag: 0x10,
                time: SpliceScheduleTime { seconds: 1 },
            },
            ScheduleComponentEntry {
                component_tag: 0x20,
                time: SpliceScheduleTime { seconds: 2 },
            },
            ScheduleComponentEntry {
                component_tag: 0x30,
                time: SpliceScheduleTime { seconds: 3 },
            },
        ],
    };
    let mut out = vec![0u8; scheduled.serialized_len()];
    assert_eq!(scheduled.serialize_into(&mut out).unwrap(), 15);
    assert_eq!(
        &out,
        &[
            0x10, 0, 0, 0, 1, 0x20, 0, 0, 0, 2, 0x30, 0, 0, 0, 3
        ]
    );
    assert_eq!(ScheduleComponentMode::parse(&out).unwrap(), scheduled);

    assert_eq!(
        ScheduleComponentMode::parse(&[0u8; 8]),
        Err(scte104::Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 10,
                have: 8,
                what: "schedule_component_mode data",
            }
        ))
    );
}

#[test]
fn segmentation_descriptor_tail_fields_sit_at_their_offsets() {
    let upid = [0xAAu8, 0xBB];
    let desc = InsertSegmentationDescriptor {
        segmentation_event_id: 0x0102_0304,
        segmentation_event_cancel_indicator: 1,
        duration: 0x0506,
        segmentation_upid_type: 0x07,
        segmentation_upid: &upid,
        segmentation_type_id: 0x31,
        segment_num: 7,
        segments_expected: 8,
        duration_extension_frames: 9,
        delivery_not_restricted_flag: 1,
        web_delivery_allowed_flag: 1,
        no_regional_blackout_flag: 1,
        archive_allowed_flag: 0,
        device_restrictions: 2,
        insert_sub_segment_info: 7,
        sub_segment_num: 3,
        sub_segments_expected: 4,
    };
    let mut out = vec![0u8; desc.serialized_len()];
    assert_eq!(desc.serialize_into(&mut out).unwrap(), 23);
    assert_eq!(
        &out[..11],
        &[0x01, 0x02, 0x03, 0x04, 0x01, 0x05, 0x06, 0x07, 0x02, 0xAA, 0xBB]
    );
    assert_eq!(
        &out[11..],
        &[0x31, 7, 8, 9, 1, 1, 1, 0, 2, 7, 3, 4],
        "tail fields, including insert_sub_segment_info at tail+9"
    );
    assert_eq!(out[20], 7, "insert_sub_segment_info");
    assert_eq!(InsertSegmentationDescriptor::parse(&out).unwrap(), desc);
}

#[test]
fn control_word_lengths_are_fixed_literals() {
    assert_eq!(UPDATE_LEN, 25);
    assert_eq!(DELETE_LEN, 1);

    let delete = DeleteControlWord { cw_index: 9 };
    assert_eq!(delete.serialized_len(), 1);
    let mut out = [0xFFu8; 1];
    assert_eq!(delete.serialize_into(&mut out).unwrap(), 1);
    assert_eq!(out, [9]);

    let update = UpdateControlWord {
        cw_index: 3,
        cw_a: 0x0102_0304_0506_0708,
        cw_b: 0x1112_1314_1516_1718,
        cw_c: 0x2122_2324_2526_2728,
    };
    assert_eq!(update.serialized_len(), 25);
    let mut out = vec![0u8; 25];
    assert_eq!(update.serialize_into(&mut out).unwrap(), 25);
    assert_eq!(out[0], 3);
    assert_eq!(&out[1..9], &0x0102_0304_0506_0708u64.to_be_bytes());
    assert_eq!(UpdateControlWord::parse(&out).unwrap(), update);
}

#[test]
fn single_operation_message_size_is_the_header_plus_body() {
    let message = SingleOperationMessage::new_request(
        0x0003,
        0,
        1,
        42,
        0,
        AnySingleOperation::AliveRequest(AliveRequest { time: Time::zero() }),
    );
    assert_eq!(message.message_size(), 21, "13-byte header + 8-byte time()");
    assert_eq!(message.serialized_len(), message.message_size());
}
