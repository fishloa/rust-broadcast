//! H.262 §6.3.3: a `sequence_header()` whose `horizontal_size_value` or
//! `vertical_size_value` is zero is rejected, by both the base parse and the
//! sequence walk.
//!
//! The first input is the exact buffer that failed the
//! `broadcast_bitstream_mpeg2` fuzz smoke run: `0x000001B3` at offset 1
//! followed by `01 00 00`, i.e. a 16x0 geometry, with a start-code prefix
//! beginning inside the third size byte.
#![cfg(feature = "mpeg2")]

use broadcast_bitstream::Error;
use broadcast_bitstream::h262::{find_sequence_header, parse_sequence_at};
use broadcast_bitstream::mpeg2::Mpeg2SeqHeader;
use broadcast_core::error::BufferTooShort;

/// The verbatim fuzz-smoke input.
const FUZZ_INPUT: [u8; 18] = [
    0x6e, 0x00, 0x00, 0x01, 0xB3, 0x01, 0x00, 0x00, 0x01, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x01, 0x01,
    0x00, 0xF0,
];

/// `0x000001B3` then `01 00 00 23 FF`: `vertical_size_value == 0`, with a
/// body long enough that no start-code prefix straddles the size fields.
const ZERO_HEIGHT: [u8; 9] = [0x00, 0x00, 0x01, 0xB3, 0x01, 0x00, 0x00, 0x23, 0xFF];

/// `0x000001B3` then `00 00 F0 23 FF`: `horizontal_size_value == 0`,
/// `vertical_size_value == 240`.
const ZERO_WIDTH: [u8; 9] = [0x00, 0x00, 0x01, 0xB3, 0x00, 0x00, 0xF0, 0x23, 0xFF];

/// A valid 320x240 header, so the rejection cannot be an over-rejection.
const VALID: [u8; 9] = [0x00, 0x00, 0x01, 0xB3, 0x14, 0x00, 0xF0, 0x23, 0xFF];

fn rejected_field(err: Error) -> &'static str {
    match err {
        Error::InvalidValue(v) => {
            assert_eq!(v.value, 0);
            v.field
        }
        other => panic!("expected InvalidValue, got {other:?}"),
    }
}

#[test]
fn fuzz_smoke_input_is_rejected_by_the_base_parse() {
    assert_eq!(
        rejected_field(Mpeg2SeqHeader::find(&FUZZ_INPUT).unwrap_err()),
        "vertical_size_value"
    );
}

#[test]
fn fuzz_smoke_input_is_rejected_by_the_sequence_walk() {
    let start = find_sequence_header(&FUZZ_INPUT).expect("start code at offset 1");
    let err = parse_sequence_at(&FUZZ_INPUT, start).unwrap_err();
    // The section body ends where the next start-code prefix begins, inside
    // the size fields, so the header is truncated before it is validated; the
    // reader reports the two bytes `horizontal_size_value` needs.
    match err {
        Error::BufferTooShort(BufferTooShort { need, have, .. }) => {
            assert_eq!((need, have), (2, 1));
        }
        other => panic!("expected BufferTooShort, got {other:?}"),
    }
}

#[test]
fn zero_vertical_size_is_rejected_by_both_parsers() {
    assert_eq!(
        rejected_field(Mpeg2SeqHeader::find(&ZERO_HEIGHT).unwrap_err()),
        "vertical_size_value"
    );
    assert_eq!(
        rejected_field(parse_sequence_at(&ZERO_HEIGHT, 0).unwrap_err()),
        "vertical_size_value"
    );
}

#[test]
fn zero_horizontal_size_is_rejected_by_both_parsers() {
    assert_eq!(
        rejected_field(Mpeg2SeqHeader::find(&ZERO_WIDTH).unwrap_err()),
        "horizontal_size_value"
    );
    assert_eq!(
        rejected_field(parse_sequence_at(&ZERO_WIDTH, 0).unwrap_err()),
        "horizontal_size_value"
    );
}

#[test]
fn non_zero_sizes_still_parse() {
    let sh = Mpeg2SeqHeader::find(&VALID).expect("320x240");
    assert_eq!((sh.width(), sh.height()), (320, 240));
    let geometry = parse_sequence_at(&VALID, 0).expect("320x240");
    assert_eq!(
        (geometry.coded_width(), geometry.coded_height()),
        (320, 240)
    );
}
