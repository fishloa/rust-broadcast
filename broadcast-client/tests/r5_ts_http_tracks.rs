use broadcast_client::{Action, Config, Event, Session};
use transmux::{DemuxEvent, StreamingTsDemux, TrackId};

const CHUNK: usize = 65_536;
const STEP_US: u64 = 20_000;

struct Run {
    tracks: Vec<Vec<TrackId>>,
    samples: Vec<(TrackId, Option<i64>, usize)>,
}

fn live_run(bytes: &[u8]) -> Run {
    let mut s = Session::open("http://h/live.ts", Config::default(), 0).unwrap();
    let mut now = 0u64;
    let mut conn = None;
    while let Some(a) = s.poll(now) {
        if let Action::HttpRequest { conn: c, .. } = a {
            conn = Some(c);
        }
    }
    let c = conn.expect("request");
    s.on_response(
        c,
        200,
        &[("Content-Type".to_string(), "video/MP2T".to_string())],
        now,
    );
    let mut run = Run {
        tracks: Vec::new(),
        samples: Vec::new(),
    };
    let mut drain = |s: &mut Session, run: &mut Run| {
        while let Some(e) = s.next_event() {
            match e {
                Event::Tracks(v) => run.tracks.push(v.iter().map(|t| t.track_id).collect()),
                Event::Sample { track, sample, .. } => {
                    run.samples.push((track, sample.pts, sample.data.len()));
                }
                _ => {}
            }
        }
    };
    for chunk in bytes.chunks(CHUNK) {
        now += STEP_US;
        s.on_data(c, chunk, now);
        while s.poll(now).is_some() {}
        drain(&mut s, &mut run);
    }
    run
}

fn reference_samples(bytes: &[u8]) -> Vec<(TrackId, Option<i64>, usize)> {
    let mut demux = StreamingTsDemux::new();
    let mut out = Vec::new();
    for chunk in bytes.chunks(CHUNK) {
        demux.feed(chunk);
        while let Some(e) = demux.poll_event() {
            if let DemuxEvent::Sample {
                track_id, sample, ..
            } = e
            {
                out.push((track_id, sample.pts, sample.data.len()));
            }
        }
    }
    out
}

fn final_track_set(bytes: &[u8]) -> Vec<TrackId> {
    let mut demux = StreamingTsDemux::new();
    let mut ids = Vec::new();
    for chunk in bytes.chunks(CHUNK) {
        demux.feed(chunk);
        while let Some(e) = demux.poll_event() {
            if let DemuxEvent::TrackAdded(spec) = e {
                ids.push(spec.track_id);
            }
        }
    }
    ids.sort_unstable();
    ids
}

fn sorted(mut v: Vec<TrackId>) -> Vec<TrackId> {
    v.sort_unstable();
    v
}

fn assert_whole_set_announced_once(rel: &str) {
    let bytes = test_fixtures::read(rel);
    let run = live_run(&bytes);
    let expected = final_track_set(&bytes);
    assert!(expected.len() >= 2, "{rel}: fixture has one track");
    assert_eq!(
        sorted(run.tracks[0].clone()),
        expected,
        "{rel}: the first Tracks announcement must list every declared stream"
    );
    assert_eq!(run.tracks.len(), 1, "{rel}: {:?}", run.tracks);
    assert_eq!(
        run.samples,
        reference_samples(&bytes),
        "{rel}: the hold must neither lose nor reorder samples"
    );
}

#[test]
fn a_multi_stream_live_ts_announces_every_track_in_the_first_tracks_event() {
    assert_whole_set_announced_once("ts/france2.ts");
}

#[test]
fn a_two_track_live_ts_announces_both_tracks_in_the_first_tracks_event() {
    assert_whole_set_announced_once("ts/h264_aac.ts");
}

#[test]
fn a_live_multiplex_announces_a_bounded_initial_set_then_updates() {
    let Some(bytes) = test_fixtures::require_bytes(
        "ts/france-tnt-uhf32-video-head-6MB.ts",
        "r5_ts_http_tracks",
    ) else {
        return;
    };
    let run = live_run(&bytes);
    let expected = final_track_set(&bytes);
    assert!(expected.len() > 10);
    let last = sorted(run.tracks.last().unwrap().clone());
    assert_eq!(last, expected, "every track is eventually announced");
    assert!(
        run.tracks[0].len() > 1,
        "initial set coalesced: {:?}",
        run.tracks[0]
    );
    assert!(
        run.tracks.len() < expected.len() / 2,
        "{} Tracks events for {} tracks",
        run.tracks.len(),
        expected.len()
    );
    assert_eq!(run.samples, reference_samples(&bytes));
}
