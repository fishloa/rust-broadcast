//! A bit-level adapter reports a short input in **bytes**, the unit of
//! `broadcast_common::error::BufferTooShort` (its `Display` says "bytes"),
//! never in bits (issue #2461).
//!
//! The VVC configuration record reader is the adapter here that is reachable
//! from outside the crate; the RBSP reader and the `&[u8]` cursor reader are
//! pinned by unit tests beside their definitions, and the shared
//! `BitError -> BufferTooShort` conversion that every one of them uses is
//! pinned in `broadcast-common`'s own unit tests.

use broadcast_common::Parse;
use broadcast_common::error::BufferTooShort;
use transmux::error::Error;
use transmux::vvc_config::VvcDecoderConfigurationRecord;

/// The `(need, have)` of a `BufferTooShort` from `result`, refusing any other
/// outcome.
fn shortfall<T: core::fmt::Debug>(result: Result<T, Error>) -> BufferTooShort {
    match result {
        Err(Error::BufferTooShort(short)) => short,
        other => panic!("expected a short buffer, got {other:?}"),
    }
}

/// One byte carries the reserved bits, `LengthSizeMinusOne` and
/// `ptl_present_flag`; the PTL fields that follow need another. Both counts
/// are bytes: `have` is the one byte present, not its eight bits.
#[test]
fn the_vvc_config_reader_reports_bytes() {
    // reserved(5) | LengthSizeMinusOne(2) = 0 | ptl_present_flag = 1
    let short = shortfall(<VvcDecoderConfigurationRecord as Parse>::parse(&[
        0b0000_0001,
    ]));
    assert_eq!(short.have, 1, "one byte present: {short:?}");
    assert!(
        (2..=3).contains(&short.need),
        "the next field starts in the second byte: {short:?}"
    );
}

/// An empty record has no bit to read: the first byte is `need` and nothing
/// is `have`.
#[test]
fn an_empty_vvc_config_record_needs_one_byte() {
    let short = shortfall(<VvcDecoderConfigurationRecord as Parse>::parse(&[]));
    assert_eq!(short.need, 1);
    assert_eq!(short.have, 0);
}
