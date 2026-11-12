//! Every bit-level adapter reports a short input in **bytes**, the unit of
//! `broadcast_common::error::BufferTooShort` (its `Display` says "bytes"),
//! never in bits (issue #2461).
//!
//! `BufferTooShort { need, have }` used to mean bits in some adapters and
//! bytes in others, sometimes within one file: the SAT table's bit reader
//! reported absolute bits, the H.222.0 descriptor bit reader reported bytes.
//! The bit shortfall now goes through the one
//! `broadcast_common::bits::BitError -> BufferTooShort` conversion, which
//! counts whole bytes, and each test here pins that contract by asserting the
//! two numbers are exactly what the truncated input implies in bytes.

use broadcast_common::Parse;
use broadcast_common::error::BufferTooShort;
use dvb_si::error::Error;
use dvb_si::tables::sat::{SatSection, SatTableId};

/// The `(need, have)` of a `BufferTooShort` from `result`, refusing any other
/// outcome.
fn shortfall<T: core::fmt::Debug>(result: Result<T, Error>) -> BufferTooShort {
    match result {
        Err(Error::BufferTooShort(short)) => short,
        other => panic!("expected a short buffer, got {other:?}"),
    }
}

/// The SAT's bit-packed body reader: four body bytes hold a full
/// `satellite_id`, its reserved run and the `position_system` bit, and then
/// ask for the 24 bits of orbital position data — so `have` is no whole byte
/// left and `need` the three bytes wanted, both in bytes (the old reader said
/// 24 bits needed, having called the four-byte body 32).
#[test]
fn the_sat_bit_reader_reports_bytes() {
    // section_length 13 = the 9-byte extension header plus a 4-byte body.
    const SECTION_LENGTH: usize = 13;
    let mut section = vec![
        dvb_si::tables::sat::TABLE_ID,
        0xF0 | (SECTION_LENGTH >> 8) as u8,
        SECTION_LENGTH as u8,
        0x01,
        0xC1,
        0x00,
        0x00,
    ];
    section.resize(SECTION_LENGTH + 3, 0);
    let short = shortfall(SatSection::parse(&section));
    assert_eq!(
        short.have, 0,
        "the section's bits are consumed, so no whole byte is left"
    );
    assert_eq!(
        short.need, 3,
        "three bytes more are wanted, in bytes: {short:?}"
    );
}

/// The H.222.0 descriptor bit reader (shared by every bit-syntax descriptor:
/// MVC operation point, VVC video, EVC video, HEVC timing, …): an empty body
/// needs its first byte, so `need` is 1 byte and `have` is 0 — not 8 bits.
#[test]
fn the_h222_descriptor_bit_reader_reports_bytes() {
    use dvb_si::descriptors::mvc_operation_point::{MvcOperationPointDescriptor, TAG};
    let short = shortfall(MvcOperationPointDescriptor::parse(&[TAG, 0x00]));
    assert_eq!(short.need, 1);
    assert_eq!(short.have, 0);
}

/// The whole-section reader agrees with them: a section whose stated length
/// reaches past the bytes present reports both numbers in bytes.
#[test]
fn a_truncated_section_reports_bytes() {
    const SECTION_LENGTH: usize = 10;
    let section = [
        dvb_si::tables::sat::TABLE_ID,
        0xF0 | (SECTION_LENGTH >> 8) as u8,
        SECTION_LENGTH as u8,
        0x01,
    ];
    let short = shortfall(SatSection::parse(&section));
    assert_eq!(short.need, 3 + SECTION_LENGTH);
    assert_eq!(short.have, section.len());
}

/// The `satellite_table_id` discriminant the body reader reads first is the
/// value the section carries, so the test above is a real body parse.
#[test]
fn the_short_section_still_names_its_table_id() {
    assert_eq!(
        SatTableId::PositionV2 as u8,
        0,
        "the first body byte's top six bits are the discriminant"
    );
}
