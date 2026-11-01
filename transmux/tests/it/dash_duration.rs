//! `parse_iso8601_duration` against the XML Schema 1.1 `xs:duration` lexical
//! space (Part 2 §3.3.6): every valid form representable as an unsigned
//! `core::time::Duration` is accepted, everything else is a structured
//! `InvalidDuration` (never a panic).
use core::time::Duration;
use transmux::dash_parse::{DashParseError, parse_iso8601_duration};

fn ok(s: &str) -> Duration {
    parse_iso8601_duration(s).unwrap_or_else(|e| panic!("{s:?}: {e}"))
}

fn assert_invalid(bad: &str) {
    assert!(
        matches!(
            parse_iso8601_duration(bad),
            Err(DashParseError::InvalidDuration { .. })
        ),
        "{bad:?} must be an InvalidDuration"
    );
}

#[test]
fn xml_schema_duration_examples() {
    assert_eq!(ok("PT1H2M3.5S"), Duration::new(3723, 500_000_000));
    assert_eq!(ok("PT0S"), Duration::ZERO);
    assert_eq!(ok("P0D"), Duration::ZERO);
    assert_eq!(ok("PT2.0S"), Duration::from_secs(2));
    assert_eq!(ok("P1DT2H"), Duration::from_secs(93_600));
    assert_eq!(ok("P2D"), Duration::from_secs(172_800));
    assert_eq!(ok("PT36H"), Duration::from_secs(129_600));
    assert_eq!(ok("PT0.000000001S"), Duration::new(0, 1));
    assert_eq!(ok("  PT4S  "), Duration::from_secs(4));
    assert_eq!(ok("PT007S"), Duration::from_secs(7));
    // XML Schema Part 2 §3.2.6's own examples (the ones without calendar units).
    assert_eq!(ok("P120D"), Duration::from_secs(10_368_000));
    assert_eq!(ok("PT1004199059S"), Duration::from_secs(1_004_199_059));
    assert_eq!(ok("PT130S"), Duration::from_secs(130));
    assert_eq!(ok("PT2M10S"), Duration::from_secs(130));
    assert_eq!(ok("P1DT2S"), Duration::from_secs(86_402));
}

/// More than nine fractional digits is valid XSD (encoders printing an f64 emit
/// them); the value is truncated to nanoseconds, as the pre-jiff parser did.
#[test]
fn long_fractions_are_truncated_to_nanoseconds() {
    assert_eq!(ok("PT1.123456789123S"), Duration::new(1, 123_456_789));
    assert_eq!(ok("PT3.6666666666666665S"), Duration::new(3, 666_666_666));
    assert_eq!(ok("PT0.0000000001S"), Duration::ZERO);
    assert_eq!(ok("PT0.9999999999S"), Duration::new(0, 999_999_999));
}

/// Lexically valid xs:duration values a `Duration` cannot hold.
#[test]
fn valid_xsd_but_unrepresentable_values_are_structured_errors() {
    // Calendar units need a reference date.
    for s in [
        "P1Y",
        "P1M",
        "P1Y2M",
        "P1Y1DT1S",
        "P1Y2M3DT4H5M6.7S",
        "P1Y2M3DT10H30M",
    ] {
        assert_invalid(s);
    }
    // Negative (valid XSD; Duration is unsigned).
    for s in ["-PT1S", "-P1D", "-P0D"] {
        assert_invalid(s);
    }
    // Magnitude beyond jiff's span limits: an error, not a panic or a wrap.
    for s in [
        "PT999999999H",
        "P999999999999D",
        "PT99999999999999999999H",
        "PT99999999999999999999S",
    ] {
        assert_invalid(s);
    }
}

/// Everything outside the XSD lexical space.
#[test]
fn lexically_invalid_forms_are_rejected() {
    for bad in [
        "",
        "P",
        "PT",
        "PT1H2",
        "P1DT",
        "T1S",
        "1H",
        "+PT1S",
        "PT+1S",
        "P+1D",
        "PT-1S",
        // lower case anywhere
        "pt1s",
        "Pt1S",
        "PT1h30m",
        "PT1H30m",
        "P1d",
        "PT1s",
        "PT1.5s",
        // fraction rules: a digit on both sides of the point, seconds only
        "PT.5S",
        "PT5.S",
        "PT.S",
        "PT1.5H",
        "PT1.5M",
        "P1.5D",
        "PT1,5S",
        // order, repetition, trailing garbage, units without digits
        "PT1S2",
        "PT1S garbage",
        "PT1M1H",
        "PT1H1H",
        "P1D1Y",
        "PT1S1S",
        "PTS",
        "PTHS",
        "PTMS",
        "PTT1S",
        "PT1H30",
        "1h 30m",
        "P1W",
        "P1WT1S",
    ] {
        assert_invalid(bad);
    }
}
