//! Exact error variants (and fields) of the converters and the DATERANGE parser.

use broadcast_common::timescale::Timescale;
use mp4_emsg::{EMSG_FLAGS, EmsgBox, PresentationTime};
use timed_metadata::convert::{SegmentTiming, emsg_to_scte35, emsg_to_v0, emsg_to_v1};
use timed_metadata::{DateRange, Error};

const SCTE35_SCHEME: &str = "urn:scte:scte35:2013:bin";
const TIMESCALE: Timescale = Timescale::const_from_nonzero(90_000);
const MILLIS: Timescale = Timescale::const_from_nonzero(1_000);

fn emsg(scheme: &'static str, presentation_time: PresentationTime) -> EmsgBox<'static> {
    EmsgBox {
        flags: EMSG_FLAGS,
        scheme_id_uri: scheme,
        value: "1",
        timescale: TIMESCALE,
        presentation_time,
        event_duration: 0,
        id: 1,
        message_data: &[0xFC, 0x30],
    }
}

fn timing(ept: u64) -> SegmentTiming {
    SegmentTiming {
        earliest_presentation_time: ept,
        presentation_time_offset: 0,
        timescale: TIMESCALE,
    }
}

#[test]
fn a_non_scte35_emsg_names_its_scheme() {
    let bytes = emsg("urn:mpeg:dash:event:2012", PresentationTime::Delta(0))
        .to_vec()
        .unwrap();
    match emsg_to_scte35(&bytes) {
        Err(Error::UnsupportedScheme { scheme }) => assert_eq!(scheme, "urn:mpeg:dash:event:2012"),
        other => panic!("{other:?}"),
    }
    let ok = emsg(SCTE35_SCHEME, PresentationTime::Delta(0))
        .to_vec()
        .unwrap();
    assert_eq!(emsg_to_scte35(&ok).unwrap(), vec![0xFC, 0x30]);
}

#[test]
fn a_delta_beyond_u32_is_reported_with_its_value() {
    let ept = 1_000;
    let delta = u64::from(u32::MAX) + 5;
    let e = emsg(SCTE35_SCHEME, PresentationTime::Absolute(ept + delta));
    assert!(matches!(
        emsg_to_v0(&e, &timing(ept)),
        Err(Error::EmsgDeltaOverflow(d)) if d == delta
    ));
    let at_limit = emsg(
        SCTE35_SCHEME,
        PresentationTime::Absolute(ept + u64::from(u32::MAX)),
    );
    let v0 = emsg_to_v0(&at_limit, &timing(ept)).unwrap();
    assert_eq!(v0.presentation_time, PresentationTime::Delta(u32::MAX));
}

#[test]
fn a_presentation_time_before_the_segment_start_cannot_be_a_v0_delta() {
    let e = emsg(SCTE35_SCHEME, PresentationTime::Absolute(99));
    assert!(matches!(
        emsg_to_v0(&e, &timing(100)),
        Err(Error::EmsgPresentationTimeBeforeEpt)
    ));
}

#[test]
fn a_timescale_mismatch_reports_both_timescales() {
    let e = emsg(SCTE35_SCHEME, PresentationTime::Delta(1));
    let mut t = timing(0);
    t.timescale = MILLIS;
    assert!(matches!(
        emsg_to_v1(&e, &t),
        Err(Error::EmsgTimescaleMismatch {
            emsg: TIMESCALE,
            timing: MILLIS
        })
    ));
    assert!(matches!(
        emsg_to_v0(&e, &t),
        Err(Error::EmsgTimescaleMismatch {
            emsg: TIMESCALE,
            timing: MILLIS
        })
    ));
}

fn attr_parse(line: &str) -> String {
    match DateRange::parse_tag_line(line) {
        Err(Error::AttrParse(msg)) => msg,
        other => panic!("{line}: {other:?}"),
    }
}

#[test]
fn daterange_parse_failures_carry_a_reason() {
    assert_eq!(
        attr_parse("#EXT-X-PART:ID=\"a\""),
        "missing #EXT-X-DATERANGE: prefix"
    );
    assert_eq!(
        attr_parse("#EXT-X-DATERANGE:START-DATE=\"2026-01-01T00:00:00Z\""),
        "DATERANGE missing ID"
    );
    let base = "#EXT-X-DATERANGE:ID=\"a\",START-DATE=\"2026-01-01T00:00:00Z\"";
    assert_eq!(
        attr_parse(&format!("{base},DURATION=abc")),
        "bad number: abc"
    );
    assert_eq!(
        attr_parse(&format!("{base},SCTE35-OUT=0xABC")),
        "odd-length hex"
    );
    assert_eq!(attr_parse(&format!("{base},SCTE35-IN=0xZZ")), "bad hex");
    assert_eq!(attr_parse(&format!("{base},SCTE35-CMD=0xGG00")), "bad hex");
}

#[test]
fn undecodable_emsg_bytes_surface_the_emsg_parser_error() {
    let bytes = [0u8; 4];
    let expected = EmsgBox::parse(&bytes).unwrap_err();
    assert_eq!(emsg_to_scte35(&bytes).unwrap_err(), Error::Emsg(expected));
}
