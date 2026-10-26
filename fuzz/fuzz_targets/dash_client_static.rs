#![no_main]

use std::collections::VecDeque;

use dash_runtime::client::{
    Action, DashClient, DashConfig, Error, Failure, FailureClass, Output, ResourceId,
};
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 1 << 20;
const HEADER: usize = 4;
const CHUNK: usize = 2;
const STEP_BUDGET: usize = 4_000;
const PENDING_CAP: usize = 4_096;
const WINDOW_MASK: u8 = 0b11;
const CAP_SHIFT: u32 = 2;
const CAP_MASK: u8 = 0b11;
const CAPS: [u32; 4] = [0, 1, 3, 8];
const PIN_ORIGIN: u8 = 0b1_0000;
const MODES: u8 = 12;
const BASE_URL: &str = "http://example/dash/";
const MPD_NAME: &str = "stream.mpd";

const MPDS: [&[u8]; 2] = [
    include_bytes!("../../fixtures/dash/manifest.mpd"),
    include_bytes!("../../fixtures/dash/ondemand-sidx/ondemand.mpd"),
];
const FILES: [(&str, &[u8]); 11] = [
    ("init-stream0.m4s", include_bytes!("../../fixtures/dash/init-stream0.m4s")),
    ("init-stream1.m4s", include_bytes!("../../fixtures/dash/init-stream1.m4s")),
    ("chunk-stream0-00001.m4s", include_bytes!("../../fixtures/dash/chunk-stream0-00001.m4s")),
    ("chunk-stream0-00002.m4s", include_bytes!("../../fixtures/dash/chunk-stream0-00002.m4s")),
    ("chunk-stream0-00003.m4s", include_bytes!("../../fixtures/dash/chunk-stream0-00003.m4s")),
    ("chunk-stream1-00001.m4s", include_bytes!("../../fixtures/dash/chunk-stream1-00001.m4s")),
    ("chunk-stream1-00002.m4s", include_bytes!("../../fixtures/dash/chunk-stream1-00002.m4s")),
    ("chunk-stream1-00003.m4s", include_bytes!("../../fixtures/dash/chunk-stream1-00003.m4s")),
    ("chunk-stream1-00004.m4s", include_bytes!("../../fixtures/dash/chunk-stream1-00004.m4s")),
    ("v_dashinit.mp4", include_bytes!("../../fixtures/dash/ondemand-sidx/v_dashinit.mp4")),
    ("a_dashinit.mp4", include_bytes!("../../fixtures/dash/ondemand-sidx/a_dashinit.mp4")),
];

fn real_body(url: &str, range: Option<(u64, u64)>) -> Option<Vec<u8>> {
    let name = url.split('?').next()?.rsplit('/').next()?;
    let (_, all) = FILES.iter().find(|(n, _)| *n == name)?;
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
    Fail(Failure),
}

fn answer(action: &Action, mode: u8, aux: u8, rest: &mut &[u8]) -> Answer {
    let real = real_body(action.url(), action.byte_range());
    match mode % MODES {
        0..=4 => real.map_or(Answer::Fail(Failure::Status(404)), Answer::Body),
        5 => match real {
            Some(mut b) => {
                b.truncate(b.len().saturating_sub(usize::from(aux) + 1));
                Answer::Body(b)
            }
            None => Answer::Fail(Failure::Status(404)),
        },
        6 => match real {
            Some(mut b) if !b.is_empty() => {
                let at = usize::from(aux) * 7 % b.len();
                b[at] ^= 0xFF;
                Answer::Body(b)
            }
            _ => Answer::Fail(Failure::Status(404)),
        },
        7 => {
            let take = (usize::from(aux) * 8).min(rest.len());
            let (head, tail) = rest.split_at(take);
            *rest = tail;
            Answer::Body(head.to_vec())
        }
        8 => Answer::Fail(Failure::Status(404)),
        9 => Answer::Fail(Failure::Status(503)),
        10 => Answer::Fail(Failure::Transport),
        _ => Answer::Fail(if aux % 2 == 0 {
            Failure::Timeout
        } else {
            Failure::TooLarge
        }),
    }
}

// Fuzz the whole static client: an MPD (a real fixture, or the fuzz bytes
// themselves) and a steered sequence of answers, real bodies, damaged bodies,
// arbitrary bytes and failures, completed in arbitrary order. Must not panic,
// never exceed its request window, stop producing output after a terminal
// event, and never request without bound.
fuzz_target!(|data: &[u8]| {
    if data.len() < HEADER || data.len() > MAX_INPUT {
        return;
    }
    let (head, mut rest) = data.split_at(HEADER);
    let mut cfg = DashConfig::default();
    cfg.max_inflight_segments = usize::from(head[0] & WINDOW_MASK) + 1;
    let cap = CAPS[usize::from((head[0] >> CAP_SHIFT) & CAP_MASK)];
    cfg.max_segment_failures = cap;
    cfg.max_init_failures = cap;
    cfg.cross_origin_urls = head[0] & PIN_ORIGIN == 0;
    let window = cfg.max_inflight_segments;

    let selector = usize::from(head[1]) % (MPDS.len() + 1);
    let mpd: Vec<u8> = if selector == 0 {
        let len = (usize::from(head[2]) << 8 | usize::from(head[3])) % (rest.len() + 1);
        let (m, tail) = rest.split_at(len);
        rest = tail;
        m.to_vec()
    } else {
        MPDS[selector - 1].to_vec()
    };

    let url = format!("{BASE_URL}{MPD_NAME}");
    let mut client = DashClient::new(url, cfg);
    let mut pending: VecDeque<Action> = VecDeque::new();
    let mut terminal = false;
    let mut chunks = rest.chunks_exact(CHUNK).map(|c| (c[0], c[1])).collect::<Vec<_>>();
    chunks.reverse();
    let mut tail_data: &[u8] = rest;
    let mut now = 0u64;

    for _ in 0..STEP_BUDGET {
        while let Some(a) = client.poll(now) {
            assert!(!terminal, "an action after a terminal output");
            pending.push_back(a);
            assert!(pending.len() < PENDING_CAP, "unbounded requests");
        }
        let segments = pending
            .iter()
            .filter(|a| matches!(a, Action::FetchSegment { .. }))
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
        let Some((mode, aux)) = chunks.pop() else {
            break;
        };
        if pending.is_empty() {
            break;
        }
        let idx = usize::from(aux) % pending.len();
        let Some(action) = pending.remove(idx) else {
            break;
        };
        now += 1_000;
        let reply = answer(&action, mode, aux, &mut tail_data);
        match (action.id(), reply) {
            (None, Answer::Body(b)) => {
                if let Err(Error::Mpd(_)) = client.on_mpd(now, &b) {
                    client.retry_mpd(now);
                }
            }
            (None, Answer::Fail(f)) => {
                client.on_mpd_failure(now, f);
                client.retry_mpd(now);
            }
            (Some(id), Answer::Fail(f)) => {
                if let Ok(FailureClass::Retry { .. }) = client.on_failure(now, id, f) {
                    client.retry(now, id);
                }
            }
            (Some(id), Answer::Body(b)) => {
                let r = match id {
                    ResourceId::Init { .. } => client.on_init(now, id, &b),
                    ResourceId::Index { .. } => client.on_index(now, id, &b),
                    _ => client.on_segment(now, id, &b),
                };
                if let Err(Error::Rejected { id, class, .. }) = r
                    && let FailureClass::Retry { .. } = class
                {
                    client.retry(now, id);
                }
            }
        }
        if terminal {
            pending.clear();
        }
    }
});
