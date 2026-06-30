//! Fixture test: a committed EN 301 775 PES data field (`data_identifier` 0x10)
//! carrying one of every typed data unit — EBU Teletext (0x02), VPS (0xC3),
//! WSS (0xC4), Closed Captioning (0xC5), monochrome 4:2:2 (0xC6), and stuffing
//! (0xFF). Asserts decoded fields, a byte-exact round-trip, and a corruption
//! bite.

use std::fs;

use dvb_vbi::{DataField, DataUnitId, DataUnitPayload};

fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dvb-vbi/vbi_data_field.bin"
    );
    fs::read(path).expect("fixture vbi_data_field.bin must be committed")
}

#[test]
fn parses_fixture_fields() {
    let data = fixture();
    let field = DataField::parse(&data).expect("fixture must parse");

    assert_eq!(field.data_identifier, 0x10);
    assert_eq!(field.data_units.len(), 6, "6 data units");

    let ids: Vec<DataUnitId> = field.data_units.iter().map(|u| u.id).collect();
    assert_eq!(
        ids,
        vec![
            DataUnitId::EbuTeletextNonSubtitle,
            DataUnitId::Vps,
            DataUnitId::Wss,
            DataUnitId::ClosedCaptioning,
            DataUnitId::Monochrome422Samples,
            DataUnitId::Stuffing,
        ]
    );

    // Teletext: data_unit_length must be the fixed 0x2C for di 0x10–0x1F.
    match &field.data_units[0].payload {
        DataUnitPayload::Teletext(f) => {
            assert_eq!(f.header.line_offset, 7);
            assert!(f.header.field_parity);
            assert_eq!(f.framing_code, dvb_vbi::FRAMING_CODE_EBU);
            assert_eq!(f.txt_data_block.len(), dvb_vbi::TXT_DATA_BLOCK_LEN);
        }
        _ => panic!("expected Teletext"),
    }
    assert_eq!(
        field.data_units[0].data_unit_length(),
        dvb_vbi::TELETEXT_DATA_UNIT_LENGTH as usize
    );

    // VPS line 16.
    match &field.data_units[1].payload {
        DataUnitPayload::Vps(f) => {
            assert_eq!(f.header.line_offset, 16);
            assert!(f.header.field_parity);
            assert_eq!(f.vps_data_block.len(), dvb_vbi::VPS_DATA_BLOCK_LEN);
        }
        _ => panic!("expected VPS"),
    }

    // WSS line 23, value 0x2A55.
    match &field.data_units[2].payload {
        DataUnitPayload::Wss(f) => {
            assert_eq!(f.header.line_offset, 23);
            assert_eq!(f.wss_data_block, 0x2A55);
        }
        _ => panic!("expected WSS"),
    }

    // CC line 21, value 0x9425.
    match &field.data_units[3].payload {
        DataUnitPayload::ClosedCaptioning(f) => {
            assert_eq!(f.header.line_offset, 21);
            assert_eq!(f.closed_captioning_data_block, 0x9425);
        }
        _ => panic!("expected Closed Captioning"),
    }

    // Monochrome: first+last segment, line 10, 6 samples.
    match &field.data_units[4].payload {
        DataUnitPayload::Monochrome(f) => {
            assert!(f.first_segment && f.last_segment && f.field_parity);
            assert_eq!(f.line_offset, 10);
            assert_eq!(f.first_pixel_position, 0);
            assert_eq!(f.samples.len(), 6);
        }
        _ => panic!("expected Monochrome"),
    }

    // Stuffing: 3 bytes.
    match &field.data_units[5].payload {
        DataUnitPayload::Stuffing { length } => assert_eq!(*length, 3),
        _ => panic!("expected Stuffing"),
    }
}

#[test]
fn fixture_byte_exact_round_trip() {
    let data = fixture();
    let field = DataField::parse(&data).unwrap();

    let mut out = vec![0u8; field.serialized_len()];
    let n = field.serialize_into(&mut out).unwrap();
    assert_eq!(n, data.len());
    assert_eq!(out, data, "serialize must be byte-identical to the fixture");

    assert_eq!(DataField::parse(&out).unwrap(), field);
}

#[test]
fn truncating_the_fixture_fails_to_parse() {
    let mut data = fixture();
    // Drop the last stuffing byte: the final data unit now under-runs.
    data.pop();
    assert!(
        DataField::parse(&data).is_err(),
        "a truncated data unit must be rejected"
    );
}

#[cfg(feature = "serde")]
#[test]
fn serde_serializes() {
    let data = fixture();
    let field = DataField::parse(&data).unwrap();
    let json = serde_json::to_string(&field).expect("serde must serialize");
    assert!(json.contains("\"data_identifier\""));
    assert!(json.contains("Vps") || json.contains("vps"));
}
