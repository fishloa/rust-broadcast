use broadcast_common::error::{BufferTooShort, FieldOverflow};
use broadcast_common::{Parse, Serialize};
use ule::{Error, Sndu, TypeField};

#[test]
fn a_three_byte_sndu_is_a_missing_base_header() {
    assert_eq!(
        Sndu::parse(&[0u8; 3]).unwrap_err(),
        Error::BufferTooShort(BufferTooShort {
            need: 4,
            have: 3,
            what: "SNDU base header",
        })
    );
}

#[test]
fn a_pdu_beyond_the_15_bit_length_field_is_a_field_overflow() {
    let pdu = vec![0u8; 0x8000];
    let sndu = Sndu::new(TypeField::EtherType(0x0800), None, &pdu);
    let length = sndu.length_field() as u64;
    let mut out = vec![0u8; sndu.serialized_len()];
    assert_eq!(
        sndu.serialize_into(&mut out).unwrap_err(),
        Error::FieldOverflow(FieldOverflow {
            field: "Length",
            value: length,
            max: 0x7FFF,
        })
    );
}
