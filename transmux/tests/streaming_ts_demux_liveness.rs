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
    let Some(data) = test_fixtures::require_bytes("csa/csa2-plain.ts", "csa2_plain_delivers")
    else {
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

type PerPid = BTreeMap<u16, Vec<(Option<i64>, Vec<u8>)>>;

fn per_source_pid(data: &[u8], strict: bool, chunk: usize) -> PerPid {
    let mut d = StreamingTsDemux::new();
    if strict {
        d = d.with_strict_pmt_order();
    }
    let mut pid_of: BTreeMap<u32, u16> = BTreeMap::new();
    let mut out = PerPid::new();
    let mut drain = |d: &mut StreamingTsDemux| {
        while let Some(e) = d.poll_event() {
            match e {
                DemuxEvent::TrackAdded(spec) => {
                    pid_of.insert(spec.track_id.value(), spec.source_pid.unwrap().value());
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => out
                    .entry(pid_of[&track_id.value()])
                    .or_default()
                    .push((sample.dts, sample.data.to_vec())),
                _ => {}
            }
        }
    };
    for ch in data.chunks(chunk) {
        d.feed(ch);
        drain(&mut d);
    }
    d.finish();
    drain(&mut d);
    out
}

#[test]
fn live_policy_loses_nothing_and_keeps_per_track_order_versus_strict() {
    for (rel, tsduck) in [("csa/csa2-plain.ts", false), ("input/test-062.ts", true)] {
        let who = "live_policy_loses_nothing";
        let path = if tsduck {
            test_fixtures::tsduck(rel, who)
        } else {
            test_fixtures::require(rel, who)
        };
        let Some(path) = path else {
            return;
        };
        let data = std::fs::read(path).unwrap();
        let strict = per_source_pid(&data, true, data.len());
        let live_big = per_source_pid(&data, false, CHUNK);
        let live_one = per_source_pid(&data, false, 188);
        assert!(!strict.is_empty(), "{rel}: strict run produced nothing");
        assert_eq!(strict, live_big, "{rel}: live policy changed a track");
        assert_eq!(live_one, live_big, "{rel}: live policy is chunk-dependent");
    }
}

#[test]
fn strict_pmt_order_keeps_the_head_of_line_wait() {
    let Some(data) = test_fixtures::require_bytes("csa/csa2-plain.ts", "strict_pmt_order") else {
        return;
    };
    let mut d = StreamingTsDemux::new().with_strict_pmt_order();
    let mut before_finish = 0usize;
    for ch in data.chunks(CHUNK) {
        d.feed(ch);
        while let Some(e) = d.poll_event() {
            before_finish += usize::from(matches!(e, DemuxEvent::Sample { .. }));
        }
    }
    assert_eq!(before_finish, 0, "strict order delivers only at finish()");
}
