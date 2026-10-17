//! `Media::degraded_at[i].track` is resolved against the FINAL PID->track
//! map (issue #1348).
//!
//! A DSM-CC carousel PID (`stream_type 0x0C`) becomes a `Data` track only
//! when the demuxer announces it at `finish()`, but its continuity damage is
//! observed long before, while the PID has no track yet. The streaming
//! `InputDegraded` event therefore carries `track: None` live (never
//! fabricated); the one-shot batch `TsDemux` folds the events into a `Media`
//! whose `tracks` are final, so it must resolve each record's `track` from
//! `provenance.pid` against those tracks' `source_pid`.
//!
//! The synthetic stream hand-builds a PAT + a PMT (section byte layout as in
//! `pmt_version_diff.rs`, ISO/IEC 13818-1 §2.4.4.3/§2.4.4.8) declaring one
//! DSM-CC PID, then damages the continuity counter on it and on a PID the
//! PMT does not declare. The real-capture test is
//! `ts/orf3-dvbs2-raw-zenith-tv.ts` (private).

use broadcast_common::{Unpackage, crc32_mpeg2};
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::{TS_PACKET_SIZE, TsHeader};
use transmux::{DemuxEvent, InputDegradation, StreamingTsDemux, TsDemux};

const PMT_PID: Pid = Pid::new(0x1000);
const DSMCC_PID: Pid = Pid::new(0x0300);
const STREAM_TYPE_DSMCC: u8 = 0x0C;
/// A PID the PMT does not declare: damage on it has no track.
const UNDECLARED_PID: Pid = Pid::new(0x0555);

fn section(table_id: u8, body: &[u8]) -> Vec<u8> {
    let len = body.len() + 4;
    let mut s = vec![table_id, 0xB0 | ((len >> 8) as u8 & 0x0F), len as u8];
    s.extend_from_slice(body);
    let crc = crc32_mpeg2::compute(&s);
    s.extend_from_slice(&crc.to_be_bytes());
    s
}

fn packet(pid: Pid, pusi: bool, cc: u8, payload: &[u8]) -> [u8; TS_PACKET_SIZE] {
    let mut pkt = [0xFFu8; TS_PACKET_SIZE];
    TsHeader {
        tei: false,
        pusi,
        pid,
        scrambling: 0,
        has_adaptation: false,
        has_payload: true,
        continuity_counter: cc,
    }
    .serialize_into(&mut pkt)
    .unwrap();
    pkt[4..4 + payload.len()].copy_from_slice(payload);
    pkt
}

fn psi(pid: Pid, cc: u8, body: &[u8], table_id: u8) -> [u8; TS_PACKET_SIZE] {
    let mut p = vec![0u8]; // pointer_field
    p.extend_from_slice(&section(table_id, body));
    packet(pid, true, cc, &p)
}

fn stream() -> Vec<u8> {
    let pat_body = [0, 1, 0xC1, 0, 0, 0, 1, 0xE0 | 0x10, 0x00];
    let mut pmt_body = vec![0, 1, 0xC1, 0, 0, 0xFF, 0xFF, 0xF0, 0x00];
    pmt_body.extend_from_slice(&[
        STREAM_TYPE_DSMCC,
        0xE0 | (DSMCC_PID.value() >> 8) as u8,
        DSMCC_PID.value() as u8,
        0xF0,
        0,
    ]);
    let mut v = Vec::new();
    v.extend_from_slice(&psi(Pid::new(0), 0, &pat_body, 0x00));
    v.extend_from_slice(&psi(PMT_PID, 0, &pmt_body, 0x02));
    // Damage first, while the PID has no track yet: a counter jump (0 -> 5)
    // on the DSM-CC PID and one on an undeclared PID. Only then one complete
    // DSM-CC section (a DDB, table_id 0x3C), which resolves the Data track.
    for (pid, cc) in [
        (DSMCC_PID, 0),
        (DSMCC_PID, 5),
        (UNDECLARED_PID, 0),
        (UNDECLARED_PID, 9),
    ] {
        v.extend_from_slice(&packet(pid, false, cc, &[0xAB; 20]));
    }
    v.extend_from_slice(&psi(DSMCC_PID, 6, &[0, 1, 0xC1, 0, 0, 1, 2, 3], 0x3C));
    // trailing nulls let the resynchroniser lock on the last real packet
    for _ in 0..8 {
        v.extend_from_slice(&packet(Pid::new(0x1FFF), false, 0, &[]));
    }
    v
}

#[test]
fn batch_degraded_at_resolves_the_data_track_announced_at_finish() {
    let media = TsDemux::new().unpackage(&stream()).unwrap();
    let track = media
        .tracks
        .iter()
        .find(|t| t.spec.source_pid == Some(DSMCC_PID))
        .expect("the DSM-CC PID is announced as a Data track");
    let on_dsmcc: Vec<_> = media
        .degraded_at
        .iter()
        .filter(|r| r.provenance.pid == Some(DSMCC_PID))
        .collect();
    assert_eq!(on_dsmcc.len(), 1, "one continuity gap on the DSM-CC PID");
    assert_eq!(on_dsmcc[0].track, Some(track.spec.track_id));
}

#[test]
fn a_pid_without_a_track_stays_none() {
    let media = TsDemux::new().unpackage(&stream()).unwrap();
    let on_undeclared: Vec<_> = media
        .degraded_at
        .iter()
        .filter(|r| r.provenance.pid == Some(UNDECLARED_PID))
        .collect();
    assert_eq!(on_undeclared.len(), 1);
    assert_eq!(on_undeclared[0].track, None);
}

#[test]
fn streaming_event_carries_no_track_before_the_track_is_announced() {
    let mut d = StreamingTsDemux::new();
    d.feed(&stream());
    d.finish();
    let mut seen = 0;
    while let Some(e) = d.poll_event() {
        if let DemuxEvent::InputDegraded {
            track,
            kind: InputDegradation::ContinuityGap { .. },
            provenance,
            ..
        } = e
            && provenance.pid == Some(DSMCC_PID)
        {
            seen += 1;
            assert_eq!(track, None, "live: the PID has no track yet");
        }
    }
    assert_eq!(seen, 1);
}

#[test]
fn orf3_every_record_on_a_track_pid_names_that_track() {
    let Some(path) =
        test_fixtures::require("ts/orf3-dvbs2-raw-zenith-tv.ts", "degraded_at_track orf3")
    else {
        return;
    };
    let media = TsDemux::new()
        .unpackage(&std::fs::read(path).unwrap())
        .unwrap();
    let carousel = media
        .tracks
        .iter()
        .find(|t| t.spec.source_pid == Some(Pid::new(0x1C90)))
        .expect("PID 0x1C90 is a track");
    let mut checked = 0;
    for r in &media.degraded_at {
        let Some(pid) = r.provenance.pid else {
            continue;
        };
        if let Some(t) = media.tracks.iter().find(|t| t.spec.source_pid == Some(pid)) {
            assert_eq!(r.track, Some(t.spec.track_id), "pid {pid:?}");
            checked += 1;
        } else {
            assert_eq!(r.track, None, "pid {pid:?} has no track");
        }
    }
    assert!(checked >= 187, "the carousel damage must be covered");
    assert!(
        media
            .degraded_at
            .iter()
            .any(|r| r.track == Some(carousel.spec.track_id))
    );
}
