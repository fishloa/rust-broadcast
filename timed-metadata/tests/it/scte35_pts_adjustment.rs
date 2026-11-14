//! Regression fixtures for issues #1039 (`pts_adjustment` ignored) and #1040
//! (`time_signal` cues lose time/kind/id). Both `.bin` fixtures are compiled
//! by TSDuck's `tstabcomp` (an independent tool) from synthetic XML input --
//! see `fixtures/scte35/PROVENANCE.md` for the exact command and the oracle
//! values read from `tstabcomp --decompile`'s own output, not from this
//! crate's parser or serializer.
use broadcast_common::Serialize;
use broadcast_common::traits::Parse;
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::{AnyCommand, SpliceInsert};
use scte35_splice::time::SpliceTime;
use std::fs;
use timed_metadata::event::{EventKind, MediaDuration, MediaTime, TimedEvent};
use timed_metadata::{TimeAnchor, Timeline};

fn fixture(name: &str) -> Vec<u8> {
    let path = test_fixtures::path(&format!("scte35/{name}"));
    fs::read(&path).unwrap_or_else(|e| panic!("read fixture {path:?}: {e}"))
}

/// #1039: `splice_insert` with a non-zero `pts_adjustment` (TSDuck oracle:
/// `pts_time = 900,000`, `pts_adjustment = 8,100,000`) must report the
/// *adjusted* time, `900,000 + 8,100,000 = 9,000,000`, not the raw
/// `pts_time`.
#[test]
fn splice_insert_applies_pts_adjustment() {
    let raw = fixture("splice-insert-pts-adjustment.bin");
    let section = scte35_splice::SpliceInfoSection::parse(&raw).unwrap();
    let ev = TimedEvent::from_scte35(&section, &raw).unwrap();
    assert_eq!(ev.id, Some(2002));
    assert_eq!(ev.kind, EventKind::BreakStart);
    assert_eq!(
        ev.at,
        Some(MediaTime(9_000_000)),
        "pts_time (900,000) + pts_adjustment (8,100,000) per TSDuck's own decompile"
    );
    assert_eq!(ev.duration, Some(MediaDuration(2_160_000)));
}

/// #1040: a `time_signal` cue (TSDuck oracle: `pts_time = 1,234,567`) with a
/// `splice_segmentation_descriptor` (`segmentation_event_id = 0x4800000A`,
/// `segmentation_type_id = 0x22` "Break Start", `segmentation_duration =
/// 900,000`) must surface the segmentation descriptor's id/kind/duration and
/// the cue's own time, not the all-`None`/`Unspecified`/`ID=""` result the
/// unfixed code produces for every `time_signal`.
#[test]
fn time_signal_reads_segmentation_descriptor() {
    let raw = fixture("time-signal-segmentation.bin");
    let section = scte35_splice::SpliceInfoSection::parse(&raw).unwrap();
    let ev = TimedEvent::from_scte35(&section, &raw).unwrap();
    assert_eq!(ev.at, Some(MediaTime(1_234_567)), "time_signal pts_time");
    assert_eq!(ev.id, Some(0x4800000A), "segmentation_event_id");
    assert_eq!(ev.kind, EventKind::BreakStart, "segmentation_type_id 0x22");
    assert_eq!(ev.duration, Some(MediaDuration(900_000)));
}

/// #1040 (DATERANGE side): a `time_signal` cue with no segmentation
/// descriptor -- and so no id -- must not silently become `ID=""` (RFC
/// 8216bis §4.4.5.1 requires unique, non-conflicting `ID`s). Built from a
/// bare TSDuck-compiled `time_signal` with no descriptor.
#[test]
fn time_signal_without_id_errors_on_daterange_conversion() {
    // A minimal, spec-correct time_signal with no descriptor loop, so the
    // event's id is genuinely absent (not just untested).
    let section = scte35_splice::SpliceInfoSection::new_clear(
        scte35_splice::commands::AnyCommand::TimeSignal(scte35_splice::commands::TimeSignal {
            splice_time: scte35_splice::time::SpliceTime::with_pts(1_000),
        }),
        &[],
    );
    let raw = broadcast_common::traits::Serialize::try_to_bytes(&section).unwrap();
    let ev = TimedEvent::from_scte35(&section, &raw).unwrap();
    assert_eq!(ev.id, None);

    let anchor = timed_metadata::anchor::TimeAnchor {
        pts_90k: 0,
        utc_epoch_ms: 0,
    };
    let result = timed_metadata::convert::scte35_to_daterange(&ev, &anchor);
    assert!(
        matches!(result, Err(timed_metadata::Error::MissingEventId)),
        "a time_signal with no id must error, not silently emit ID=\"\""
    );
}

fn splice_at_pts(pts: u64) -> Vec<u8> {
    let cmd = SpliceInsert {
        splice_event_id: 7,
        out_of_network_indicator: true,
        splice_time: Some(SpliceTime::with_pts(pts)),
        ..Default::default()
    };
    SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(cmd), &[])
        .try_to_bytes()
        .unwrap()
}

#[test]
fn a_cue_just_after_the_33_bit_wrap_starts_just_after_an_anchor_just_before_it() {
    const WRAP: u64 = 1 << 33;
    let anchor = TimeAnchor {
        pts_90k: WRAP - 90_000,
        utc_epoch_ms: 1_700_000_000_000,
    };
    let mut tl = Timeline::new(anchor);
    let ev = tl.push_scte35(&splice_at_pts(90_000)).unwrap();
    let dr = tl.to_daterange(&ev).unwrap();
    assert_eq!(
        dr.start_date, "2023-11-14T22:13:22.000Z",
        "cue 2 s after the anchor (across the PTS wrap) must start 2 s after the anchor UTC"
    );
}
