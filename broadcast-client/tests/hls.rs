mod support;

use std::collections::HashMap;

use broadcast_client::{Action, Config, Credentials, Error, Event, Session, SourceKind, Status};
use support::hls_host::*;
use url::Url;

const MASTER_URL: &str = "http://origin.test/m/master.m3u8";
const LOW_PLAYLIST_URL: &str = "http://origin.test/m/low/index.m3u8";
const HIGH_PLAYLIST_URL: &str = "http://origin.test/m/high/index.m3u8";
const BACKOFF_SECS: [u64; 6] = [2, 4, 8, 10, 10, 10];
const BEARER_TOKEN: &str = "tok-123";
const CDN_SEG_URL: &str = "http://cdn.test/seg/index1.ts";

fn index_of<F: Fn(&Event) -> bool>(events: &[Event], f: F) -> Option<usize> {
    events.iter().position(f)
}

fn count<F: Fn(&Event) -> bool>(events: &[Event], f: F) -> usize {
    events.iter().filter(|e| f(e)).count()
}

fn is_sample(e: &Event) -> bool {
    matches!(e, Event::Sample { .. })
}

fn is_tracks(e: &Event) -> bool {
    matches!(e, Event::Tracks(_))
}

fn is_reconnecting(e: &Event) -> bool {
    matches!(e, Event::Status(Status::Reconnecting { .. }))
}

fn is_not_found(e: &Event) -> bool {
    matches!(e, Event::Status(Status::NotFound))
}

fn reconnect_attempts(events: &[Event]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Status(Status::Reconnecting { attempt }) => Some(*attempt),
            _ => None,
        })
        .collect()
}

fn respond_ok(session: &mut Session, conn: u32, body: &[u8], now: u64) {
    session.on_response(conn, 200, &[], 0);
    session.on_data(conn, body, now);
    session.on_closed(conn, None, now);
}

fn single_http_request(actions: &[Action]) -> (u32, String) {
    let reqs = http_requests(actions);
    assert_eq!(reqs.len(), 1, "expected exactly one request in {actions:?}");
    reqs[0].clone()
}

fn wake_times(actions: &[Action]) -> Vec<u64> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::WakeAt(t) => Some(*t),
            _ => None,
        })
        .collect()
}

fn closes(actions: &[Action]) -> Vec<u32> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Close { conn } => Some(*conn),
            _ => None,
        })
        .collect()
}

fn two_variant_host(config: Config) -> Host {
    let mut h = Host::new(MASTER_URL, config);
    h.ok(MASTER_URL, two_variant_master_text().into_bytes());
    for dir in ["low", "high"] {
        h.ok(
            &format!("http://origin.test/m/{dir}/index.m3u8"),
            ts_playlist(),
        );
        h.ok(
            &format!("http://origin.test/m/{dir}/index0.ts"),
            ts_segment(0),
        );
        h.ok(
            &format!("http://origin.test/m/{dir}/index1.ts"),
            ts_segment(1),
        );
    }
    h
}

fn padded_playlist(total_len: usize) -> Vec<u8> {
    let mut text = String::from_utf8(ts_playlist()).expect("utf8 playlist");
    assert!(total_len > text.len() + 2);
    let pad = total_len - text.len() - 2;
    text.push('#');
    text.push_str(&"x".repeat(pad));
    text.push('\n');
    assert_eq!(text.len(), total_len);
    text.into_bytes()
}

#[test]
fn auto_detects_m3u8_suffix() {
    for url in [
        "http://origin.test/hls/index.m3u8",
        "http://origin.test/hls/index.M3U8?token=abc",
        "http://origin.test/hls/index.m3u",
    ] {
        let mut s = Session::open(url, Config::default(), 0).expect("open");
        let first = drain_actions(&mut s, 0);
        let (conn, req_url) = single_http_request(&first);
        assert_eq!(req_url, url);
        respond_ok(&mut s, conn, &ts_playlist(), 0);
        let next = http_requests(&drain_actions(&mut s, 0));
        assert_eq!(
            next.iter().map(|(_, u)| u.as_str()).collect::<Vec<_>>(),
            vec![
                url.replace("index.M3U8?token=abc", "index0.ts")
                    .replace("index.m3u8", "index0.ts")
                    .replace("index.m3u", "index0.ts"),
                url.replace("index.M3U8?token=abc", "index1.ts")
                    .replace("index.m3u8", "index1.ts")
                    .replace("index.m3u", "index1.ts"),
            ],
            "HLS session must fetch the playlist's segments for {url}"
        );
    }

    let mut ts =
        Session::open("http://origin.test/hls/live.ts", Config::default(), 0).expect("open");
    let (conn, _) = single_http_request(&drain_actions(&mut ts, 0));
    respond_ok(&mut ts, conn, &ts_playlist(), 0);
    assert!(
        http_requests(&drain_actions(&mut ts, 0)).is_empty(),
        "a .ts URL must stay TS-over-HTTP"
    );

    let forced = {
        let mut new_config = Config::default();
        new_config.kind = SourceKind::TsHttp;
        new_config
    };
    let mut ts2 = Session::open("http://origin.test/hls/index.m3u8", forced, 0).expect("open");
    let (conn, _) = single_http_request(&drain_actions(&mut ts2, 0));
    respond_ok(&mut ts2, conn, &ts_playlist(), 0);
    assert!(
        http_requests(&drain_actions(&mut ts2, 0)).is_empty(),
        "explicit TsHttp kind overrides the suffix"
    );
}

#[test]
fn media_playlist_ts_segments_emit_tracks_then_samples() {
    let mut h = host_with(&ts_bodies(), TS_PLAYLIST_URL, Config::default());
    h.run_to_end();
    let tracks = index_of(&h.events, is_tracks).expect("tracks event");
    let first_sample = index_of(&h.events, is_sample).expect("sample event");
    assert!(
        tracks < first_sample,
        "Tracks must precede the first Sample"
    );
    match &h.events[tracks] {
        Event::Tracks(t) => assert_eq!(t.len(), 2, "h264+aac fixture has two tracks"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        h.seen.iter().map(|s| s.url.as_str()).collect::<Vec<_>>(),
        vec![TS_PLAYLIST_URL, TS_SEG0_URL, TS_SEG1_URL]
    );
    let starts = h
        .events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Buffer(broadcast_client::BufferEvent::Start { .. })
            )
        })
        .count();
    assert_eq!(starts, 1, "the VOD playout gate starts once");
    assert_eq!(
        h.events
            .iter()
            .filter(|e| matches!(e, Event::Buffer(_)))
            .count(),
        1,
        "and emits nothing else without a playback position"
    );
    let video = index_of(&h.events, is_tracks)
        .and_then(|i| match &h.events[i] {
            Event::Tracks(t) => t
                .iter()
                .find(|s| transmux::LiveJoin::for_config(&s.config).is_some())
                .map(|s| s.track_id),
            _ => None,
        })
        .expect("a video track");
    let mut video_joins = h.events.iter().filter_map(|e| match e {
        Event::Sample { track, join, .. } if *track == video => Some(*join),
        _ => None,
    });
    assert!(matches!(
        video_joins.next(),
        Some(Some(transmux::JoinDecision::JoinPoint(_)))
    ));
    assert!(video_joins.all(|j| j.is_some()));
    assert!(h.events.iter().all(|e| match e {
        Event::Sample { track, join, .. } if *track != video => join.is_none(),
        _ => true,
    }));
    let expected = oracle_events(TS_PLAYLIST_URL, &ts_bodies());
    assert!(count(&expected, is_sample) > 100);
    assert_eq!(
        count(&h.events, is_sample),
        count(&expected, is_sample),
        "every demuxed sample must be delivered"
    );
}

#[test]
fn media_playlist_fmp4_segments_emit_tracks_then_samples() {
    let bodies = cmaf_bodies();
    let mut h = host_with(&bodies, CMAF_PLAYLIST_URL, Config::default());
    h.run_to_end();
    let tracks = index_of(&h.events, is_tracks).expect("tracks event");
    let first_sample = index_of(&h.events, is_sample).expect("sample event");
    assert!(tracks < first_sample);
    match &h.events[tracks] {
        Event::Tracks(t) => assert_eq!(t.len(), 2, "avc1 + mp4a"),
        other => panic!("{other:?}"),
    }
    let requested: Vec<&str> = h.seen.iter().map(|s| s.url.as_str()).collect();
    assert!(requested.contains(&"http://origin.test/c/init.mp4"));
    for i in 0..3 {
        assert!(requested.contains(&format!("{CMAF_BASE}/index{i}.m4s").as_str()));
    }
    let expected = oracle_events(CMAF_PLAYLIST_URL, &bodies);
    assert!(count(&expected, is_sample) > 100);
    assert_eq!(count(&h.events, is_sample), count(&expected, is_sample));
    let starts = h
        .events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Buffer(broadcast_client::BufferEvent::Start { .. })
            )
        })
        .count();
    assert_eq!(starts, 1, "the VOD playout gate starts once");
    assert_eq!(
        h.events
            .iter()
            .filter(|e| matches!(e, Event::Buffer(_)))
            .count(),
        1,
        "and emits nothing else without a playback position"
    );
}

#[test]
fn master_emits_variants_event_then_fetches_chosen_variant() {
    let mut s = Session::open(MASTER_URL, hls_config(), 0).expect("open");
    let (conn, url) = single_http_request(&drain_actions(&mut s, 0));
    assert_eq!(url, MASTER_URL);
    respond_ok(&mut s, conn, two_variant_master_text().as_bytes(), 0);
    match s.next_event() {
        Some(Event::Status(Status::Establishing)) => {}
        other => panic!("expected Establishing first, got {other:?}"),
    }
    let sel = match s.next_event() {
        Some(Event::Variants(sel)) => sel,
        other => panic!("expected Variants, got {other:?}"),
    };
    assert_eq!(sel.variants.len(), 2);
    assert_eq!(sel.chosen, 1, "highest bandwidth under the default policy");
    assert_eq!(sel.variants[0].url, LOW_PLAYLIST_URL);
    assert_eq!(sel.variants[1].url, HIGH_PLAYLIST_URL);
    let (_, next_url) = single_http_request(&drain_actions(&mut s, 0));
    assert_eq!(
        next_url, HIGH_PLAYLIST_URL,
        "the chosen variant is fetched next"
    );

    let real = String::from_utf8(test_fixtures::read("hls/real/bipbop-ts/master.m3u8"))
        .expect("utf8 bipbop master");
    let n_variants = real.matches("#EXT-X-STREAM-INF").count();
    let mut s2 =
        Session::open("http://origin.test/bip/master.m3u8", hls_config(), 0).expect("open");
    let (c2, _) = single_http_request(&drain_actions(&mut s2, 0));
    respond_ok(&mut s2, c2, real.as_bytes(), 0);
    let evs = drain_events(&mut s2);
    let sel2 = evs
        .iter()
        .find_map(|e| match e {
            Event::Variants(sel) => Some(sel.clone()),
            _ => None,
        })
        .expect("bipbop Variants");
    assert_eq!(sel2.variants.len(), n_variants);
    assert_eq!(
        sel2.variants[sel2.chosen].resolution,
        Some((1920, 1080)),
        "highest bandwidth bipbop variant"
    );
    let (_, chosen_req) = single_http_request(&drain_actions(&mut s2, 0));
    assert_eq!(chosen_req, sel2.variants[sel2.chosen].url);
}

#[test]
fn variant_policy_in_config_applied() {
    let by_bandwidth = {
        let mut new_config = Config::default();
        new_config.kind = SourceKind::Hls;
        new_config.variant_policy = hls_runtime::client::VariantPolicy {
            max_bandwidth: Some(300_000),
            ..Default::default()
        };
        new_config
    };
    let mut s = Session::open(MASTER_URL, by_bandwidth, 0).expect("open");
    let (conn, _) = single_http_request(&drain_actions(&mut s, 0));
    respond_ok(&mut s, conn, two_variant_master_text().as_bytes(), 0);
    let sel = drain_events(&mut s)
        .into_iter()
        .find_map(|e| match e {
            Event::Variants(sel) => Some(sel),
            _ => None,
        })
        .expect("Variants");
    assert_eq!(sel.chosen, 0);
    let (_, url) = single_http_request(&drain_actions(&mut s, 0));
    assert_eq!(url, LOW_PLAYLIST_URL);

    let by_height = {
        let mut new_config = Config::default();
        new_config.kind = SourceKind::Hls;
        new_config.variant_policy = hls_runtime::client::VariantPolicy {
            max_height: Some(180),
            ..Default::default()
        };
        new_config
    };
    let mut s2 = Session::open(MASTER_URL, by_height, 0).expect("open");
    let (conn, _) = single_http_request(&drain_actions(&mut s2, 0));
    respond_ok(&mut s2, conn, two_variant_master_text().as_bytes(), 0);
    let (_, url2) = single_http_request(&drain_actions(&mut s2, 0));
    assert_eq!(url2, LOW_PLAYLIST_URL);
}

#[test]
fn select_variant_switches_and_emits_discontinuity() {
    let mut h = two_variant_host(hls_config());
    h.run_until(|evs| evs.iter().any(is_sample), 100);
    let before = h.events.len();
    let in_flight: Vec<u32> = h.seen.iter().map(|s| s.conn).collect();
    assert!(h.hits("http://origin.test/m/high/index0.ts") == 1);
    h.session.select_variant(0).expect("select low");
    h.collect();
    assert!(
        h.events[before..]
            .iter()
            .any(|e| matches!(e, Event::Discontinuity)),
        "switching variants restarts the timeline"
    );
    assert!(
        h.closes.iter().any(|c| in_flight.contains(c)),
        "in-flight requests of the old variant are closed"
    );
    h.run_to_end();
    assert!(h.hits(LOW_PLAYLIST_URL) >= 1);
    assert!(h.hits("http://origin.test/m/low/index0.ts") >= 1);
    assert_eq!(count(&h.events, is_ended), 1);
    assert!(matches!(
        h.session.select_variant(9),
        Err(Error::NoSuchVariant(9))
    ));

    let mut ts = Session::open("http://origin.test/x.ts", Config::default(), 0).expect("open");
    assert!(matches!(ts.select_variant(0), Err(Error::NotHls)));
}

#[test]
fn vod_endlist_emits_ended_once_after_all_samples() {
    let mut h = host_with(&ts_bodies(), TS_PLAYLIST_URL, Config::default());
    h.run_to_end();
    for _ in 0..20 {
        h.step();
    }
    h.collect();
    assert_eq!(count(&h.events, is_ended), 1);
    let ended = index_of(&h.events, is_ended).expect("ended");
    let last_sample = h.events.iter().rposition(is_sample).expect("sample");
    assert!(ended > last_sample, "Ended only after every sample");
    assert_eq!(ended, h.events.len() - 1, "nothing follows Ended");
    assert!(h.session.next_event().is_none());
}

#[test]
fn status_establishing_then_live_once() {
    let mut h = host_with(&ts_bodies(), TS_PLAYLIST_URL, Config::default());
    h.run_to_end();
    assert_eq!(
        statuses(&h.events),
        vec![Status::Establishing, Status::Live, Status::Ended]
    );
    let live = index_of(&h.events, |e| matches!(e, Event::Status(Status::Live))).expect("live");
    let first_sample = index_of(&h.events, is_sample).expect("sample");
    assert!(live < first_sample);
    assert!(
        h.events[live + 1..first_sample]
            .iter()
            .all(|e| matches!(e, Event::DisplayChanged { .. })),
        "Live is emitted at the first sample"
    );
}

#[test]
fn live_blocking_reload_query_in_request_url() {
    let ll = "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:2\n\
              #EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.0\n\
              #EXT-X-PART-INF:PART-TARGET=0.5\n#EXT-X-MEDIA-SEQUENCE:100\n\
              #EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2.0,\nseg100.m4s\n\
              #EXT-X-PART:DURATION=0.5,URI=\"p101.0.m4s\",INDEPENDENT=YES\n";
    let url = "http://origin.test/ll/live.m3u8";
    let mut s = Session::open(url, hls_config(), 0).expect("open");
    let (conn, first) = single_http_request(&drain_actions(&mut s, 0));
    assert_eq!(first, url, "the first fetch is a plain GET");
    respond_ok(&mut s, conn, ll.as_bytes(), 0);
    let reqs = http_requests(&drain_actions(&mut s, 0));
    let reload = reqs
        .iter()
        .map(|(_, u)| Url::parse(u).expect("url"))
        .find(|u| u.path() == "/ll/live.m3u8")
        .expect("blocking reload request");
    let pairs: HashMap<String, String> = reload
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert_eq!(pairs.get("_HLS_msn").map(String::as_str), Some("101"));
    assert_eq!(pairs.get("_HLS_part").map(String::as_str), Some("1"));
}

#[test]
fn prefetch_concurrent_connections_tracked() {
    let bodies = cmaf_bodies();
    let mut s = Session::open(CMAF_PLAYLIST_URL, hls_config(), 0).expect("open");
    let (conn, _) = single_http_request(&drain_actions(&mut s, 0));
    respond_ok(&mut s, conn, &bodies[CMAF_PLAYLIST_URL], 0);
    let reqs = http_requests(&drain_actions(&mut s, 0));
    assert_eq!(
        reqs.len(),
        4,
        "init + 3 segments in flight at once: {reqs:?}"
    );
    let mut conns: Vec<u32> = reqs.iter().map(|(c, _)| *c).collect();
    conns.sort_unstable();
    conns.dedup();
    assert_eq!(conns.len(), 4, "each prefetch has its own connection");

    for (c, _) in &reqs {
        s.on_response(*c, 200, &[], 0);
    }
    let bodies_by_conn: Vec<(u32, &Vec<u8>)> = reqs.iter().map(|(c, u)| (*c, &bodies[u])).collect();
    let max_len = bodies_by_conn
        .iter()
        .map(|(_, b)| b.len())
        .max()
        .expect("max");
    let chunk = 1500;
    let mut offset = 0;
    while offset < max_len {
        for (c, body) in &bodies_by_conn {
            if offset < body.len() {
                s.on_data(*c, &body[offset..(offset + chunk).min(body.len())], 0);
            }
        }
        offset += chunk;
    }
    for (c, _) in reqs.iter().rev() {
        s.on_closed(*c, None, 0);
    }
    let events = drain_events(&mut s);
    let expected = oracle_events(CMAF_PLAYLIST_URL, &bodies);
    assert_eq!(
        data_events(&events),
        expected,
        "interleaved bodies and reverse completion still yield in-order samples"
    );
    assert_eq!(count(&events, is_ended), 1);
}

#[test]
fn chunking_invariant_1_1316_65536_body_chunks() {
    let mut runs: Vec<Vec<Event>> = Vec::new();
    for chunk in [1usize, 1316, 65_536] {
        let mut h = host_with(&ts_bodies(), TS_PLAYLIST_URL, Config::default());
        h.chunk = chunk;
        h.run_to_end();
        runs.push(h.events);
    }
    assert!(count(&runs[2], is_sample) > 100);
    assert_eq!(runs[0], runs[2], "1-byte chunks");
    assert_eq!(runs[1], runs[2], "1316-byte chunks");
}

#[test]
fn session_samples_equal_bare_hls_client_oracle() {
    let bodies = ts_bodies();
    let mut h = host_with(&bodies, TS_PLAYLIST_URL, Config::default());
    h.run_to_end();
    let expected = oracle_events(TS_PLAYLIST_URL, &bodies);
    assert!(count(&expected, is_sample) > 100);
    assert_eq!(data_events(&h.events), expected);

    let cmaf = cmaf_bodies();
    let mut h2 = host_with(&cmaf, CMAF_PLAYLIST_URL, Config::default());
    h2.run_to_end();
    assert_eq!(
        data_events(&h2.events),
        oracle_events(CMAF_PLAYLIST_URL, &cmaf)
    );
}

#[test]
fn backoff_2_4_8_then_10_forever() {
    let mut s = Session::open(TS_PLAYLIST_URL, hls_config(), 0).expect("open");
    let mut now = 0u64;
    let mut deltas = Vec::new();
    let mut attempts = Vec::new();
    for _ in 0..BACKOFF_SECS.len() {
        let actions = drain_actions(&mut s, now);
        let (conn, url) = http_requests(&actions)
            .last()
            .cloned()
            .expect("a playlist request");
        assert_eq!(url, TS_PLAYLIST_URL);
        s.on_closed(conn, Some("reset".to_string()), now);
        let after = drain_actions(&mut s, now);
        let wakes = wake_times(&after);
        assert_eq!(wakes.len(), 1, "one retry wake: {after:?}");
        assert_eq!(after.len(), 1);
        deltas.push(wakes[0] - now);
        attempts.extend(reconnect_attempts(&drain_events(&mut s)));
        assert!(
            drain_actions(&mut s, wakes[0] - 1).is_empty(),
            "no request before the wake time"
        );
        now = wakes[0];
    }
    assert_eq!(
        deltas,
        BACKOFF_SECS.iter().map(|s| s * SEC).collect::<Vec<_>>()
    );
    assert_eq!(attempts, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn backoff_resets_after_success() {
    let live = live_playlist_text(0, &["index0.ts"]).into_bytes();
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.route(
        TS_PLAYLIST_URL,
        vec![
            Reply::TransportError,
            Reply::TransportError,
            Reply::Ok(live.clone()),
            Reply::TransportError,
            Reply::Ok(live),
        ],
    );
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.run_until(|evs| count(evs, is_reconnecting) >= 3, 200);
    h.run_until(|_| false, 6);
    assert_eq!(reconnect_attempts(&h.events), vec![1, 2, 1]);
    let times: Vec<u64> = h
        .requests_to(TS_PLAYLIST_URL)
        .iter()
        .map(|s| s.at_us)
        .collect();
    assert_eq!(times[0], 0);
    assert_eq!(times[1], 2 * SEC);
    assert_eq!(times[2], 6 * SEC);
    assert_eq!(
        times[4] - times[3],
        2 * SEC,
        "after a success the schedule restarts at 2 s"
    );
}

#[test]
fn main_playlist_404_emits_not_found_and_retries() {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.route(
        TS_PLAYLIST_URL,
        vec![
            Reply::Status(404),
            Reply::Status(410),
            Reply::Ok(ts_playlist()),
        ],
    );
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.ok(TS_SEG1_URL, ts_segment(1));
    h.run_to_end();
    let early: Vec<Status> = statuses(&h.events).into_iter().take(4).collect();
    assert_eq!(
        early,
        vec![
            Status::NotFound,
            Status::Reconnecting { attempt: 1 },
            Status::NotFound,
            Status::Reconnecting { attempt: 2 },
        ]
    );
    let times: Vec<u64> = h
        .requests_to(TS_PLAYLIST_URL)
        .iter()
        .map(|s| s.at_us)
        .collect();
    assert_eq!(times, vec![0, 2 * SEC, 6 * SEC]);
    assert_eq!(count(&h.events, is_not_found), 2);
    assert_eq!(count(&h.events, is_ended), 1);
    assert!(count(&h.events, is_sample) > 0);
}

#[test]
fn segment_404_is_resource_failure_not_not_found() {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.ok(TS_PLAYLIST_URL, ts_playlist());
    h.route(
        TS_SEG0_URL,
        vec![Reply::Status(404), Reply::Ok(ts_segment(0))],
    );
    h.ok(TS_SEG1_URL, ts_segment(1));
    h.run_to_end();
    assert_eq!(
        count(&h.events, is_not_found),
        0,
        "segment 404 is not NotFound"
    );
    assert_eq!(reconnect_attempts(&h.events), vec![1]);
    assert_eq!(
        h.hits(TS_SEG0_URL),
        2,
        "the failed segment is fetched again"
    );
    let expected = oracle_events(TS_PLAYLIST_URL, &ts_bodies());
    assert_eq!(
        data_events(&h.events),
        expected,
        "late first segment is still delivered before the second"
    );
    assert_eq!(count(&h.events, is_ended), 1);

    let mut gone = Host::new(TS_PLAYLIST_URL, hls_config());
    gone.ok(TS_PLAYLIST_URL, ts_playlist());
    gone.route(TS_SEG0_URL, vec![Reply::Status(410)]);
    gone.ok(TS_SEG1_URL, ts_segment(1));
    gone.run_until(|evs| count(evs, is_reconnecting) >= 2, 100);
    assert_eq!(count(&gone.events, is_not_found), 0);
    assert!(count(&gone.events, is_reconnecting) >= 2);
}

#[test]
fn non_2xx_playlist_calls_on_error_and_retries() {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.route(
        TS_PLAYLIST_URL,
        vec![
            Reply::Status(403),
            Reply::Status(500),
            Reply::Status(503),
            Reply::Ok(ts_playlist()),
        ],
    );
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.ok(TS_SEG1_URL, ts_segment(1));
    h.run_to_end();
    assert_eq!(reconnect_attempts(&h.events), vec![1, 2, 3]);
    assert_eq!(
        count(&h.events, |e| matches!(e, Event::Forbidden { status: 403 })),
        1
    );
    let times: Vec<u64> = h
        .requests_to(TS_PLAYLIST_URL)
        .iter()
        .map(|s| s.at_us)
        .collect();
    assert_eq!(times, vec![0, 2 * SEC, 6 * SEC, 14 * SEC]);
    assert_eq!(count(&h.events, is_not_found), 0);
    assert_eq!(count(&h.events, is_ended), 1);
}

#[test]
fn oversized_playlist_body_closes_and_fails() {
    let limit = 4096usize;
    let config = {
        let mut new_config = Config::default();
        new_config.kind = SourceKind::Hls;
        new_config.max_manifest_bytes = limit;
        new_config
    };
    let exact = padded_playlist(limit);
    let mut ok = Session::open(TS_PLAYLIST_URL, config.clone(), 0).expect("open");
    let (conn, _) = single_http_request(&drain_actions(&mut ok, 0));
    respond_ok(&mut ok, conn, &exact, 0);
    assert_eq!(
        http_requests(&drain_actions(&mut ok, 0)).len(),
        2,
        "a body of exactly the limit is accepted"
    );

    let mut s = Session::open(TS_PLAYLIST_URL, config, 0).expect("open");
    let (conn, _) = single_http_request(&drain_actions(&mut s, 0));
    s.on_response(conn, 200, &[], 0);
    s.on_data(conn, &exact, 0);
    assert!(closes(&drain_actions(&mut s, 0)).is_empty());
    s.on_data(conn, b"#", 0);
    let after = drain_actions(&mut s, 0);
    assert_eq!(
        closes(&after),
        vec![conn],
        "limit crossed: connection closed"
    );
    assert_eq!(wake_times(&after), vec![2 * SEC]);
    assert_eq!(reconnect_attempts(&drain_events(&mut s)), vec![1]);
    s.on_closed(conn, None, 0);
    assert!(
        drain_events(&mut s).is_empty(),
        "the aborted body is never parsed"
    );
}

#[test]
fn oversized_segment_body_closes_and_fails() {
    let limit = 1000usize;
    let config = {
        let mut new_config = Config::default();
        new_config.kind = SourceKind::Hls;
        new_config.max_segment_bytes = limit;
        new_config
    };
    let mut s = Session::open(TS_PLAYLIST_URL, config, 0).expect("open");
    let (conn, _) = single_http_request(&drain_actions(&mut s, 0));
    respond_ok(&mut s, conn, &ts_playlist(), 0);
    let reqs = http_requests(&drain_actions(&mut s, 0));
    assert_eq!(reqs.len(), 2);
    let (seg_conn, seg_url) = reqs[0].clone();
    assert_eq!(seg_url, TS_SEG0_URL);
    let body = ts_segment(0);
    assert!(body.len() > limit + 1);
    s.on_response(seg_conn, 200, &[], 0);
    s.on_data(seg_conn, &body[..limit], 0);
    assert!(closes(&drain_actions(&mut s, 0)).is_empty());
    s.on_data(seg_conn, &body[limit..=limit], 0);
    let after = drain_actions(&mut s, 0);
    assert_eq!(closes(&after), vec![seg_conn]);
    assert_eq!(reconnect_attempts(&drain_events(&mut s)), vec![1]);
    s.on_closed(seg_conn, None, 0);
    assert_eq!(count(&drain_events(&mut s), is_sample), 0);
}

#[test]
fn request_timeout_aborts_playlist_and_segment() {
    let mut s = Session::open(TS_PLAYLIST_URL, hls_config(), 0).expect("open");
    let (conn, _) = single_http_request(&drain_actions(&mut s, 0));
    assert!(drain_actions(&mut s, 20 * SEC - 1).is_empty());
    let aborted = drain_actions(&mut s, 20 * SEC);
    assert_eq!(closes(&aborted), vec![conn], "playlist times out at 20 s");
    assert_eq!(reconnect_attempts(&drain_events(&mut s)), vec![1]);
    assert_eq!(wake_times(&aborted), vec![22 * SEC]);

    let mut s2 = Session::open(TS_PLAYLIST_URL, hls_config(), 0).expect("open");
    let (c, _) = single_http_request(&drain_actions(&mut s2, 0));
    respond_ok(&mut s2, c, &ts_playlist(), 0);
    let reqs = http_requests(&drain_actions(&mut s2, 0));
    let seg_conns: Vec<u32> = reqs.iter().map(|(c, _)| *c).collect();
    assert_eq!(seg_conns.len(), 2);
    for t in [20 * SEC, 59 * SEC + 999_999] {
        assert!(
            closes(&drain_actions(&mut s2, t)).is_empty(),
            "segments get the longer 60 s budget (t={t})"
        );
    }
    let mut closed = closes(&drain_actions(&mut s2, 60 * SEC));
    closed.sort_unstable();
    assert_eq!(closed, seg_conns);
    assert!(!reconnect_attempts(&drain_events(&mut s2)).is_empty());
}

#[test]
fn auth_bearer_on_every_request_to_origin() {
    let mut h = host_with(&ts_bodies(), TS_PLAYLIST_URL, Config::default());
    h.session
        .set_credentials(Credentials::bearer(BEARER_TOKEN))
        .unwrap();
    h.run_to_end();
    assert_eq!(h.seen.len(), 3);
    let expected = format!("Bearer {BEARER_TOKEN}");
    for seen in &h.seen {
        assert_eq!(
            seen.authorization(),
            Some(expected.as_str()),
            "{}",
            seen.url
        );
    }
}

#[test]
fn cross_origin_segment_carries_no_authorization() {
    let playlist = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:2.0,\nindex0.ts\n#EXTINF:1.0,\n{CDN_SEG_URL}\n\
         #EXT-X-ENDLIST\n"
    );
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.session
        .set_credentials(Credentials::bearer(BEARER_TOKEN))
        .unwrap();
    h.ok(TS_PLAYLIST_URL, playlist.into_bytes());
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.ok(CDN_SEG_URL, ts_segment(1));
    h.run_to_end();
    let cdn = h.requests_to(CDN_SEG_URL);
    assert_eq!(cdn.len(), 1);
    assert_eq!(
        cdn[0].authorization(),
        None,
        "no credentials leak to the CDN"
    );
    assert!(h.requests_to(TS_PLAYLIST_URL)[0].authorization().is_some());
    assert!(h.requests_to(TS_SEG0_URL)[0].authorization().is_some());
    assert_eq!(count(&h.events, is_ended), 1);
}

#[test]
fn cross_origin_401_is_ordinary_failure_not_auth_prompt() {
    let playlist = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:2.0,\nindex0.ts\n#EXTINF:1.0,\n{CDN_SEG_URL}\n\
         #EXT-X-ENDLIST\n"
    );
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.session
        .set_credentials(Credentials::bearer(BEARER_TOKEN))
        .unwrap();
    h.ok(TS_PLAYLIST_URL, playlist.into_bytes());
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.route(
        CDN_SEG_URL,
        vec![Reply::Status(401), Reply::Ok(ts_segment(1))],
    );
    h.run_to_end();
    assert_eq!(
        count(&h.events, |e| matches!(e, Event::AuthRequired { .. })),
        0
    );
    assert_eq!(
        count(&h.events, |e| matches!(
            e,
            Event::Status(Status::WaitingForCredentials)
        )),
        0
    );
    let cdn = h.requests_to(CDN_SEG_URL);
    assert_eq!(
        cdn.len(),
        2,
        "401 from another origin is retried like any failure"
    );
    assert!(cdn.iter().all(|r| r.authorization().is_none()));
    assert_eq!(count(&h.events, is_ended), 1);
}

#[test]
fn per_request_redirect_followed_and_counted_separately() {
    let hop = |seg: usize, n: usize| format!("http://origin.test/hls/r{seg}h{n}");
    let build = |chain: [usize; 2]| {
        let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
        h.ok(TS_PLAYLIST_URL, ts_playlist());
        for (seg, (url, body)) in [(TS_SEG0_URL, ts_segment(0)), (TS_SEG1_URL, ts_segment(1))]
            .into_iter()
            .enumerate()
        {
            let len = chain[seg];
            h.route(url, vec![Reply::Redirect(hop(seg, 1))]);
            for n in 1..len {
                h.route(&hop(seg, n), vec![Reply::Redirect(hop(seg, n + 1))]);
            }
            h.route(&hop(seg, len), vec![Reply::Ok(body)]);
        }
        h
    };

    let mut h = build([5, 5]);
    h.run_to_end();
    assert_eq!(
        reconnect_attempts(&h.events),
        Vec::<u64>::new(),
        "two requests with five redirects each both succeed (10 > max 5 if shared)"
    );
    assert_eq!(count(&h.events, is_ended), 1);
    assert_eq!(h.hits(&hop(0, 5)), 1);
    assert_eq!(h.hits(&hop(1, 5)), 1);

    let mut over = build([6, 1]);
    over.run_until(|evs| count(evs, is_reconnecting) >= 1, 200);
    assert_eq!(
        reconnect_attempts(&over.events).first(),
        Some(&1),
        "a sixth redirect for one request exceeds max_redirects"
    );
    assert_eq!(
        over.hits(&hop(0, 6)),
        0,
        "the over-limit hop is never followed"
    );

    let mut auth = Host::new(TS_PLAYLIST_URL, hls_config());
    auth.session
        .set_credentials(Credentials::bearer(BEARER_TOKEN))
        .unwrap();
    auth.ok(TS_PLAYLIST_URL, ts_playlist());
    auth.route(
        TS_SEG0_URL,
        vec![Reply::Redirect(
            "http://cdn.test/moved/index0.ts".to_string(),
        )],
    );
    auth.ok("http://cdn.test/moved/index0.ts", ts_segment(0));
    auth.ok(TS_SEG1_URL, ts_segment(1));
    auth.run_to_end();
    assert!(auth.requests_to(TS_SEG0_URL)[0].authorization().is_some());
    assert_eq!(
        auth.requests_to("http://cdn.test/moved/index0.ts")[0].authorization(),
        None,
        "a cross-origin redirect drops credentials"
    );
}

#[test]
fn credentials_401_on_playlist_waits_no_retry() {
    let mut s = Session::open(TS_PLAYLIST_URL, hls_config(), 0).expect("open");
    let (conn, _) = single_http_request(&drain_actions(&mut s, 0));
    s.on_response(
        conn,
        401,
        &[(
            "WWW-Authenticate".to_string(),
            "Bearer realm=\"example\"".to_string(),
        )],
        0,
    );
    let events = drain_events(&mut s);
    assert!(
        matches!(events[0], Event::AuthRequired { .. }),
        "{events:?}"
    );
    assert!(matches!(
        events[1],
        Event::Status(Status::WaitingForCredentials)
    ));
    assert_eq!(events.len(), 2);
    assert_eq!(closes(&drain_actions(&mut s, 0)), vec![conn]);
    let mut t = 0u64;
    for _ in 0..100 {
        t += 10 * 60 * SEC / 100;
        assert!(
            s.poll(t).is_none(),
            "no actions while waiting for credentials"
        );
    }
    assert!(s.next_event().is_none());

    s.set_credentials(Credentials::bearer(BEARER_TOKEN))
        .unwrap();
    let actions = drain_actions(&mut s, t);
    let (_, url) = single_http_request(&actions);
    assert_eq!(url, TS_PLAYLIST_URL);
    match &actions[0] {
        Action::HttpRequest { headers, .. } => assert!(
            headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v == "Bearer tok-123")
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn late_callbacks_after_close_ignored() {
    let mut s = Session::open(TS_PLAYLIST_URL, hls_config(), 0).expect("open");
    let (conn, _) = single_http_request(&drain_actions(&mut s, 0));
    respond_ok(&mut s, conn, &ts_playlist(), 0);
    let reqs = http_requests(&drain_actions(&mut s, 0));
    assert_eq!(reqs.len(), 2);
    let _ = drain_events(&mut s);
    s.close();
    let mut closed = closes(&drain_actions(&mut s, 0));
    closed.sort_unstable();
    let mut expected: Vec<u32> = reqs.iter().map(|(c, _)| *c).collect();
    expected.sort_unstable();
    assert_eq!(closed, expected);
    for (c, _) in &reqs {
        s.on_response(*c, 200, &[], 0);
        s.on_data(*c, &ts_segment(0), 0);
        s.on_closed(*c, None, 0);
    }
    s.on_data(9_999, b"x", 0);
    assert!(s.next_event().is_none());
    assert!(s.poll(1).is_none());

    let config = {
        let mut new_config = Config::default();
        new_config.kind = SourceKind::Hls;
        new_config.max_manifest_bytes = 100;
        new_config
    };
    let mut o = Session::open(TS_PLAYLIST_URL, config, 0).expect("open");
    let (conn, _) = single_http_request(&drain_actions(&mut o, 0));
    o.on_response(conn, 200, &[], 1);
    o.on_data(conn, &[b'#'; 101], 0);
    assert_eq!(closes(&drain_actions(&mut o, 0)), vec![conn]);
    assert_eq!(reconnect_attempts(&drain_events(&mut o)), vec![1]);
    o.on_data(conn, &ts_playlist(), 0);
    o.on_response(conn, 200, &[], 1);
    o.on_closed(conn, None, 0);
    o.on_closed(conn, Some("late".to_string()), 0);
    assert!(
        drain_events(&mut o).is_empty(),
        "late callbacks add no events"
    );
    assert!(
        drain_actions(&mut o, 0).is_empty(),
        "late callbacks add no actions"
    );
}

#[test]
fn unsupported_container_error_is_failure_not_panic() {
    let bad_playlists: [(&str, Vec<u8>); 2] = [
        (
            "unparsable tag",
            b"#EXTM3U\n#EXT-X-TARGETDURATION:notanumber\n#EXTINF:2.0,\nseg.ts\n".to_vec(),
        ),
        ("not utf-8", vec![0xFF, 0xFE, 0xFD, 0x00, 0x80]),
    ];
    for (label, body) in bad_playlists {
        let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
        h.route(
            TS_PLAYLIST_URL,
            vec![Reply::Ok(body), Reply::Ok(ts_playlist())],
        );
        h.ok(TS_SEG0_URL, ts_segment(0));
        h.ok(TS_SEG1_URL, ts_segment(1));
        h.run_to_end();
        assert_eq!(
            reconnect_attempts(&h.events),
            vec![1],
            "{label}: a rejected playlist is one failure"
        );
        let times: Vec<u64> = h
            .requests_to(TS_PLAYLIST_URL)
            .iter()
            .map(|s| s.at_us)
            .collect();
        assert_eq!(
            times,
            vec![0, 2 * SEC],
            "{label}: retried after the backoff"
        );
        assert_eq!(count(&h.events, is_ended), 1, "{label}: recovers on retry");
        assert!(count(&h.events, is_sample) > 0);
    }

    let matroska = vec![0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x86, 0x81, 0x01];
    let mut ts = Host::new(TS_PLAYLIST_URL, hls_config());
    ts.ok(TS_PLAYLIST_URL, ts_playlist());
    ts.ok(TS_SEG0_URL, matroska.clone());
    ts.ok(TS_SEG1_URL, matroska.clone());
    ts.run_to_end();
    assert_eq!(
        count(&ts.events, is_sample),
        0,
        "unsupported container yields no samples"
    );
    assert_eq!(count(&ts.events, is_tracks), 0);

    let bodies = cmaf_bodies();
    let mut fm = Host::new(CMAF_PLAYLIST_URL, hls_config());
    fm.ok(CMAF_PLAYLIST_URL, bodies[CMAF_PLAYLIST_URL].clone());
    fm.ok(&format!("{CMAF_BASE}/init.mp4"), cmaf_init());
    for i in 0..3 {
        fm.ok(&format!("{CMAF_BASE}/index{i}.m4s"), matroska.clone());
    }
    fm.run_to_end();
    assert_eq!(count(&fm.events, is_sample), 0);
}
