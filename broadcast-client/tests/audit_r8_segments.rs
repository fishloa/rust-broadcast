//! Round-8 findings F1 + F2: a 200 answer that is not media for its container is a failed fetch
//! (never a silently consumed segment), and a segment that keeps failing walks the retry schedule
//! and ends fatal (VOD) or is skipped with a discontinuity (live).

mod support;
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
    let (init, segs) = fmp4_parts();
    let playlist = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-PLAYLIST-TYPE:VOD\n\
                    #EXT-X-MAP:URI=\"i.mp4\"\n#EXTINF:1.0,\ns0.m4s\n#EXTINF:1.0,\ns1.m4s\n\
                    #EXT-X-ENDLIST\n";
    let mut h = Host::new(FMP4_PLAYLIST_URL, config);
    h.ok(FMP4_PLAYLIST_URL, playlist.as_bytes().to_vec());
    h.ok(FMP4_INIT_URL, init);
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
