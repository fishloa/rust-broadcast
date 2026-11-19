//! `TypeField` boundary/labels (RFC 4326 §4.4) and extension-header H-Type
//! wire values / lengths (RFC 4326 §5, RFC 5163 §3).

use ule::{
    ExtensionHeader, H_TYPE_BRIDGED_FRAME, H_TYPE_EXT_PADDING, H_TYPE_PDU_CONCAT, H_TYPE_TEST_SNDU,
    H_TYPE_TIMESTAMP, H_TYPE_TS_CONCAT, MandatoryHType, OptionalHType, PayloadChain, TypeField,
};

#[test]
fn type_field_ethertype_boundary_is_inclusive() {
    assert_eq!(
        TypeField::from_u16(0x05FF),
        TypeField::NextHeader {
            h_len: 5,
            h_type: 0xFF,
        }
    );
    assert_eq!(TypeField::from_u16(0x0600), TypeField::EtherType(0x0600));
}

#[test]
fn type_field_labels_name_each_kind() {
    assert_eq!(
        TypeField::NextHeader {
            h_len: 0,
            h_type: H_TYPE_TEST_SNDU,
        }
        .name(),
        "next-header"
    );
    assert_eq!(TypeField::EtherType(0x0800).name(), "ethertype");
}

#[test]
fn mandatory_h_types_encode_their_registered_values() {
    let cases = [
        (MandatoryHType::TestSndu, H_TYPE_TEST_SNDU, 0u8),
        (MandatoryHType::BridgedFrame, H_TYPE_BRIDGED_FRAME, 1u8),
        (MandatoryHType::TsConcat, H_TYPE_TS_CONCAT, 2u8),
        (MandatoryHType::PduConcat, H_TYPE_PDU_CONCAT, 3u8),
    ];
    for (typed, raw, literal) in cases {
        assert_eq!(raw, literal);
        assert_eq!(typed.to_u8(), literal);
        assert_eq!(MandatoryHType::from_u8(literal), typed);
    }
    assert_eq!(MandatoryHType::Other(0x42).to_u8(), 0x42);
}

#[test]
fn optional_h_types_encode_their_registered_values() {
    let cases = [
        (OptionalHType::ExtPadding, H_TYPE_EXT_PADDING, 0u8),
        (OptionalHType::TimeStamp, H_TYPE_TIMESTAMP, 1u8),
    ];
    for (typed, raw, literal) in cases {
        assert_eq!(raw, literal);
        assert_eq!(typed.to_u8(), literal);
        assert_eq!(OptionalHType::from_u8(literal), typed);
    }
    assert_eq!(OptionalHType::Other(0x42).to_u8(), 0x42);
}

#[test]
fn a_mandatory_wire_len_includes_its_two_byte_type_field() {
    let header = ExtensionHeader::Mandatory {
        h_type: H_TYPE_TEST_SNDU,
        body: vec![0u8; 4],
    };
    assert_eq!(header.wire_len(), 6);
}

#[test]
fn an_optional_header_ending_at_the_data_end_is_accepted() {
    let first = TypeField::NextHeader {
        h_len: 2,
        h_type: 0x42,
    };
    let data = [0x11u8, 0x22, 0x08, 0x00];
    let chain = PayloadChain::parse(first, &data).expect("final type field is the last word");
    assert_eq!(chain.headers.len(), 1);
    assert_eq!(chain.final_type, TypeField::EtherType(0x0800));
    assert!(chain.pdu.is_empty());
}
