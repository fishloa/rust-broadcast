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
    let drain = |s: &mut Session, run: &mut Run| {
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
fn a_live_multiplex_announces_a_coalesced_initial_set_then_every_later_track() {
    let Some(bytes) =
        test_fixtures::require_bytes("ts/france-tnt-uhf32-video-head-6MB.ts", "r5_ts_http_tracks")
    else {
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
    assert_eq!(run.samples, reference_samples(&bytes));
}

const TS_PACKET: usize = 188;
const PID_MASK: u16 = 0x1fff;
const WALL_LIMIT_US: u64 = 3_000_000;
const LAST_FRANCE2_PID: u16 = 142;
const HOLD_START_CHUNKS: usize = 8;

fn without_pid(bytes: &[u8], pid: u16) -> Vec<u8> {
    bytes
        .chunks(TS_PACKET)
        .filter(|p| (u16::from(p[1]) << 8 | u16::from(p[2])) & PID_MASK != pid)
        .flatten()
        .copied()
        .collect()
}

fn open_live(now: u64) -> (Session, u32) {
    let mut s = Session::open("http://h/live.ts", Config::default(), now).unwrap();
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
    (s, c)
}

fn drain(s: &mut Session) -> Vec<Event> {
    let mut out = Vec::new();
    while let Some(e) = s.next_event() {
        out.push(e);
    }
    out
}

#[test]
fn an_undeclared_gap_releases_the_hold_at_the_wall_limit_without_losing_samples() {
    let bytes = without_pid(&test_fixtures::read("ts/france2.ts"), LAST_FRANCE2_PID);
    let fed = bytes[..HOLD_START_CHUNKS * CHUNK].to_vec();
    let expected = final_track_set(&fed);
    let (mut s, c) = open_live(0);
    let mut now = 0u64;
    let mut wake = None;
    let mut events = Vec::new();
    for chunk in bytes.chunks(CHUNK).take(HOLD_START_CHUNKS) {
        now += STEP_US;
        s.on_data(c, chunk, now);
        while let Some(a) = s.poll(now) {
            if let Action::WakeAt(t) = a {
                wake = wake.max(Some(t));
            }
        }
        events.extend(drain(&mut s));
    }
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::Tracks(_) | Event::Sample { .. })),
        "media must be held while a declared stream is unresolved"
    );
    assert!(
        wake.is_some_and(|t| t <= HOLD_START_CHUNKS as u64 * STEP_US + WALL_LIMIT_US),
        "a wake-up at the hold deadline: {wake:?}"
    );
    let due = HOLD_START_CHUNKS as u64 * STEP_US + WALL_LIMIT_US;
    while s.poll(due).is_some() {}
    events.extend(drain(&mut s));
    let first = events
        .iter()
        .position(|e| matches!(e, Event::Tracks(_)))
        .expect("Tracks released at the deadline");
    let Event::Tracks(set) = &events[first] else {
        unreachable!()
    };
    assert_eq!(
        sorted(set.iter().map(|t| t.track_id).collect()),
        expected,
        "the resolved subset is announced"
    );
    assert!(
        events[..first]
            .iter()
            .all(|e| !matches!(e, Event::Sample { .. }))
    );
    let held: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, sample, .. } => Some((*track, sample.pts, sample.data.len())),
            _ => None,
        })
        .collect();
    assert_eq!(held, reference_samples(&fed));
    assert!(!held.is_empty());
}

#[test]
fn an_undeclared_gap_releases_the_hold_at_the_media_limit() {
    let bytes = without_pid(&test_fixtures::read("ts/france2.ts"), LAST_FRANCE2_PID);
    let run = live_run(&bytes);
    assert_eq!(
        sorted(run.tracks[0].clone()),
        final_track_set(&bytes),
        "{:?}",
        run.tracks
    );
    assert_eq!(run.samples, reference_samples(&bytes));
}

#[test]
fn a_connection_that_ends_while_holding_delivers_everything_in_order() {
    let bytes = without_pid(&test_fixtures::read("ts/h264_aac.ts"), 0x101);
    let (mut s, c) = open_live(0);
    s.on_data(c, &bytes, STEP_US);
    while s.poll(STEP_US).is_some() {}
    let before = drain(&mut s);
    assert!(!before.iter().any(|e| matches!(e, Event::Sample { .. })));
    s.on_closed(c, None, 2 * STEP_US);
    while s.poll(2 * STEP_US).is_some() {}
    let events = drain(&mut s);
    let tracks = events
        .iter()
        .position(|e| matches!(e, Event::Tracks(_)))
        .expect("Tracks at EOF");
    let ended = events
        .iter()
        .position(|e| {
            matches!(
                e,
                Event::Status(
                    broadcast_client::Status::Ended | broadcast_client::Status::Reconnecting { .. }
                )
            )
        })
        .unwrap_or_else(|| panic!("a terminal status: {events:?}"));
    let first_sample = events
        .iter()
        .position(|e| matches!(e, Event::Sample { .. }))
        .expect("samples");
    let last_sample = events
        .iter()
        .rposition(|e| matches!(e, Event::Sample { .. }))
        .unwrap();
    assert!(tracks < first_sample && last_sample < ended);
}

#[test]
fn a_track_that_resolves_after_the_hold_is_announced_as_an_update() {
    let original = test_fixtures::read("ts/france2.ts");
    let cut = HOLD_START_CHUNKS * CHUNK / TS_PACKET * TS_PACKET;
    let mut combined = without_pid(&original[..cut], LAST_FRANCE2_PID);
    let prefix_len = combined.len();
    combined.extend_from_slice(&original[cut..]);
    let (mut s, c) = open_live(0);
    let mut now = 0u64;
    let mut events = Vec::new();
    for chunk in combined[..prefix_len].chunks(CHUNK) {
        now += STEP_US;
        s.on_data(c, chunk, now);
        while s.poll(now).is_some() {}
        events.extend(drain(&mut s));
    }
    now += WALL_LIMIT_US;
    while s.poll(now).is_some() {}
    events.extend(drain(&mut s));
    let released = events
        .iter()
        .filter(|e| matches!(e, Event::Tracks(_)))
        .count();
    assert_eq!(released, 1, "the hold released at its deadline");
    for chunk in combined[prefix_len..].chunks(CHUNK) {
        now += STEP_US;
        s.on_data(c, chunk, now);
        while s.poll(now).is_some() {}
        events.extend(drain(&mut s));
    }
    let sets: Vec<Vec<TrackId>> = events
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(v) => Some(sorted(v.iter().map(|t| t.track_id).collect())),
            _ => None,
        })
        .collect();
    assert!(sets.len() >= 2, "{sets:?}");
    assert!(sets[0].len() < sets.last().unwrap().len());
    assert_eq!(sets.last().unwrap(), &final_track_set(&combined));
    let delivered: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, sample, .. } => Some((*track, sample.pts, sample.data.len())),
            _ => None,
        })
        .collect();
    assert_eq!(delivered, reference_samples(&combined));
}
