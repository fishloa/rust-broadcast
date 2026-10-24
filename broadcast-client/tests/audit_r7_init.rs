//! Round-7 findings F1 + F2: an EXT-X-MAP init fetch that fails is retried with the same bounded
//! schedule as every other dependency (VOD and live alike), a permanent failure ends in exactly
//! one typed fatal error, and a re-signed MAP URI is the same init.

mod support;
use broadcast_client::{Config, ErrorKind, Event, events::DEFAULT_MAX_KEY_FAILURES};
use support::fmp4_split::{group, split};
use support::hls_host::{Host, Reply, hls_config, is_ended};

const BASE: &str = "http://origin.test/v";
const PL: &str = "http://origin.test/v/index.m3u8";
const FRAGMENT_FIXTURE: &str = "transmux/h264_aac_frag.mp4";
const SEGMENT_GROUP: usize = 3;
const STEPS: usize = 20_000;
const VOD_HEAD: &str = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-PLAYLIST-TYPE:VOD\n";

fn parts() -> (Vec<u8>, Vec<Vec<u8>>) {
    let b = test_fixtures::read(FRAGMENT_FIXTURE);
    let s = split(&b).unwrap();
    (s.init, group(&s.segments, SEGMENT_GROUP))
}

#[derive(Debug, Default)]
struct Tally {
    tracks: usize,
    samples: usize,
    ended: bool,
    fatals: Vec<ErrorKind>,
    non_fatals: Vec<ErrorKind>,
}

fn tally(h: &Host) -> Tally {
    let mut t = Tally::default();
    for e in &h.events {
        match e {
            Event::Tracks(_) => t.tracks += 1,
            Event::Sample { .. } => t.samples += 1,
            Event::Error { kind, fatal: true } => t.fatals.push(kind.clone()),
            Event::Error { kind, fatal: false } => t.non_fatals.push(kind.clone()),
            e if is_ended(e) => t.ended = true,
            _ => {}
        }
    }
    t
}

fn vod_playlist(maps: &[&str]) -> Vec<u8> {
    let mut p = String::from(VOD_HEAD);
    for (i, m) in maps.iter().enumerate() {
        p.push_str(&format!("#EXT-X-MAP:URI=\"{m}\"\n#EXTINF:1.0,\ns{i}.m4s\n"));
    }
    p.push_str("#EXT-X-ENDLIST\n");
    p.into_bytes()
}

fn vod_host(maps: &[&str], init_replies: &[(&str, Vec<Reply>)]) -> Host {
    let (init, segs) = parts();
    let mut h = Host::new(PL, hls_config());
    h.ok(PL, vod_playlist(maps));
    for m in maps {
        h.ok(&format!("{BASE}/{m}"), init.clone());
    }
    for (m, replies) in init_replies {
        h.route(&format!("{BASE}/{m}"), replies.clone());
    }
    for i in 0..maps.len() {
        h.ok(&format!("{BASE}/s{i}.m4s"), segs[i].clone());
    }
    h
}

fn clean_samples(maps: &[&str]) -> usize {
    let mut h = vod_host(maps, &[]);
    h.run_until(|e| e.iter().any(is_ended), STEPS);
    let t = tally(&h);
    assert!(t.ended && t.fatals.is_empty(), "clean baseline: {t:?}");
    t.samples
}

fn stop_on_end_or_fatal(e: &[Event]) -> bool {
    e.iter()
        .any(|x| is_ended(x) || matches!(x, Event::Error { fatal: true, .. }))
}

#[test]
fn vod_init_transient_503_recovers_and_plays_everything() {
    let want = clean_samples(&["i1.mp4"]);
    let mut h = vod_host(&["i1.mp4"], &[("i1.mp4", vec![Reply::Status(503), Reply::Ok(parts().0)])]);
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert!(t.ended, "{t:?}");
    assert!(t.fatals.is_empty(), "{t:?}");
    assert_eq!(t.samples, want);
    assert_eq!(t.tracks, 1);
    assert_eq!(h.hits(&format!("{BASE}/i1.mp4")), 2);
}

#[test]
fn vod_init_permanent_404_is_one_typed_fatal_after_the_cap() {
    let mut h = vod_host(&["i1.mp4"], &[("i1.mp4", vec![Reply::Status(404)])]);
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert!(!t.ended, "{t:?}");
    assert_eq!(t.fatals, vec![ErrorKind::InitFetchFailed { status: Some(404) }]);
    assert_eq!(
        h.hits(&format!("{BASE}/i1.mp4")),
        DEFAULT_MAX_KEY_FAILURES as usize
    );
    h.run_until(|_| false, STEPS);
    assert_eq!(tally(&h).fatals.len(), 1);
    assert_eq!(
        h.hits(&format!("{BASE}/i1.mp4")),
        DEFAULT_MAX_KEY_FAILURES as usize,
        "no fetch after fatal"
    );
}

#[test]
fn vod_init_permanent_503_is_one_typed_fatal() {
    let mut h = vod_host(&["i1.mp4"], &[("i1.mp4", vec![Reply::Status(503)])]);
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert_eq!(t.fatals, vec![ErrorKind::InitFetchFailed { status: Some(503) }], "{t:?}");
}

#[test]
fn vod_init_transport_error_reports_no_status() {
    let mut h = vod_host(&["i1.mp4"], &[("i1.mp4", vec![Reply::TransportError])]);
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert_eq!(t.fatals, vec![ErrorKind::InitFetchFailed { status: None }], "{t:?}");
}

#[test]
fn live_init_transient_503_still_recovers() {
    let (init, segs) = parts();
    let mut pl = vec![];
    for r in 0..6usize {
        pl.push(Reply::Ok(
            format!(
                "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{r}\n#EXT-X-MAP:URI=\"i1.mp4\"\n#EXTINF:1.0,\ns{}.m4s\n",
                r.min(2)
            )
            .into_bytes(),
        ));
    }
    let mut h = Host::new(PL, hls_config());
    h.route(PL, pl);
    h.route(&format!("{BASE}/i1.mp4"), vec![Reply::Status(503), Reply::Ok(init)]);
    for (i, seg) in segs.iter().enumerate().take(3) {
        h.ok(&format!("{BASE}/s{i}.m4s"), seg.clone());
    }
    h.run_until(|_| false, 3_000);
    let t = tally(&h);
    assert!(t.fatals.is_empty(), "{t:?}");
    assert!(t.samples > 0, "{t:?}");
    assert_eq!(t.tracks, 1);
}

#[test]
fn live_init_permanent_failure_is_one_typed_fatal() {
    let (_, segs) = parts();
    let pl: Vec<Reply> = (0..40usize)
        .map(|r| {
            Reply::Ok(
                format!(
                    "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{r}\n#EXT-X-MAP:URI=\"i1.mp4\"\n#EXTINF:1.0,\ns0.m4s\n"
                )
                .into_bytes(),
            )
        })
        .collect();
    let mut h = Host::new(PL, hls_config());
    h.route(PL, pl);
    h.route(&format!("{BASE}/i1.mp4"), vec![Reply::Status(404)]);
    h.ok(&format!("{BASE}/s0.m4s"), segs[0].clone());
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert_eq!(t.fatals, vec![ErrorKind::InitFetchFailed { status: Some(404) }], "{t:?}");
}

#[test]
fn vod_second_map_init_transient_failure_recovers() {
    let maps = ["i1.mp4", "i2.mp4"];
    let want = clean_samples(&maps);
    let mut h = vod_host(&maps, &[("i2.mp4", vec![Reply::Status(503), Reply::Ok(parts().0)])]);
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert!(t.ended && t.fatals.is_empty(), "{t:?}");
    assert_eq!(t.samples, want);
}

#[test]
fn vod_second_map_init_permanent_failure_is_one_typed_fatal() {
    let mut h = vod_host(&["i1.mp4", "i2.mp4"], &[("i2.mp4", vec![Reply::Status(404)])]);
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert!(!t.ended, "{t:?}");
    assert_eq!(t.fatals, vec![ErrorKind::InitFetchFailed { status: Some(404) }], "{t:?}");
}

#[test]
fn init_failure_cap_follows_max_key_failures() {
    let cfg = Config { max_key_failures: 2, ..hls_config() };
    let (init, segs) = parts();
    let mut h = Host::new(PL, cfg);
    h.ok(PL, vod_playlist(&["i1.mp4"]));
    h.route(&format!("{BASE}/i1.mp4"), vec![Reply::Status(500)]);
    h.ok(&format!("{BASE}/s0.m4s"), segs[0].clone());
    let _ = init;
    h.run_until(stop_on_end_or_fatal, STEPS);
    assert_eq!(h.hits(&format!("{BASE}/i1.mp4")), 2);
    assert_eq!(tally(&h).fatals.len(), 1);
}

#[test]
fn vod_init_retry_does_not_refetch_the_ended_playlist() {
    let mut h = vod_host(&["i1.mp4"], &[("i1.mp4", vec![Reply::Status(503), Reply::Ok(parts().0)])]);
    h.run_until(stop_on_end_or_fatal, STEPS);
    assert!(tally(&h).ended);
    assert_eq!(h.hits(PL), 1);
}

const RESIGN_WINDOWS: usize = 3;

struct Resign {
    init_fetches: usize,
    t: Tally,
}

fn resign(map_resign: bool, seg_resign: bool) -> Resign {
    let (init, segs) = parts();
    let n = segs.len().min(RESIGN_WINDOWS);
    let mut h = Host::new(PL, hls_config());
    let mq = |r: usize| if map_resign { format!("?sig={r}") } else { String::new() };
    let sq = |r: usize| if seg_resign { format!("?sig={r}") } else { String::new() };
    let mut replies = vec![];
    for r in 0..n {
        let mut p = format!(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{}\n#EXT-X-MAP:URI=\"init.mp4{}\"\n",
            r.saturating_sub(1),
            mq(r)
        );
        for i in r.saturating_sub(1)..=r {
            p.push_str(&format!("#EXTINF:1.0,\ns{i}.m4s{}\n", sq(r)));
        }
        replies.push(Reply::Ok(p.into_bytes()));
    }
    h.route(PL, replies);
    for r in 0..n {
        h.ok(&format!("{BASE}/init.mp4{}", mq(r)), init.clone());
        for (i, seg) in segs.iter().enumerate().take(n) {
            h.ok(&format!("{BASE}/s{i}.m4s{}", sq(r)), seg.clone());
        }
    }
    h.run_until(|_| false, 400);
    let init_fetches = (0..n)
        .map(|r| h.hits(&format!("{BASE}/init.mp4{}", mq(r))))
        .sum();
    Resign { init_fetches, t: tally(&h) }
}

#[test]
fn resigned_map_uri_is_one_init_one_tracks_event() {
    let stable = resign(false, false);
    for (m, s) in [(true, false), (true, true)] {
        let r = resign(m, s);
        assert_eq!(r.init_fetches, 1, "map_resign={m} seg_resign={s}");
        assert_eq!(r.t.tracks, 1, "map_resign={m} seg_resign={s}: {:?}", r.t);
        assert_eq!(r.t.samples, stable.t.samples, "map_resign={m} seg_resign={s}");
        assert!(r.t.fatals.is_empty());
    }
    assert_eq!(stable.init_fetches, 1);
    assert_eq!(stable.t.tracks, 1);
}

#[test]
fn changed_map_path_is_a_new_init() {
    let (init, segs) = parts();
    let mut p = format!("{VOD_HEAD}#EXT-X-MAP:URI=\"a.mp4?sig=1\"\n#EXTINF:1.0,\ns0.m4s\n");
    p.push_str("#EXT-X-MAP:URI=\"b.mp4?sig=1\"\n#EXTINF:1.0,\ns1.m4s\n#EXT-X-ENDLIST\n");
    let mut h = Host::new(PL, hls_config());
    h.ok(PL, p.into_bytes());
    h.ok(&format!("{BASE}/a.mp4?sig=1"), init.clone());
    h.ok(&format!("{BASE}/b.mp4?sig=1"), init);
    h.ok(&format!("{BASE}/s0.m4s"), segs[0].clone());
    h.ok(&format!("{BASE}/s1.m4s"), segs[1].clone());
    h.run_until(stop_on_end_or_fatal, STEPS);
    assert!(tally(&h).ended);
    assert_eq!(h.hits(&format!("{BASE}/a.mp4?sig=1")), 1);
    assert_eq!(h.hits(&format!("{BASE}/b.mp4?sig=1")), 1);
}

#[test]
fn changed_map_byte_range_is_a_new_init() {
    let (init, segs) = parts();
    let len = init.len();
    let mut file = init.clone();
    file.extend_from_slice(&init);
    let mut p = format!("{VOD_HEAD}#EXT-X-MAP:URI=\"m.mp4?sig=1\",BYTERANGE=\"{len}@0\"\n#EXTINF:1.0,\ns0.m4s\n");
    p.push_str(&format!(
        "#EXT-X-MAP:URI=\"m.mp4?sig=2\",BYTERANGE=\"{len}@{len}\"\n#EXTINF:1.0,\ns1.m4s\n#EXT-X-ENDLIST\n"
    ));
    let mut h = Host::new(PL, hls_config());
    h.ok(PL, p.into_bytes());
    h.ok(&format!("{BASE}/m.mp4?sig=1"), file.clone());
    h.ok(&format!("{BASE}/m.mp4?sig=2"), file);
    h.ok(&format!("{BASE}/s0.m4s"), segs[0].clone());
    h.ok(&format!("{BASE}/s1.m4s"), segs[1].clone());
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert!(t.ended && t.fatals.is_empty(), "{t:?}");
    assert_eq!(h.hits(&format!("{BASE}/m.mp4?sig=1")), 1);
    assert_eq!(h.hits(&format!("{BASE}/m.mp4?sig=2")), 1);
}

#[test]
fn failed_resigned_init_is_retried_with_the_newest_url() {
    let (init, segs) = parts();
    let mut pl = vec![];
    for r in 0..8usize {
        pl.push(Reply::Ok(
            format!(
                "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{r}\n#EXT-X-MAP:URI=\"init.mp4?sig={r}\"\n#EXTINF:1.0,\ns0.m4s\n"
            )
            .into_bytes(),
        ));
    }
    let mut h = Host::new(PL, hls_config());
    h.route(PL, pl);
    h.route(&format!("{BASE}/init.mp4?sig=0"), vec![Reply::Status(503)]);
    for r in 1..8 {
        h.ok(&format!("{BASE}/init.mp4?sig={r}"), init.clone());
    }
    h.ok(&format!("{BASE}/s0.m4s"), segs[0].clone());
    h.run_until(|_| false, 600);
    let t = tally(&h);
    assert!(t.fatals.is_empty(), "{t:?}");
    assert!(t.samples > 0, "{t:?}");
    assert_eq!(t.tracks, 1);
}
