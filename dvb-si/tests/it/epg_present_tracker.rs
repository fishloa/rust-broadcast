//! Real-capture validation of [`PresentEventTracker`].
//!
//! Fixture: `fixtures/ts/tnt-5w-12732v-isi6-10s.ts`, a real 10 s DVB-T
//! capture. TF1 (`service_id` `0x0601`) carries an EIT present/following
//! actual table whose present event is `event_id` `0x7857` ("50' inside...").
//! The capture is a snapshot, so it carries no genuine present event
//! transition; the tracker must report exactly one `Baseline` and no
//! `Transition`. The same capture must also yield that `Baseline` when it
//! starts mid-packet and when it is reframed as 192-byte M2TS or 204-byte
//! Reed-Solomon packets (`feed_ts` recovers alignment through
//! `mpeg_ts::resync::TsResync`). Ground truth cross-checked with
//! `cargo run -p dvb-tools -- epg fixtures/ts/tnt-5w-12732v-isi6-10s.ts --json`.
#![cfg(all(feature = "ts", feature = "jiff"))]

use dvb_si::epg::{PRESENT_CHANGE_QUEUE_CAP, PresentEventChange, PresentEventTracker};
use mpeg_ts::resync::{M2TS_EXTRA_HEADER_LEN, RS_PARITY_LEN};
use mpeg_ts::ts::TS_PACKET_SIZE;

const TF1_SERVICE_ID: u16 = 0x0601;
const TF1_PRESENT_EVENT_ID: u16 = 0x7857;

#[test]
fn odd_sized_reads_report_one_baseline_and_no_transition() {
    let ts_bytes = test_fixtures::read("ts/tnt-5w-12732v-isi6-10s.ts");
    let mut tracker = PresentEventTracker::new(TF1_SERVICE_ID);

    for chunk in ts_bytes.chunks(1000) {
        tracker.feed_ts(chunk);
    }

    let changes = drain(&mut tracker);

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

/// Drain the tracker's bounded change queue.
fn drain(tracker: &mut PresentEventTracker) -> Vec<PresentEventChange> {
    let mut changes = Vec::new();
    for _ in 0..=PRESENT_CHANGE_QUEUE_CAP {
        let Some(change) = tracker.poll_change() else {
            break;
        };
        changes.push(change);
    }
    changes
}

fn feed_in_chunks(tracker: &mut PresentEventTracker, bytes: &[u8]) {
    for chunk in bytes.chunks(1000) {
        tracker.feed_ts(chunk);
    }
}

/// The capture reframed as `4-byte TP_extra_header + 188-byte TS` packets.
fn as_m2ts(ts_bytes: &[u8]) -> Vec<u8> {
    let header = [0x40, 0x00, 0x12, 0x34];
    assert_eq!(header.len(), M2TS_EXTRA_HEADER_LEN);
    let packet_len = TS_PACKET_SIZE + M2TS_EXTRA_HEADER_LEN;
    let mut out = Vec::with_capacity(ts_bytes.len() / TS_PACKET_SIZE * packet_len);
    for packet in ts_bytes.chunks_exact(TS_PACKET_SIZE) {
        out.extend_from_slice(&header);
        out.extend_from_slice(packet);
    }
    out
}

/// The capture reframed as `188-byte TS + 16 parity bytes` packets.
fn as_rs(ts_bytes: &[u8]) -> Vec<u8> {
    let parity = [0u8; RS_PARITY_LEN];
    let packet_len = TS_PACKET_SIZE + RS_PARITY_LEN;
    let mut out = Vec::with_capacity(ts_bytes.len() / TS_PACKET_SIZE * packet_len);
    for packet in ts_bytes.chunks_exact(TS_PACKET_SIZE) {
        out.extend_from_slice(packet);
        out.extend_from_slice(&parity);
    }
    out
}

fn assert_single_baseline(tracker: &mut PresentEventTracker) {
    let changes = drain(tracker);
    assert_eq!(changes.len(), 1, "the capture carries one present event");
    match &changes[0] {
        PresentEventChange::Baseline(present) => {
            assert_eq!(present.service_id, TF1_SERVICE_ID);
            assert_eq!(present.event.event_id, TF1_PRESENT_EVENT_ID);
            assert_eq!(
                present.event.event_name.as_deref(),
                Some("50' inside. \"Le mag\" (55/106)")
            );
        }
        other => panic!("expected one Baseline, got {other:?}"),
    }
}

#[test]
fn a_stream_starting_mid_packet_still_reports_the_baseline() {
    let ts_bytes = test_fixtures::read("ts/tnt-5w-12732v-isi6-10s.ts");
    let mut tracker = PresentEventTracker::new(TF1_SERVICE_ID);
    feed_in_chunks(&mut tracker, &ts_bytes[100..]);
    assert_single_baseline(&mut tracker);
}

#[test]
fn an_m2ts_capture_still_reports_the_baseline() {
    let ts_bytes = test_fixtures::read("ts/tnt-5w-12732v-isi6-10s.ts");
    let m2ts = as_m2ts(&ts_bytes);
    assert_eq!(m2ts.len() % (TS_PACKET_SIZE + M2TS_EXTRA_HEADER_LEN), 0);

    let mut tracker = PresentEventTracker::new(TF1_SERVICE_ID);
    feed_in_chunks(&mut tracker, &m2ts);
    assert_single_baseline(&mut tracker);
}

#[test]
fn a_204_byte_capture_still_reports_the_baseline() {
    let ts_bytes = test_fixtures::read("ts/tnt-5w-12732v-isi6-10s.ts");
    let rs = as_rs(&ts_bytes);

    let mut tracker = PresentEventTracker::new(TF1_SERVICE_ID);
    feed_in_chunks(&mut tracker, &rs);
    assert_single_baseline(&mut tracker);
}
