//! RFC 3339 range/overflow behaviour of the anchor (SP5, jiff).
use std::fs;

use broadcast_common::traits::Parse;
use scte35_splice::SpliceInfoSection;
use timed_metadata::anchor::{format_rfc3339_ms, try_format_rfc3339_ms};
use timed_metadata::convert::scte35_to_daterange;
use timed_metadata::daterange::DateRange;
use timed_metadata::event::{MediaTime, TimedEvent};
use timed_metadata::{Error, TimeAnchor};

/// `jiff::Timestamp::MIN` (-9999-01-01T00:00:00Z) in epoch milliseconds.
const MIN_MS: i64 = -377_705_116_800_000;
/// `jiff::Timestamp::MAX` truncated to milliseconds (9999-12-31T23:59:59.999Z).
const MAX_MS: i64 = 253_402_300_799_999;

#[test]
fn the_fallible_form_reports_out_of_range_epochs() {
    for ms in [i64::MAX, i64::MIN, MAX_MS + 1, MIN_MS - 1] {
        assert!(
            matches!(try_format_rfc3339_ms(ms), Err(Error::TimestampOutOfRange(v)) if v == ms),
            "{ms}"
        );
    }
    assert_eq!(
        try_format_rfc3339_ms(MAX_MS).unwrap(),
        "9999-12-31T23:59:59.999Z"
    );
    assert!(try_format_rfc3339_ms(MIN_MS).is_ok());
}

#[test]
fn the_infallible_form_clamps_instead_of_printing_a_garbage_year() {
    assert_eq!(format_rfc3339_ms(i64::MAX), "9999-12-31T23:59:59.999Z");
    assert_eq!(
        format_rfc3339_ms(i64::MIN),
        try_format_rfc3339_ms(MIN_MS).unwrap()
    );
}

#[test]
fn media_time_arithmetic_saturates_instead_of_wrapping_or_panicking() {
    let hi = TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: i64::MAX,
    };
    assert_eq!(hi.media_to_epoch_ms(MediaTime(u64::MAX)), i64::MAX);
    let lo = TimeAnchor {
        pts_90k: u64::MAX,
        utc_epoch_ms: i64::MIN,
    };
    assert_eq!(lo.media_to_epoch_ms(MediaTime(0)), i64::MIN);
    // A PTS above i64::MAX must still count as LATER than one below it.
    let a = TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: 0,
    };
    assert!(a.media_to_epoch_ms(MediaTime(1 << 63)) > a.media_to_epoch_ms(MediaTime(1)));
}

#[test]
fn the_converter_errors_instead_of_emitting_an_unrepresentable_start_date() {
    let line = fs::read_to_string(format!(
        "{}/../fixtures/timed-metadata/daterange_2002.txt",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("fixture");
    let dr = DateRange::parse_tag_line(line.trim()).unwrap();
    let raw = dr.scte35.unwrap().raw;
    let section = SpliceInfoSection::parse(&raw).unwrap();
    let ev = TimedEvent::from_scte35(&section, &raw).unwrap();
    let anchor = TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: i64::MAX,
    };
    assert!(matches!(
        scte35_to_daterange(&ev, &anchor),
        Err(Error::TimestampOutOfRange(_))
    ));
}

#[test]
fn well_known_instants_format_as_before() {
    assert_eq!(format_rfc3339_ms(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(format_rfc3339_ms(-1), "1969-12-31T23:59:59.999Z");
    assert_eq!(
        format_rfc3339_ms(951_782_400_000),
        "2000-02-29T00:00:00.000Z"
    );
    assert_eq!(
        format_rfc3339_ms(4_107_542_400_000),
        "2100-03-01T00:00:00.000Z"
    );
    // RFC 3339 §5.8 example "1985-04-12T23:20:50.52Z".
    assert_eq!(
        format_rfc3339_ms(482_196_050_520),
        "1985-04-12T23:20:50.520Z"
    );
}
