//! Regression: `StreamingTsDemux` must promote every PMT-declared track of a
//! real French TNT capture (video + three E-AC-3 audio PIDs signalled as
//! PMT stream_type 0x06 + `enhanced_AC-3_descriptor` 0x7A + a DVB subtitle /
//! data PID), not just the first one (zenith report; transmux 0.26.0 did).
//!
//! Fixture: `private/fixtures/ts/france-tnt-m6-eac3-zenith.ts` (see its
//! PROVENANCE.md); skips loudly when the private repo is not checked out.

use std::collections::BTreeMap;
use transmux::pipeline::CodecConfig;
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux};

const FIXTURE: &str = "ts/france-tnt-m6-eac3-zenith.ts";
const WHO: &str = "transmux/tests/streaming_ts_demux_eac3_promotion.rs";

fn run(data: &[u8]) -> (Vec<(u16, String)>, BTreeMap<u16, usize>, usize, usize) {
    let live_tracks;
    let mut d = StreamingTsDemux::new();
    let mut ev = Vec::new();
    for c in data.chunks(188 * 7) {
        d.feed(c);
        while let Some(e) = d.poll_event() {
            ev.push(e);
        }
    }
    live_tracks = ev
        .iter()
        .filter(|e| matches!(e, DemuxEvent::TrackAdded(_)))
        .count();
    d.finish();
    while let Some(e) = d.poll_event() {
        ev.push(e);
    }
    let mut pid_of = BTreeMap::new();
    let mut tracks = Vec::new();
    let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
    let mut abandoned = 0;
    for e in ev {
        match e {
            DemuxEvent::TrackAdded(s) => {
                let pid = s.source_pid.map(|p| p.value()).unwrap_or(0);
                pid_of.insert(s.track_id, pid);
                let tag = match &s.config {
                    CodecConfig::Eac3 { .. } => "eac3".to_string(),
                    CodecConfig::Data { .. } => "data".to_string(),
                    other => format!("{other:?}").chars().take(12).collect(),
                };
                tracks.push((pid, tag));
            }
            DemuxEvent::Sample { track_id, .. } => {
                *counts.entry(pid_of[&track_id]).or_default() += 1;
            }
            DemuxEvent::TrackAbandoned { .. } => abandoned += 1,
            _ => {}
        }
    }
    (tracks, counts, abandoned, live_tracks)
}

#[test]
fn real_tnt_eac3_capture_promotes_video_three_eac3_and_data() {
    let Some(path) = test_fixtures::require(FIXTURE, WHO) else {
        return;
    };
    let data = std::fs::read(path).unwrap();
    let (tracks, counts, abandoned, live) = run(&data);
    eprintln!(
        "tracks={tracks:?} counts={counts:?} abandoned={abandoned} live_before_finish={live}"
    );
    assert!(
        live >= 5,
        "only {live} TrackAdded before finish(): live promotion is stuck"
    );
    let pids: Vec<u16> = tracks.iter().map(|t| t.0).collect();
    for want in [0x78u16, 0x82, 0x83, 0x84, 0x8C] {
        assert!(
            pids.contains(&want),
            "PID {want:#x} never promoted: {tracks:?}"
        );
    }
    for pid in [0x82u16, 0x83, 0x84] {
        assert!(
            tracks.contains(&(pid, "eac3".to_string())),
            "PID {pid:#x} not E-AC-3: {tracks:?}"
        );
    }
    assert!(tracks.contains(&(0x8C, "data".to_string())), "{tracks:?}");
    // 0.26.0 (and every build since) abandons two unrelated PIDs at finish();
    // pinned so a change in that count is noticed, not because it is desired.
    assert_eq!(abandoned, 2, "abandoned-PID count changed");
    for (pid, min) in [(0x78u16, 46usize), (0x82, 7), (0x83, 6), (0x84, 3)] {
        assert!(
            counts.get(&pid).copied().unwrap_or(0) >= min,
            "PID {pid:#x}: {counts:?}"
        );
    }
}
