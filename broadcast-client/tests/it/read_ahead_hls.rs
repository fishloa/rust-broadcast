//! Read-ahead gate for the HLS source (issue #4086): a VOD presentation stops
//! being fetched once the media already held runs the configured cap ahead of
//! the playback position, and fetching resumes at half the cap. The default,
//! [`ReadAhead::Unlimited`], makes the same requests it made before the gate
//! existed.

use crate::support::hls_host::{
    Host, Reply, TS_PLAYLIST_URL, TS_SEG0_URL, TS_SEG1_URL, ts_playlist, ts_segment,
};
use crate::support::read_ahead_pace::{
    Media, assert_gate_held, clock, drive, requests, segment_count,
};
use broadcast_client::{Config, ReadAhead, SourceKind};
use broadcast_common::clock33::PCR_HZ;
use broadcast_common::stage::Timestamp;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const ORIGIN: &str = "http://origin.test/vod";
const LIVE_ORIGIN: &str = "http://origin.test/live";
/// One segment of the generated presentation: 167 TS packets, about a second
/// of the 40 s fixture.
const SLICE_BYTES: usize = 188 * 167;
const TARGET_DURATION_SECS: u64 = 2;
const LIVE_WINDOW: u64 = 3;

fn segment_url(origin: &str, n: u64) -> String {
    format!("{origin}/seg{n}.ts")
}

fn playlist_url(origin: &str) -> String {
    format!("{origin}/index.m3u8")
}

/// The 40 s TS fixture cut into continuing slices: a presentation of about 40
/// segments, each about a second of media, whose timeline runs straight through
/// them.
fn generated_presentation() -> Vec<Vec<u8>> {
    test_fixtures::read("ts/h264-aac-40s.ts")
        .chunks(SLICE_BYTES)
        .map(<[u8]>::to_vec)
        .collect()
}

fn vod_playlist(count: u64) -> Vec<u8> {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{TARGET_DURATION_SECS}\n\
         #EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n"
    );
    for n in 0..count {
        text.push_str(&format!("#EXTINF:1.000,\nseg{n}.ts\n"));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    text.into_bytes()
}

fn live_playlist(first_sequence: u64, count: u64) -> Vec<u8> {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{TARGET_DURATION_SECS}\n\
         #EXT-X-MEDIA-SEQUENCE:{first_sequence}\n"
    );
    for n in first_sequence..first_sequence + count {
        text.push_str(&format!("#EXTINF:1.000,\n{LIVE_ORIGIN}/seg{n}.ts\n"));
    }
    text.into_bytes()
}

fn config(read_ahead: ReadAhead, kind: SourceKind) -> Config {
    let mut config = Config::default();
    config.kind = kind;
    config.max_inflight_segments = 1;
    config.ts_http = config.ts_http.with_read_ahead(read_ahead);
    config
}

fn seconds(n: u64) -> u64 {
    n * PCR_HZ
}

struct Vod {
    host: Host,
    per_segment: u64,
    playlist_bytes: u64,
}

/// A host serving the generated VOD presentation.
fn vod_host(read_ahead: ReadAhead) -> Vod {
    let segments = generated_presentation();
    let playlist = vod_playlist(u64::try_from(segments.len()).unwrap_or(0));
    let mut host = Host::new(&playlist_url(ORIGIN), config(read_ahead, SourceKind::Hls));
    host.ok(&playlist_url(ORIGIN), playlist.clone());
    for (n, body) in segments.iter().enumerate() {
        host.ok(
            &segment_url(ORIGIN, u64::try_from(n).unwrap_or(0)),
            body.clone(),
        );
    }
    Vod {
        host,
        per_segment: u64::try_from(segments.first().map_or(0, Vec::len)).unwrap_or(0),
        playlist_bytes: u64::try_from(playlist.len()).unwrap_or(0),
    }
}

#[test]
fn a_vod_presentation_stops_fetching_at_the_time_cap() {
    let cap = seconds(10);
    let Vod { mut host, .. } = vod_host(ReadAhead::time(core::time::Duration::from_secs(10)));
    let mut media = Media::default();

    let log = drive(&mut host, &mut media, 400, |_| Some(clock(0)));
    assert_gate_held(&log, cap);
    let fetched = segment_count(&requests(&log));
    assert!(fetched >= 2, "only {fetched} segments were fetched");
    assert!(
        media.end_ticks() >= cap,
        "the presentation stopped before the cap: {} ticks delivered",
        media.end_ticks()
    );
    assert!(
        log.last().is_some_and(|step| step.requests.is_empty()),
        "the host never ran dry: the gate did not close"
    );

    let before = host.seen.len();
    let stalled = drive(&mut host, &mut media, 50, |media| {
        Some(clock(media.position_behind(cap)))
    });
    assert!(
        requests(&stalled).is_empty(),
        "a lead still at the cap resumed fetching: {:?}",
        requests(&stalled)
    );
    assert_eq!(host.seen.len(), before, "a request appeared while held");

    let anchor = media.end_ticks();
    assert!(
        anchor >= cap,
        "the pause point is under the cap: {anchor} ticks"
    );
    let resumed = drive(&mut host, &mut media, 400, move |_| Some(clock(anchor)));
    assert_gate_held(&resumed, cap);
    let refetched = segment_count(&requests(&resumed));
    assert!(
        refetched > 0,
        "a lead under half the cap did not resume fetching"
    );
    assert!(
        resumed.last().is_some_and(|step| step.requests.is_empty()),
        "the resumed run never stopped again"
    );
    let total = usize::try_from(fetched + refetched).unwrap_or(usize::MAX);
    assert!(
        total < generated_presentation().len(),
        "the presentation ended before the gate closed again"
    );
}

#[test]
fn a_vod_presentation_stops_fetching_at_the_byte_cap() {
    let cap = 128 * 1024u64;
    let Vod {
        mut host,
        per_segment,
        playlist_bytes,
    } = vod_host(ReadAhead::bytes(cap));
    let mut media = Media::default();

    let log = drive(&mut host, &mut media, 400, |_| Some(clock(0)));
    let fetched = segment_count(&requests(&log));
    assert!(fetched >= 2, "only {fetched} segments were fetched");
    assert!(media.samples > 0, "the fetched segments delivered no media");
    let fed = playlist_bytes + fetched * per_segment;
    assert!(fed >= cap, "the byte cap was never reached: {fed} bytes");
    assert!(
        fed <= cap + per_segment,
        "{fed} bytes were fetched, over the {cap}-byte cap plus one {per_segment}-byte segment"
    );
    assert!(
        log.last().is_some_and(|step| step.requests.is_empty()),
        "the host never ran dry: the gate did not close"
    );

    let anchor = media.end_ticks();
    let resumed = drive(&mut host, &mut media, 400, move |_| Some(clock(anchor)));
    let refetched = segment_count(&requests(&resumed));
    assert!(
        refetched > 0,
        "consuming every delivered byte did not resume fetching"
    );
    assert!(
        resumed.last().is_some_and(|step| step.requests.is_empty()),
        "the resumed byte-capped run never stopped again"
    );
    let refetched_bytes = refetched * per_segment;
    assert!(
        refetched_bytes <= cap + per_segment,
        "{refetched_bytes} bytes were re-fetched, over the {cap}-byte cap plus one segment"
    );
}

/// The request sequence the committed two-segment VOD fixture produced before
/// the read-ahead gate existed for HLS (pinned by
/// `hls::media_playlist_ts_segments_emit_tracks_then_samples`).
const VOD_REQUESTS: [&str; 3] = [TS_PLAYLIST_URL, TS_SEG0_URL, TS_SEG1_URL];

#[test]
fn an_unlimited_session_fetches_the_committed_vod_presentation_as_before() {
    let mut host = Host::new(
        TS_PLAYLIST_URL,
        config(ReadAhead::Unlimited, SourceKind::Hls),
    );
    host.ok(TS_PLAYLIST_URL, ts_playlist());
    host.ok(TS_SEG0_URL, ts_segment(0));
    host.ok(TS_SEG1_URL, ts_segment(1));
    host.run_to_end();
    assert_eq!(
        host.seen
            .iter()
            .map(|seen| seen.url.as_str())
            .collect::<Vec<_>>(),
        VOD_REQUESTS
    );
}

/// A live presentation driven at 1x for the same number of host steps under two
/// read-ahead settings.
fn live_run(read_ahead: ReadAhead, steps: usize) -> Vec<String> {
    let segments = generated_presentation();
    let mut host = Host::new(
        &playlist_url(LIVE_ORIGIN),
        config(read_ahead, SourceKind::Hls),
    );
    host.route(
        &playlist_url(LIVE_ORIGIN),
        vec![Reply::Ok(live_playlist(0, LIVE_WINDOW))],
    );
    for n in 0..LIVE_WINDOW {
        let body = segments
            .get(usize::try_from(n).unwrap_or(0))
            .cloned()
            .unwrap_or_default();
        host.ok(&segment_url(LIVE_ORIGIN, n), body);
    }
    let mut media = Media::default();
    for _ in 0..steps {
        host.collect();
        let position = media.end_ticks().saturating_sub(PCR_HZ);
        host.session
            .on_playback_position(ts(host.now), Some(clock(position)));
        host.collect();
        for event in core::mem::take(&mut host.events) {
            media.observe(&event);
        }
        host.step();
        for event in core::mem::take(&mut host.events) {
            media.observe(&event);
        }
    }
    host.seen.iter().map(|seen| seen.url.clone()).collect()
}

#[test]
fn a_live_presentation_with_a_cap_over_its_window_makes_the_same_requests() {
    let unlimited = live_run(ReadAhead::Unlimited, 80);
    let capped = live_run(ReadAhead::time(core::time::Duration::from_secs(30)), 80);
    assert!(
        unlimited.iter().any(|url| url.contains("/seg")),
        "the live run fetched no segment"
    );
    assert!(
        unlimited
            .iter()
            .filter(|url| url.ends_with(".m3u8"))
            .count()
            > 1,
        "the live run never reloaded its playlist"
    );
    assert_eq!(capped, unlimited);
}
