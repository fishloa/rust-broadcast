//! Round-5 findings F2/F3: a live source must start emitting before its input
//! ends, with no end-of-stream flush to lean on.

use std::collections::VecDeque;

use broadcast_client::{Action, Config, Event, Session, Status};

const TS_PACKET: usize = 188;
const SEGMENTS: usize = 3;
const TS_HTTP_CHUNK: usize = 64 * 1024;
const CHUNK_GAP_US: u64 = 10_000;
const TICK_US: u64 = 100_000;
const HLS_RUN_US: u64 = 12_000_000;
const SETTLE_TICKS: usize = 100;

#[derive(Default, Debug)]
struct Seen {
    tracks: usize,
    samples: usize,
    live: usize,
    reconnecting: usize,
    errors: Vec<String>,
}

impl Seen {
    fn drain(&mut self, s: &mut Session) {
        while let Some(e) = s.next_event() {
            match &e {
                Event::Tracks(_) => self.tracks += 1,
                Event::Sample { .. } => self.samples += 1,
                Event::Status(Status::Live) => self.live += 1,
                Event::Status(Status::Reconnecting { .. }) => self.reconnecting += 1,
                Event::Error { kind, .. }
                    if !matches!(kind, broadcast_client::ErrorKind::TrackDropped { .. }) =>
                {
                    self.errors.push(format!("{e:?}"))
                }
                _ => {}
            }
        }
    }

    fn assert_flowing(&self, what: &str) {
        assert!(self.tracks > 0, "{what}: no Tracks: {self:?}");
        assert!(self.samples > 0, "{what}: no Sample: {self:?}");
        assert!(self.errors.is_empty(), "{what}: errors: {self:?}");
        assert_eq!(self.reconnecting, 0, "{what}: reconnected: {self:?}");
    }
}

fn aligned(mut b: Vec<u8>) -> Vec<u8> {
    b.truncate(b.len() / TS_PACKET * TS_PACKET);
    b
}

fn ts_http_live(bytes: &[u8]) -> Seen {
    let mut s = Session::open("http://h/live.ts", Config::default(), 0).unwrap();
    let mut seen = Seen::default();
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
    for chunk in bytes.chunks(TS_HTTP_CHUNK) {
        now += CHUNK_GAP_US;
        s.on_data(c, chunk, now);
        while s.poll(now).is_some() {}
        seen.drain(&mut s);
    }
    seen
}

fn hls_live(bytes: &[u8]) -> Seen {
    let per = bytes.len() / TS_PACKET / SEGMENTS;
    let segs: Vec<Vec<u8>> = (0..SEGMENTS)
        .map(|i| bytes[i * per * TS_PACKET..(i + 1) * per * TS_PACKET].to_vec())
        .collect();
    let mut pl = String::from(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n",
    );
    for i in 0..SEGMENTS {
        pl.push_str(&format!("#EXTINF:3.5,\nseg{i}.ts\n"));
    }
    let mut s = Session::open("http://h/p/index.m3u8", Config::default(), 0).unwrap();
    let mut seen = Seen::default();
    let mut now = 0u64;
    while now <= HLS_RUN_US {
        let mut pending: VecDeque<(u32, String)> = VecDeque::new();
        loop {
            while let Some(a) = s.poll(now) {
                if let Action::HttpRequest { conn, url, .. } = a {
                    pending.push_back((conn, url));
                }
            }
            let Some((c, u)) = pending.pop_front() else {
                break;
            };
            let body: Vec<u8> = if u.ends_with(".m3u8") {
                pl.clone().into_bytes()
            } else {
                let i: usize = u
                    .trim_end_matches(".ts")
                    .rsplit("seg")
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                segs[i].clone()
            };
            s.on_response(
                c,
                200,
                &[("Content-Length".into(), body.len().to_string())],
                now,
            );
            for chunk in body.chunks(TS_HTTP_CHUNK) {
                s.on_data(c, chunk, now);
            }
            s.on_closed(c, None, now);
            seen.drain(&mut s);
        }
        seen.drain(&mut s);
        now += TICK_US;
    }
    for _ in 0..SETTLE_TICKS {
        seen.drain(&mut s);
    }
    seen
}

fn hls_case(bytes: Vec<u8>, what: &str) {
    let seen = hls_live(&aligned(bytes));
    seen.assert_flowing(what);
    assert!(seen.live > 0, "{what}: never Live: {seen:?}");
}

#[test]
fn hls_live_m6_single_emits_without_endlist() {
    hls_case(test_fixtures::read("ts/m6-single.ts"), "m6-single");
}

#[test]
fn hls_live_isi6_emits_without_endlist() {
    hls_case(
        test_fixtures::read("dvb-si/tnt-5w-12732v-isi6-10s.ts"),
        "isi6-10s",
    );
}

#[test]
fn hls_live_csa2_plain_emits_without_endlist() {
    let Some(b) = test_fixtures::require_bytes("csa/csa2-plain.ts", "hls_live_csa2") else {
        return;
    };
    hls_case(b, "csa2-plain");
}

#[test]
fn ts_http_live_csa2_plain_emits_before_the_stream_ends() {
    let Some(b) = test_fixtures::require_bytes("csa/csa2-plain.ts", "ts_http_live_csa2") else {
        return;
    };
    ts_http_live(&aligned(b)).assert_flowing("csa2-plain");
}

#[test]
fn ts_http_live_test_062_emits_before_the_stream_ends() {
    let Some(p) = test_fixtures::tsduck("input/test-062.ts", "ts_http_live_062") else {
        return;
    };
    ts_http_live(&aligned(std::fs::read(p).unwrap())).assert_flowing("test-062");
}
