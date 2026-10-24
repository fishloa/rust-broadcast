mod support;

use std::collections::{BTreeSet, HashMap, VecDeque};

use broadcast_client::{Action, Config, Credentials, Event, Session, Status};
use support::hls_host::{
    Host, ORIGIN, Reply, SEC, TS_PLAYLIST_URL, TS_SEG0_URL, TS_SEG1_URL, drain_actions,
    drain_events, hls_config, http_requests, is_ended, live_playlist_text, master_text,
    ts_playlist, ts_segment,
};

const TS_PACKET_LEN: usize = 188;
const TS_SYNC: u8 = 0x47;
const NULL_PID_HI: u8 = 0x1F;
const NULL_PID_LO: u8 = 0xFF;
const PAYLOAD_ONLY: u8 = 0x10;
const VOD_BASE: &str = "http://origin.test/vod";
const VOD_PLAYLIST_URL: &str = "http://origin.test/vod/index.m3u8";
const OVERLOAD_PROBE: usize = 64;
const BASIC_CHALLENGE: &str = "Basic realm=\"r\"";
const DIGEST_CHALLENGE: &str = "Digest realm=\"r\", nonce=\"abc123\", qop=\"auth\"";

fn null_packet() -> Vec<u8> {
    let mut packet = vec![0xFF; TS_PACKET_LEN];
    packet[0] = TS_SYNC;
    packet[1] = NULL_PID_HI;
    packet[2] = NULL_PID_LO;
    packet[3] = PAYLOAD_ONLY;
    packet
}

fn vod_playlist(segments: usize) -> Vec<u8> {
    let mut text = String::from(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n",
    );
    for n in 0..segments {
        text.push_str(&format!("#EXTINF:1,\n{n}.ts\n"));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    text.into_bytes()
}

#[derive(Debug, Default)]
struct Stats {
    peak_segments: usize,
    peak_total: usize,
    steps: usize,
    ended: bool,
    stuck: bool,
    overloaded: bool,
    requested: Vec<String>,
    succeeded: Vec<String>,
}

fn run_vod(
    session: &mut Session,
    playlist_url: &str,
    playlist: &[u8],
    mut segment_status: impl FnMut(&str, usize) -> u16,
    max_steps: usize,
) -> Stats {
    let mut stats = Stats::default();
    let mut now = 0u64;
    let mut outstanding: VecDeque<(u32, String)> = VecDeque::new();
    let mut wakes: BTreeSet<u64> = BTreeSet::new();
    let mut hits: HashMap<String, usize> = HashMap::new();
    let body = null_packet();
    loop {
        while let Some(action) = session.poll(now) {
            match action {
                Action::HttpRequest { conn, url, .. } => {
                    stats.requested.push(url.clone());
                    outstanding.push_back((conn, url));
                }
                Action::Close { conn } => outstanding.retain(|(c, _)| *c != conn),
                Action::WakeAt(t) if t > now => {
                    wakes.insert(t);
                }
                _ => {}
            }
            if outstanding.len() > OVERLOAD_PROBE {
                stats.overloaded = true;
                stats.peak_total = stats.peak_total.max(outstanding.len());
                stats.peak_segments = stats.peak_segments.max(outstanding.len());
                return stats;
            }
        }
        while let Some(event) = session.next_event() {
            stats.ended |= is_ended(&event);
        }
        let segments = outstanding
            .iter()
            .filter(|(_, u)| u != playlist_url)
            .count();
        stats.peak_segments = stats.peak_segments.max(segments);
        stats.peak_total = stats.peak_total.max(outstanding.len());
        if stats.ended || stats.steps >= max_steps {
            return stats;
        }
        stats.steps += 1;
        if let Some((conn, url)) = outstanding.pop_front() {
            if url == playlist_url {
                session.on_response(conn, 200, &[]);
                session.on_data(conn, playlist, now);
                session.on_closed(conn, None, now);
                continue;
            }
            let nth = hits.entry(url.clone()).or_insert(0);
            let code = segment_status(&url, *nth);
            *nth += 1;
            session.on_response(conn, code, &[]);
            if code == 200 {
                stats.succeeded.push(url);
                session.on_data(conn, &body, now);
            }
            session.on_closed(conn, None, now);
        } else if let Some(t) = wakes.range((now + 1)..).next().copied() {
            wakes.remove(&t);
            now = t;
        } else {
            stats.stuck = true;
            return stats;
        }
    }
}

fn vod_session(config: Config) -> Session {
    Session::open(VOD_PLAYLIST_URL, config, 0).expect("open")
}

#[test]
fn a_20k_segment_vod_keeps_at_most_the_cap_in_flight_and_completes() {
    let mut session = vod_session(hls_config());
    let n = 20_000;
    let stats = run_vod(
        &mut session,
        VOD_PLAYLIST_URL,
        &vod_playlist(n),
        |_, _| 200,
        n * 2,
    );
    assert!(!stats.overloaded, "unbounded fan-out: {stats:?}");
    assert!(stats.peak_segments <= 4, "{}", stats.peak_segments);
    assert!(stats.peak_total <= 5, "{}", stats.peak_total);
    assert!(stats.ended && !stats.stuck, "{stats:?}");
    assert_eq!(stats.requested.len(), n + 1);
    assert!(stats.steps <= n + 100, "steps {}", stats.steps);
}

#[test]
fn a_hostile_150k_segment_playlist_completes_in_bounded_steps() {
    let mut session = vod_session(hls_config());
    let n = 150_000;
    let playlist = vod_playlist(n);
    assert!(playlist.len() < broadcast_client::events::DEFAULT_MAX_PLAYLIST_BYTES);
    let stats = run_vod(&mut session, VOD_PLAYLIST_URL, &playlist, |_, _| 200, n * 2);
    assert!(!stats.overloaded, "unbounded fan-out: {stats:?}");
    assert!(stats.peak_total <= 5, "{}", stats.peak_total);
    assert!(stats.ended && !stats.stuck, "{stats:?}");
    assert!(stats.steps <= n + 100, "steps {}", stats.steps);
}

#[test]
fn the_configured_cap_is_honoured_and_zero_means_one() {
    for (configured, effective) in [(2usize, 2usize), (1, 1), (0, 1), (7, 7)] {
        let config = Config {
            hls_max_inflight_segments: configured,
            ..hls_config()
        };
        let mut session = vod_session(config);
        let n = 60;
        let stats = run_vod(
            &mut session,
            VOD_PLAYLIST_URL,
            &vod_playlist(n),
            |_, _| 200,
            n * 2,
        );
        assert!(stats.ended && !stats.stuck, "cap {configured}: {stats:?}");
        assert!(
            stats.peak_segments <= effective,
            "cap {configured}: peak {}",
            stats.peak_segments
        );
        assert_eq!(
            stats.peak_segments, effective,
            "cap {configured}: the window is actually used"
        );
    }
}

#[test]
fn a_failed_segment_holds_later_ones_within_the_cap_and_is_retried_in_order() {
    let mut session = vod_session(hls_config());
    let n = 40;
    let first = format!("{VOD_BASE}/0.ts");
    let stats = run_vod(
        &mut session,
        VOD_PLAYLIST_URL,
        &vod_playlist(n),
        |url, nth| if url == first && nth < 2 { 500 } else { 200 },
        n * 20,
    );
    assert!(stats.ended && !stats.stuck, "deadlocked: {stats:?}");
    let third_attempt = stats
        .requested
        .iter()
        .enumerate()
        .filter(|(_, u)| **u == first)
        .nth(2)
        .map(|(i, _)| i)
        .expect("segment 0 is requested three times");
    let window: BTreeSet<&String> = stats.requested[..third_attempt]
        .iter()
        .filter(|u| **u != VOD_PLAYLIST_URL)
        .collect();
    assert!(
        window.len() <= 4,
        "{} distinct segments requested while segment 0 kept failing",
        window.len()
    );
    assert!(
        stats.requested.iter().filter(|u| **u == first).count() >= 3,
        "segment 0 was retried"
    );
}

#[test]
fn playlist_and_init_requests_bypass_a_full_segment_window() {
    let init_url = format!("{ORIGIN}/c/init.mp4");
    let playlist_url = format!("{ORIGIN}/c/index.m3u8");
    let config = Config {
        hls_max_inflight_segments: 1,
        ..hls_config()
    };
    let mut h = Host::new(&playlist_url, config);
    h.ok(
        &playlist_url,
        support::hls_host::cmaf_playlist_text().into_bytes(),
    );
    h.ok(&init_url, support::hls_host::cmaf_init());
    for n in 0..3 {
        h.route(&format!("{ORIGIN}/c/index{n}.m4s"), vec![Reply::Hang]);
    }
    let mut outstanding_segments = 0;
    for _ in 0..50 {
        if h.now > 0 || !h.step() {
            break;
        }
        h.collect();
        outstanding_segments = outstanding_segments.max(h.hung().len());
    }
    assert!(
        outstanding_segments <= 1,
        "{outstanding_segments} segment requests outstanding with a cap of 1"
    );
    assert_eq!(
        h.hits(&init_url),
        1,
        "init is fetched while the window is full"
    );
    assert_eq!(h.hits(&playlist_url), 1);
}

#[test]
fn live_playlist_reloads_are_not_starved_by_a_full_segment_window() {
    let playlist_url = "http://origin.test/live/index.m3u8";
    let names: Vec<String> = (0..10).map(|n| format!("s{n}.ts")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let config = Config {
        hls_max_inflight_segments: 1,
        ..hls_config()
    };
    let mut h = Host::new(playlist_url, config);
    h.ok(playlist_url, live_playlist_text(0, &refs).into_bytes());
    for name in &names {
        h.route(
            &format!("http://origin.test/live/{name}"),
            vec![Reply::Hang],
        );
    }
    let mut peak = 0;
    for _ in 0..60 {
        if h.now >= 10 * SEC || !h.step() {
            break;
        }
        h.collect();
        peak = peak.max(h.hung().len());
    }
    assert!(peak <= 1, "{peak} segment requests outstanding");
    assert!(
        h.hits(playlist_url) >= 2,
        "the playlist was reloaded {} times",
        h.hits(playlist_url)
    );
}

#[test]
fn zero_backoff_entries_and_cap_are_floored_at_one_millisecond() {
    let config = Config {
        hls_backoff_ms: vec![0],
        hls_backoff_cap_ms: 0,
        ..hls_config()
    };
    let mut h = Host::new(TS_PLAYLIST_URL, config);
    h.route(TS_PLAYLIST_URL, vec![Reply::Status(500)]);
    for _ in 0..300 {
        if !h.step() {
            break;
        }
    }
    h.collect();
    let mut per_instant: HashMap<u64, usize> = HashMap::new();
    for seen in &h.seen {
        *per_instant.entry(seen.at_us).or_default() += 1;
    }
    let worst = per_instant.values().copied().max().unwrap_or(0);
    assert!(worst <= 2, "{worst} requests at one instant");
    assert!(h.now >= 100, "virtual time advanced: {}", h.now);
}

fn ended_master_host() -> Host {
    let master_url = "http://origin.test/hls/master.m3u8";
    let mut h = Host::new(master_url, hls_config());
    h.ok(master_url, master_text("index.m3u8").into_bytes());
    h.ok(TS_PLAYLIST_URL, ts_playlist());
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.ok(TS_SEG1_URL, ts_segment(1));
    h.run_to_end();
    h
}

#[test]
fn hls_vod_mutators_after_ended_are_no_ops() {
    let mut h = ended_master_host();
    assert_eq!(h.events.iter().filter(|e| is_ended(e)).count(), 1);
    drain_events(&mut h.session);
    let now = h.now + SEC;

    h.session
        .select_variant(0)
        .expect("known variant is accepted");
    h.session.reconnect(now);
    h.session.select_audio_rendition(None).ok();
    let actions = drain_actions(&mut h.session, now);
    assert!(actions.is_empty(), "{actions:?}");
    let events = drain_events(&mut h.session);
    assert!(events.is_empty(), "{events:?}");
    assert!(matches!(
        h.session.select_variant(9),
        Err(broadcast_client::Error::NoSuchVariant(9))
    ));
    assert!(drain_actions(&mut h.session, now + SEC).is_empty());
}

#[test]
fn live_hls_reconnect_and_select_variant_still_restart_the_playlist() {
    let master_url = "http://origin.test/hls/master.m3u8";
    let live = "http://origin.test/hls/index.m3u8";
    let mut h = Host::new(master_url, hls_config());
    h.ok(master_url, master_text("index.m3u8").into_bytes());
    h.ok(live, live_playlist_text(0, &["a.ts"]).into_bytes());
    h.route("http://origin.test/hls/a.ts", vec![Reply::Hang]);
    for _ in 0..6 {
        if !h.step() {
            break;
        }
    }
    h.collect();
    drain_events(&mut h.session);
    let hits = h.hits(live);

    h.session.reconnect(h.now);
    let reconnect = http_requests(&drain_actions(&mut h.session, h.now));
    assert!(
        reconnect.iter().any(|(_, u)| u.starts_with(live)),
        "reconnect refetches the playlist: {reconnect:?} (hits before {hits})"
    );

    h.session.select_variant(0).expect("variant");
    let switched = http_requests(&drain_actions(&mut h.session, h.now));
    assert!(
        switched.iter().any(|(_, u)| u.starts_with(live)),
        "select_variant refetches the playlist: {switched:?}"
    );
}

#[test]
fn finite_ts_reconnect_after_ended_still_restarts() {
    let bytes = support::read_tf1_bytes();
    let (status, base_headers) = support::read_tf1_headers();
    let mut headers = base_headers;
    headers.push(("Content-Length".to_string(), bytes.len().to_string()));
    let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
    let conn = match session.poll(0).unwrap() {
        Action::HttpRequest { conn, .. } => conn,
        other => panic!("{other:?}"),
    };
    session.on_response(conn, status, &headers);
    for chunk in bytes.chunks(1316) {
        session.on_data(conn, chunk, 0);
    }
    session.on_closed(conn, None, 1000);
    let ended = drain_events(&mut session)
        .iter()
        .filter(|e| matches!(e, Event::Status(Status::Ended)))
        .count();
    assert_eq!(ended, 1);
    session.reconnect(2000);
    let reopened = http_requests(&drain_actions(&mut session, 2000));
    assert_eq!(reopened.len(), 1, "{reopened:?}");
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[test]
fn a_407_is_answered_with_proxy_authorization_not_authorization() {
    let url = "http://user:pw@origin.test/hls/index.m3u8";
    let mut h = Host::new(url, hls_config());
    h.route(
        TS_PLAYLIST_URL,
        vec![
            Reply::StatusHeaders(
                407,
                vec![(
                    "Proxy-Authenticate".to_string(),
                    BASIC_CHALLENGE.to_string(),
                )],
            ),
            Reply::Ok(ts_playlist()),
        ],
    );
    for _ in 0..4 {
        if !h.step() {
            break;
        }
    }
    h.collect();
    let sent = h.requests_to(TS_PLAYLIST_URL);
    assert!(sent.len() >= 2, "{} requests", sent.len());
    let first = &sent[0].headers;
    assert!(header(first, "proxy-authorization").is_none());
    let replay = &sent[1].headers;
    let proxy = header(replay, "proxy-authorization").expect("Proxy-Authorization on the replay");
    assert!(proxy.starts_with("Basic "), "{proxy}");
    assert!(
        header(replay, "authorization").is_none(),
        "origin Authorization must not answer a proxy challenge"
    );
}

#[test]
fn a_401_is_still_answered_with_authorization() {
    let url = "http://user:pw@origin.test/hls/index.m3u8";
    let mut h = Host::new(url, hls_config());
    h.route(
        TS_PLAYLIST_URL,
        vec![
            Reply::StatusHeaders(
                401,
                vec![("WWW-Authenticate".to_string(), BASIC_CHALLENGE.to_string())],
            ),
            Reply::Ok(ts_playlist()),
        ],
    );
    for _ in 0..4 {
        if !h.step() {
            break;
        }
    }
    h.collect();
    let sent = h.requests_to(TS_PLAYLIST_URL);
    let replay = &sent[1].headers;
    assert!(header(replay, "authorization").is_some_and(|v| v.starts_with("Basic ")));
    assert!(header(replay, "proxy-authorization").is_none());
}

#[test]
fn the_strongest_challenge_among_all_www_authenticate_lines_is_chosen() {
    let url = "http://user:pw@origin.test/hls/index.m3u8";
    let mut h = Host::new(url, hls_config());
    h.route(
        TS_PLAYLIST_URL,
        vec![
            Reply::StatusHeaders(
                401,
                vec![
                    ("WWW-Authenticate".to_string(), BASIC_CHALLENGE.to_string()),
                    ("WWW-Authenticate".to_string(), DIGEST_CHALLENGE.to_string()),
                ],
            ),
            Reply::Ok(ts_playlist()),
        ],
    );
    for _ in 0..4 {
        if !h.step() {
            break;
        }
    }
    h.collect();
    let sent = h.requests_to(TS_PLAYLIST_URL);
    let auth = header(&sent[1].headers, "authorization").expect("Authorization on the replay");
    assert!(auth.starts_with("Digest "), "{auth}");
}

#[test]
fn bearer_credentials_answer_with_bearer_whatever_the_challenge_lines() {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.session
        .set_credentials(Credentials::bearer("tok"))
        .expect("credentials");
    h.route(
        TS_PLAYLIST_URL,
        vec![
            Reply::StatusHeaders(
                401,
                vec![
                    ("WWW-Authenticate".to_string(), BASIC_CHALLENGE.to_string()),
                    ("WWW-Authenticate".to_string(), DIGEST_CHALLENGE.to_string()),
                ],
            ),
            Reply::Ok(ts_playlist()),
        ],
    );
    for _ in 0..4 {
        if !h.step() {
            break;
        }
    }
    h.collect();
    for seen in h.requests_to(TS_PLAYLIST_URL) {
        assert_eq!(seen.authorization(), Some("Bearer tok"));
    }
}
