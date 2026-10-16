//! Adversarial review (branch `review/dvbsi-descriptors`): the EN 300 468
//! V1.20.1 2038 MJD wrap rule extends the representable range to MJD 98303
//! (2128-01-09, as `time.rs`'s own module doc states), but the conversion is
//! still the Annex C closed-form formula, which EN 300 468 (NOTE 2 of
//! Annex C, unchanged in V1.20.1) says is only valid "between the inclusive
//! dates 1900 March 1 to 2100 February 28". It treats 2100 as a leap year, so
//! every wire value from 0x5840 (MJD 88128 = 2100-03-01) to 0x7FFF is decoded
//! one day early, including the non-existent date 2100-02-29, and the encoder
//! accepts 2100-02-29. Reference MJDs from the proleptic Gregorian calendar
//! (MJD 0 = 1858-11-17): 2100-02-28 = 88127, 2100-03-01 = 88128,
//! 2128-01-09 = 98303.

use broadcast_common::time::{MjdBcdDateTime, decode_mjd_bcd, encode_mjd_bcd};

fn date(year: u16, month: u8, day: u8) -> MjdBcdDateTime {
    MjdBcdDateTime {
        year,
        month,
        day,
        hour: 0,
        minute: 0,
        second: 0,
    }
}

#[test]
fn last_valid_annex_c_day_still_decodes() {
    // 88127 - 65536 = 22591 = 0x583F: control, passes today.
    assert_eq!(
        decode_mjd_bcd([0x58, 0x3F, 0, 0, 0]),
        Some(date(2100, 2, 28))
    );
}

#[test]
fn mjd_88128_is_2100_03_01_not_2100_02_29() {
    // 88128 - 65536 = 22592 = 0x5840.
    assert_eq!(
        decode_mjd_bcd([0x58, 0x40, 0, 0, 0]),
        Some(date(2100, 3, 1))
    );
}

#[test]
fn top_of_wrapped_range_is_2128_01_09() {
    // 0x7FFF -> MJD 0x10000 + 0x7FFF = 98303 = 2128-01-09 (time.rs module doc).
    assert_eq!(
        decode_mjd_bcd([0x7F, 0xFF, 0, 0, 0]),
        Some(date(2128, 1, 9))
    );
}

#[test]
fn encoder_rejects_2100_02_29_and_encodes_2100_03_01_correctly() {
    assert_eq!(
        encode_mjd_bcd(date(2100, 2, 29)),
        None,
        "2100 is not a leap year"
    );
    assert_eq!(
        encode_mjd_bcd(date(2100, 3, 1)),
        Some([0x58, 0x40, 0, 0, 0])
    );
}
