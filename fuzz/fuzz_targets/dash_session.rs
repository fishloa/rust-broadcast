#![no_main]

use std::collections::BTreeMap;

use broadcast_client::{Action, Config, Credentials, DashClockPolicy, Event, Session, Status};
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 1 << 20;
const HEADER: usize = 8;
const STEP_BYTES: usize = 4;
const STEP_BUDGET: usize = 5_000;
const MAX_PUMP_ACTIONS: usize = 4_096;
const LIVE_CONNECTIONS_CAP: usize = 4_096;
const MICROS: u64 = 1_000_000;
const POOL: usize = 4;
const ORIGIN_PREFIX: &str = "http://example/";
const MPD_URL: &str = "http://example/dash/stream.mpd";
const AUDIT_HEADERS: [&str; 2] = ["authorization", "cookie"];
const WINDOWS: [usize; 4] = [1, 2, 4, 8];
const FAILURE_CAPS: [u32; 4] = [0, 1, 3, 8];
const RESYNC_US: [u64; 4] = [MICROS, 30 * MICROS, 600 * MICROS, u64::MAX];
const GRACE_US: [u64; 3] = [0, MICROS, 5 * MICROS];
const REFRESH_US: [u64; 4] = [1_000, 500_000, 2 * MICROS, 30 * MICROS];
const TIMEOUT_US: [u64; 4] = [1_000, 2 * MICROS, 20 * MICROS, 120 * MICROS];
const STEPS_US: [u64; 6] = [1_000, 50_000, 500_000, 2 * MICROS, 30 * MICROS, 700 * MICROS];
const CHUNK_BYTES: [usize; 4] = [1, 7, 1_316, usize::MAX];
const SMALL_BODY: usize = 2_048;
const MODES: u8 = 21;
const HOST_SKEW_MODE: u8 = 19;
const SKEW_BASE_US: i64 = 100 * MICROS as i64;
const DEFAULT_PUBLISH_US: i64 = 1_788_000_000 * MICROS as i64;
const HTTP_DATES: [&str; 3] = [
    "Mon, 26 Oct 2026 09:18:20 GMT",
    "Thu, 01 Jan 1970 00:00:00 GMT",
    "not a date",
];
const RETRY_AFTER: [&str; 5] = ["0", "3", "99999999999999999999", "Mon, 26 Oct 2026 09:18:30 GMT", "soon"];
const LOCATIONS: [&str; 3] = [
    "http://example/dash/moved.mpd",
    "http://other.example/dash/stream.mpd",
    "/dash/relative.mpd",
];

const MPDS: [&[u8]; 10] = [
    include_bytes!("../../fixtures/dash/live-number/live.mpd"),
    include_bytes!("../../fixtures/dash/live-timeline/live.mpd"),
    include_bytes!("../../fixtures/dash/livesim2/segtimeline_time_a.mpd"),
    include_bytes!("../../fixtures/dash/livesim2/segtimeline_time_b.mpd"),
    include_bytes!("../../fixtures/dash/livesim2/segtimelinenr_a.mpd"),
    include_bytes!("../../fixtures/dash/livesim2/segtimelinenr_b.mpd"),
    include_bytes!("../../fixtures/dash/manifest.mpd"),
    include_bytes!("../../fixtures/dash/multiperiod-restart/multiperiod.mpd"),
    include_bytes!("../../fixtures/dash/segmentlist/list.mpd"),
    include_bytes!("../../fixtures/dash/ondemand-sidx/ondemand.mpd"),
];
const VIDEO_SEGS: [&[u8]; POOL] = [
    include_bytes!("../../fixtures/dash/live-number/seg-0-005.m4s"),
    include_bytes!("../../fixtures/dash/live-number/seg-0-006.m4s"),
    include_bytes!("../../fixtures/dash/live-number/seg-0-007.m4s"),
    include_bytes!("../../fixtures/dash/live-number/seg-0-008.m4s"),
];
const AUDIO_SEGS: [&[u8]; POOL] = [
    include_bytes!("../../fixtures/dash/live-number/seg-1-005.m4s"),
    include_bytes!("../../fixtures/dash/live-number/seg-1-006.m4s"),
    include_bytes!("../../fixtures/dash/live-number/seg-1-007.m4s"),
    include_bytes!("../../fixtures/dash/live-number/seg-1-008.m4s"),
];
const VIDEO_INIT: &[u8] = include_bytes!("../../fixtures/dash/live-number/init-0.m4s");
const AUDIO_INIT: &[u8] = include_bytes!("../../fixtures/dash/live-number/init-1.m4s");

struct Request {
    method: String,
    url: String,
    byte_range: Option<(u64, u64)>,
    sent: u64,
}

#[derive(Default)]
struct Watch {
    live: BTreeMap<u32, Request>,
    fatal: bool,
    ended: bool,
    closed: bool,
    wake: Option<u64>,
}

impl Watch {
    fn pump(&mut self, session: &mut Session, now: u64) {
        let mut steps = 0usize;
        while let Some(action) = session.poll(now) {
            steps += 1;
            assert!(steps <= MAX_PUMP_ACTIONS, "runaway action loop at {now}");
            match action {
                Action::HttpRequest {
                    conn,
                    method,
                    url,
                    headers,
                    byte_range,
                } => {
                    assert!(!self.closed, "HttpRequest after close");
                    assert!(!self.fatal, "HttpRequest after a fatal error");
                    if !url.starts_with(ORIGIN_PREFIX) {
                        for (name, _) in &headers {
                            assert!(
                                !AUDIT_HEADERS.iter().any(|h| name.eq_ignore_ascii_case(h)),
                                "{name} sent to {url}"
                            );
                        }
                    }
                    let request = Request {
                        method,
                        url,
                        byte_range,
                        sent: now,
                    };
                    assert!(
                        self.live.insert(conn, request).is_none(),
                        "live conn id {conn} issued twice"
                    );
                    assert!(
                        self.live.len() < LIVE_CONNECTIONS_CAP,
                        "unbounded requests: {}",
                        self.census()
                    );
                }
                Action::Close { conn } => {
                    self.live.remove(&conn);
                }
                Action::WakeAt(at) => {
                    assert!(at > now, "WakeAt({at}) is not after now {now}");
                    self.wake = Some(self.wake.map_or(at, |w| w.min(at)));
                }
                _ => {}
            }
        }
        if self.wake.is_some_and(|w| w <= now) {
            self.wake = None;
        }
    }

    fn census(&self) -> String {
        let mut by_url: BTreeMap<&str, usize> = BTreeMap::new();
        for r in self.live.values() {
            *by_url.entry(r.url.as_str()).or_default() += 1;
        }
        by_url
            .iter()
            .take(8)
            .map(|(u, n)| format!("{n} x {u}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn events(&mut self, session: &mut Session) {
        while let Some(event) = session.next_event() {
            assert!(!self.closed, "event after close: {event:?}");
            if let Event::Sample { track, .. } = &event {
                assert!(!self.fatal, "a sample after a fatal error");
                assert!(!self.ended, "a sample after the stream ended");
                assert!(track.value() >= 1);
            }
            assert!(
                !(self.fatal && matches!(event, Event::Error { fatal: true, .. })),
                "two fatal errors"
            );
            match event {
                Event::Error { fatal: true, .. } => self.fatal = true,
                Event::Status(Status::Ended) => self.ended = true,
                _ => {}
            }
        }
    }
}

fn name_hash(name: &str) -> usize {
    name.bytes()
        .fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(usize::from(b)))
}

fn real_body(url: &str, range: Option<(u64, u64)>) -> Option<Vec<u8>> {
    let path = url.split('?').next()?;
    let name = path.rsplit('/').next()?;
    let audio = path.contains("-1-") || path.contains("A48") || path.contains("init-1");
    let all: &[u8] = if name.contains("init") {
        if audio { AUDIO_INIT } else { VIDEO_INIT }
    } else if audio {
        AUDIO_SEGS[name_hash(name) % POOL]
    } else {
        VIDEO_SEGS[name_hash(name) % POOL]
    };
    match range {
        None => Some(all.to_vec()),
        Some((off, len)) => {
            let start = usize::try_from(off).ok()?;
            let end = start.saturating_add(usize::try_from(len).ok()?);
            all.get(start..end.min(all.len())).map(<[u8]>::to_vec)
        }
    }
}

fn arbitrary(rest: &mut &[u8], aux: u8) -> Vec<u8> {
    let take = (usize::from(aux) * 8).min(rest.len());
    let (head, tail) = rest.split_at(take);
    *rest = tail;
    head.to_vec()
}

fn utc_body(now: u64, base_unix: i64, aux: u8, rest: &mut &[u8]) -> Vec<u8> {
    match aux % 5 {
        0 | 1 => {
            let unix = base_unix.saturating_add(i64::try_from(now).unwrap_or(0));
            let secs = unix.div_euclid(1_000_000);
            let ms = unix.rem_euclid(1_000_000) / 1_000;
            let days = secs.div_euclid(86_400);
            let tod = secs.rem_euclid(86_400);
            format!(
                "2026-10-{:02}T{:02}:{:02}:{:02}.{:03}Z",
                1 + days.rem_euclid(28),
                tod / 3_600,
                tod % 3_600 / 60,
                tod % 60,
                ms
            )
            .into_bytes()
        }
        2 => arbitrary(rest, aux),
        3 => Vec::new(),
        _ => vec![b'9'; 5_000],
    }
}

fn deliver(
    session: &mut Session,
    conn: u32,
    now: u64,
    status: u16,
    headers: &[(String, String)],
    body: &[u8],
    chunk: usize,
) {
    session.on_response(conn, status, headers, now);
    let chunk = if chunk == 1 && body.len() > SMALL_BODY {
        CHUNK_BYTES[2]
    } else {
        chunk
    };
    for piece in body.chunks(chunk.max(1)) {
        session.on_data(conn, piece, now);
    }
}

fn header(name: &str, value: &str) -> (String, String) {
    (name.to_owned(), value.to_owned())
}

#[allow(clippy::too_many_arguments)]
fn answer(
    session: &mut Session,
    conn: u32,
    req: &Request,
    now: u64,
    mode: u8,
    aux: u8,
    mpd: &[u8],
    publish: i64,
    rest: &mut &[u8],
) {
    let is_mpd = req.url.split('?').next().is_some_and(|p| p.ends_with(".mpd"));
    let is_utc = !is_mpd && !req.url.starts_with(ORIGIN_PREFIX);
    let real: Vec<u8> = if is_mpd {
        mpd.to_vec()
    } else if is_utc {
        utc_body(now, publish, aux, rest)
    } else {
        real_body(&req.url, req.byte_range).unwrap_or_default()
    };
    let chunk = CHUNK_BYTES[usize::from(aux >> 2) % CHUNK_BYTES.len()];
    let mut headers = vec![header("Content-Length", &real.len().to_string())];
    if req.method == "HEAD" || aux % 3 == 0 {
        headers.push(header("Date", HTTP_DATES[usize::from(aux) % HTTP_DATES.len()]));
    }
    match mode % MODES {
        0..=7 => {
            if req.method == "HEAD" {
                session.on_response(conn, 200, &headers, now);
            } else {
                deliver(session, conn, now, 200, &headers, &real, chunk);
            }
            session.on_closed(conn, None, now);
        }
        8 => {
            let cut = real.len().saturating_sub(usize::from(aux) + 1);
            deliver(session, conn, now, 200, &headers, &real[..cut], chunk);
            session.on_closed(conn, None, now);
        }
        9 => {
            let mut body = real.clone();
            if !body.is_empty() {
                let at = usize::from(aux) * 7 % body.len();
                body[at] ^= 0xFF;
            }
            deliver(session, conn, now, 200, &headers, &body, chunk);
            session.on_closed(conn, None, now);
        }
        10 => {
            let body = arbitrary(rest, aux);
            deliver(session, conn, now, 200, &[], &body, chunk);
            session.on_closed(conn, None, now);
        }
        11..=14 => {
            let status = [404, 410, 425, 503][usize::from(mode % MODES - 11)];
            let mut h = Vec::new();
            if aux % 2 == 0 {
                h.push(header("Retry-After", RETRY_AFTER[usize::from(aux / 2) % RETRY_AFTER.len()]));
            }
            deliver(session, conn, now, status, &h, b"nope", chunk);
            session.on_closed(conn, None, now);
        }
        15 => session.on_closed(conn, Some("reset".to_owned()), now),
        16 => {
            let half = real.len() / 2;
            deliver(session, conn, now, 200, &headers, &real[..half], chunk);
            session.on_closed(conn, Some("reset".to_owned()), now);
        }
        17 => {
            let status = if is_mpd { 304 } else { 200 };
            let h = [header("ETag", "\"x\"")];
            if status == 304 {
                session.on_response(conn, 304, &h, now);
            } else {
                deliver(session, conn, now, 200, &headers, &real, chunk);
            }
            session.on_closed(conn, None, now);
        }
        18 => {
            let alt = MPDS[usize::from(aux) % MPDS.len()];
            deliver(session, conn, now, 200, &[], alt, chunk);
            session.on_closed(conn, None, now);
        }
        19 => {
            let h = [header("Location", LOCATIONS[usize::from(aux) % LOCATIONS.len()])];
            session.on_response(conn, [301, 302, 307][usize::from(aux / 4) % 3], &h, now);
        }
        _ => {
            session.on_response(conn, 405, &[], now);
            session.on_closed(conn, None, now);
        }
    }
}

// Fuzz a DASH `Session` end to end: a fixture, damaged or arbitrary MPD (static, dynamic,
// multi-Period, sidx, list), MPD refreshes that swap in other fixtures, UTCTiming answers, host
// clock calls with skew, and a steered mix of real, damaged and failing answers delivered in
// 1-byte to whole-body chunks in arbitrary order, with redirects, Retry-After values, reconnects
// and variant switches in between. Must not panic, issue one connection id at most once while
// live, send no credentials off the origin, never run a wake before its time, stop after a fatal
// error and after the stream ended, and keep its request count bounded.
fuzz_target!(|data: &[u8]| {
    if data.len() < HEADER || data.len() > MAX_INPUT {
        return;
    }
    let (head, mut rest) = data.split_at(HEADER);
    let mut config = Config::default();
    config.max_inflight_segments = WINDOWS[usize::from(head[3] & 3)];
    config.max_segment_failures = FAILURE_CAPS[usize::from((head[3] >> 2) & 3)];
    config.max_key_failures = config.max_segment_failures;
    config.request_timeout_us = TIMEOUT_US[usize::from(head[6] & 3)];
    config.segment_timeout_us = TIMEOUT_US[usize::from((head[6] >> 2) & 3)];
    config.max_manifest_bytes = [4_096, 1 << 20, 1 << 20, 1 << 20][usize::from((head[6] >> 4) & 3)];
    config.initial_cookies = vec![("seed".to_owned(), "1".to_owned())];
    if head[3] & 0x10 != 0 {
        config.dash.clock_policy = DashClockPolicy::PreferHost;
    }
    config.dash.cross_origin_urls = head[3] & 0x20 == 0;
    config.dash.use_availability_time_offset = head[3] & 0x40 != 0;
    config.dash.utc_resync_us = RESYNC_US[usize::from(head[4] & 3)];
    config.dash.early_grace_us = GRACE_US[usize::from((head[4] >> 2) % 3)];
    config.dash.min_mpd_refresh_us = REFRESH_US[usize::from((head[4] >> 4) & 3)];

    let selector = usize::from(head[0]) % (MPDS.len() + 1);
    let mpd: Vec<u8> = if selector == 0 {
        let len = (usize::from(head[1]) << 8 | usize::from(head[2])) % (rest.len() + 1);
        let (m, tail) = rest.split_at(len);
        rest = tail;
        m.to_vec()
    } else {
        MPDS[selector - 1].to_vec()
    };
    let publish = dash_runtime::mpd::Mpd::parse(&mpd)
        .ok()
        .and_then(|m| m.publish_time_us)
        .unwrap_or(DEFAULT_PUBLISH_US);

    let Ok(mut session) = Session::open(MPD_URL, config, 0) else {
        return;
    };
    if head[7] & 1 != 0 {
        let _ = session.set_credentials(Credentials::new("user", "pass"));
    }
    let give_clock = |session: &mut Session, now: u64, skew: i64| {
        let unix = publish.saturating_add(i64::try_from(now).unwrap_or(0)).saturating_add(skew);
        session.set_wall_clock(now, u64::try_from(unix.max(0)).unwrap_or(0));
    };
    match head[5] % 4 {
        1 => give_clock(&mut session, 0, 0),
        2 => give_clock(&mut session, 0, 7 * MICROS as i64),
        3 => give_clock(&mut session, 0, -90 * MICROS as i64),
        _ => {}
    }

    let mut watch = Watch::default();
    let mut tail: &[u8] = rest;
    let mut now = 0u64;
    let steps: Vec<[u8; STEP_BYTES]> = rest
        .chunks_exact(STEP_BYTES)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect();
    for [mode, aux, jump, pick] in steps.into_iter().take(STEP_BUDGET) {
        watch.pump(&mut session, now);
        watch.events(&mut session);
        let previous = now;
        // A wake at u64::MAX is a saturated "never" (resync period u64::MAX); the broadcast_client
        // target already covers time at u64::MAX.
        now = if jump % 7 == 0 {
            watch.wake.filter(|w| *w != u64::MAX).map_or(now, |w| w.max(now))
        } else {
            now.saturating_add(STEPS_US[usize::from(jump) % STEPS_US.len()])
        };
        assert!(now >= previous, "time ran backwards");
        if mode % MODES == HOST_SKEW_MODE && aux % 4 == 0 {
            give_clock(&mut session, now, i64::from(aux) * MICROS as i64 - SKEW_BASE_US);
        }
        match pick % 16 {
            0 => {
                let _ = session.select_variant(usize::from(aux % 4));
            }
            1 => {
                let _ = session.select_audio_rendition(Some(usize::from(aux % 3)).filter(|_| aux % 5 != 0));
            }
            2 => session.reconnect(now),
            3 if aux == u8::MAX => {
                session.close();
                watch.closed = true;
                break;
            }
            _ => {}
        }
        if watch.live.is_empty() {
            continue;
        }
        let conns: Vec<u32> = watch.live.keys().copied().collect();
        let conn = conns[usize::from(aux) % conns.len()];
        let Some(req) = watch.live.remove(&conn) else {
            continue;
        };
        now = now.max(req.sent);
        answer(&mut session, conn, &req, now, mode, aux, &mpd, publish, &mut tail);
        watch.pump(&mut session, now);
        watch.events(&mut session);
    }
    if !watch.closed {
        session.close();
        watch.closed = true;
    }
    watch.live.clear();
    while let Some(action) = session.poll(now) {
        if let Action::HttpRequest { .. } = action {
            panic!("a request after close()");
        }
    }
    assert!(session.poll(now).is_none(), "action after a drained close()");
});
