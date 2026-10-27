//! Regression guard for the `dash_client_live` fuzz crash (bwg2): after a
//! terminal output the client must not emit another output, even when the host
//! clock is updated.
use std::collections::VecDeque;

use dash_runtime::client::{
    Action, DashClient, DashConfig, DashClockPolicy, Error, Failure, FailureClass, Output,
    PullLimits, ResourceId, ResponseMeta, UtcResponse,
};

const MAX_INPUT: usize = 1 << 20;
const HEADER: usize = 6;
const CHUNK: usize = 3;
const STEP_BUDGET: usize = 6_000;
const PENDING_CAP: usize = 4_096;
const WINDOW_MASK: u8 = 0b11;
const CAP_SHIFT: u32 = 2;
const CAP_MASK: u8 = 0b11;
const CAPS: [u32; 4] = [0, 1, 3, 8];
const PREFER_HOST: u8 = 0b1_0000;
const PIN_ORIGIN: u8 = 0b10_0000;
const USE_ATO: u8 = 0b100_0000;
const MODES: u8 = 20;
const POOL: usize = 4;
const MICROS: u64 = 1_000_000;
const MAX_DONE: usize = 4_096;
const BASE_URL: &str = "http://example/dash/";
const MPD_NAME: &str = "stream.mpd";
const STEPS_US: [u64; 6] = [1_000, 50_000, 500_000, 2 * MICROS, 30 * MICROS, 700 * MICROS];
const HTTP_DATES: [&str; 3] = [
    "Mon, 26 Oct 2026 09:18:20 GMT",
    "Thu, 01 Jan 1970 00:00:00 GMT",
    "not a date",
];

const MPDS: [&[u8]; 6] = [
    include_bytes!("../../fixtures/dash/live-number/live.mpd"),
    include_bytes!("../../fixtures/dash/live-timeline/live.mpd"),
    include_bytes!("../../fixtures/dash/livesim2/segtimeline_time_a.mpd"),
    include_bytes!("../../fixtures/dash/livesim2/segtimeline_time_b.mpd"),
    include_bytes!("../../fixtures/dash/livesim2/segtimelinenr_a.mpd"),
    include_bytes!("../../fixtures/dash/livesim2/segtimelinenr_b.mpd"),
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

fn name_hash(name: &str) -> usize {
    name.bytes().fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(usize::from(b)))
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

enum Answer {
    Body(Vec<u8>),
    NotModified,
    Fail(Failure),
}

fn arbitrary(rest: &mut &[u8], aux: u8) -> Vec<u8> {
    let take = (usize::from(aux) * 8).min(rest.len());
    let (head, tail) = rest.split_at(take);
    *rest = tail;
    head.to_vec()
}

fn answer(real: Option<Vec<u8>>, mode: u8, aux: u8, rest: &mut &[u8], alt: Option<&[u8]>) -> Answer {
    match mode % MODES {
        0..=7 => real.map_or(Answer::Fail(Failure::Status(404)), Answer::Body),
        8 => match real {
            Some(mut b) => {
                b.truncate(b.len().saturating_sub(usize::from(aux) + 1));
                Answer::Body(b)
            }
            None => Answer::Fail(Failure::Status(404)),
        },
        9 => match real {
            Some(mut b) if !b.is_empty() => {
                let at = usize::from(aux) * 7 % b.len();
                b[at] ^= 0xFF;
                Answer::Body(b)
            }
            _ => Answer::Fail(Failure::Status(404)),
        },
        10 => Answer::Body(arbitrary(rest, aux)),
        11 => Answer::Fail(Failure::Status(404)),
        12 => Answer::Fail(Failure::Status(410)),
        13 => Answer::Fail(Failure::Status(425)),
        14 => Answer::Fail(Failure::Status(503)),
        15 => Answer::Fail(Failure::Transport),
        16 => Answer::Fail(if aux % 2 == 0 { Failure::Timeout } else { Failure::TooLarge }),
        17 => Answer::NotModified,
        18 => alt.map_or(Answer::Fail(Failure::Status(500)), |b| Answer::Body(b.to_vec())),
        _ => Answer::Fail(Failure::Status(405)),
    }
}

fn utc_body(mode: u8, aux: u8, now: u64, base_unix: i64, rest: &mut &[u8]) -> Option<Vec<u8>> {
    match mode % 5 {
        0 | 1 => {
            let unix = base_unix.saturating_add(i64::try_from(now).unwrap_or(0));
            let secs = unix.div_euclid(1_000_000);
            let ms = unix.rem_euclid(1_000_000) / 1_000;
            let days = secs.div_euclid(86_400);
            let tod = secs.rem_euclid(86_400);
            Some(
                format!(
                    "2026-10-{:02}T{:02}:{:02}:{:02}.{:03}Z",
                    1 + days.rem_euclid(28),
                    tod / 3_600,
                    tod % 3_600 / 60,
                    tod % 60,
                    ms
                )
                .into_bytes(),
            )
        }
        2 => Some(arbitrary(rest, aux)),
        3 => Some(Vec::new()),
        _ => Some(vec![b'9'; 5_000]),
    }
}

#[test]
fn dash_client_live_crash_regression() {
    let data = test_fixtures::read("dash/fuzz/dash_client_live-crash.bin");
    assert!(data.len() >= HEADER && data.len() <= MAX_INPUT);
    let (head, mut rest) = data.split_at(HEADER);
    let mut limits = PullLimits::default();
    limits.max_inflight_segments = usize::from(head[0] & WINDOW_MASK) + 1;
    let cap = CAPS[usize::from((head[0] >> CAP_SHIFT) & CAP_MASK)];
    limits.max_segment_failures = cap;
    limits.max_init_failures = cap;
    let mut dash = DashConfig::default();
    if head[0] & PREFER_HOST != 0 {
        dash.clock_policy = DashClockPolicy::PreferHost;
    }
    dash.cross_origin_urls = head[0] & PIN_ORIGIN == 0;
    dash.use_availability_time_offset = head[0] & USE_ATO != 0;
    dash.utc_resync_us = [MICROS, 30 * MICROS, 600 * MICROS, u64::MAX][usize::from(head[4] & 3)];
    dash.early_grace_us = [0, MICROS, 5 * MICROS][usize::from((head[4] >> 2) % 3)];
    let window = limits.max_inflight_segments;

    let selector = usize::from(head[1]) % (MPDS.len() + 1);
    let mpd: Vec<u8> = if selector == 0 {
        let len = (usize::from(head[2]) << 8 | usize::from(head[3])) % (rest.len() + 1);
        let (m, tail) = rest.split_at(len);
        rest = tail;
        m.to_vec()
    } else {
        MPDS[selector - 1].to_vec()
    };
    let publish = dash_runtime::mpd::Mpd::parse(&mpd)
        .ok()
        .and_then(|m| m.publish_time_us)
        .unwrap_or(1_788_000_000 * 1_000_000);
    let host_mode = head[5] % 4;

    let url = format!("{BASE_URL}{MPD_NAME}");
    let mut client = DashClient::new(url, dash, limits);
    let mut pending: VecDeque<(Action, u64)> = VecDeque::new();
    let mut terminal = false;
    let mut chunks = rest
        .chunks_exact(CHUNK)
        .map(|c| (c[0], c[1], c[2]))
        .collect::<Vec<_>>();
    chunks.reverse();
    let mut tail_data: &[u8] = rest;
    let mut now = 0u64;

    let give_clock = |client: &mut DashClient, now: u64, skew: i64| {
        let unix = publish.saturating_add(i64::try_from(now).unwrap_or(0)).saturating_add(skew);
        client.set_wall_clock(now, u64::try_from(unix.max(0)).unwrap_or(0));
    };
    match host_mode {
        1 => give_clock(&mut client, now, 0),
        2 => give_clock(&mut client, now, 7 * 1_000_000),
        3 => give_clock(&mut client, now, -90 * 1_000_000),
        _ => {}
    }

    for _ in 0..STEP_BUDGET {
        while let Some(a) = client.poll(now) {
            assert!(!terminal, "an action after a terminal output");
            pending.push_back((a, now));
            assert!(pending.len() < PENDING_CAP, "unbounded requests");
        }
        pending.retain(|(a, _)| a.id().is_none_or(|id| client.awaits(id)));
        let segments = pending
            .iter()
            .filter(|(a, _)| matches!(a, Action::FetchSegment { .. }))
            .count();
        assert!(segments <= window, "{segments} segment requests, window {window}");
        while let Some(o) = client.next_output() {
            assert!(!terminal, "an output after a terminal output");
            match o {
                Output::EndOfStream | Output::Fatal(_) => terminal = true,
                Output::Samples { track_id, .. } => assert!(track_id.value() >= 1),
                _ => {}
            }
        }
        let st = client.state_size();
        assert!(st.done <= MAX_DONE, "{st:?}");
        let Some((mode, aux, jump)) = chunks.pop() else {
            break;
        };
        let wake = client.next_wake();
        let step = STEPS_US[usize::from(jump) % STEPS_US.len()];
        let previous = now;
        now = if jump % 7 == 0 {
            wake.map_or(now, |w| w.max(now))
        } else {
            now.saturating_add(step)
        };
        assert!(now >= previous, "time ran backwards");
        if mode % MODES == 19 && aux % 4 == 0 {
            give_clock(&mut client, now, i64::from(aux) * 1_000_000 - 100_000_000);
        }
        if pending.is_empty() {
            if terminal || (wake.is_none() && jump % 7 != 0) {
                break;
            }
            continue;
        }
        let idx = usize::from(aux) % pending.len();
        let Some((action, sent)) = pending.remove(idx) else {
            break;
        };
        let received = now.max(sent);
        let alt = MPDS[usize::from(aux) % MPDS.len()];
        match action {
            Action::FetchMpd { .. } => {
                let reply = answer(Some(mpd.clone()), mode, aux, &mut tail_data, Some(alt));
                let date = HTTP_DATES[usize::from(aux) % HTTP_DATES.len()];
                let mut meta = ResponseMeta::new(sent, received);
                if aux % 3 == 0 {
                    meta = meta.with_date(date).with_etag("\"x\"");
                }
                if aux % 11 == 0 {
                    meta = meta.with_final_url("http://example/dash/moved.mpd");
                }
                match reply {
                    Answer::Body(b) => {
                        if let Err(Error::Mpd(_)) = client.on_mpd_with(received, &b, &meta) {
                            client.retry_mpd(received);
                        }
                    }
                    Answer::NotModified => {
                        let _ = client.on_mpd_unchanged(received, &meta);
                    }
                    Answer::Fail(f) => {
                        client.on_mpd_failure(received, f);
                        client.retry_mpd(received);
                    }
                }
            }
            Action::FetchUtcTiming { id, .. } => {
                let reply = answer(None, mode, aux, &mut tail_data, None);
                match reply {
                    Answer::Fail(f) => {
                        let _ = client.on_utc_failure(received, id, f);
                    }
                    _ => {
                        let body = utc_body(mode, aux, received, publish, &mut tail_data)
                            .unwrap_or_default();
                        let mut resp = UtcResponse::new(sent, received, &body);
                        if aux % 2 == 0 {
                            resp = resp.with_date(HTTP_DATES[usize::from(aux / 2) % HTTP_DATES.len()]);
                        }
                        let _ = client.on_utc_response(received, id, &resp);
                    }
                }
            }
            other => {
                let Some(id) = other.id() else { continue };
                let real = real_body(other.url(), other.byte_range());
                match answer(real, mode, aux, &mut tail_data, None) {
                    Answer::Fail(f) => {
                        if let Ok(FailureClass::Retry { .. }) = client.on_failure(received, id, f) {
                            client.retry(received, id);
                        }
                    }
                    Answer::Body(b) => {
                        let r = match id {
                            ResourceId::Init { .. } => client.on_init(received, id, &b),
                            ResourceId::Index { .. } => client.on_index(received, id, &b),
                            _ => client.on_segment(received, id, &b),
                        };
                        if let Err(Error::Rejected { id, class, .. }) = r
                            && let FailureClass::Retry { .. } = class
                        {
                            client.retry(received, id);
                        }
                    }
                    Answer::NotModified => {}
                }
            }
        }
        if terminal {
            pending.clear();
        }
    }
}
