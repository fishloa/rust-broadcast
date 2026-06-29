//! Real `EXT-X-DATERANGE` lines from a production packager (Unified Streaming).
//! The SCTE35-OUT hex is the splice input; PLANNED-DURATION is the golden output.
use broadcast_common::traits::Parse;
use scte35_splice::SpliceInfoSection;
use timed_metadata::convert::scte35_to_daterange;
use timed_metadata::daterange::{DateRange, Scte35Cue};
use timed_metadata::event::TimedEvent;
use timed_metadata::TimeAnchor;

fn check(line: &str, expect_id: &str) {
    // 1. Parse the real DATERANGE line.
    let dr = DateRange::parse_tag_line(line.trim()).expect("parse fixture line");
    assert_eq!(dr.id, expect_id);
    assert_eq!(dr.planned_duration, Some(24.0));
    let attr = dr.scte35.as_ref().expect("scte35 attr");
    assert_eq!(attr.cue, Scte35Cue::Out);

    // 2. The hex IS a valid splice; break_duration = 2160000 ticks = 24s.
    let section = SpliceInfoSection::parse(&attr.raw).expect("hex decodes to splice");
    let ev = TimedEvent::from_scte35(&section, &attr.raw).unwrap();
    assert_eq!(ev.duration.unwrap().0, 2_160_000);

    // 3. Round-trip our converter: feed splice + anchor at the fixture's START-DATE.
    //    (anchor epoch arbitrary here; we assert duration + lossless hex, not START-DATE.)
    let anchor = TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: 0,
    };
    let regen = scte35_to_daterange(&ev, &anchor).unwrap();
    assert_eq!(regen.planned_duration, Some(24.0));
    assert_eq!(regen.scte35.unwrap().raw, attr.raw); // verbatim survives
}

#[test]
fn unified_daterange_2002() {
    check(include_str!("fixtures/daterange_2002.txt"), "2002");
}

#[test]
fn unified_daterange_2004() {
    check(include_str!("fixtures/daterange_2004.txt"), "2004");
}
