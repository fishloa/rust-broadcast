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

#[test]
fn leap_day_2000_and_2038_wrap_boundaries() {
    // 2000-02-29 = MJD 51603 = 0xC993 (MSB set: the field is the MJD).
    assert_eq!(decode_mjd_bcd([0xC9, 0x93, 0, 0, 0]), Some(date(2000, 2, 29)));
    assert_eq!(encode_mjd_bcd(date(2000, 2, 29)), Some([0xC9, 0x93, 0, 0, 0]));
    // 2038-04-22 = 65535 = 0xFFFF, 2038-04-23 = 65536 = field 0x0000.
    assert_eq!(decode_mjd_bcd([0xFF, 0xFF, 0, 0, 0]), Some(date(2038, 4, 22)));
    assert_eq!(decode_mjd_bcd([0x00, 0x00, 0, 0, 0]), Some(date(2038, 4, 23)));
    // 1948-08-05 = 32768 = 0x8000, the lowest representable date.
    assert_eq!(decode_mjd_bcd([0x80, 0x00, 0, 0, 0]), Some(date(1948, 8, 5)));
    // 2128-01-09 encodes to 0x7FFF; the day after is not representable.
    assert_eq!(encode_mjd_bcd(date(2128, 1, 9)), Some([0x7F, 0xFF, 0, 0, 0]));
    assert_eq!(encode_mjd_bcd(date(2128, 1, 10)), None);
    assert_eq!(encode_mjd_bcd(date(1948, 8, 4)), None);
}

#[test]
fn every_wire_mjd_round_trips_and_days_are_consecutive() {
    // Both halves of the 16-bit space (MSB set and MSB clear), every value,
    // walked in MJD order: 0x8000..=0xFFFF then 0x0000..=0x7FFF.
    let mut prev: Option<MjdBcdDateTime> = None;
    for field in (0x8000u32..=0xFFFF).chain(0x0000..=0x7FFF) {
        let [b0, b1] = (field as u16).to_be_bytes();
        let raw = [b0, b1, 0x12, 0x34, 0x56];
        let dt = decode_mjd_bcd(raw).unwrap_or_else(|| panic!("decode {field:#06x}"));
        assert_eq!(encode_mjd_bcd(dt), Some(raw), "round trip {field:#06x}");
        if let Some(p) = prev {
            let leap = (p.year % 4 == 0 && p.year % 100 != 0) || p.year % 400 == 0;
            let dim = match p.month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 if leap => 29,
                _ => 28,
            };
            let expect = if p.day < dim {
                (p.year, p.month, p.day + 1)
            } else if p.month < 12 {
                (p.year, p.month + 1, 1)
            } else {
                (p.year + 1, 1, 1)
            };
            assert_eq!((dt.year, dt.month, dt.day), expect, "after {field:#06x}");
        }
        prev = Some(dt);
    }
}
