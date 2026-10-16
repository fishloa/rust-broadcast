//! Adversarial review (dvb-si tables/core batch): the v1.20.1 2038 MJD
//! wrap-around (EN 300 468 clause 5.2.4) extends the decodable range to
//! MJD 98303 (= 2128-01-09, as the module doc itself states), but the MJD <->
//! calendar conversion is still the EN 300 468 Annex C formula, whose NOTE 2
//! limits it to "the inclusive dates 1900 March 1 to 2100 February 28". 2100
//! is not a leap year; the formula treats it as one, so every wire value from
//! 0x5840 (MJD 88128) onward decodes one day off, and the non-existent date
//! 2100-02-29 is produced and accepted by the encoder.
//!
//! Expected values: MJD = days since 1858-11-17 (proleptic Gregorian),
//! computed independently: 2100-02-28 = 88127, 2100-03-01 = 88128,
//! 2128-01-09 = 98303; wire field = MJD - 0x10000.
use broadcast_common::time::{MjdBcdDateTime, decode_mjd_bcd, encode_mjd_bcd};

fn day(y: u16, m: u8, d: u8) -> MjdBcdDateTime {
    MjdBcdDateTime {
        year: y,
        month: m,
        day: d,
        hour: 0,
        minute: 0,
        second: 0,
    }
}

fn ymd(field: u16) -> (u16, u8, u8) {
    let [a, b] = field.to_be_bytes();
    let t = decode_mjd_bcd([a, b, 0, 0, 0]).expect("decodes");
    (t.year, t.month, t.day)
}

#[test]
fn last_annex_c_day_is_still_correct() {
    assert_eq!(ymd(0x583F), (2100, 2, 28));
}

#[test]
fn mjd_88128_is_2100_03_01() {
    assert_eq!(ymd(0x5840), (2100, 3, 1));
}

#[test]
fn last_representable_day_matches_the_module_doc() {
    // time.rs module doc: "65536 = 2038-04-23 ... 98303 = 2128-01-09".
    assert_eq!(ymd(0x7FFF), (2128, 1, 9));
}

#[test]
fn encoder_rejects_the_non_existent_2100_02_29() {
    assert_eq!(encode_mjd_bcd(day(2100, 2, 29)), None);
}

#[test]
fn encoder_maps_2100_03_01_to_mjd_88128() {
    assert_eq!(
        encode_mjd_bcd(day(2100, 3, 1)),
        Some([0x58, 0x40, 0x00, 0x00, 0x00])
    );
}
