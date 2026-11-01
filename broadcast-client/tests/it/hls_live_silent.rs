use crate::support;

use std::collections::VecDeque;

use broadcast_client::{Action, Config, Event, Session, Status};

const INDEX: &str = "http://h/index.m3u8";
const FIXTURE: &str = "zenith-http/tf1-live.ts";
const REAL_SHAPE_SEGMENTS: usize = 3;
const LONG_LIVED_SEGMENTS: usize = 5;
const TS_PACKET: usize = 188;
const STEP_US: u64 = 50_000;
const END_US: u64 = 8_000_000;
const CHUNK: usize = 64 * 1024;

fn playlist(first_seq: u64, count: usize) -> String {
    let mut s = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:{first_seq}\n"
    );
    for n in 0..count {
        s.push_str(&format!(
            "#EXTINF:3.5,\nhttp://h/seg{}.ts\n",
            first_seq + n as u64
        ));
    }
    s
}

fn segments(total: usize) -> Vec<Vec<u8>> {
    let Some(bytes) = test_fixtures::require_bytes(FIXTURE, "hls_live_silent") else {
        eprintln!(
            "hls_live_silent: SKIPPED - the rest of this test binary needs the private capture"
        );
        std::process::exit(0)
    };
    let packets = bytes.len() / TS_PACKET;
    let per = packets / total;
    (0..total)
        .map(|i| bytes[i * per * TS_PACKET..(i + 1) * per * TS_PACKET].to_vec())
        .collect()
}

#[derive(Default)]
struct Tally {
    tracks: usize,
    samples: usize,
    live: usize,
    reconnecting: usize,
    urls: Vec<String>,
    log: Vec<String>,
}

impl Tally {
    fn drain(&mut self, session: &mut Session) {
        while let Some(e) = session.next_event() {
            match &e {
                Event::Tracks(_) => self.tracks += 1,
                Event::Sample { .. } => self.samples += 1,
                Event::Status(Status::Live) => self.live += 1,
                Event::Status(Status::Reconnecting { .. }) => self.reconnecting += 1,
                _ => {}
            }
            if matches!(e, Event::Status(_)) {
                self.log.push(format!("{e:?}"));
            }
        }
    }
}

struct Origin {
    playlist: String,
    segs: Vec<(String, Vec<u8>)>,
    content_length: bool,
}

impl Origin {
    fn body(&self, url: &str) -> (Vec<u8>, &'static str) {
        if url.ends_with(".m3u8") {
            return (
                self.playlist.clone().into_bytes(),
                "application/vnd.apple.mpegurl",
            );
        }
        let seg = self
            .segs
            .iter()
            .find(|(u, _)| u == url)
            .unwrap_or_else(|| panic!("unexpected url {url}"));
        (seg.1.clone(), "video/MP2T")
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Order {
    InPoll,
    OneAtATime,
    PlaylistLast,
}

fn answer(session: &mut Session, origin: &Origin, conn: u32, url: &str, now: u64, t: &mut Tally) {
    let (body, ctype) = origin.body(url);
    let mut headers = vec![("Content-Type".to_string(), ctype.to_string())];
    if origin.content_length {
        headers.push(("Content-Length".to_string(), body.len().to_string()));
    }
    session.on_response(conn, 200, &headers, now);
    for chunk in body.chunks(CHUNK) {
        session.on_data(conn, chunk, now);
    }
    session.on_closed(conn, None, now);
    t.drain(session);
}

fn run(mut origin: Origin, order: Order, mut on_tick: impl FnMut(u64, &mut Origin)) -> Tally {
    let mut session = Session::open(INDEX, Config::default(), 0).unwrap();
    let mut t = Tally::default();
    let mut now = 0;
    while now <= END_US {
        on_tick(now, &mut origin);
        let mut pending: VecDeque<(u32, String)> = VecDeque::new();
        loop {
            while let Some(a) = session.poll(now) {
                if let Action::HttpRequest { conn, url, .. } = a {
                    t.urls.push(url.clone());
                    pending.push_back((conn, url));
                }
            }
            if pending.is_empty() {
                break;
            }
            let mut batch: Vec<_> = pending.drain(..).collect();
            if order == Order::PlaylistLast {
                batch.sort_by_key(|(_, u)| u.ends_with(".m3u8"));
            }
            let take = if order == Order::OneAtATime {
                1
            } else {
                batch.len()
            };
            for (c, u) in batch.drain(..take) {
                answer(&mut session, &origin, c, &u, now, &mut t);
            }
            pending.extend(batch);
        }
        t.drain(&mut session);
        now += STEP_US;
    }
    t
}

fn origin(first_seq: u64, count: usize, content_length: bool) -> Origin {
    origin_with(first_seq, count, REAL_SHAPE_SEGMENTS, content_length)
}

fn origin_with(first_seq: u64, count: usize, total: usize, content_length: bool) -> Origin {
    let segs = segments(total);
    Origin {
        playlist: playlist(first_seq, count),
        segs: segs
            .into_iter()
            .enumerate()
            .map(|(i, b)| (format!("http://h/seg{}.ts", 12 + i), b))
            .collect(),
        content_length,
    }
}

fn assert_flowing(t: &Tally) {
    assert!(t.tracks > 0, "no Tracks; log {:#?}", t.log);
    assert!(t.samples > 0, "no Samples; log {:#?}", t.log);
    assert!(t.live > 0, "never Live; log {:#?}", t.log);
    assert_eq!(t.reconnecting, 0, "reconnected; log {:#?}", t.log);
}

#[test]
fn live_hls_without_endlist_emits_tracks_and_samples() {
    let t = run(origin(12, 3, true), Order::InPoll, |_, _| {});
    assert_flowing(&t);
}

#[test]
fn live_hls_emits_when_responses_arrive_one_at_a_time() {
    let t = run(origin(12, 3, true), Order::OneAtATime, |_, _| {});
    assert_flowing(&t);
}

#[test]
fn live_hls_emits_when_playlist_is_answered_last() {
    let t = run(origin(12, 3, true), Order::PlaylistLast, |_, _| {});
    assert_flowing(&t);
}

#[test]
fn live_hls_emits_without_content_length() {
    let t = run(origin(12, 3, false), Order::InPoll, |_, _| {});
    assert_flowing(&t);
}

#[test]
fn live_hls_keeps_flowing_across_playlist_reloads() {
    let mut grown = 0;
    let t = run(
        origin_with(12, 3, LONG_LIVED_SEGMENTS, true),
        Order::InPoll,
        |now, o| {
            let want = match now {
                0..=499_999 => 0,
                500_000..=999_999 => 1,
                _ => 2,
            };
            if want > grown {
                grown = want;
                o.playlist = playlist(12 + grown as u64, 3);
            }
        },
    );
    assert_flowing(&t);
    for n in 12..12 + LONG_LIVED_SEGMENTS {
        let url = format!("http://h/seg{n}.ts");
        assert!(t.urls.contains(&url), "{url} never fetched: {:?}", t.urls);
    }
}
