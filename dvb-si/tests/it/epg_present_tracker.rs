//! Real-capture validation of [`PresentEventTracker`].
//!
//! Fixture: `fixtures/ts/tnt-5w-12732v-isi6-10s.ts`, a real 10 s DVB-T
//! capture. TF1 (`service_id` `0x0601`) carries an EIT present/following
//! actual table whose present event is `event_id` `0x7857` ("50' inside...")
//! and whose following event is `0x7858`. The capture is a snapshot, so it
//! carries no genuine present event transition; the tracker must report
//! exactly one `Baseline` and no `Transition`. Re-signalling the capture's
//! own following event as running, as a service does at a programme boundary
//! (`docs/dvr-runtime/DESIGN.md` §8.3), must then give exactly one
//! `Transition`, and the same bytes again none. The same capture must also
//! yield that `Baseline` when it starts mid-packet and when it is reframed as
//! 192-byte M2TS or 204-byte Reed-Solomon packets (`feed_ts` recovers
//! alignment through `mpeg_ts::resync::TsResync`). Ground truth cross-checked
//! with
//! `cargo run -p dvb-tools -- epg fixtures/ts/tnt-5w-12732v-isi6-10s.ts --json`.
#![cfg(all(feature = "ts", feature = "jiff"))]

use broadcast_common::{Parse, Serialize};
use dvb_si::demux::{SiDemux, SiDemuxConfig};
use dvb_si::epg::{PRESENT_CHANGE_QUEUE_CAP, PresentEventChange, PresentEventTracker};
use dvb_si::tables::RunningStatus;
use dvb_si::tables::eit::{EitSection, TABLE_ID_PF_ACTUAL};
use mpeg_ts::mux::SectionPacketiser;
use mpeg_ts::pid::Pid;
use mpeg_ts::resync::{M2TS_EXTRA_HEADER_LEN, RS_PARITY_LEN};
use mpeg_ts::ts::TS_PACKET_SIZE;

const TF1_SERVICE_ID: u16 = 0x0601;
const TF1_PRESENT_EVENT_ID: u16 = 0x7857;
const TF1_FOLLOWING_EVENT_ID: u16 = 0x7858;
const EIT_PID: u16 = 0x12;

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

/// The capture's EIT present/following-actual section carrying the TF1
/// following event, wire bytes (the capture splits present and following
/// across two sections).
fn tf1_following_section(ts_bytes: &[u8]) -> Vec<u8> {
    let mut demux = SiDemux::new(SiDemuxConfig::default());
    for packet in ts_bytes.chunks_exact(TS_PACKET_SIZE) {
        for event in demux.feed(packet) {
            let section = event.bytes();
            if u16::from(event.pid()) != EIT_PID || section[0] != TABLE_ID_PF_ACTUAL {
                continue;
            }
            let Ok(parsed) = EitSection::parse(section) else {
                continue;
            };
            if parsed.service_id == TF1_SERVICE_ID
                && parsed
                    .events
                    .iter()
                    .any(|e| e.event_id == TF1_FOLLOWING_EVENT_ID)
            {
                return section.to_vec();
            }
        }
    }
    panic!("the capture must carry TF1's EIT section with the following event");
}

/// The capture's own EIT section with its following event re-signalled as
/// running: what a service sends when the next programme starts.
fn resignalled_following(ts_bytes: &[u8]) -> Vec<u8> {
    let section_bytes = tf1_following_section(ts_bytes);
    let mut section = EitSection::parse(&section_bytes).expect("the capture's EIT section parses");
    for event in &mut section.events {
        event.running_status = if event.event_id == TF1_FOLLOWING_EVENT_ID {
            RunningStatus::Running
        } else {
            RunningStatus::NotRunning
        };
    }
    section.version_number = (section.version_number + 1) & 0x1F;
    let mut out = vec![0u8; section.serialized_len()];
    section
        .serialize_into(&mut out)
        .expect("the re-signalled section serializes");
    out
}

/// The section as 188-byte TS packets on the EIT PID.
fn section_packets(section: &[u8]) -> Vec<u8> {
    let mut packets = Vec::new();
    for packet in SectionPacketiser::new(Pid::from_wire_masked(EIT_PID)).packetise(&[section]) {
        packets.extend_from_slice(&packet);
    }
    packets
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

#[test]
fn a_resignalled_following_event_is_one_transition() {
    let ts_bytes = test_fixtures::read("ts/tnt-5w-12732v-isi6-10s.ts");
    let mut tracker = PresentEventTracker::new(TF1_SERVICE_ID);
    feed_in_chunks(&mut tracker, &ts_bytes);
    assert_single_baseline(&mut tracker);

    let section_bytes = resignalled_following(&ts_bytes);
    let transition = section_packets(&section_bytes);
    feed_in_chunks(&mut tracker, &transition);
    let changes = drain(&mut tracker);
    assert_eq!(
        changes.len(),
        1,
        "the following event becoming present is one transition"
    );
    match &changes[0] {
        PresentEventChange::Transition {
            previous_event_id,
            current,
        } => {
            assert_eq!(*previous_event_id, TF1_PRESENT_EVENT_ID);
            assert_eq!(current.service_id, TF1_SERVICE_ID);
            assert_eq!(current.event.event_id, TF1_FOLLOWING_EVENT_ID);
            assert_eq!(
                current.event.event_name.as_deref(),
                Some("Plus belle la vie, encore...")
            );
            assert_eq!(
                current.event.start_time.map(|t| t.to_string()).as_deref(),
                Some("2026-04-11T01:15:00Z")
            );
            assert_eq!(current.event.duration.map(|d| d.as_secs()), Some(1200));
        }
        other => panic!("expected one Transition, got {other:?}"),
    }

    feed_in_chunks(&mut tracker, &transition);
    assert!(
        drain(&mut tracker).is_empty(),
        "the same section again changes nothing"
    );
}
#[test]
fn the_tracker_is_debug_printable() {
    let ts_bytes = test_fixtures::read("ts/tnt-5w-12732v-isi6-10s.ts");
    let mut tracker = PresentEventTracker::new(TF1_SERVICE_ID);
    feed_in_chunks(&mut tracker, &ts_bytes);

    let rendered = format!("{tracker:?}");
    assert!(rendered.contains("service_id: 1537"), "{rendered}");
    assert!(rendered.contains("event_id: 30807"), "{rendered}");
    assert!(rendered.contains("changes_dropped: 0"), "{rendered}");
}
