//! `OutputBufferTooSmall { need, have }` for every buffer shorter than the real
//! VBI PES data field, and for each of its data units on its own.

use dvb_vbi::{DataField, DataUnitId, DataUnitPayload, Error, ID_VPS};

fn too_small(e: &Error) -> Option<(usize, usize)> {
    match e {
        Error::OutputBufferTooSmall(broadcast_common::error::OutputBufferTooSmall {
            need,
            have,
        }) => Some((*need, *have)),
        _ => None,
    }
}

#[test]
fn real_data_field_and_its_units_reject_every_short_buffer() {
    let data = test_fixtures::read("subtitles/vbi-data-field.bin");
    let field = DataField::parse(&data).expect("real VBI data field");
    assert!(!field.data_units.is_empty());
    test_fixtures::assert_short_buffers_rejected(
        field.serialized_len(),
        |buf| field.serialize_into(buf),
        too_small,
    );
    for unit in &field.data_units {
        test_fixtures::assert_short_buffers_rejected(
            unit.serialized_len(),
            |buf| unit.serialize_into(buf),
            too_small,
        );
    }
}

#[test]
fn serializing_a_unit_under_an_inconsistent_id_is_rejected_with_the_id_and_payload_kind() {
    let data = test_fixtures::read("subtitles/vbi-data-field.bin");
    let field = DataField::parse(&data).expect("real VBI data field");
    let mut checked = 0;
    for unit in &field.data_units {
        assert!(
            unit.payload.is_consistent_with(unit.id),
            "parsed pairs are consistent"
        );
        let mut mislabelled = unit.clone();
        mislabelled.id = if unit.id == DataUnitId::Vps {
            DataUnitId::Wss
        } else {
            DataUnitId::Vps
        };
        let mut buf = vec![0u8; mislabelled.serialized_len().max(unit.serialized_len())];
        match mislabelled.serialize_into(&mut buf) {
            Err(Error::DataUnitIdPayloadMismatch { id, payload }) => {
                assert_eq!(id, mislabelled.id.to_u8());
                assert!(!payload.is_empty());
            }
            other => panic!("{:?} under {:?}: {other:?}", unit.id, mislabelled.id),
        }
        checked += 1;
    }
    assert!(checked > 0);
}

#[test]
fn an_opaque_payload_must_carry_the_canonical_reserved_or_user_defined_id() {
    let opaque = DataUnitPayload::Opaque(&[1, 2, 3]);
    assert!(opaque.is_consistent_with(DataUnitId::Reserved(0x10)));
    assert!(opaque.is_consistent_with(DataUnitId::UserDefined(0x85)));
    assert!(!opaque.is_consistent_with(DataUnitId::Vps));
    assert!(
        !opaque.is_consistent_with(DataUnitId::Reserved(ID_VPS)),
        "a Reserved(0xC3) is a mislabelled VPS unit"
    );
}
