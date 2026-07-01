//! Real-fixture integration tests for emsg version conversion.
//!
//! Validates `emsg_to_v1`/`emsg_to_v0` against the committed real broadcast
//! captures.
use std::fs;

use mp4_emsg::{EmsgBox, EmsgVersion, PresentationTime};
use timed_metadata::convert::{emsg_to_v0, emsg_to_v1, SegmentTiming};

fn read_fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/../fixtures/shared/{}", env!("CARGO_MANIFEST_DIR"), name);
    fs::read(path).unwrap_or_else(|e| panic!("fixture {name} must be present: {e}"))
}

#[test]
fn v1_fixture_round_trips_through_v0_and_back_byte_identical() {
    let raw = read_fixture("emsg_v1_scte35_livesim.bin");
    let v1 = EmsgBox::parse(&raw).unwrap();

    // timescale 90_000, presentation_time Absolute(160_391_675_700_000),
    // event_duration 900_000, id 1_782_129_730
    assert_eq!(v1.timescale, 90_000);
    assert_eq!(
        v1.presentation_time,
        PresentationTime::Absolute(160_391_675_700_000)
    );
    assert_eq!(v1.event_duration, 900_000);
    assert_eq!(v1.id, 1_782_129_730);
    assert_eq!(v1.scheme_id_uri, "urn:scte:scte35:2013:bin");

    // v1 → v0: delta = T - EPT = 160_391_675_700_000 - (160_391_675_700_000 - 90_000) = 90_000
    let timing = SegmentTiming {
        earliest_presentation_time: 160_391_675_700_000 - 90_000,
        presentation_time_offset: 0,
        timescale: 90_000,
    };
    let v0 = emsg_to_v0(&v1, &timing).unwrap();
    assert_eq!(v0.presentation_time, PresentationTime::Delta(90_000));
    assert_eq!(v0.version(), EmsgVersion::SegmentRelative);
    assert_eq!(v0.scheme_id_uri, v1.scheme_id_uri);
    assert_eq!(v0.value, v1.value);
    assert_eq!(v0.timescale, v1.timescale);
    assert_eq!(v0.event_duration, v1.event_duration);
    assert_eq!(v0.id, v1.id);
    assert_eq!(v0.message_data, v1.message_data);

    // v0 → v1: back to original
    let v1_back = emsg_to_v1(&v0, &timing).unwrap();
    assert_eq!(
        v1_back.presentation_time,
        PresentationTime::Absolute(160_391_675_700_000)
    );

    // Full byte-identical round-trip: v1 → v0 → v1 reproduces the real box
    assert_eq!(
        v1_back.to_vec().unwrap(),
        raw,
        "v1→v0→v1 must match original fixture bytes"
    );
}

#[test]
fn v0_fixture_round_trips_through_v1_and_back_byte_identical() {
    let raw = read_fixture("scte35_emsg_v0.bin");
    let v0 = EmsgBox::parse(&raw).unwrap();
    assert_eq!(v0.timescale, 90_000);
    assert_eq!(v0.presentation_time, PresentationTime::Delta(0));

    let timing = SegmentTiming {
        earliest_presentation_time: 500_000,
        presentation_time_offset: 0,
        timescale: 90_000,
    };

    // v0 → v1: T = EPT + delta = 500_000 + 0 = 500_000
    let v1 = emsg_to_v1(&v0, &timing).unwrap();
    assert_eq!(v1.presentation_time, PresentationTime::Absolute(500_000));

    // v1 → v0: back to Delta(0)
    let v0_back = emsg_to_v0(&v1, &timing).unwrap();
    assert_eq!(v0_back.presentation_time, PresentationTime::Delta(0));

    // Byte-identical round-trip
    assert_eq!(
        v0_back.to_vec().unwrap(),
        raw,
        "v0→v1→v0 must match original fixture bytes"
    );
}

#[test]
fn timescale_mismatch_errors() {
    let raw = read_fixture("emsg_v1_scte35_livesim.bin");
    let v1 = EmsgBox::parse(&raw).unwrap();
    let timing = SegmentTiming {
        earliest_presentation_time: 0,
        presentation_time_offset: 0,
        timescale: 48_000,
    };
    assert!(emsg_to_v1(&v1, &timing).is_err());
    assert!(emsg_to_v0(&v1, &timing).is_err());
}

#[test]
fn v0_with_past_start_errors() {
    let raw = read_fixture("scte35_emsg_v0.bin"); // Delta(0)
    let v0 = EmsgBox::parse(&raw).unwrap();

    // Convert v0→v1 first to get v1 with T = 500_000
    let timing_ok = SegmentTiming {
        earliest_presentation_time: 500_000,
        presentation_time_offset: 0,
        timescale: 90_000,
    };
    let v1 = emsg_to_v1(&v0, &timing_ok).unwrap();
    assert_eq!(v1.presentation_time, PresentationTime::Absolute(500_000));

    // Now try to convert v1→v0 with EPT > T (EPT = 1_000_000 > 500_000)
    let timing_past = SegmentTiming {
        earliest_presentation_time: 1_000_000,
        presentation_time_offset: 0,
        timescale: 90_000,
    };
    let err = emsg_to_v0(&v1, &timing_past).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("precedes"),
        "expected 'precedes' in error message, got: {msg}"
    );
}
