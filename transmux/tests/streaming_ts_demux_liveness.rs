//! Live liveness of `StreamingTsDemux` on sparse multi-program captures: a PID
//! stuck in `Probing` must not hold already-resolved PIDs back until
//! `finish()`.

use std::collections::BTreeMap;
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux};

const CHUNK: usize = 64 * 1024;

struct Run {
    first_sample_byte: Option<usize>,
    before_finish_samples: usize,
    before_finish_tracks: usize,
    total_samples: usize,
    total_tracks: usize,
    per_track: BTreeMap<u32, usize>,
}

fn run(data: &[u8]) -> Run {
    let mut d = StreamingTsDemux::new();
    let mut r = Run {
        first_sample_byte: None,
        before_finish_samples: 0,
        before_finish_tracks: 0,
        total_samples: 0,
        total_tracks: 0,
        per_track: BTreeMap::new(),
    };
    let mut fed = 0usize;
    for ch in data.chunks(CHUNK) {
        d.feed(ch);
        fed += ch.len();
        while let Some(e) = d.poll_event() {
            match e {
                DemuxEvent::Sample { track_id, .. } => {
                    r.before_finish_samples += 1;
                    r.total_samples += 1;
                    *r.per_track.entry(track_id.value()).or_default() += 1;
                    r.first_sample_byte.get_or_insert(fed);
                }
                DemuxEvent::TrackAdded(_) => {
                    r.before_finish_tracks += 1;
                    r.total_tracks += 1;
                }
                _ => {}
            }
        }
    }
    d.finish();
    while let Some(e) = d.poll_event() {
        match e {
            DemuxEvent::Sample { track_id, .. } => {
                r.total_samples += 1;
                *r.per_track.entry(track_id.value()).or_default() += 1;
            }
            DemuxEvent::TrackAdded(_) => r.total_tracks += 1,
            _ => {}
        }
    }
    r
}

fn report(name: &str, r: &Run) {
    eprintln!(
        "{name}: before finish tracks={} samples={} first_sample_byte={:?}; total tracks={} samples={}; per_track={:?}",
        r.before_finish_tracks,
        r.before_finish_samples,
        r.first_sample_byte,
        r.total_tracks,
        r.total_samples,
        r.per_track
    );
}

#[test]
fn csa2_plain_delivers_before_finish() {
    let Some(data) = test_fixtures::require_bytes("csa/csa2-plain.ts", "csa2_plain_delivers") else {
        return;
    };
    let r = run(&data);
    report("csa2-plain", &r);
    assert!(r.before_finish_samples > 0, "no sample before finish()");
}

#[test]
fn test_062_delivers_before_finish() {
    let Some(p) = test_fixtures::tsduck("input/test-062.ts", "test_062_delivers") else {
        return;
    };
    let data = std::fs::read(p).unwrap();
    let r = run(&data);
    report("test-062", &r);
    assert!(r.before_finish_samples > 0, "no sample before finish()");
}
