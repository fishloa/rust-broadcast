//! The MJD + BCD time codecs and the monotonic-to-UTC mapping at their edges
//! (audit 21, #2921): a BCD 12:60:00 or 24:00:00 is not a time of day, the
//! calendar gate must keep refusing the dates the conversion arithmetic would
//! silently roll over, and the anchor mapping must move by exactly the elapsed
//! read.

use broadcast_common::time::{
    MjdBcdDateTime, decode_mjd_bcd, encode_mjd_bcd, mjd_to_ymd, ymd_to_mjd,
};

/// The 16-bit MJD field and MSB-clear wrap offset of EN 300 468 v1.20.1
/// clause 5.2.4 example 3, `0x03D1` = 2040-12-25.
const Y2040_MJD: [u8; 2] = [0x03, 0xD1];

fn at(hour: u8, minute: u8, second: u8) -> [u8; 5] {
    [Y2040_MJD[0], Y2040_MJD[1], hour, minute, second]
}

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
fn a_bcd_field_above_its_range_is_not_a_time_of_day() {
    assert_eq!(decode_mjd_bcd(at(0x12, 0x60, 0x00)), None);
    assert_eq!(decode_mjd_bcd(at(0x12, 0x30, 0x60)), None);
    assert_eq!(decode_mjd_bcd(at(0x24, 0x00, 0x00)), None);
    assert_eq!(
        decode_mjd_bcd(at(0x23, 0x59, 0x59)),
        Some(MjdBcdDateTime {
            hour: 23,
            minute: 59,
            second: 59,
            ..date(2040, 12, 25)
        })
    );
    assert!(decode_mjd_bcd(at(0x12, 0x59, 0x00)).is_some());
    assert!(decode_mjd_bcd(at(0x12, 0x00, 0x59)).is_some());
}

#[test]
fn an_out_of_range_field_cannot_be_encoded_either() {
    assert_eq!(
        encode_mjd_bcd(MjdBcdDateTime {
            hour: 24,
            ..date(2040, 12, 25)
        }),
        None
    );
    assert_eq!(
        encode_mjd_bcd(MjdBcdDateTime {
            minute: 60,
            ..date(2040, 12, 25)
        }),
        None
    );
    assert_eq!(
        encode_mjd_bcd(MjdBcdDateTime {
            second: 60,
            ..date(2040, 12, 25)
        }),
        None
    );
    assert_eq!(
        encode_mjd_bcd(MjdBcdDateTime {
            hour: 23,
            minute: 59,
            second: 59,
            ..date(2040, 12, 25)
        }),
        Some(at(0x23, 0x59, 0x59))
    );
}

#[test]
fn the_calendar_gate_refuses_the_days_the_arithmetic_would_roll_over() {
    assert_eq!(ymd_to_mjd(2023, 2, 31), None);
    assert_eq!(ymd_to_mjd(2023, 13, 1), None);
    assert_eq!(ymd_to_mjd(2023, 1, 0), None);
    assert_eq!(ymd_to_mjd(2023, 2, 29), None);
    assert!(ymd_to_mjd(2024, 2, 29).is_some());
    assert_eq!(ymd_to_mjd(2023, 1, 31), Some(59_975));
    assert_eq!(mjd_to_ymd(59_975), (2023, 1, 31));
}

#[cfg(feature = "jiff")]
mod jiff_faces {
    use broadcast_common::stage::Timestamp;
    use broadcast_common::time::{
        MonoUtc, SECS_2000_EPOCH, decode_seconds_since_2000_utc, encode_seconds_since_2000_utc,
    };

    /// `seconds_since_2000` for `jiff::Timestamp::MAX` (9999-12-30T22:00:00Z,
    /// its own last instant): 253 402 207 200 Unix seconds less the 2000
    /// epoch's 946 684 800.
    const LAST_40_BIT_ERA: u64 = 252_455_522_400;

    #[test]
    fn the_whole_representable_timestamp_range_encodes() {
        assert_eq!(
            encode_seconds_since_2000_utc(jiff::Timestamp::MAX, 0),
            Some((LAST_40_BIT_ERA, 999_999_999))
        );
        assert_eq!(
            encode_seconds_since_2000_utc(jiff::Timestamp::MIN, 0),
            None,
            "a date before the 2000 epoch has no seconds_since_2000"
        );
        assert_eq!(
            decode_seconds_since_2000_utc(LAST_40_BIT_ERA, 999_999_999, 0),
            Some(jiff::Timestamp::MAX)
        );
    }

    #[test]
    fn the_anchor_moves_by_exactly_the_elapsed_microseconds() {
        let anchor_utc =
            jiff::Timestamp::from_microsecond(1_686_227_696_000_000).expect("in range");
        let mapping = MonoUtc::new(Timestamp::from_micros(1_000_000), anchor_utc);
        assert_eq!(
            mapping.utc_at(Timestamp::from_micros(999_999)),
            jiff::Timestamp::from_microsecond(1_686_227_695_999_999).expect("in range")
        );
        assert_eq!(
            mapping.utc_at(Timestamp::from_micros(1_000_001)),
            jiff::Timestamp::from_microsecond(1_686_227_696_000_001).expect("in range")
        );
        assert_eq!(
            mapping.mono_for_utc(anchor_utc),
            Timestamp::from_micros(1_000_000)
        );
    }

    #[test]
    fn a_reading_off_the_wall_clock_saturates_at_its_own_edge() {
        let late = MonoUtc::new(Timestamp::from_micros(0), jiff::Timestamp::MAX);
        assert_eq!(
            late.utc_at(Timestamp::from_micros(u64::MAX)),
            jiff::Timestamp::MAX
        );
        let early = MonoUtc::new(Timestamp::from_micros(u64::MAX), jiff::Timestamp::MIN);
        assert_eq!(early.utc_at(Timestamp::ZERO), jiff::Timestamp::MIN);
    }

    #[test]
    fn the_epoch_constant_is_the_year_2000() {
        let dt = jiff::tz::TimeZone::UTC
            .to_datetime(jiff::Timestamp::from_second(SECS_2000_EPOCH).expect("in range"));
        assert_eq!((dt.year(), dt.month(), dt.day()), (2000, 1, 1));
    }
}
