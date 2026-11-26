//! The formatting and conversion faces of the shared value types (audit 21,
//! #2926): a `Display`/`Debug` that renders nothing and a `From`/`TryFrom`
//! that answers a default both compile silently, so the exact text and the
//! exact converted value are pinned here.

use core::time::Duration;

use broadcast_common::backoff::{GrowthFactor, InvalidBackoff, RetrySchedule};
use broadcast_common::clock33::{Clock27, Clock33, ClockOutOfRange};
use broadcast_common::klv::KlvError;
use broadcast_common::net::{InterfaceError, JoinMismatch};
use broadcast_core::bits::BitError;
use broadcast_core::timescale::Timescale;

#[test]
fn a_27_mhz_clock_reads_in_seconds() {
    assert_eq!(
        Clock27::from_27mhz(27_000_000)
            .expect("one second")
            .seconds(),
        1.0
    );
    assert_eq!(
        Clock27::from_27mhz(54_000_000)
            .expect("two seconds")
            .seconds(),
        2.0
    );
    assert_eq!(
        Clock27::from_27mhz(13_500_000)
            .expect("half a second")
            .seconds(),
        0.5
    );
    assert_eq!(Clock27::from_27mhz(0).expect("the origin").seconds(), 0.0);
}

#[test]
fn a_33_bit_clock_sample_converts_from_its_own_width() {
    assert_eq!(Clock33::try_from(5i64).expect("five ticks").ticks(), 5);
    assert_eq!(Clock33::try_from(0i64).expect("zero ticks").ticks(), 0);
    assert_eq!(
        Clock33::try_from(-5i64)
            .expect("a negative sample reports its magnitude")
            .ticks(),
        5
    );
    assert_eq!(
        Clock33::try_from(u64::MAX).err().map(|e| e.value),
        Some(u64::MAX)
    );
    let out_of_range: ClockOutOfRange =
        Clock33::try_new(1u64 << 33).expect_err("2^33 is past the field");
    assert_eq!(out_of_range.field, "33-bit ticks");
    assert_eq!(out_of_range.value, 8_589_934_592);
    assert_eq!(
        out_of_range.to_string(),
        "33-bit ticks value 8589934592 is out of range"
    );
    let negative = Clock33::try_from(i64::MIN).expect_err("i64::MIN's magnitude is past the field");
    assert_eq!(negative.value, 9_223_372_036_854_775_808u64);
}

#[test]
fn the_clock_values_render_their_tick_counts() {
    let ticks = Clock33::try_new(90_000).expect("90 kHz");
    assert_eq!(ticks.to_string(), "90000");
    assert_eq!(format!("{ticks:?}"), "Clock33(90000)");
    let pcr = Clock27::from_27mhz(27_000_000).expect("27 MHz");
    assert_eq!(pcr.to_string(), "27000000");
    assert_eq!(format!("{pcr:?}"), "Clock27(27000000)");
}

#[test]
fn a_timescale_converts_to_each_wider_integer() {
    let timescale = Timescale::try_new(90_000).expect("90 kHz");
    assert_eq!(timescale.to_string(), "90000");
    assert_eq!(format!("{timescale:?}"), "90000");
    assert_eq!(u64::from(timescale), 90_000);
    assert_eq!(i128::from(timescale), 90_000);
    assert_eq!(usize::from(timescale), 90_000);
    assert_eq!(u32::from(timescale), 90_000);
    assert_eq!(timescale.as_u64(), 90_000);
    assert_eq!(Timescale::try_new(48_000).map(usize::from), Ok(48_000));
}

#[test]
fn a_schedule_renders_its_bounds() {
    let schedule = RetrySchedule::new(Duration::from_secs(1), Duration::from_secs(60), 2.0, 1)
        .expect("a one-second floor");
    assert_eq!(
        format!("{schedule:?}"),
        "RetrySchedule { min: 1s, max: 60s, .. }"
    );
}

#[test]
fn a_backoff_refusal_renders_its_bound() {
    assert_eq!(
        InvalidBackoff::CapBelowFloor {
            floor: Duration::from_secs(1),
            cap: Duration::ZERO
        }
        .to_string(),
        "retry cap 0ns is below the first retry's 1s"
    );
    assert_eq!(
        InvalidBackoff::FloorAboveCeiling {
            floor: Duration::from_secs(60),
            ceiling: Duration::from_secs(30)
        }
        .to_string(),
        "retry floor 60s is above the largest accepted 30s"
    );
    assert_eq!(
        InvalidBackoff::GrowthFactor {
            factor: GrowthFactor::Nan
        }
        .to_string(),
        "retry growth factor is NaN"
    );
    assert_eq!(
        InvalidBackoff::GrowthFactor {
            factor: GrowthFactor::Below(0.5)
        }
        .to_string(),
        "retry growth factor 0.5 is under 1"
    );
    assert_eq!(GrowthFactor::Nan.name(), "is NaN");
    assert_eq!(GrowthFactor::Below(0.25).name(), "is under 1");
}

#[test]
fn an_interface_error_names_its_reason_and_appends_the_text() {
    let address = InterfaceError::Address("1.2.3".to_string());
    assert_eq!(
        address.name(),
        "not an IPv4 address (and not an interface name or index)"
    );
    assert_eq!(
        address.to_string(),
        "not an IPv4 address (and not an interface name or index): 1.2.3"
    );
    let index = InterfaceError::Index("4294967296".to_string());
    assert_eq!(index.name(), "not an interface index (above the u32 range)");
    assert_eq!(
        index.to_string(),
        "not an interface index (above the u32 range): 4294967296"
    );
}

#[test]
fn a_join_mismatch_names_its_reason() {
    assert_eq!(
        JoinMismatch::Group.reason(),
        "the joined address is not a multicast group"
    );
    assert_eq!(
        JoinMismatch::Family.reason(),
        "multicast interface kind does not match the group's address family"
    );
    assert_eq!(
        JoinMismatch::SourceFamily.reason(),
        "multicast source must be a unicast address of the group's family"
    );
}

#[test]
fn a_bit_error_renders_each_condition() {
    assert_eq!(
        BitError::OutOfBounds {
            needed_bits: 5,
            remaining_bits: 3
        }
        .to_string(),
        "bit buffer out of bounds: need 5 bit(s), 3 remaining"
    );
    assert_eq!(
        BitError::TooManyBits { requested: 65 }.to_string(),
        "requested 65 bits exceeds the 64-bit carrier"
    );
    assert_eq!(
        BitError::ValueTooWide {
            value: 0x1ff,
            bits: 8
        }
        .to_string(),
        "value 0x1ff does not fit in 8 bit(s)"
    );
}

#[test]
fn a_klv_error_renders_each_condition() {
    assert_eq!(
        KlvError::BerIndefiniteLength.to_string(),
        "BER length used the reserved indefinite-length token (0x80)"
    );
    assert_eq!(
        KlvError::FixedBerLengthTooSmall { len: 128, width: 1 }.to_string(),
        "length 128 does not fit in the fixed BER width of 1 byte(s)"
    );
    assert_eq!(
        KlvError::BerLengthTooLong { bytes: 9 }.to_string(),
        "BER long-form length has 9 following bytes, exceeding the 8-byte cap"
    );
}

#[test]
fn a_klv_error_wrapping_a_shared_kind_renders_it_without_a_source() {
    use broadcast_core::error::{BufferTooShort, InvalidValue, OutputBufferTooSmall};
    use core::error::Error as _;

    let short = KlvError::BufferTooShort(BufferTooShort {
        need: 4,
        have: 1,
        what: "KLV",
    });
    assert_eq!(
        short.to_string(),
        "buffer too short: need 4 bytes, have 1 (while parsing KLV)"
    );
    assert!(short.source().is_none());
    let small = KlvError::OutputBufferTooSmall(OutputBufferTooSmall { need: 4, have: 1 });
    assert_eq!(small.to_string(), "output buffer too small: need 4, have 1");
    assert!(small.source().is_none());
    let invalid = KlvError::InvalidLength(InvalidValue {
        field: "length",
        value: 7,
        reason: "bad",
    });
    assert_eq!(invalid.to_string(), "field length value 7 invalid: bad");
    assert!(invalid.source().is_none());
}

#[cfg(feature = "xml")]
#[test]
fn an_xml_reader_error_renders_the_readers_message() {
    use broadcast_common::xml_chars::XmlCharError;

    assert_eq!(
        XmlCharError::Reader {
            message: "bad markup".to_string()
        }
        .to_string(),
        "bad markup"
    );
}
