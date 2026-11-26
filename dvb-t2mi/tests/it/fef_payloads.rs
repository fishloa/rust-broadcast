//! FEF / arbitrary-cells / aux-IQ / P2-bias payload frames — ETSI TS 102 773
//! §5.2.3, §5.2.6, §5.2.9–§5.2.12.
//!
//! Each value puts distinct data in every sub-field (including the high bits of
//! 22-bit fields) so a shifted, masked or dropped field cannot round-trip.

use broadcast_core::len::FieldOverflow;
use broadcast_core::{Parse, Serialize};
use dvb_t2mi::Error;
use dvb_t2mi::payload::{
    ArbitraryCellsPayload, FefCompositePayload, FefIqPayload, FefNullPayload, FefSubPartPayload,
    FrequencySource, L1CurrentPayload, P2BiasPayload, PrbsType, S1Field, SubpartVariety,
};

macro_rules! assert_framed {
    ($value:expr, $parse:path, $bytes:expr) => {{
        let value = &$value;
        let bytes: &[u8] = $bytes;
        assert_eq!(value.serialized_len(), bytes.len(), "serialized_len");
        let mut buf = vec![0u8; value.serialized_len()];
        value.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes, "serialized bytes");
        let parsed = $parse(bytes).unwrap();
        assert_eq!(&parsed, value, "parsed value");
    }};
}

#[test]
fn arbitrary_cells_round_trips_every_field() {
    let data = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77];
    let mut bytes = vec![0x42, 0xAB, 0xCD, 0x00, 0x00, 0x3F, 0xFF, 0xFF];
    bytes.extend_from_slice(&data);
    let payload = ArbitraryCellsPayload {
        frame_idx: 0x42,
        tx_identifier: 0xABCD,
        start_cell_address: 0x3F_FFFF,
        arbitrary_cell_data: &data,
    };
    assert_framed!(payload, ArbitraryCellsPayload::parse, &bytes);
}

#[test]
fn arbitrary_cells_cell_address_overflow_is_rejected() {
    let mut payload = ArbitraryCellsPayload {
        frame_idx: 0,
        tx_identifier: 0,
        start_cell_address: 0x3F_FFFF,
        arbitrary_cell_data: &[],
    };
    let mut buf = [0u8; 8];
    assert!(payload.serialize_into(&mut buf).is_ok());
    payload.start_cell_address = 0x40_0000;
    assert_eq!(
        payload.serialize_into(&mut buf).err(),
        Some(Error::FieldOverflow(FieldOverflow {
            field: "start_cell_address",
            value: 0x40_0000,
            max: 0x3F_FFFF,
        }))
    );
}

#[test]
fn arbitrary_cells_rfu_reports_the_combined_value() {
    // byte 4 carries RFU bits [9:2] of the 18-bit reserved field.
    let buf = [0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00];
    assert_eq!(
        ArbitraryCellsPayload::parse(&buf).err(),
        Some(Error::ReservedBits(broadcast_core::error::ReservedBits {
            field: "18-bit RFU",
            expected: 0,
            found: 4,
        }))
    );
}

#[test]
fn fef_composite_round_trips_every_field() {
    let bytes = [0x0A, 0x1A, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34];
    let payload = FefCompositePayload {
        fef_idx: 0x0A,
        s1_field: S1Field::V1,
        s2_field: 0x0A,
        num_subparts: 0x1234,
    };
    assert_framed!(payload, FefCompositePayload::parse, &bytes);
    assert!(!payload.to_string().is_empty());
}

#[test]
fn fef_composite_rejects_each_rfu2_byte_on_its_own() {
    for i in 2..=5 {
        let mut buf = [0u8; 8];
        buf[i] = 0x01;
        assert!(
            matches!(
                FefCompositePayload::parse(&buf),
                Err(Error::ReservedBits(_))
            ),
            "rfu2 byte {i} must be rejected"
        );
    }
}

#[test]
fn fef_composite_s2_field_is_a_four_bit_field() {
    let mut payload = FefCompositePayload {
        fef_idx: 0,
        s1_field: S1Field::V0,
        s2_field: 0x0F,
        num_subparts: 0,
    };
    let mut buf = [0u8; 8];
    assert!(payload.serialize_into(&mut buf).is_ok());
    payload.s2_field = 0x10;
    assert!(matches!(
        payload.serialize_into(&mut buf),
        Err(Error::FieldOverflow(_))
    ));
}

#[test]
fn fef_iq_round_trips_every_field() {
    let data = [0xDEu8, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03];
    let mut bytes = vec![0x05, 0x00, 0x2A];
    bytes.extend_from_slice(&data);
    let payload = FefIqPayload {
        fef_idx: 0x05,
        s1_field: S1Field::V2,
        s2_field: 0x0A,
        fef_part_data: &data,
    };
    assert_framed!(payload, FefIqPayload::parse, &bytes);
    assert!(!payload.to_string().is_empty());
}

#[test]
fn fef_iq_rfu_reports_the_combined_nine_bit_value() {
    // The nine reserved bits are byte 1 (high 8) then byte 2 bit 7 (lowest).
    let buf = [0x00, 0x00, 0x80];
    assert_eq!(
        FefIqPayload::parse(&buf).err(),
        Some(Error::ReservedBits(broadcast_core::error::ReservedBits {
            field: "9-bit rfu",
            expected: 0,
            found: 1,
        }))
    );
}

#[test]
fn fef_iq_s2_field_is_a_four_bit_field() {
    let mut payload = FefIqPayload {
        fef_idx: 0,
        s1_field: S1Field::V0,
        s2_field: 0x0F,
        fef_part_data: &[],
    };
    let mut buf = [0u8; 3];
    assert!(payload.serialize_into(&mut buf).is_ok());
    payload.s2_field = 0x10;
    assert!(matches!(
        payload.serialize_into(&mut buf),
        Err(Error::FieldOverflow(_))
    ));
}

#[test]
fn fef_null_round_trips_every_field() {
    let bytes = [0x05, 0x00, 0x1A];
    let payload = FefNullPayload {
        fef_idx: 0x05,
        s1_field: S1Field::V1,
        s2_field: 0x0A,
    };
    assert_eq!(payload.serialized_len(), 3);
    assert_framed!(payload, FefNullPayload::parse, &bytes);
}

#[test]
fn fef_null_rfu_reports_the_combined_nine_bit_value() {
    let buf = [0x00, 0x00, 0x80];
    assert_eq!(
        FefNullPayload::parse(&buf).err(),
        Some(Error::ReservedBits(broadcast_core::error::ReservedBits {
            field: "9-bit rfu",
            expected: 0,
            found: 1,
        }))
    );
}

#[test]
fn fef_null_s2_field_is_a_four_bit_field() {
    let mut payload = FefNullPayload {
        fef_idx: 0,
        s1_field: S1Field::V0,
        s2_field: 0x0F,
    };
    let mut buf = [0u8; 3];
    assert!(payload.serialize_into(&mut buf).is_ok());
    payload.s2_field = 0x10;
    assert!(matches!(
        payload.serialize_into(&mut buf),
        Err(Error::FieldOverflow(_))
    ));
}

#[test]
fn fef_sub_part_round_trips_every_field() {
    let data = [0xC1u8, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7];
    let mut bytes = vec![
        0x02, 0xAB, 0xCD, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x00, 0x03, 0x00, 0x3F, 0xFF, 0xFF,
    ];
    bytes.extend_from_slice(&data);
    let payload = FefSubPartPayload {
        fef_idx: 0x02,
        tx_identifier: 0xABCD,
        subpart_idx: 0x1234,
        subpart_variety: SubpartVariety::TxSigFef,
        subpart_length: 0x3F_FFFF,
        subpart_data: &data,
    };
    assert_framed!(payload, FefSubPartPayload::parse, &bytes);
    assert!(!payload.to_string().is_empty());
}

#[test]
fn fef_sub_part_rejects_each_rfu1_byte_on_its_own() {
    for i in 3..=6 {
        let mut buf = [0u8; 15];
        buf[i] = 0x01;
        assert!(
            matches!(FefSubPartPayload::parse(&buf), Err(Error::ReservedBits(_))),
            "rfu1 byte {i} must be rejected"
        );
    }
}

#[test]
fn fef_sub_part_length_overflow_is_rejected() {
    let mut payload = FefSubPartPayload {
        fef_idx: 0,
        tx_identifier: 0,
        subpart_idx: 0,
        subpart_variety: SubpartVariety::Null,
        subpart_length: 0x3F_FFFF,
        subpart_data: &[],
    };
    let mut buf = [0u8; 15];
    assert!(payload.serialize_into(&mut buf).is_ok());
    payload.subpart_length = 0x40_0000;
    assert_eq!(
        payload.serialize_into(&mut buf).err(),
        Some(Error::FieldOverflow(FieldOverflow {
            field: "subpart_length",
            value: 0x40_0000,
            max: 0x3F_FFFF,
        }))
    );
}

#[test]
fn subpart_variety_and_prbs_type_convert_to_their_wire_values() {
    assert_eq!(u16::from(SubpartVariety::Null), 0);
    assert_eq!(u16::from(SubpartVariety::Iq), 1);
    assert_eq!(u16::from(SubpartVariety::Prbs), 2);
    assert_eq!(u16::from(SubpartVariety::TxSigFef), 3);
    assert_eq!(u8::from(PrbsType::UserDefined), 0);
    assert_eq!(SubpartVariety::TxSigFef.name(), "TX-SIG FEF");
    assert_eq!(PrbsType::UserDefined.name(), "user-defined");
}

#[test]
fn p2_bias_round_trips_every_field() {
    let bytes = [0x42, 0x00, 0x00, 0x12, 0x34];
    let payload = P2BiasPayload {
        frame_idx: 0x42,
        num_active_bias_cells_per_p2: 0x1234,
    };
    assert_framed!(payload, P2BiasPayload::parse, &bytes);
}

#[test]
fn p2_bias_rfu_reports_the_combined_seventeen_bit_value() {
    let buf = [0x00, 0x00, 0x40, 0x00, 0x00];
    assert_eq!(
        P2BiasPayload::parse(&buf).err(),
        Some(Error::ReservedBits(broadcast_core::error::ReservedBits {
            field: "17-bit RFU",
            expected: 0,
            found: 0x80,
        }))
    );
}

#[test]
fn p2_bias_active_cells_is_a_fifteen_bit_field() {
    let mut payload = P2BiasPayload {
        frame_idx: 0,
        num_active_bias_cells_per_p2: 0x7FFF,
    };
    let mut buf = [0u8; 5];
    assert!(payload.serialize_into(&mut buf).is_ok());
    assert_eq!(buf[3], 0x7F);
    payload.num_active_bias_cells_per_p2 = 0x8000;
    assert!(matches!(
        payload.serialize_into(&mut buf),
        Err(Error::FieldOverflow(_))
    ));
}

#[test]
fn l1_current_serializes_the_frequency_source_in_the_top_two_bits() {
    let data = [0x12u8, 0x34];
    let payload = L1CurrentPayload {
        frame_idx: 0x42,
        freq_source: FrequencySource::ManualPerModulator,
        l1_current_data: &data,
    };
    let mut bytes = vec![0x42, 0x80];
    bytes.extend_from_slice(&data);
    assert_framed!(payload, L1CurrentPayload::parse, &bytes);
}
