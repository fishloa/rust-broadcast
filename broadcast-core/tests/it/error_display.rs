//! Pins the `Display` text of every error kind in this crate.
//!
//! The strings here were captured from the `thiserror` derives that used to
//! produce them, before the hand-written impls in `error.rs` and `bits.rs`
//! replaced those derives. A change to any message is a change to observable
//! output and must fail this test.

use broadcast_core::bits::BitError;
use broadcast_core::error::{
    BufferTooShort, CrcMismatch, FieldOverflow, InvalidField, InvalidSync, InvalidValue,
    LengthMismatch, OutputBufferTooSmall, ReservedBits, UnsupportedVersion,
};

#[test]
fn buffer_too_short_display() {
    assert_eq!(
        BufferTooShort {
            need: 4,
            have: 1,
            what: "PAT"
        }
        .to_string(),
        "buffer too short: need 4 bytes, have 1 (while parsing PAT)"
    );
}

#[test]
fn output_buffer_too_small_display() {
    assert_eq!(
        OutputBufferTooSmall { need: 4, have: 1 }.to_string(),
        "output buffer too small: need 4, have 1"
    );
}

#[test]
fn field_overflow_display() {
    assert_eq!(
        FieldOverflow {
            field: "section_length",
            value: 5000,
            max: 4095
        }
        .to_string(),
        "section_length = 5000 exceeds the field maximum 4095"
    );
}

#[test]
fn crc_mismatch_display() {
    assert_eq!(
        CrcMismatch {
            what: "PSM",
            computed: 0xAB,
            expected: 0xCD
        }
        .to_string(),
        "PSM CRC-32 mismatch: computed 0x000000ab, expected 0x000000cd"
    );
}

#[test]
fn invalid_sync_display() {
    assert_eq!(
        InvalidSync {
            what: "TS sync byte",
            expected: 0x47,
            found: 0
        }
        .to_string(),
        "invalid TS sync byte: expected 0x47, got 0x0"
    );
}

#[test]
fn reserved_bits_display() {
    assert_eq!(
        ReservedBits {
            field: "reserved",
            expected: 0x7,
            found: 0x2
        }
        .to_string(),
        "reserved field reserved must be 0x7, found 0x2"
    );
}

#[test]
fn invalid_value_display() {
    assert_eq!(
        InvalidValue {
            field: "version",
            value: 9,
            reason: "out of range"
        }
        .to_string(),
        "field version value 9 invalid: out of range"
    );
}

#[test]
fn invalid_field_display() {
    assert_eq!(
        InvalidField {
            field: "pts",
            reason: "missing"
        }
        .to_string(),
        "invalid field pts: missing"
    );
}

#[test]
fn unsupported_version_display() {
    assert_eq!(
        UnsupportedVersion {
            what: "RTP",
            found: 1
        }
        .to_string(),
        "unsupported RTP version 1"
    );
}

#[test]
fn length_mismatch_display() {
    assert_eq!(
        LengthMismatch {
            what: "APDU",
            declared: 9,
            actual: 7
        }
        .to_string(),
        "length mismatch for APDU: declared 9, actual 7"
    );
}

#[test]
fn bit_error_out_of_bounds_display() {
    assert_eq!(
        BitError::OutOfBounds {
            needed_bits: 17,
            remaining_bits: 6
        }
        .to_string(),
        "bit buffer out of bounds: need 17 bit(s), 6 remaining"
    );
}

#[test]
fn bit_error_too_many_bits_display() {
    assert_eq!(
        BitError::TooManyBits { requested: 65 }.to_string(),
        "requested 65 bits exceeds the 64-bit carrier"
    );
}

#[test]
fn bit_error_value_too_wide_display() {
    assert_eq!(
        BitError::ValueTooWide {
            value: 0x1FF,
            bits: 8
        }
        .to_string(),
        "value 0x1ff does not fit in 8 bit(s)"
    );
}
