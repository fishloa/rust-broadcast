//! Serialized-length, 12-bit field and ceiling contracts for the remaining
//! CI-extension resources (ETSI TS 101 699 Tables 22-30, 41-51, 55-60, 69-73,
//! and the Resource Manager v2 module-id objects).

use broadcast_core::error::LengthMismatch;
use broadcast_core::{Parse, Serialize};
use dvb_ci::Error;
use dvb_ci::ci_ext::copy_protection::{CpCommand, CpQuery, CpReply, CpStatus};
use dvb_ci::ci_ext::event_manager::{EventReply, EventRequestAck, EventType};
use dvb_ci::ci_ext::resource_manager_v2::{ModuleIdCommand, ModuleIdCommandKind, ModuleIdSend};
use dvb_ci::ci_ext::service_gateway::{
    GetServiceAck, GetServiceReq, ServiceDescAck, ServiceDescReq, ServiceListAck, ServiceReference,
};
use dvb_ci::ci_ext::status_query::{
    ActivationState, InPortDescription, OutputSignal, PortDescription, PortProfile,
    SelectionInformation, ViewedService,
};
use dvb_ci::reserved::{ReservedOnes, ReservedZero};

fn serialize_len<T: Serialize<Error = Error>>(value: &T) -> (usize, Vec<u8>) {
    let mut buf = vec![0u8; value.serialized_len()];
    let n = value.serialize_into(&mut buf).expect("serialize");
    (n, buf)
}

fn reference(onid: u16, sid: u16) -> ServiceReference {
    ServiceReference {
        original_network_id: onid,
        service_id: sid,
    }
}

#[test]
fn service_list_ack_wire_and_return() {
    let ack = ServiceListAck {
        length_form: dvb_ci::length::LengthForm::Minimal,
        version_number: 0x03,
        services: vec![reference(0x0001, 0x0102), reference(0x0003, 0x0204)],
    };
    let (n, buf) = serialize_len(&ack);
    assert_eq!(n, 15);
    assert_eq!(n, ack.serialized_len());
    assert_eq!(
        buf,
        [
            0x9F, 0x80, 0x01, 0x0B, 0x03, 0x00, 0x02, 0x00, 0x01, 0x01, 0x02, 0x00, 0x03, 0x02,
            0x04
        ]
    );
    assert_eq!(ServiceListAck::parse(&buf).unwrap(), ack);
}

#[test]
fn service_list_ack_truncated_list_reports_the_declared_ref_bytes() {
    let wire = [
        0x9F, 0x80, 0x01, 0x0A, 0x03, 0x00, 0x02, 0x00, 0x01, 0x01, 0x02, 0x00, 0x03, 0x02,
    ];
    match ServiceListAck::parse(&wire).unwrap_err() {
        Error::LengthMismatch(LengthMismatch {
            declared, actual, ..
        }) => assert_eq!((declared, actual), (8, 7)),
        other => panic!("expected LengthMismatch, got {other:?}"),
    }
}

#[test]
fn service_list_ack_service_count_ceiling() {
    for (count, ok) in [(65535usize, true), (65536, false), (65537, false)] {
        let ack = ServiceListAck {
            length_form: dvb_ci::length::LengthForm::Minimal,
            version_number: 1,
            services: vec![reference(1, 1); count],
        };
        let mut out = vec![0u8; ack.serialized_len()];
        let result = ack.serialize_into(&mut out);
        match result {
            Err(Error::InvalidObject {
                what: "ServiceListAck",
                ..
            }) => assert!(!ok, "{count}"),
            Err(Error::LengthTooLarge(_)) => assert!(ok, "{count}"),
            other => panic!("count {count}: {other:?}"),
        }
    }
}

#[test]
fn service_desc_req_wire_and_return() {
    let req = ServiceDescReq {
        length_form: dvb_ci::length::LengthForm::Minimal,
        service: reference(0x0001, 0x0102),
    };
    let (n, buf) = serialize_len(&req);
    assert_eq!(n, 8);
    assert_eq!(n, req.serialized_len());
    assert_eq!(buf, [0x9F, 0x80, 0x05, 0x04, 0x00, 0x01, 0x01, 0x02]);
    assert_eq!(ServiceDescReq::parse(&buf).unwrap(), req);
}

fn service_desc_ack<'a>(descriptors: &'a [u8]) -> ServiceDescAck<'a> {
    ServiceDescAck {
        length_form: dvb_ci::length::LengthForm::Minimal,
        service: reference(0x0001, 0x0102),
        eit_schedule_flag: true,
        eit_present_following_flag: false,
        running_status: 2,
        free_ca_mode: true,
        descriptors,
        reserved: ReservedOnes::from_bits(0b11_1111),
    }
}

#[test]
fn service_desc_ack_12_bit_descriptor_loop() {
    let descriptors = vec![0xA5u8; 300];
    let ack = service_desc_ack(&descriptors);
    let (n, buf) = serialize_len(&ack);
    assert_eq!(n, 313);
    assert_eq!(n, ack.serialized_len());
    assert_eq!(&buf[..6], &[0x9F, 0x80, 0x06, 0x82, 0x01, 0x33]);
    assert_eq!(buf[10], 0xFE);
    assert_eq!(buf[11], 0x51);
    assert_eq!(buf[12], 0x2C);
    assert_eq!(ServiceDescAck::parse(&buf).unwrap(), ack);
}

#[test]
fn service_desc_ack_descriptor_loop_ceiling() {
    for (len, ok) in [(4095usize, true), (4096, false)] {
        let descriptors = vec![0xA5u8; len];
        let ack = service_desc_ack(&descriptors);
        let mut out = vec![0u8; ack.serialized_len()];
        let result = ack.serialize_into(&mut out);
        assert_eq!(result.is_ok(), ok, "descriptor len {len}");
        if ok {
            let n = result.unwrap();
            assert_eq!(n, 3 + 3 + 7 + 4095);
            assert_eq!(out[11], 0x5F);
            assert_eq!(out[12], 0xFF);
        }
    }
}

#[test]
fn service_desc_ack_truncated_loop_reports_declared_and_actual() {
    let wire = [
        0x9F, 0x80, 0x06, 0x09, 0x00, 0x01, 0x01, 0x02, 0x00, 0x00, 0x0A, 0xAA, 0xBB,
    ];
    match ServiceDescAck::parse(&wire).unwrap_err() {
        Error::LengthMismatch(LengthMismatch {
            declared, actual, ..
        }) => assert_eq!((declared, actual), (10, 2)),
        other => panic!("expected LengthMismatch, got {other:?}"),
    }
}

#[test]
fn get_service_req_and_ack_wire_and_return() {
    let req = GetServiceReq {
        length_form: dvb_ci::length::LengthForm::Minimal,
        service: Some(reference(0x0001, 0x0102)),
    };
    let (n, buf) = serialize_len(&req);
    assert_eq!(n, 8);
    assert_eq!(n, req.serialized_len());
    assert_eq!(buf, [0x9F, 0x80, 0x07, 0x04, 0x00, 0x01, 0x01, 0x02]);
    assert_eq!(GetServiceReq::parse(&buf).unwrap(), req);

    let ack = GetServiceAck {
        length_form: dvb_ci::length::LengthForm::Minimal,
        service: reference(0x0001, 0x0102),
        service_terminated: true,
        service_not_available: false,
        ca_service_flag: true,
        actual_service: 0x0304,
        reserved: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&ack);
    assert_eq!(n, 11);
    assert_eq!(n, ack.serialized_len());
    assert_eq!(
        buf,
        [
            0x9F, 0x80, 0x08, 0x07, 0x00, 0x01, 0x01, 0x02, 0x05, 0x03, 0x04
        ]
    );
    assert_eq!(GetServiceAck::parse(&buf).unwrap(), ack);
}

#[test]
fn copy_protection_objects_wire_and_return() {
    let query = CpQuery {
        length_form: dvb_ci::length::LengthForm::Minimal,
        copy_protection_id: 0x0000_1B67,
    };
    let (n, buf) = serialize_len(&query);
    assert_eq!(n, 7);
    assert_eq!(n, query.serialized_len());
    assert_eq!(buf, [0x9F, 0x80, 0x00, 0x03, 0x00, 0x1B, 0x67]);
    assert_eq!(CpQuery::parse(&buf).unwrap(), query);

    let reply = CpReply {
        length_form: dvb_ci::length::LengthForm::Minimal,
        copy_protection_id: 0x0000_1B67,
        status: CpStatus::Active,
    };
    let (n, buf) = serialize_len(&reply);
    assert_eq!(n, 8);
    assert_eq!(n, reply.serialized_len());
    assert_eq!(buf, [0x9F, 0x80, 0x01, 0x04, 0x00, 0x1B, 0x67, 0x02]);
    assert_eq!(CpReply::parse(&buf).unwrap(), reply);

    let command = CpCommand {
        length_form: dvb_ci::length::LengthForm::Minimal,
        copy_protection_id: 0x0000_1B67,
        command_bytes: &[0x77, 0x88],
    };
    assert_eq!(command.serialized_len(), 9);
    let (n, buf) = serialize_len(&command);
    assert_eq!(n, 9);
    assert_eq!(buf, [0x9F, 0x80, 0x02, 0x05, 0x00, 0x1B, 0x67, 0x77, 0x88]);
    assert_eq!(CpCommand::parse(&buf).unwrap(), command);
}

#[test]
fn event_request_ack_wire_and_labels() {
    let ack = EventRequestAck {
        length_form: dvb_ci::length::LengthForm::Minimal,
        event_type: EventType::Timer,
        reply: EventReply::ResourcesConsumed,
    };
    let (n, buf) = serialize_len(&ack);
    assert_eq!(n, 6);
    assert_eq!(n, ack.serialized_len());
    assert_eq!(buf, [0x9F, 0x80, 0x01, 0x02, 0x00, 0x02]);
    assert_eq!(EventRequestAck::parse(&buf).unwrap(), ack);
    assert_eq!(EventType::Timer.name(), "Timer");
    assert_eq!(EventType::Reserved(0x7F).name(), "reserved");
}

#[test]
fn resource_manager_v2_module_id_returns() {
    let send = ModuleIdSend {
        length_form: dvb_ci::length::LengthForm::Minimal,
        module_id: 0x2A,
        reserved: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&send);
    assert_eq!(n, 5);
    assert_eq!(n, send.serialized_len());
    assert_eq!(buf, [0x9F, 0x80, 0x13, 0x01, 0x2A]);
    assert_eq!(ModuleIdSend::parse(&buf).unwrap(), send);

    let command = ModuleIdCommand {
        length_form: dvb_ci::length::LengthForm::Minimal,
        command: ModuleIdCommandKind::Acknowledgement,
        module_id: 0x2A,
        reserved: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&command);
    assert_eq!(n, 6);
    assert_eq!(n, command.serialized_len());
    assert_eq!(buf, [0x9F, 0x80, 0x14, 0x02, 0x01, 0x2A]);
    assert_eq!(ModuleIdCommand::parse(&buf).unwrap(), command);
}

fn output(port: u8, desc: &[u8]) -> OutputSignal<'_> {
    OutputSignal {
        out_port_id: port,
        out_signal_desc: desc,
    }
}

#[test]
fn selection_information_truncated_prefixes_are_not_misparsed() {
    let first = vec![0xA1u8; 3];
    let info = SelectionInformation {
        time: &[0xE0, 0x01, 0x02, 0x03, 0x04],
        ports: vec![InPortDescription {
            in_port_id: 1,
            in_signal_desc: &[0xAA, 0xBB, 0xCC],
            outputs: vec![output(2, &first)],
        }],
    };
    let (_, buf) = serialize_len(&info);
    for cut in 0..buf.len() {
        if let Ok(parsed) = SelectionInformation::parse(&buf[..cut]) {
            assert_ne!(parsed, info, "cut {cut} parsed as the full value");
        }
    }
    match SelectionInformation::parse(&buf[..8]) {
        Err(Error::BufferTooShort(broadcast_core::error::BufferTooShort {
            need, have, ..
        })) => assert_eq!((need, have), (7, 3)),
        other => panic!("expected BufferTooShort(7,3), got {other:?}"),
    }
}

#[test]
fn port_profile_truncated_prefixes_are_not_misparsed() {
    let profile = PortProfile {
        receiver_identification: &[0x10, 0x20, 0x30],
        ports: vec![PortDescription {
            in_port_id: 1,
            in_port_desc: &[0x11, 0x12, 0x13, 0x14, 0x15],
            out_port_id: 2,
            out_port_desc: &[0x21, 0x22],
        }],
    };
    let (_, buf) = serialize_len(&profile);
    for cut in 0..buf.len() {
        if let Ok(parsed) = PortProfile::parse(&buf[..cut]) {
            assert_ne!(parsed, profile, "cut {cut} parsed as the full value");
        }
    }
    match PortProfile::parse(&buf[..7]) {
        Err(Error::BufferTooShort(broadcast_core::error::BufferTooShort {
            need, have, ..
        })) => assert_eq!((need, have), (9, 3)),
        other => panic!("expected BufferTooShort(9,3), got {other:?}"),
    }
}

#[test]
fn selection_information_12_bit_output_length() {
    let first = vec![0xA1u8; 150];
    let second = vec![0xA2u8; 146];
    let info = SelectionInformation {
        time: &[0xE0, 0x01, 0x02, 0x03, 0x04],
        ports: vec![InPortDescription {
            in_port_id: 1,
            in_signal_desc: &[0xAA, 0xBB, 0xCC],
            outputs: vec![output(2, &first), output(3, &second)],
        }],
    };
    let (n, buf) = serialize_len(&info);
    assert_eq!(n, 312);
    assert_eq!(n, info.serialized_len());
    assert_eq!(n, buf.len());
    assert_eq!(
        &buf[..10],
        &[0xE0, 0x01, 0x02, 0x03, 0x04, 0x01, 0x03, 0xAA, 0xBB, 0xCC]
    );
    assert_eq!(buf[10], 0x01);
    assert_eq!(buf[11], 0x2C);
    assert_eq!(buf[12], 0x02);
    assert_eq!(buf[13], 0x96);
    assert_eq!(SelectionInformation::parse(&buf).unwrap(), info);
}

#[test]
fn selection_information_length_outputs_ceiling() {
    for (total, ok) in [(4095usize, true), (4096, false)] {
        let mut descs = Vec::new();
        let mut remaining = total;
        while remaining > 0 {
            let desc_len = if remaining >= 241 { 239 } else { remaining - 2 };
            descs.push(vec![0xA5u8; desc_len]);
            remaining -= 2 + desc_len;
        }
        let outputs: Vec<OutputSignal<'_>> = descs
            .iter()
            .enumerate()
            .map(|(i, d)| output(i as u8, d))
            .collect();
        let info = SelectionInformation {
            time: &[0, 0, 0, 0, 0],
            ports: vec![InPortDescription {
                in_port_id: 1,
                in_signal_desc: &[],
                outputs,
            }],
        };
        let mut out = vec![0u8; info.serialized_len()];
        let result = info.serialize_into(&mut out);
        assert_eq!(result.is_ok(), ok, "length_outputs {total}");
        if ok {
            assert_eq!(result.unwrap(), 9 + 4095);
            assert_eq!(out[7], 0x0F);
        }
    }
}

#[test]
fn selection_information_string_ceilings() {
    let too_long = vec![0xAAu8; 256];
    let mut info = SelectionInformation {
        time: &[0, 0, 0, 0, 0],
        ports: vec![InPortDescription {
            in_port_id: 1,
            in_signal_desc: &too_long,
            outputs: Vec::new(),
        }],
    };
    let mut out = vec![0u8; info.serialized_len()];
    assert!(matches!(
        info.serialize_into(&mut out),
        Err(Error::InvalidObject { .. })
    ));
    let max = vec![0xAAu8; 255];
    info.ports[0].in_signal_desc = &max;
    let mut out = vec![0u8; info.serialized_len()];
    assert!(info.serialize_into(&mut out).is_ok());

    let too_long = vec![0xBBu8; 256];
    let info = SelectionInformation {
        time: &[0, 0, 0, 0, 0],
        ports: vec![InPortDescription {
            in_port_id: 1,
            in_signal_desc: &[],
            outputs: vec![output(2, &too_long)],
        }],
    };
    let mut out = vec![0u8; info.serialized_len()];
    assert!(matches!(
        info.serialize_into(&mut out),
        Err(Error::InvalidObject { .. })
    ));
    let max = vec![0xBBu8; 255];
    let info = SelectionInformation {
        time: &[0, 0, 0, 0, 0],
        ports: vec![InPortDescription {
            in_port_id: 1,
            in_signal_desc: &[],
            outputs: vec![output(2, &max)],
        }],
    };
    let mut out = vec![0u8; info.serialized_len()];
    assert!(info.serialize_into(&mut out).is_ok());
}

#[test]
fn port_profile_wire_length_and_ceiling() {
    let profile = PortProfile {
        receiver_identification: &[0x10, 0x20, 0x30],
        ports: vec![PortDescription {
            in_port_id: 1,
            in_port_desc: &[0x11, 0x12, 0x13, 0x14, 0x15],
            out_port_id: 2,
            out_port_desc: &[0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27],
        }],
    };
    assert_eq!(profile.serialized_len(), 20);
    let (n, buf) = serialize_len(&profile);
    assert_eq!(n, 20);
    assert_eq!(
        buf,
        [
            0x03, 0x10, 0x20, 0x30, 0x01, 0x05, 0x11, 0x12, 0x13, 0x14, 0x15, 0x02, 0x07, 0x21,
            0x22, 0x23, 0x24, 0x25, 0x26, 0x27
        ]
    );
    assert_eq!(PortProfile::parse(&buf).unwrap(), profile);

    let max = vec![0x11u8; 255];
    let over = vec![0x11u8; 256];
    let base = PortProfile {
        receiver_identification: &max,
        ports: vec![PortDescription {
            in_port_id: 1,
            in_port_desc: &max,
            out_port_id: 2,
            out_port_desc: &max,
        }],
    };
    let mut out = vec![0u8; base.serialized_len()];
    assert!(base.serialize_into(&mut out).is_ok());
    let bad_receiver = PortProfile {
        receiver_identification: &over,
        ..base.clone()
    };
    let mut out = vec![0u8; bad_receiver.serialized_len()];
    assert!(matches!(
        bad_receiver.serialize_into(&mut out),
        Err(Error::InvalidObject { .. })
    ));
    let bad_in = PortProfile {
        ports: vec![PortDescription {
            in_port_id: 1,
            in_port_desc: &over,
            out_port_id: 2,
            out_port_desc: &max,
        }],
        ..base.clone()
    };
    let mut out = vec![0u8; bad_in.serialized_len()];
    assert!(matches!(
        bad_in.serialize_into(&mut out),
        Err(Error::InvalidObject { .. })
    ));
    let bad_out = PortProfile {
        ports: vec![PortDescription {
            in_port_id: 1,
            in_port_desc: &max,
            out_port_id: 2,
            out_port_desc: &over,
        }],
        ..base
    };
    let mut out = vec![0u8; bad_out.serialized_len()];
    assert!(matches!(
        bad_out.serialize_into(&mut out),
        Err(Error::InvalidObject { .. })
    ));
}

#[test]
fn viewed_service_truncated_component_list_is_rejected() {
    let short = [0x12, 0x34, 0x03, 0xAA];
    assert!(ViewedService::parse(&short).is_err());
}

#[test]
fn viewed_service_wire_and_ceiling() {
    let viewed = ViewedService {
        service_id: 0x1234,
        component_tags: vec![1, 2, 3],
    };
    let (n, buf) = serialize_len(&viewed);
    assert_eq!(n, 6);
    assert_eq!(n, viewed.serialized_len());
    assert_eq!(buf, [0x12, 0x34, 0x03, 0x01, 0x02, 0x03]);
    assert_eq!(ViewedService::parse(&buf).unwrap(), viewed);

    let max = ViewedService {
        service_id: 1,
        component_tags: vec![0xAA; 255],
    };
    let mut out = vec![0u8; max.serialized_len()];
    assert!(max.serialize_into(&mut out).is_ok());
    let over = ViewedService {
        service_id: 1,
        component_tags: vec![0xAA; 256],
    };
    let mut out = vec![0u8; over.serialized_len()];
    assert!(matches!(
        over.serialize_into(&mut out),
        Err(Error::InvalidObject { .. })
    ));
}

#[test]
fn activation_state_labels() {
    assert_eq!(ActivationState::StandbyActive.name(), "Standby-active");
    assert_eq!(ActivationState::On.name(), "On");
    assert_eq!(ActivationState::Reserved(0).name(), "reserved");
}
