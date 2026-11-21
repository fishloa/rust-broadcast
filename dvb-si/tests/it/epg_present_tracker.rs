//! Real-capture validation of [`PresentEventTracker`].
//!
//! Fixture: `fixtures/ts/tnt-5w-12732v-isi6-10s.ts`, a real 10 s DVB-T
//! capture. TF1 (`service_id` `0x0601`) carries an EIT present/following
//! actual table whose present event is `event_id` `0x7857` ("50' inside...").
//! The capture is a snapshot, so it carries no genuine present event
//! transition; the tracker must report exactly one `Baseline` and no
//! `Transition`. Ground truth cross-checked with
//! `cargo run -p dvb-tools -- epg fixtures/ts/tnt-5w-12732v-isi6-10s.ts --json`.
#![cfg(all(feature = "ts", feature = "jiff"))]

use dvb_si::epg::{PRESENT_CHANGE_QUEUE_CAP, PresentEventChange, PresentEventTracker};

const TF1_SERVICE_ID: u16 = 0x0601;
const TF1_PRESENT_EVENT_ID: u16 = 0x7857;

#[test]
fn odd_sized_reads_report_one_baseline_and_no_transition() {
    let ts_bytes = test_fixtures::read("ts/tnt-5w-12732v-isi6-10s.ts");
    let mut tracker = PresentEventTracker::new(TF1_SERVICE_ID);

    for chunk in ts_bytes.chunks(1000) {
        tracker.feed_ts(chunk);
    }

    let mut changes = Vec::new();
    for _ in 0..=PRESENT_CHANGE_QUEUE_CAP {
        let Some(change) = tracker.poll_change() else {
            break;
        };
        changes.push(change);
    }

    assert_eq!(changes.len(), 1, "the snapshot carries one present event");
    match &changes[0] {
        PresentEventChange::Baseline(present) => {
            assert_eq!(present.service_id, TF1_SERVICE_ID);
            assert_eq!(present.event.event_id, TF1_PRESENT_EVENT_ID);
            assert_eq!(
                present.event.event_name.as_deref(),
                Some("50' inside. \"Le mag\" (55/106)")
            );
            assert_eq!(
                present.event.start_time.map(|t| t.to_string()).as_deref(),
                Some("2026-04-11T00:45:00Z")
            );
            assert_eq!(present.event.duration.map(|d| d.as_secs()), Some(1920));
        }
        other => panic!("expected one Baseline, got {other:?}"),
    }

    let current = tracker.current().expect("a present event is tracked");
    assert_eq!(current.service_id, TF1_SERVICE_ID);
    assert_eq!(current.event.event_id, TF1_PRESENT_EVENT_ID);
    assert_eq!(tracker.changes_dropped(), 0);
}
