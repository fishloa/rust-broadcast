//! Monochrome data-field flag bits and the `broadcast_common::Serialize`
//! delegation of [`DataUnitPayload`] — ETSI EN 301 775 §4.9.1 (Table 12) and
//! §4.4.1 (Table 1).

use broadcast_common::Serialize;
use dvb_vbi::{DataUnitId, DataUnitPayload, MonochromeDataField, TXT_DATA_BLOCK_LEN};

fn sample(first_segment: bool, last_segment: bool, field_parity: bool, line_offset: u8) -> [u8; 5] {
    let mut b0 = line_offset & 0x1F;
    if first_segment {
        b0 |= 0b1000_0000;
    }
    if last_segment {
        b0 |= 0b0100_0000;
    }
    if field_parity {
        b0 |= 0b0010_0000;
    }
    // first_pixel_position = 0x0123, n_pixels = 1, one Y_value.
    [b0, 0x01, 0x23, 0x01, 0x80]
}

#[test]
fn every_flag_combination_decodes_and_reencodes() {
    for line_offset in [7u8, 23] {
        for bits in 0u8..8 {
            let first_segment = bits & 0b100 != 0;
            let last_segment = bits & 0b010 != 0;
            let field_parity = bits & 0b001 != 0;
            let wire = sample(first_segment, last_segment, field_parity, line_offset);
            let field = MonochromeDataField::parse(&wire).unwrap();
            assert_eq!(field.first_segment, first_segment, "wire {wire:02X?}");
            assert_eq!(field.last_segment, last_segment, "wire {wire:02X?}");
            assert_eq!(field.field_parity, field_parity, "wire {wire:02X?}");
            assert_eq!(field.line_offset, line_offset, "wire {wire:02X?}");
            assert_eq!(field.first_pixel_position, 0x0123);
            assert_eq!(field.samples, &[0x80u8][..]);

            let mut out = [0u8; 5];
            let n = field.serialize_into(&mut out).unwrap();
            assert_eq!(n, 5);
            assert_eq!(out, wire);
        }
    }
}

#[test]
fn flag_bits_do_not_leak_from_neighbouring_header_bits() {
    // Every raw header byte: first/last/parity must be exactly bits 7/6/5.
    for raw in 0u16..=0xFF {
        let raw = raw as u8;
        let b0 = raw | 0x1F;
        let wire = [b0, 0, 0, 1, 0x80];
        let field = MonochromeDataField::parse(&wire).unwrap();
        assert_eq!(field.first_segment, raw & 0b1000_0000 != 0, "raw {raw:04X}");
        assert_eq!(field.last_segment, raw & 0b0100_0000 != 0, "raw {raw:04X}");
        assert_eq!(field.field_parity, raw & 0b0010_0000 != 0, "raw {raw:04X}");
    }
}

fn trait_len<T: Serialize>(value: &T) -> usize {
    value.serialized_len()
}

fn trait_serialize<T: Serialize>(value: &T, out: &mut [u8]) -> usize
where
    <T as Serialize>::Error: core::fmt::Debug,
{
    value.serialize_into(out).unwrap()
}

#[test]
fn data_unit_payload_serialize_trait_delegates_to_inherent_methods() {
    let block = [0xAAu8; TXT_DATA_BLOCK_LEN];
    let payload = DataUnitPayload::Opaque(&block);
    assert_eq!(trait_len(&payload), TXT_DATA_BLOCK_LEN);

    let mut out = [0u8; TXT_DATA_BLOCK_LEN];
    assert_eq!(trait_serialize(&payload, &mut out), TXT_DATA_BLOCK_LEN);
    assert_eq!(out, block);

    let stuffing = DataUnitPayload::Stuffing { length: 7 };
    assert_eq!(trait_len(&stuffing), 7);
    let mut out = [0u8; 7];
    assert_eq!(trait_serialize(&stuffing, &mut out), 7);
    assert_eq!(out, [0xFFu8; 7]);
}

#[test]
fn stuffing_body_ceiling_is_255_bytes() {
    let ok = [0xFFu8; 255];
    let payload = DataUnitPayload::parse(DataUnitId::Stuffing, &ok).unwrap();
    assert_eq!(payload, DataUnitPayload::Stuffing { length: 255 });

    let too_long = [0xFFu8; 256];
    assert!(
        DataUnitPayload::parse(DataUnitId::Stuffing, &too_long).is_err(),
        "a 256-byte stuffing body does not fit the 8-bit data_unit_length"
    );
}
