//! Read-ahead memory bound for a segment-pull source (issue #4086): a VOD
//! presentation thousands of segments long must play through a host that reports
//! its playback position in bounded memory, whatever its length.
//!
//! The whole test thread runs under a live-byte budget
//! (`test_alloc::ThreadCapped`): the presentation is generated segment by segment
//! and is many times larger than the budget, so a session that fetched it whole
//! would abort instead of exhausting the host. The same budget is used at both
//! presentation lengths.

#![cfg(target_pointer_width = "64")]

use broadcast_client::{Action, Config, ConnId, Event, ReadAhead, Session, SourceKind, Status};
use broadcast_common::clock33::{Clock27, PCR_HZ};
use broadcast_common::stage::Timestamp;
use test_alloc::ThreadCapped;
use test_harness::big_media::TsSegmentStream;
use url::Url;

#[global_allocator]
static ALLOC: ThreadCapped<BUDGET> = ThreadCapped::new();

/// Live bytes the whole play loop may hold, whatever the presentation length:
/// the byte cap plus the session's own buffers, never the presentation.
const BUDGET: usize = 16 << 20;
/// The read-ahead byte cap.
const BYTE_CAP: u64 = 128 * 1024;
/// One segment: 334 TS packets, about two seconds of the 40 s fixture.
const SEGMENT_BYTES: usize = 188 * 334;
/// Playback advance per host step, in 27 MHz ticks and in the host clock.
const POSITION_STEP: u64 = PCR_HZ / 2;
const STEP_US: u64 = 10_000;
/// The two presentation lengths: `[segments]`.
const LENGTHS: [u64; 2] = [1000, 2000];
/// A loop that cannot reach `Ended` is broken, not slow.
const MAX_STEPS: usize = 400_000;
const FIXTURE: &str = "ts/h264-aac-40s.ts";
const PLAYLIST_URL: &str = "http://example.invalid/vod/index.m3u8";
const SEGMENT_URL: &str = "http://example.invalid/vod/seg";

fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

fn clock(ticks: u64) -> Clock27 {
    Clock27::from_27mhz_wrapping(ticks)
}

fn playlist(segments: u64) -> Vec<u8> {
    let mut text = String::from(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n",
    );
    for n in 0..segments {
        text.push_str(&format!("#EXTINF:2.000,\n{SEGMENT_URL}{n}.ts\n"));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    text.into_bytes()
}

fn config() -> Config {
    let mut config = Config::default();
    config.kind = SourceKind::Hls;
    config.ts_http = config.ts_http.with_read_ahead(ReadAhead::bytes(BYTE_CAP));
    config
}

fn headers(len: usize) -> http::HeaderMap {
    let mut map = http::HeaderMap::new();
    map.insert(
        http::header::CONTENT_LENGTH,
        http::HeaderValue::from_str(&len.to_string()).expect("a test writes a valid length"),
    );
    map
}

fn respond(session: &mut Session, conn: ConnId, body: &[u8], now: u64) {
    session.on_response(conn, http::StatusCode::OK, &headers(body.len()), ts(now));
    session.on_data(conn, body, ts(now));
    session.on_closed(conn, None, ts(now));
}

/// Play a generated presentation of `segments` segments through the session,
/// returning the samples delivered.
fn play(segments: u64) -> u64 {
    let path = test_fixtures::path(FIXTURE);
    let mut stream = TsSegmentStream::new(&path, segments, SEGMENT_BYTES);
    let playlist_body = playlist(segments);
    let mut session = Session::open(
        Url::parse(PLAYLIST_URL).expect("a test names a URL"),
        config(),
        ts(0),
    )
    .expect("open an HLS session");
    let mut now_us = 0u64;
    let mut position = 0u64;
    let mut samples = 0u64;
    let mut ended = false;

    for _ in 0..MAX_STEPS {
        while let Some(action) = session.poll(ts(now_us)) {
            let Action::HttpRequest { conn, url, .. } = action else {
                continue;
            };
            if url.as_str() == PLAYLIST_URL {
                respond(&mut session, conn, &playlist_body, now_us);
            } else if let Some(body) = stream.next_segment() {
                respond(&mut session, conn, &body, now_us);
            }
        }
        while let Some(event) = session.next_event() {
            match event {
                Event::Sample { .. } => samples = samples.saturating_add(1),
                Event::Status(Status::Ended) => ended = true,
                _ => {}
            }
        }
        position = position.saturating_add(POSITION_STEP);
        session.on_playback_position(ts(now_us), Some(clock(position)));
        now_us = now_us.saturating_add(STEP_US);
        if ended {
            break;
        }
    }
    assert!(
        ended,
        "a {segments}-segment presentation never reached Ended within {MAX_STEPS} steps"
    );
    assert!(samples > 0, "the presentation delivered no sample");
    samples
}

#[test]
fn a_long_generated_vod_plays_in_bounded_memory_at_two_lengths() {
    for segments in LENGTHS {
        let presentation = segments * u64::try_from(SEGMENT_BYTES).expect("a segment length");
        assert!(
            presentation > 3 * u64::try_from(BUDGET).expect("a byte budget"),
            "a {presentation}-byte presentation does not outgrow the {BUDGET}-byte budget"
        );
    }
    let short = play(LENGTHS[0]);
    let long = play(LENGTHS[1]);
    assert!(
        long > short,
        "the longer presentation delivered no more samples ({short} then {long})"
    );
}
