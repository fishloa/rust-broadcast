//! The shared big-endian wire readers: each reads the value at its offset, and
//! a short slice is `BufferTooShort` carrying `need`/`have`.

use broadcast_common::error::BufferTooShort;
use broadcast_common::wire::{
    be_u16, be_u32, be_u64, fourcc_at, i16_at, i32_at, read_u8, read_u16, read_u48, u16_at, u24_at,
    u32_at, u64_at,
};

#[test]
fn every_reader_returns_the_value_at_the_offset() {
    let b = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];

    assert_eq!(be_u16(&b, 0, "t").unwrap(), 0x0102);
    assert_eq!(be_u16(&b, 6, "t").unwrap(), 0x0708);
    assert_eq!(be_u32(&b, 0, "t").unwrap(), 0x0102_0304);
    assert_eq!(be_u32(&b, 4, "t").unwrap(), 0x0506_0708);
    assert_eq!(be_u64(&b, 0, "t").unwrap(), 0x0102_0304_0506_0708);

    assert_eq!(u16_at(&b, 2), 0x0304);
    assert_eq!(i16_at(&b, 2), 0x0304);
    assert_eq!(u24_at(&b, 1), 0x0002_0304);
    assert_eq!(u32_at(&b, 4), 0x0506_0708);
    assert_eq!(i32_at(&b, 4), 0x0506_0708);
    assert_eq!(u64_at(&b, 0), 0x0102_0304_0506_0708);
    assert_eq!(fourcc_at(&b, 0), [0x01, 0x02, 0x03, 0x04]);

    let mut cursor = 2usize;
    assert_eq!(read_u8(&b, &mut cursor, "t").unwrap(), 0x03);
    assert_eq!(read_u16(&b, &mut cursor, "t").unwrap(), 0x0405);
    assert_eq!(cursor, 5);
    assert_eq!(read_u16(&b, &mut cursor, "t").unwrap(), 0x0607);
    assert_eq!(read_u8(&b, &mut cursor, "t").unwrap(), 0x08);
    assert_eq!(cursor, 8);
}

#[test]
fn read_u48_widens_the_six_bytes() {
    let b = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
    let mut cursor = 0usize;
    assert_eq!(
        read_u48(&b, &mut cursor, "t").unwrap(),
        0x0000_0102_0304_0506
    );
}

#[test]
fn a_short_slice_reports_need_and_have() {
    assert_eq!(
        be_u16(&[0u8; 1], 0, "t").unwrap_err(),
        BufferTooShort {
            need: 2,
            have: 1,
            what: "t",
        }
    );
    assert_eq!(
        be_u32(&[0u8; 3], 0, "t").unwrap_err(),
        BufferTooShort {
            need: 4,
            have: 3,
            what: "t",
        }
    );
    assert_eq!(
        be_u64(&[0u8; 7], 0, "t").unwrap_err(),
        BufferTooShort {
            need: 8,
            have: 7,
            what: "t",
        }
    );

    // A hostile cursor saturates rather than wrapping.
    assert_eq!(
        be_u64(&[0u8; 8], usize::MAX, "t").unwrap_err(),
        BufferTooShort {
            need: usize::MAX,
            have: 8,
            what: "t",
        }
    );

    let mut cursor = 0usize;
    assert_eq!(
        read_u8(&[], &mut cursor, "t").unwrap_err(),
        BufferTooShort {
            need: 1,
            have: 0,
            what: "t",
        }
    );
}
