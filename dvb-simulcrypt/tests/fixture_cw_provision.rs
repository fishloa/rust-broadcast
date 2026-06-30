//! Hand-built ECMG⇔SCS `CW_provision` fixture (TS 103 197 §5.5.7): parse the
//! committed `cw_provision.bin`, assert its decoded fields, and verify a
//! byte-exact round-trip. The CW inside `CP_CW_combination` is opaque.

use std::fs;

use broadcast_common::traits::Serialize;
use dvb_simulcrypt::{
    EcmgScsMessageType, EcmgScsParameterType, Interface, MessageType, ParameterType,
    SimulcryptMessage,
};

fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dvb-simulcrypt/cw_provision.bin"
    );
    fs::read(path).expect("fixture cw_provision.bin must be committed")
}

#[test]
fn parses_cw_provision_fields() {
    let data = fixture();
    assert_eq!(data.len(), 37);

    let msg = SimulcryptMessage::parse_on(Interface::EcmgScs, &data).expect("must parse");

    assert_eq!(msg.protocol_version, 0x03);
    assert_eq!(msg.interface(), Interface::EcmgScs);
    assert_eq!(
        msg.message_type,
        MessageType::EcmgScs(EcmgScsMessageType::CwProvision)
    );
    assert_eq!(msg.message_type.to_u16(), 0x0201);

    // body_len matches the 0x0020 message_length in the header.
    assert_eq!(msg.body_len(), 0x20);
    assert_eq!(data[3..5], [0x00, 0x20]);

    // Four parameters in wire order.
    assert_eq!(msg.parameters.len(), 4);
    assert_eq!(
        msg.parameters[0].ptype,
        ParameterType::EcmgScs(EcmgScsParameterType::EcmChannelId)
    );
    assert_eq!(msg.parameters[0].value, &[0x00, 0x2A]);
    assert_eq!(
        msg.parameters[1].ptype,
        ParameterType::EcmgScs(EcmgScsParameterType::EcmStreamId)
    );
    assert_eq!(msg.parameters[1].value, &[0x00, 0x01]);
    assert_eq!(
        msg.parameters[2].ptype,
        ParameterType::EcmgScs(EcmgScsParameterType::CpNumber)
    );
    assert_eq!(msg.parameters[2].value, &[0x12, 0x34]);

    // CP_CW_combination: CP (2B) + opaque CW (8B).
    let cpcw = msg
        .find(ParameterType::EcmgScs(
            EcmgScsParameterType::CpCwCombination,
        ))
        .expect("CP_CW_combination present");
    assert_eq!(cpcw.value.len(), 10);
    assert_eq!(&cpcw.value[..2], &[0x12, 0x34], "CP");
    assert_eq!(
        &cpcw.value[2..],
        &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
        "opaque CW"
    );
}

#[test]
fn cw_provision_byte_exact_round_trip() {
    let data = fixture();
    let msg = SimulcryptMessage::parse_on(Interface::EcmgScs, &data).unwrap();

    let mut out = vec![0u8; msg.serialized_len()];
    let n = msg.serialize_into(&mut out).unwrap();
    assert_eq!(n, data.len());
    assert_eq!(out, data, "serialize must be byte-identical to the fixture");

    // serialize -> parse -> equal.
    assert_eq!(
        SimulcryptMessage::parse_on(Interface::EcmgScs, &out).unwrap(),
        msg
    );
}

#[test]
fn truncated_parameter_length_rejected() {
    let mut data = fixture();
    // The CP_CW_combination's parameter_length sits at offset 25..27 (0x000A).
    // Bump it past the remaining body to force a TruncatedParameter error.
    let last = data.len() - 1;
    data[26] = 0xFF; // low byte of that parameter_length
    let err = SimulcryptMessage::parse_on(Interface::EcmgScs, &data).unwrap_err();
    assert!(
        matches!(err, dvb_simulcrypt::Error::TruncatedParameter { .. }),
        "over-long parameter_length must be rejected, got {err:?}"
    );
    let _ = last;
}

#[test]
fn message_length_overrun_rejected() {
    let mut data = fixture();
    data[4] = 0xFF; // inflate message_length beyond the buffer
    let err = SimulcryptMessage::parse_on(Interface::EcmgScs, &data).unwrap_err();
    assert!(matches!(
        err,
        dvb_simulcrypt::Error::InvalidMessageLength { .. }
    ));
}
