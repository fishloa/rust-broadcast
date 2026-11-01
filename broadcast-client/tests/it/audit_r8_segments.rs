//! Round-8 findings F1 to F4: a 200 answer that is not media for its container is a failed fetch
//! (never a silently consumed segment); a segment that keeps failing walks the retry schedule and
//! ends fatal (VOD) or is skipped with a discontinuity (live); `Retry-After` on 429/503 stretches
//! the wait, clamped; an init that never answers times out on the playlist-class timeout.

use crate::support;
use broadcast_client::{Config, ErrorKind, Event};
use support::fmp4_split::{group, split};
use support::hls_host::{
    Host, Reply, SEC, TS_PLAYLIST_URL, TS_SEG0_URL, TS_SEG1_URL, hls_config, is_ended, ts_playlist,
    ts_segment,
};

const FMP4_BASE: &str = "http://origin.test/v";
const FMP4_PLAYLIST_URL: &str = "http://origin.test/v/index.m3u8";
const FMP4_INIT_URL: &str = "http://origin.test/v/i.mp4";
const FRAGMENT_FIXTURE: &str = "transmux/h264_aac_frag.mp4";
const SEGMENT_GROUP: usize = 3;
const STEPS: usize = 40_000;
const GARBAGE_LEN: u32 = 4096;
const GARBAGE_MULTIPLIER: u32 = 2_654_435_761;
const GARBAGE_SHIFT: u32 = 13;
const TS_SYNC_BYTE: u8 = 0x47;
const HTML_ERROR_PAGE: &[u8] = b"<html><body>Service unavailable, try later</body></html>";

fn random_bytes() -> Vec<u8> {
    (0..GARBAGE_LEN)
        .map(|i| (i.wrapping_mul(GARBAGE_MULTIPLIER) >> GARBAGE_SHIFT) as u8)
        .collect()
}

fn sync_led_garbage() -> Vec<u8> {
    let mut g = random_bytes();
    g[0] = TS_SYNC_BYTE;
    g
}

fn non_media_bodies() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("random", random_bytes()),
        ("sync_led_random", sync_led_garbage()),
        ("html", HTML_ERROR_PAGE.to_vec()),
        ("empty", Vec::new()),
    ]
}

#[derive(Debug, Default)]
struct Tally {
    tracks: usize,
    samples: usize,
    discontinuities: usize,
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
            Event::Discontinuity => t.discontinuities += 1,
            Event::Error { kind, fatal: true } => t.fatals.push(kind.clone()),
            Event::Error { kind, fatal: false } => t.non_fatals.push(kind.clone()),
            e if is_ended(e) => t.ended = true,
            _ => {}
        }
    }
    t
}

fn stop_on_end_or_fatal(e: &[Event]) -> bool {
    e.iter()
        .any(|x| is_ended(x) || matches!(x, Event::Error { fatal: true, .. }))
}

fn gaps_secs(h: &Host, url: &str) -> Vec<u64> {
    let at: Vec<u64> = h.requests_to(url).iter().map(|s| s.at_us).collect();
    at.windows(2).map(|w| (w[1] - w[0]) / SEC).collect()
}

fn ts_vod(seg0: Vec<Reply>, config: Config) -> Host {
    let mut h = Host::new(TS_PLAYLIST_URL, config);
    h.ok(TS_PLAYLIST_URL, ts_playlist());
    h.route(TS_SEG0_URL, seg0);
    h.ok(TS_SEG1_URL, ts_segment(1));
    h
}

fn ts_clean_samples() -> usize {
    let mut h = ts_vod(vec![Reply::Ok(ts_segment(0))], hls_config());
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert!(t.ended && t.fatals.is_empty(), "clean baseline: {t:?}");
    t.samples
}

fn fmp4_parts() -> (Vec<u8>, Vec<Vec<u8>>) {
    let b = test_fixtures::read(FRAGMENT_FIXTURE);
    let s = split(&b).unwrap();
    (s.init, group(&s.segments, SEGMENT_GROUP))
}

fn fmp4_vod(seg0: Vec<Reply>, config: Config) -> Host {
    fmp4_host(vec![Reply::Ok(fmp4_parts().0)], seg0, config)
}

fn fmp4_host(init_replies: Vec<Reply>, seg0: Vec<Reply>, config: Config) -> Host {
    let segs = fmp4_parts().1;
    let playlist = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-PLAYLIST-TYPE:VOD\n\
                    #EXT-X-MAP:URI=\"i.mp4\"\n#EXTINF:1.0,\ns0.m4s\n#EXTINF:1.0,\ns1.m4s\n\
                    #EXT-X-ENDLIST\n";
    let mut h = Host::new(FMP4_PLAYLIST_URL, config);
    h.ok(FMP4_PLAYLIST_URL, playlist.as_bytes().to_vec());
    h.route(FMP4_INIT_URL, init_replies);
    h.route(&format!("{FMP4_BASE}/s0.m4s"), seg0);
    h.ok(&format!("{FMP4_BASE}/s1.m4s"), segs[1].clone());
    h
}

fn fmp4_clean_samples() -> usize {
    let (_, segs) = fmp4_parts();
    let mut h = fmp4_vod(vec![Reply::Ok(segs[0].clone())], hls_config());
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert!(t.ended && t.fatals.is_empty(), "clean baseline: {t:?}");
    t.samples
}

#[test]
fn ts_vod_non_media_segment_is_retried_not_consumed() {
    let want = ts_clean_samples();
    for (name, body) in non_media_bodies() {
        let mut h = ts_vod(
            vec![Reply::Ok(body), Reply::Ok(ts_segment(0))],
            hls_config(),
        );
        h.run_until(stop_on_end_or_fatal, STEPS);
        let t = tally(&h);
        assert!(t.ended && t.fatals.is_empty(), "{name}: {t:?}");
        assert_eq!(t.samples, want, "{name}: every sample of the clean run");
        assert_eq!(h.hits(TS_SEG0_URL), 2, "{name}: refetched once");
        assert_eq!(gaps_secs(&h, TS_SEG0_URL), vec![2], "{name}: first backoff");
    }
}

#[test]
fn ts_vod_permanent_non_media_segment_is_one_fatal_never_a_clean_end() {
    for (name, body) in non_media_bodies() {
        let mut h = ts_vod(vec![Reply::Ok(body)], hls_config());
        h.run_until(stop_on_end_or_fatal, STEPS);
        let t = tally(&h);
        assert!(!t.ended, "{name}: ended cleanly with a lost segment: {t:?}");
        assert_eq!(
            t.fatals,
            vec![ErrorKind::SegmentFetchFailed { status: None }],
            "{name}: {t:?}"
        );
        h.run_until(|_| false, STEPS);
        assert_eq!(tally(&h).fatals.len(), 1, "{name}: exactly one fatal");
    }
}

#[test]
fn fmp4_vod_non_media_segment_is_retried_not_consumed() {
    let want = fmp4_clean_samples();
    let (_, segs) = fmp4_parts();
    for (name, body) in non_media_bodies() {
        let mut h = fmp4_vod(
            vec![Reply::Ok(body), Reply::Ok(segs[0].clone())],
            hls_config(),
        );
        h.run_until(stop_on_end_or_fatal, STEPS);
        let t = tally(&h);
        assert!(t.ended && t.fatals.is_empty(), "{name}: {t:?}");
        assert_eq!(t.samples, want, "{name}");
        assert_eq!(h.hits(&format!("{FMP4_BASE}/s0.m4s")), 2, "{name}");
    }
}

#[test]
fn fmp4_vod_permanent_non_media_segment_is_one_fatal() {
    for (name, body) in non_media_bodies() {
        let mut h = fmp4_vod(vec![Reply::Ok(body)], hls_config());
        h.run_until(stop_on_end_or_fatal, STEPS);
        let t = tally(&h);
        assert!(!t.ended, "{name}: {t:?}");
        assert_eq!(
            t.fatals,
            vec![ErrorKind::SegmentFetchFailed { status: None }],
            "{name}: {t:?}"
        );
    }
}

#[test]
fn a_truncated_fmp4_segment_without_its_mdat_is_a_failed_fetch() {
    let (_, segs) = fmp4_parts();
    let whole = segs[0].clone();
    let cut = whole.len() / 2;
    let mut h = fmp4_vod(
        vec![Reply::Ok(whole[..cut].to_vec()), Reply::Ok(whole)],
        hls_config(),
    );
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert!(t.ended && t.fatals.is_empty(), "{t:?}");
    assert_eq!(h.hits(&format!("{FMP4_BASE}/s0.m4s")), 2);
}

#[test]
fn a_structurally_valid_ts_segment_with_no_samples_is_not_a_failure() {
    const TS_PACKET_LEN: usize = 188;
    const PSI_PACKETS: usize = 2;
    let psi_only = ts_segment(0)[..PSI_PACKETS * TS_PACKET_LEN].to_vec();
    let mut h = ts_vod(vec![Reply::Ok(psi_only)], hls_config());
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert!(t.fatals.is_empty(), "{t:?}");
    assert!(t.ended, "{t:?}");
    assert_eq!(h.hits(TS_SEG0_URL), 1, "no retry for valid media");
}

const LIVE_BASE: &str = "http://origin.test/hls";
const LIVE_BAD_URL: &str = "http://origin.test/hls/bad.ts";
const DEFAULT_SCHEDULE_SECS: [u64; 7] = [2, 4, 8, 10, 10, 10, 10];

fn live_host(bad: Vec<Reply>, config: Config) -> Host {
    let mut h = Host::new(TS_PLAYLIST_URL, config);
    h.ok(
        TS_PLAYLIST_URL,
        support::hls_host::live_playlist_text(0, &["index0.ts", "bad.ts", "index1.ts"])
            .into_bytes(),
    );
    h.ok(TS_SEG0_URL, ts_segment(0));
    h.route(LIVE_BAD_URL, bad);
    h.ok(TS_SEG1_URL, ts_segment(1));
    assert!(TS_SEG0_URL.starts_with(LIVE_BASE));
    h
}

fn played_past_the_gap(e: &[Event]) -> bool {
    let Some(gap) = e.iter().position(|x| matches!(x, Event::Discontinuity)) else {
        return false;
    };
    e[gap..].iter().any(|x| matches!(x, Event::Sample { .. }))
}

#[test]
fn live_permanent_segment_failure_is_skipped_with_one_error_then_a_discontinuity() {
    let cases: Vec<(&str, Reply, Option<u16>)> = vec![
        ("404", Reply::Status(404), Some(404)),
        ("503", Reply::Status(503), Some(503)),
        ("transport", Reply::TransportError, None),
        ("garbage", Reply::Ok(random_bytes()), None),
    ];
    for (name, reply, status) in cases {
        let mut h = live_host(vec![reply], hls_config());
        h.run_until(played_past_the_gap, STEPS);
        let t = tally(&h);
        assert!(t.fatals.is_empty(), "{name}: {t:?}");
        let kind = ErrorKind::SegmentFetchFailed { status };
        assert_eq!(t.non_fatals, vec![kind.clone()], "{name}: {t:?}");
        assert_eq!(t.discontinuities, 1, "{name}: {t:?}");
        let marks: Vec<&Event> = h
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Event::Error { .. } | Event::Discontinuity | Event::Sample { .. }
                )
            })
            .collect();
        let err_at = marks
            .iter()
            .position(|e| matches!(e, Event::Error { .. }))
            .unwrap();
        assert!(
            matches!(marks[err_at + 1], Event::Discontinuity),
            "{name}: the discontinuity follows the error directly"
        );
        assert!(
            marks[err_at + 2..]
                .iter()
                .any(|e| matches!(e, Event::Sample { .. })),
            "{name}: the next segment plays"
        );
        assert!(
            marks[..err_at]
                .iter()
                .any(|e| matches!(e, Event::Sample { .. })),
            "{name}: the segment before the gap played"
        );
        assert_eq!(
            h.hits(LIVE_BAD_URL),
            DEFAULT_SCHEDULE_SECS.len() + 1,
            "{name}: exactly the cap"
        );
    }
}

#[test]
fn live_failing_segment_walks_the_whole_schedule_across_playlist_reloads() {
    let mut h = live_host(vec![Reply::Status(404)], hls_config());
    h.run_until(played_past_the_gap, STEPS);
    assert!(h.hits(TS_PLAYLIST_URL) > 2, "the playlist kept reloading");
    assert_eq!(gaps_secs(&h, LIVE_BAD_URL), DEFAULT_SCHEDULE_SECS.to_vec());
}

fn with_cap(cap: u32) -> Config {
    {
        let mut new_config = hls_config();
        new_config.max_segment_failures = cap;
        new_config
    }
}

#[test]
fn vod_failing_segment_walks_the_schedule_without_reloading_the_playlist() {
    let mut h = ts_vod(vec![Reply::Status(500)], hls_config());
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert_eq!(
        t.fatals,
        vec![ErrorKind::SegmentFetchFailed { status: Some(500) }]
    );
    assert_eq!(gaps_secs(&h, TS_SEG0_URL), DEFAULT_SCHEDULE_SECS.to_vec());
    assert_eq!(
        h.hits(TS_PLAYLIST_URL),
        1,
        "VOD never refetches its playlist"
    );
}

#[test]
fn the_failure_count_is_per_segment_and_resets_only_on_a_delivery() {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.ok(TS_PLAYLIST_URL, ts_playlist());
    h.route(
        TS_SEG0_URL,
        vec![
            Reply::Status(500),
            Reply::Status(500),
            Reply::Status(500),
            Reply::Ok(ts_segment(0)),
        ],
    );
    h.route(
        TS_SEG1_URL,
        vec![Reply::Status(500), Reply::Ok(ts_segment(1))],
    );
    h.run_until(stop_on_end_or_fatal, STEPS);
    let t = tally(&h);
    assert!(t.ended && t.fatals.is_empty(), "{t:?}");
    assert_eq!(gaps_secs(&h, TS_SEG0_URL), vec![2, 4, 8]);
    let second_gap = gaps_secs(&h, TS_SEG1_URL);
    assert_eq!(
        second_gap.first(),
        Some(&2),
        "{second_gap:?}: its own first step"
    );
}

#[test]
fn the_cap_is_configurable_zero_is_first_failure_and_max_retries_on() {
    for (cap, requests) in [(0u32, 1usize), (1, 1), (3, 3)] {
        let mut h = ts_vod(vec![Reply::Status(500)], with_cap(cap));
        h.run_until(stop_on_end_or_fatal, STEPS);
        let t = tally(&h);
        assert_eq!(
            t.fatals,
            vec![ErrorKind::SegmentFetchFailed { status: Some(500) }],
            "cap {cap}"
        );
        assert_eq!(h.hits(TS_SEG0_URL), requests, "cap {cap}");
    }
    let mut h = ts_vod(vec![Reply::Status(500)], with_cap(u32::MAX));
    h.run_until(|_| false, 400);
    let t = tally(&h);
    assert!(t.fatals.is_empty() && !t.ended, "{t:?}");
    assert!(h.hits(TS_SEG0_URL) > DEFAULT_SCHEDULE_SECS.len() + 1);
}

#[test]
fn live_with_the_cap_at_zero_skips_on_the_first_failure() {
    let mut h = live_host(vec![Reply::Status(404)], with_cap(0));
    h.run_until(played_past_the_gap, STEPS);
    let t = tally(&h);
    assert_eq!(
        t.non_fatals,
        vec![ErrorKind::SegmentFetchFailed { status: Some(404) }]
    );
    assert_eq!(h.hits(LIVE_BAD_URL), 1);
}

#[test]
fn live_skipped_segment_is_never_requested_again() {
    let mut h = live_host(vec![Reply::Status(404)], hls_config());
    h.run_until(played_past_the_gap, STEPS);
    h.run_until(|_| false, 200);
    let t = tally(&h);
    assert_eq!(t.discontinuities, 1, "{t:?}");
    assert_eq!(
        t.non_fatals.len(),
        1,
        "one error per skipped segment: {t:?}"
    );
    assert_eq!(
        h.hits(LIVE_BAD_URL),
        DEFAULT_SCHEDULE_SECS.len() + 1,
        "never re-requested"
    );
}

const RETRY_AFTER: &str = "Retry-After";
const FIRST_BACKOFF_SECS: u64 = 2;
const MAX_RETRY_AFTER_SECS: u64 = 60;
const KEY_URL: &str = "http://origin.test/hls/key.bin";
const KEY: [u8; 16] = [0x11; 16];
const IV: [u8; 16] = [0x22; 16];
const INIT_TIMEOUT_SECS: u64 = 20;
const SEGMENT_TIMEOUT_SECS: u64 = 60;

fn after(status: u16, value: &str) -> Reply {
    Reply::StatusHeaders(status, vec![(RETRY_AFTER.to_string(), value.to_string())])
}

fn first_gap(h: &Host, url: &str) -> u64 {
    *gaps_secs(h, url)
        .first()
        .unwrap_or_else(|| panic!("{url} requested once: {:?}", h.requests_to(url)))
}

fn waits() -> Vec<(String, Reply, u64)> {
    let huge = u64::MAX.to_string();
    vec![
        ("429 30s".into(), after(429, "30"), 30),
        ("503 7s".into(), after(503, "7"), 7),
        ("padded".into(), after(429, " 9 "), 9),
        (
            "below the backoff step".into(),
            after(429, "1"),
            FIRST_BACKOFF_SECS,
        ),
        ("zero".into(), after(503, "0"), FIRST_BACKOFF_SECS),
        (
            "u64::MAX is clamped".into(),
            after(429, &huge),
            MAX_RETRY_AFTER_SECS,
        ),
        (
            "just over the clamp".into(),
            after(503, "61"),
            MAX_RETRY_AFTER_SECS,
        ),
        (
            "HTTP-date has no wall clock to count from".into(),
            after(429, "Wed, 21 Oct 2026 07:28:00 GMT"),
            FIRST_BACKOFF_SECS,
        ),
        ("negative".into(), after(429, "-5"), FIRST_BACKOFF_SECS),
        ("garbage".into(), after(503, "soon"), FIRST_BACKOFF_SECS),
        (
            "500 ignores it".into(),
            after(500, "30"),
            FIRST_BACKOFF_SECS,
        ),
        (
            "404 ignores it".into(),
            after(404, "30"),
            FIRST_BACKOFF_SECS,
        ),
    ]
}

fn keyed_playlist() -> Vec<u8> {
    let mut hex = String::new();
    for b in IV {
        hex.push_str(&format!("{b:02X}"));
    }
    format!(
        "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x{hex}\n\
         #EXTINF:2.0,\nindex0.ts\n#EXTINF:1.0,\nindex1.ts\n#EXT-X-ENDLIST\n"
    )
    .into_bytes()
}

fn playlist_retry_waits_for_retry_after() {
    for (name, reply, want) in waits() {
        let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
        h.route(TS_PLAYLIST_URL, vec![reply, Reply::Ok(ts_playlist())]);
        h.ok(TS_SEG0_URL, ts_segment(0));
        h.ok(TS_SEG1_URL, ts_segment(1));
        h.run_until(stop_on_end_or_fatal, STEPS);
        assert!(tally(&h).ended, "{name}");
        assert_eq!(first_gap(&h, TS_PLAYLIST_URL), want, "{name}");
    }
}

fn segment_retry_waits_for_retry_after() {
    for (name, reply, want) in waits() {
        let mut h = ts_vod(vec![reply, Reply::Ok(ts_segment(0))], hls_config());
        h.run_until(stop_on_end_or_fatal, STEPS);
        assert!(tally(&h).ended, "{name}");
        assert_eq!(first_gap(&h, TS_SEG0_URL), want, "{name}");
    }
}

fn init_retry_waits_for_retry_after() {
    let init = fmp4_parts().0;
    for (name, reply, want) in waits() {
        let mut h = fmp4_host(
            vec![reply, Reply::Ok(init.clone())],
            vec![Reply::Ok(fmp4_parts().1[0].clone())],
            hls_config(),
        );
        h.run_until(stop_on_end_or_fatal, STEPS);
        assert!(tally(&h).ended, "{name}");
        assert_eq!(first_gap(&h, FMP4_INIT_URL), want, "{name}");
    }
}

fn key_retry_waits_for_retry_after() {
    use transmux::sample_aes::aes128_encrypt_segment;
    for (name, reply, want) in waits() {
        let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
        h.ok(TS_PLAYLIST_URL, keyed_playlist());
        h.route(KEY_URL, vec![reply, Reply::Ok(KEY.to_vec())]);
        h.ok(
            TS_SEG0_URL,
            aes128_encrypt_segment(&KEY, &IV, &ts_segment(0)),
        );
        h.ok(
            TS_SEG1_URL,
            aes128_encrypt_segment(&KEY, &IV, &ts_segment(1)),
        );
        h.run_until(stop_on_end_or_fatal, STEPS);
        assert!(tally(&h).ended, "{name}: {:?}", tally(&h));
        assert_eq!(first_gap(&h, KEY_URL), want, "{name}");
    }
}

#[test]
fn retry_after_is_waited_for_by_every_resource_class() {
    playlist_retry_waits_for_retry_after();
    segment_retry_waits_for_retry_after();
    init_retry_waits_for_retry_after();
    key_retry_waits_for_retry_after();
}

#[test]
fn an_init_that_never_answers_times_out_on_the_playlist_class_timeout() {
    let mut config = hls_config();
    config.segment_inactivity_timeout_us = u64::MAX;
    let mut h = fmp4_host(vec![Reply::Hang], vec![Reply::Hang], config);
    h.run_until(stop_on_end_or_fatal, STEPS);
    assert_eq!(
        first_gap(&h, FMP4_INIT_URL),
        INIT_TIMEOUT_SECS + FIRST_BACKOFF_SECS
    );
    let t = tally(&h);
    assert_eq!(
        t.fatals,
        vec![ErrorKind::InitFetchFailed { status: None }],
        "{t:?}"
    );
    let cap = u64::from(broadcast_client::events::DEFAULT_MAX_KEY_FAILURES);
    assert!(
        h.now <= cap * (INIT_TIMEOUT_SECS + 10) * SEC,
        "fatal after {} s",
        h.now / SEC
    );
}

#[test]
fn a_segment_that_never_answers_keeps_the_segment_timeout() {
    let mut config = hls_config();
    config.segment_inactivity_timeout_us = u64::MAX;
    let mut h = ts_vod(vec![Reply::Hang, Reply::Ok(ts_segment(0))], config);
    h.run_until(stop_on_end_or_fatal, STEPS);
    assert!(tally(&h).ended);
    assert_eq!(
        first_gap(&h, TS_SEG0_URL),
        SEGMENT_TIMEOUT_SECS + FIRST_BACKOFF_SECS
    );
}

#[test]
fn a_delivery_clears_the_segments_failure_count_for_a_reused_media_sequence_number() {
    const RELOADS_BEFORE_RESTART: usize = 10;
    let window = |names: [&str; 3]| support::hls_host::live_playlist_text(0, &names).into_bytes();
    let mut playlists =
        vec![Reply::Ok(window(["a0.ts", "a1.ts", "a2.ts"])); RELOADS_BEFORE_RESTART];
    playlists.push(Reply::Ok(window(["b0.ts", "b1.ts", "b2.ts"])));
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.route(TS_PLAYLIST_URL, playlists);
    for name in ["a0", "a2", "b0", "b2"] {
        h.ok(&format!("{LIVE_BASE}/{name}.ts"), ts_segment(0));
    }
    h.route(
        &format!("{LIVE_BASE}/a1.ts"),
        vec![
            Reply::Status(500),
            Reply::Status(500),
            Reply::Status(500),
            Reply::Ok(ts_segment(1)),
        ],
    );
    let b1 = format!("{LIVE_BASE}/b1.ts");
    h.route(&b1, vec![Reply::Status(500), Reply::Ok(ts_segment(1))]);
    h.run_until(|_| false, 600);
    assert_eq!(gaps_secs(&h, &format!("{LIVE_BASE}/a1.ts")), vec![2, 4, 8]);
    assert_eq!(
        h.hits(&b1),
        2,
        "the restarted stream fetched its own segment 1"
    );
    assert_eq!(
        gaps_secs(&h, &b1),
        vec![2],
        "a fresh count, not the old one's fourth step"
    );
}

fn segment_faults() -> Vec<(&'static str, Reply, Option<u16>)> {
    let whole = ts_segment(0);
    let half = whole[..whole.len() / 2].to_vec();
    let short_headers = vec![("Content-Length".to_string(), whole.len().to_string())];
    vec![
        ("500", Reply::Status(500), Some(500)),
        ("503", Reply::Status(503), Some(503)),
        ("429", after(429, "3"), Some(429)),
        ("404", Reply::Status(404), Some(404)),
        ("403", Reply::Status(403), Some(403)),
        ("transport", Reply::TransportError, None),
        ("reset mid body", Reply::ResetMid(half.clone()), None),
        ("short body", Reply::OkHeaders(half, short_headers), None),
        ("no response", Reply::Hang, None),
        ("garbage", Reply::Ok(random_bytes()), None),
        ("html", Reply::Ok(HTML_ERROR_PAGE.to_vec()), None),
        ("empty", Reply::Ok(Vec::new()), None),
    ]
}

#[test]
fn every_permanent_segment_fault_ends_fatal_in_vod_and_skips_with_a_discontinuity_live() {
    for (name, reply, status) in segment_faults() {
        let mut vod = ts_vod(vec![reply.clone()], hls_config());
        vod.run_until(stop_on_end_or_fatal, STEPS);
        let t = tally(&vod);
        assert!(!t.ended, "vod {name}: {t:?}");
        assert_eq!(
            t.fatals,
            vec![ErrorKind::SegmentFetchFailed { status }],
            "vod {name}"
        );
        assert_eq!(
            vod.hits(TS_SEG0_URL),
            DEFAULT_SCHEDULE_SECS.len() + 1,
            "vod {name}"
        );

        let mut live = live_host(vec![reply], hls_config());
        live.run_until(played_past_the_gap, STEPS);
        let t = tally(&live);
        assert!(t.fatals.is_empty(), "live {name}: {t:?}");
        assert_eq!(
            t.non_fatals
                .iter()
                .filter(|k| matches!(k, ErrorKind::SegmentFetchFailed { .. }))
                .count(),
            1,
            "live {name}: {t:?}"
        );
        assert_eq!(t.discontinuities, 1, "live {name}");
        assert!(played_past_the_gap(&live.events), "live {name}");
    }
}

#[test]
fn every_transient_segment_fault_recovers_with_every_sample_and_no_error() {
    let want = ts_clean_samples();
    for (name, reply, _) in segment_faults() {
        let mut h = ts_vod(vec![reply, Reply::Ok(ts_segment(0))], hls_config());
        h.run_until(stop_on_end_or_fatal, STEPS);
        let t = tally(&h);
        assert!(t.ended && t.fatals.is_empty(), "{name}: {t:?}");
        assert_eq!(t.samples, want, "{name}");
        assert_eq!(h.hits(TS_SEG0_URL), 2, "{name}");
    }
}

const MID_PACKET_CUT: usize = 100;
const PARTIAL_LOSS_SAMPLES: usize = 2;

#[test]
fn a_mid_packet_cut_or_junk_led_ts_segment_still_plays() {
    let want = ts_clean_samples();
    let whole = ts_segment(0);
    let mut junk = vec![0xA5; 187];
    junk.extend_from_slice(&whole);
    for (name, body) in [("cut", whole[MID_PACKET_CUT..].to_vec()), ("junk", junk)] {
        let mut h = ts_vod(vec![Reply::Ok(body.clone())], hls_config());
        h.run_until(stop_on_end_or_fatal, STEPS);
        let t = tally(&h);
        assert!(
            t.ended && t.fatals.is_empty() && t.non_fatals.is_empty(),
            "{name}: {t:?}"
        );
        assert_eq!(h.hits(TS_SEG0_URL), 1, "{name}: no retry");
        assert!(
            want - t.samples <= PARTIAL_LOSS_SAMPLES,
            "{name}: {} of {want}",
            t.samples
        );

        let mut live = live_host(vec![Reply::Ok(body)], hls_config());
        live.run_until(
            |e| {
                e.iter()
                    .filter(|x| matches!(x, Event::Sample { .. }))
                    .count()
                    > want
            },
            STEPS,
        );
        let t = tally(&live);
        assert!(
            t.fatals.is_empty() && t.non_fatals.is_empty(),
            "live {name}: {t:?}"
        );
        assert_eq!(live.hits(LIVE_BAD_URL), 1, "live {name}: no retry");
    }
}
