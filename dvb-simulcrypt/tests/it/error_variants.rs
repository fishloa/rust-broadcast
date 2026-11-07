use broadcast_common::error::{BufferTooShort, FieldOverflow};
use broadcast_common::Serialize;
use dvb_simulcrypt::{Error, Interface, Parameter, SimulcryptMessage};

fn fixture() -> Vec<u8> {
    test_fixtures::read("simulcrypt/cw-provision.bin")
}

#[test]
fn a_truncated_header_is_a_missing_generic_message_header() {
    assert_eq!(
        SimulcryptMessage::parse_on(Interface::EcmgScs, &[0x03, 0x00]).unwrap_err(),
        Error::BufferTooShort(BufferTooShort {
            need: 5,
            have: 2,
            what: "generic_message header",
        })
    );
}

#[test]
fn a_parameter_beyond_the_16_bit_message_length_is_a_field_overflow() {
    let data = fixture();
    let mut msg = SimulcryptMessage::parse_on(Interface::EcmgScs, &data).expect("fixture");
    let big = vec![0u8; 0x1_0000];
    let ptype = msg.parameters[0].ptype;
    msg.parameters = vec![Parameter::new(ptype, &big)];
    let body_len = 4 + big.len();
    let mut out = vec![0u8; msg.serialized_len()];
    assert_eq!(
        msg.serialize_into(&mut out).unwrap_err(),
        Error::FieldOverflow(FieldOverflow {
            field: "message_length",
            value: body_len as u64,
            max: 0xFFFF,
        })
    );
}
