//! `DataUnit::validate` conformance rules — ETSI EN 301 775 §4.5.2, §4.7.2,
//! §4.8.2, §4.9.2 and the Table 1/4/6/8/10/12 "Valid Range" columns.
//!
//! Every case deviates from a passing unit by exactly one field, so an
//! `||`/`&&`/comparison mutation in one rule cannot hide behind another.

use dvb_vbi::{
    ClosedCaptioningDataField, DataUnit, DataUnitId, FRAMING_CODE_EBU, LineHeader,
    MonochromeDataField, TXT_DATA_BLOCK_LEN, TeletextDataField, WSS_RESERVED_TAIL, WssDataField,
};

/// A `data_identifier` outside `0x10..=0x1F`, so the §4.4.2 fixed-length rule
/// never masks a per-unit rule under test.
const NON_TELETEXT_DI: u8 = 0x99;

fn teletext(line_offset: u8) -> DataUnit<'static> {
    DataUnit::teletext(
        DataUnitId::EbuTeletextNonSubtitle,
        TeletextDataField {
            header: LineHeader::new(true, line_offset),
            framing_code: FRAMING_CODE_EBU,
            txt_data_block: [0u8; TXT_DATA_BLOCK_LEN],
        },
    )
}

fn wss(field_parity: bool, line_offset: u8) -> DataUnit<'static> {
    DataUnit::wss(WssDataField {
        header: LineHeader::new(field_parity, line_offset),
        wss_data_block: 0,
        reserved_tail: WSS_RESERVED_TAIL,
    })
}

fn closed_captioning(line_offset: u8) -> DataUnit<'static> {
    DataUnit::closed_captioning(ClosedCaptioningDataField {
        header: LineHeader::new(true, line_offset),
        closed_captioning_data_block: 0,
    })
}

fn monochrome<'a>(first_pixel_position: u16, samples: &'a [u8]) -> DataUnit<'a> {
    DataUnit::monochrome(MonochromeDataField {
        first_segment: true,
        last_segment: true,
        field_parity: true,
        line_offset: 7,
        first_pixel_position,
        samples,
    })
}

const ONE_SAMPLE: [u8; 1] = [0x80];

#[test]
fn teletext_line_offset_range() {
    for offset in [0, 7, 22] {
        teletext(offset)
            .validate(NON_TELETEXT_DI)
            .unwrap_or_else(|e| {
                panic!("line_offset {offset} shall be valid for Teletext (§4.5.2): {e}")
            });
    }
    for offset in [6, 23] {
        assert!(
            teletext(offset).validate(NON_TELETEXT_DI).is_err(),
            "line_offset {offset} is outside the §4.5.2 range"
        );
    }
}

#[test]
fn wss_requires_first_field_and_line_23() {
    wss(true, 23).validate(NON_TELETEXT_DI).unwrap();
    assert!(
        wss(false, 23).validate(NON_TELETEXT_DI).is_err(),
        "WSS on field_parity 0 is a §4.7.2 violation"
    );
    assert!(
        wss(true, 22).validate(NON_TELETEXT_DI).is_err(),
        "WSS on line 22 is a §4.7.2 violation"
    );
}

#[test]
fn closed_captioning_requires_line_21() {
    closed_captioning(21).validate(NON_TELETEXT_DI).unwrap();
    for offset in [20, 22] {
        assert!(
            closed_captioning(offset).validate(NON_TELETEXT_DI).is_err(),
            "Closed Captioning on line {offset} is a §4.8.2 violation"
        );
    }
}

#[test]
fn monochrome_first_pixel_ceiling_is_719() {
    monochrome(719, &ONE_SAMPLE)
        .validate(NON_TELETEXT_DI)
        .unwrap();
    assert!(
        monochrome(720, &ONE_SAMPLE)
            .validate(NON_TELETEXT_DI)
            .is_err(),
        "first_pixel_position 720 exceeds the Table 12 maximum 719"
    );
}

#[test]
fn monochrome_sample_count_bounds_are_1_to_251() {
    let one = [0x80u8; 1];
    let two_fifty_one = [0x80u8; 251];
    let two_fifty_two = [0x80u8; 252];

    monochrome(0, &one).validate(NON_TELETEXT_DI).unwrap();
    monochrome(0, &two_fifty_one)
        .validate(NON_TELETEXT_DI)
        .unwrap();
    assert!(
        monochrome(0, &[]).validate(NON_TELETEXT_DI).is_err(),
        "n_pixels 0 is a §4.9.2 violation"
    );
    assert!(
        monochrome(0, &two_fifty_two)
            .validate(NON_TELETEXT_DI)
            .is_err(),
        "n_pixels 252 exceeds the Table 12 maximum 251"
    );
}

#[test]
fn padded_to_exactly_the_current_length_is_ok() {
    let unit = wss(true, 23);
    let len = unit.data_unit_length();
    let padded = unit.clone().padded_to(len as u8).unwrap();
    assert_eq!(padded, unit);
    assert_eq!(padded.stuffing, 0);

    let err = unit.padded_to((len - 1) as u8).unwrap_err();
    assert!(
        matches!(
            err,
            dvb_vbi::Error::InvalidDataUnitLength { length, .. } if length == (len - 1) as u8
        ),
        "unexpected error {err:?}"
    );
}

#[test]
fn conformant_units_round_trip_and_validate() {
    for unit in [
        teletext(0),
        teletext(22),
        wss(true, 23),
        closed_captioning(21),
        monochrome(719, &ONE_SAMPLE),
    ] {
        let bytes = broadcast_core::Serialize::try_to_bytes(&unit).unwrap();
        let (back, used) = DataUnit::parse(&bytes).unwrap();
        assert_eq!(used, bytes.len());
        assert_eq!(back, unit);
        back.validate(NON_TELETEXT_DI).unwrap();
    }
}
