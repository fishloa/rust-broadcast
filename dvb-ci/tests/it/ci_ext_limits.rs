//! CI-ext limit tables (audit 23): the 12-bit EIT loop lengths, 4095-byte
//! ceilings, 255-byte application strings and the dispatch of the EIT objects
//! (ETSI TS 101 699 §6.1.3.3 Tab. 32-34; §6.5 Tab. 62-66).

use broadcast_common::error::LengthMismatch;
use broadcast_common::{Parse, Serialize};
use dvb_ci::Error;
use dvb_ci::ci_ext::application_mmi::{AckCode, ApplicationMmiApdu, RequestStart};
use dvb_ci::ci_ext::broadcast_service_gateway::{
    BroadcastServiceGatewayApdu, EitEvent, EitResponseCode, EitSectionAck, EitSectionReq,
};
use dvb_ci::reserved::ReservedZero;

fn serialize_len<T: Serialize<Error = Error>>(value: &T) -> (usize, Vec<u8>) {
    let mut buf = vec![0u8; value.serialized_len()];
    let n = value.serialize_into(&mut buf).expect("serialize");
    (n, buf)
}

fn eit_event<'a>(id: u16, descriptors: &'a [u8]) -> EitEvent<'a> {
    EitEvent {
        event_id: id,
        start_time: [0x05, 0x06, 0x07, 0x08, 0x09],
        duration: [0x04, 0x05, 0x06],
        running_status: 5,
        free_ca_mode: true,
        descriptors,
    }
}

#[test]
fn eit_section_req_exact_bytes_and_return() {
    let req = EitSectionReq {
        length_form: dvb_ci::length::LengthForm::Minimal,
        table_id: 0x004E,
        service_id: 0x0064,
        section_number: 0x01,
        original_network_id: 0x0001,
        ok_to_disrupt_service: true,
        reserved: ReservedZero::from_bits(0x7F),
    };
    let (n, buf) = serialize_len(&req);
    assert_eq!(n, 12);
    assert_eq!(n, req.serialized_len());
    assert_eq!(
        buf,
        [
            0x9F, 0x80, 0x10, 0x08, 0x00, 0x4E, 0x00, 0x64, 0x01, 0x00, 0x01, 0xFF
        ]
    );
    assert_eq!(EitSectionReq::parse(&buf).unwrap(), req);
}

#[test]
fn eit_section_ack_two_events_carry_the_12_bit_lengths() {
    let descriptors = vec![0xA5u8; 300];
    let ack = EitSectionAck {
        length_form: dvb_ci::length::LengthForm::Minimal,
        response_code: EitResponseCode::SectionFound,
        events: vec![
            EitEvent {
                event_id: 0x0102,
                start_time: [0x00, 0x11, 0x22, 0x33, 0x44],
                duration: [0x01, 0x02, 0x03],
                running_status: 3,
                free_ca_mode: false,
                descriptors: &[0xAA, 0xBB],
            },
            eit_event(0x0203, &descriptors),
        ],
        reserved: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&ack);
    assert_eq!(n, 334);
    assert_eq!(n, ack.serialized_len());
    assert_eq!(n, buf.len());
    assert_eq!(&buf[..6], &[0x9F, 0x80, 0x11, 0x82, 0x01, 0x48]);
    assert_eq!(buf[6], 0x21);
    assert_eq!(buf[7], 0x46);
    assert_eq!(
        &buf[8..22],
        &[
            0x01, 0x02, 0x00, 0x11, 0x22, 0x33, 0x44, 0x01, 0x02, 0x03, 0x60, 0x02, 0xAA, 0xBB
        ]
    );
    assert_eq!(buf[32], 0xB1);
    assert_eq!(buf[33], 0x2C);
    assert!(buf[34..].iter().all(|&b| b == 0xA5));
    assert_eq!(EitSectionAck::parse(&buf).unwrap(), ack);
}

#[test]
fn eit_section_ack_event_loop_ceiling() {
    let within = vec![0xA5u8; 4083];
    let ack = EitSectionAck {
        length_form: dvb_ci::length::LengthForm::Minimal,
        response_code: EitResponseCode::SectionFound,
        events: vec![eit_event(1, &within)],
        reserved: ReservedZero::from_bits(0),
    };
    assert_eq!(ack.serialized_len(), 3 + 3 + 2 + 4095);
    let (n, buf) = serialize_len(&ack);
    assert_eq!(n, 4103);
    assert_eq!(buf[6], 0x2F);
    assert_eq!(buf[7], 0xFF);
    assert_eq!(EitSectionAck::parse(&buf).unwrap(), ack);

    let over = vec![0xA5u8; 4084];
    let too_long = EitSectionAck {
        events: vec![eit_event(1, &over)],
        ..ack
    };
    let mut out = vec![0u8; too_long.serialized_len()];
    assert!(matches!(
        too_long.serialize_into(&mut out),
        Err(Error::InvalidObject {
            what: "EITSectionAck",
            ..
        })
    ));
}

#[test]
fn eit_section_ack_truncated_loop_reports_declared_and_actual() {
    let wire = [0x9F, 0x80, 0x11, 0x05, 0x20, 0x0A, 0x00, 0x00, 0x00];
    match EitSectionAck::parse(&wire).unwrap_err() {
        Error::LengthMismatch(LengthMismatch {
            declared, actual, ..
        }) => assert_eq!((declared, actual), (10, 3)),
        other => panic!("expected LengthMismatch, got {other:?}"),
    }
}

#[test]
fn eit_section_ack_truncated_event_header_is_rejected() {
    let wire = [
        0x9F, 0x80, 0x11, 0x0C, 0x20, 0x0A, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    assert!(matches!(
        EitSectionAck::parse(&wire),
        Err(Error::InvalidObject {
            what: "EITSectionAck event",
            ..
        })
    ));
}

#[test]
fn eit_section_ack_truncated_event_descriptors_report_declared_and_actual() {
    let wire = [
        0x9F, 0x80, 0x11, 0x10, 0x20, 0x0E, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x05, 0, 0,
    ];
    match EitSectionAck::parse(&wire).unwrap_err() {
        Error::LengthMismatch(LengthMismatch {
            declared, actual, ..
        }) => assert_eq!((declared, actual), (5, 2)),
        other => panic!("expected LengthMismatch, got {other:?}"),
    }
}

#[test]
fn request_start_accepts_255_byte_strings() {
    let domain = vec![0x11u8; 255];
    let object = vec![0x22u8; 1];
    let start = RequestStart {
        length_form: dvb_ci::length::LengthForm::Minimal,
        app_domain_identifier: &domain,
        initial_object: &object,
    };
    let (n, buf) = serialize_len(&start);
    assert_eq!(n, 264);
    assert_eq!(n, start.serialized_len());
    assert_eq!(&buf[..6], &[0x9F, 0x80, 0x00, 0x82, 0x01, 0x02]);
    assert_eq!(buf[6], 255);
    assert_eq!(buf[7], 1);
    assert_eq!(RequestStart::parse(&buf).unwrap(), start);

    let max_object = vec![0x22u8; 255];
    let start = RequestStart {
        length_form: dvb_ci::length::LengthForm::Minimal,
        app_domain_identifier: &[0x11],
        initial_object: &max_object,
    };
    let (n, buf) = serialize_len(&start);
    assert_eq!(n, 264);
    assert_eq!(buf[6], 1);
    assert_eq!(buf[7], 255);
    assert_eq!(RequestStart::parse(&buf).unwrap(), start);

    let too_long = vec![0x11u8; 256];
    let mut over = start.clone();
    over.app_domain_identifier = &too_long;
    let mut out = vec![0u8; over.serialized_len()];
    assert!(matches!(
        over.serialize_into(&mut out),
        Err(Error::InvalidObject {
            what: "RequestStart",
            ..
        })
    ));
    let mut over_object = start.clone();
    over_object.initial_object = &too_long;
    let mut out = vec![0u8; over_object.serialized_len()];
    assert!(matches!(
        over_object.serialize_into(&mut out),
        Err(Error::InvalidObject {
            what: "RequestStart",
            ..
        })
    ));
}

#[test]
fn ack_code_labels() {
    let table = [
        (AckCode::Ok, "OK"),
        (AckCode::WrongApi, "Wrong API"),
        (AckCode::ApiBusy, "API busy"),
        (
            AckCode::DomainSpecificApiBusy(0x80),
            "Domain specific API busy",
        ),
        (AckCode::Reserved(0x04), "reserved"),
    ];
    for (code, label) in table {
        assert_eq!(code.name(), label);
    }
}

#[test]
fn broadcast_service_gateway_dispatches_every_eit_tag() {
    let req = [
        0x9F, 0x80, 0x10, 0x08, 0x00, 0x4E, 0x00, 0x64, 0x01, 0x00, 0x01, 0x00,
    ];
    assert!(matches!(
        BroadcastServiceGatewayApdu::parse(&req).unwrap(),
        BroadcastServiceGatewayApdu::EitSectionReq(_)
    ));
    let ack = [0x9F, 0x80, 0x11, 0x02, 0x20, 0x00];
    assert!(matches!(
        BroadcastServiceGatewayApdu::parse(&ack).unwrap(),
        BroadcastServiceGatewayApdu::EitSectionAck(_)
    ));
    assert!(matches!(
        ApplicationMmiApdu::parse(&[0x9F, 0x80, 0x01, 0x01, 0x01]).unwrap(),
        ApplicationMmiApdu::RequestStartAck(_)
    ));
}
