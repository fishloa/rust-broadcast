use broadcast_client::{Action, Config, Error, Event, Session, TrackId};

const CHUNK: usize = 65_536;
const STEP_US: u64 = 20_000;
const TS_PACKET: usize = 188;
const PID_MASK: u16 = 0x1fff;
const LAST_FRANCE2_PID: u16 = 142;
const HOLD_START_CHUNKS: usize = 8;
const SHORT_WALL_US: u64 = 500_000;

fn without_pid(bytes: &[u8], pid: u16) -> Vec<u8> {
    bytes
        .chunks(TS_PACKET)
        .filter(|p| (u16::from(p[1]) << 8 | u16::from(p[2])) & PID_MASK != pid)
        .flatten()
        .copied()
        .collect()
}

fn open_live(config: Config) -> (Session, u32) {
    let mut s = Session::open("http://h/live.ts", config, 0).unwrap();
    let mut conn = None;
    while let Some(a) = s.poll(0) {
        if let Action::HttpRequest { conn: c, .. } = a {
            conn = Some(c);
        }
    }
    let c = conn.expect("request");
    s.on_response(
        c,
        200,
        &[("Content-Type".to_string(), "video/MP2T".to_string())],
        0,
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

fn tracks_sets(events: &[Event]) -> Vec<Vec<TrackId>> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(v) => Some(v.iter().map(|t| t.track_id).collect()),
            _ => None,
        })
        .collect()
}

fn feed_chunks(s: &mut Session, c: u32, bytes: &[u8], chunks: usize) -> (u64, Vec<Event>) {
    let (now, events, _) = feed_chunks_waking(s, c, bytes, chunks);
    (now, events)
}

fn feed_chunks_waking(
    s: &mut Session,
    c: u32,
    bytes: &[u8],
    chunks: usize,
) -> (u64, Vec<Event>, Option<u64>) {
    let mut now = 0;
    let mut wake = None;
    let mut events = Vec::new();
    for chunk in bytes.chunks(CHUNK).take(chunks) {
        now += STEP_US;
        s.on_data(c, chunk, now);
        while let Some(a) = s.poll(now) {
            if let Action::WakeAt(t) = a {
                wake = wake.max(Some(t));
            }
        }
        events.extend(drain(s));
    }
    (now, events, wake)
}

#[test]
fn a_zero_wall_limit_announces_tracks_as_each_stream_resolves() {
    let bytes = test_fixtures::read("ts/france2.ts");
    let config = Config {
        tracks_hold_max_wall_us: 0,
        ..Config::default()
    };
    let (mut s, c) = open_live(config);
    let (_, events) = feed_chunks(&mut s, c, &bytes, usize::MAX);
    let sets = tracks_sets(&events);
    assert!(sets.len() >= 2, "{sets:?}");
    assert_eq!(sets[0].len(), 1, "video-only first: {sets:?}");
    assert!(matches!(
        events
            .iter()
            .find(|e| matches!(e, Event::Tracks(_) | Event::Sample { .. })),
        Some(Event::Tracks(_))
    ));
}

#[test]
fn a_zero_media_limit_announces_tracks_as_each_stream_resolves() {
    let bytes = test_fixtures::read("ts/france2.ts");
    let config = Config {
        tracks_hold_max_media_us: 0,
        ..Config::default()
    };
    let (mut s, c) = open_live(config);
    let (_, events) = feed_chunks(&mut s, c, &bytes, usize::MAX);
    let sets = tracks_sets(&events);
    assert_eq!(sets[0].len(), 1, "{sets:?}");
}

#[test]
fn the_default_config_holds_until_every_stream_resolves() {
    let bytes = test_fixtures::read("ts/france2.ts");
    let (mut s, c) = open_live(Config::default());
    let (_, events) = feed_chunks(&mut s, c, &bytes, usize::MAX);
    let sets = tracks_sets(&events);
    assert_eq!(sets.len(), 1, "{sets:?}");
    assert!(sets[0].len() > 1);
}

#[test]
fn a_short_wall_limit_releases_a_stream_whose_declared_pid_never_sends() {
    let bytes = without_pid(&test_fixtures::read("ts/france2.ts"), LAST_FRANCE2_PID);
    let fed_until = HOLD_START_CHUNKS as u64 * STEP_US;
    let due = fed_until + SHORT_WALL_US;

    let config = Config {
        tracks_hold_max_wall_us: SHORT_WALL_US,
        ..Config::default()
    };
    let (mut s, c) = open_live(config);
    let (now, mut events, wake) = feed_chunks_waking(&mut s, c, &bytes, HOLD_START_CHUNKS);
    assert_eq!(now, fed_until);
    assert!(tracks_sets(&events).is_empty(), "held while unresolved");
    assert!(
        wake.is_some_and(|t| t <= due),
        "wake-up at the deadline: {wake:?}"
    );
    while s.poll(due).is_some() {}
    events.extend(drain(&mut s));
    assert_eq!(tracks_sets(&events).len(), 1, "released by {due} us");

    let (mut d, dc) = open_live(Config::default());
    let (_, mut default_events) = feed_chunks(&mut d, dc, &bytes, HOLD_START_CHUNKS);
    while d.poll(due).is_some() {}
    default_events.extend(drain(&mut d));
    assert!(
        tracks_sets(&default_events).is_empty(),
        "the 1 s default is still holding at {due} us"
    );
}

fn rejected(config: Config) -> &'static str {
    match Session::open("http://h/live.ts", config, 0) {
        Err(Error::InvalidConfig { field }) => field,
        other => panic!("expected InvalidConfig, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn a_zero_quiet_limit_is_rejected() {
    let field = rejected(Config {
        tracks_hold_quiet_input_bytes: 0,
        ..Config::default()
    });
    assert_eq!(field, "tracks_hold_quiet_input_bytes");
}

#[test]
fn a_max_input_below_the_quiet_limit_is_rejected() {
    let field = rejected(Config {
        tracks_hold_max_input_bytes: 1024,
        tracks_hold_quiet_input_bytes: 2048,
        ..Config::default()
    });
    assert_eq!(field, "tracks_hold_max_input_bytes");
}

#[test]
fn the_documented_hold_defaults_are_unchanged_and_valid() {
    let c = Config::default();
    assert_eq!(c.tracks_hold_max_input_bytes, 4 * 1024 * 1024);
    assert_eq!(c.tracks_hold_quiet_input_bytes, 512 * 1024);
    assert_eq!(c.tracks_hold_max_media_us, 1_000_000);
    assert_eq!(c.tracks_hold_max_wall_us, 1_000_000);
    assert!(Session::open("http://h/live.ts", c, 0).is_ok());
}

#[test]
fn zero_wall_and_media_limits_are_valid() {
    let c = Config {
        tracks_hold_max_wall_us: 0,
        tracks_hold_max_media_us: 0,
        ..Config::default()
    };
    assert!(Session::open("http://h/live.ts", c, 0).is_ok());
}
