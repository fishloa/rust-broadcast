#![no_main]

use std::collections::VecDeque;

use broadcast_common::stage::Timestamp;
use dash_runtime::client::{Action, DashClient, DashConfig, Error, Failure, FailureClass, Output};
use libfuzzer_sys::fuzz_target;
use test_fixtures::fuzz::{
    MAX_INPUT, flip_byte, pick_mpd, slice_range, take_arbitrary, truncate_tail,
};

#[path = "../../dash-runtime/tests/it/common/fuzz_pull.rs"]
mod pull;
use pull::{BASE_URL, MPD_NAME, PENDING_CAP, deliver_resource, pull_limits};

const HEADER: usize = 4;
const CHUNK: usize = 2;
const STEP_BUDGET: usize = 4_000;
const PIN_ORIGIN: u8 = 0b1_0000;
const MODES: u8 = 12;

const MPDS: [&[u8]; 2] = [
    include_bytes!("../../fixtures/dash/manifest.mpd"),
    include_bytes!("../../fixtures/dash/ondemand-sidx/ondemand.mpd"),
];
const FILES: [(&str, &[u8]); 11] = [
    (
        "init-stream0.m4s",
        include_bytes!("../../fixtures/dash/init-stream0.m4s"),
    ),
    (
        "init-stream1.m4s",
        include_bytes!("../../fixtures/dash/init-stream1.m4s"),
    ),
    (
        "chunk-stream0-00001.m4s",
        include_bytes!("../../fixtures/dash/chunk-stream0-00001.m4s"),
    ),
    (
        "chunk-stream0-00002.m4s",
        include_bytes!("../../fixtures/dash/chunk-stream0-00002.m4s"),
    ),
    (
        "chunk-stream0-00003.m4s",
        include_bytes!("../../fixtures/dash/chunk-stream0-00003.m4s"),
    ),
    (
        "chunk-stream1-00001.m4s",
        include_bytes!("../../fixtures/dash/chunk-stream1-00001.m4s"),
    ),
    (
        "chunk-stream1-00002.m4s",
        include_bytes!("../../fixtures/dash/chunk-stream1-00002.m4s"),
    ),
    (
        "chunk-stream1-00003.m4s",
        include_bytes!("../../fixtures/dash/chunk-stream1-00003.m4s"),
    ),
    (
        "chunk-stream1-00004.m4s",
        include_bytes!("../../fixtures/dash/chunk-stream1-00004.m4s"),
    ),
    (
        "v_dashinit.mp4",
        include_bytes!("../../fixtures/dash/ondemand-sidx/v_dashinit.mp4"),
    ),
    (
        "a_dashinit.mp4",
        include_bytes!("../../fixtures/dash/ondemand-sidx/a_dashinit.mp4"),
    ),
];

fn real_body(url: &str, range: Option<(u64, u64)>) -> Option<Vec<u8>> {
    let name = url.split('?').next()?.rsplit('/').next()?;
    let (_, all) = FILES.iter().find(|(n, _)| *n == name)?;
    slice_range(all, range)
}

enum Answer {
    Body(Vec<u8>),
    Fail(Failure),
}

fn answer(real: Option<Vec<u8>>, mode: u8, aux: u8, rest: &mut &[u8]) -> Answer {
    match mode % MODES {
        0..=4 => real.map_or(Answer::Fail(Failure::Status(404)), Answer::Body),
        5 => match real {
            Some(mut b) => {
                truncate_tail(&mut b, aux);
                Answer::Body(b)
            }
            None => Answer::Fail(Failure::Status(404)),
        },
        6 => match real {
            Some(mut b) if !b.is_empty() => {
                flip_byte(&mut b, aux);
                Answer::Body(b)
            }
            _ => Answer::Fail(Failure::Status(404)),
        },
        7 => Answer::Body(take_arbitrary(rest, aux)),
        8 => Answer::Fail(Failure::Status(404)),
        9 => Answer::Fail(Failure::Status(503)),
        10 => Answer::Fail(Failure::Transport),
        _ => Answer::Fail(if aux.is_multiple_of(2) {
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
    let cfg = pull_limits(head[0]);
    let mut dash = DashConfig::default();
    dash.cross_origin_urls = head[0] & PIN_ORIGIN == 0;
    let window = cfg.max_inflight_segments;

    let mpd = pick_mpd(&MPDS, head[1], head[2], head[3], &mut rest);

    let url = format!("{BASE_URL}{MPD_NAME}");
    let mut client = DashClient::new(dash_runtime::client::DashClientConfig::new(url).with_dash(dash).with_limits(cfg));
    let mut pending: VecDeque<Action> = VecDeque::new();
    let mut terminal = false;
    let mut chunks = rest
        .chunks_exact(CHUNK)
        .map(|c| (c[0], c[1]))
        .collect::<Vec<_>>();
    chunks.reverse();
    let mut tail_data: &[u8] = rest;
    let mut now_us = 0u64;

    for _ in 0..STEP_BUDGET {
        while let Some(a) = client.poll(Timestamp::from_micros(now_us)) {
            assert!(!terminal, "an action after a terminal output");
            pending.push_back(a);
            assert!(pending.len() < PENDING_CAP, "unbounded requests");
        }
        let segments = pending
            .iter()
            .filter(|a| matches!(a, Action::FetchSegment { .. }))
            .count();
        assert!(
            segments <= window,
            "{segments} segment requests, window {window}"
        );
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
        now_us += 1_000;
        let real = match action.id() {
            None => Some(mpd.clone()),
            Some(_) => real_body(action.url(), action.byte_range()),
        };
        let reply = answer(real, mode, aux, &mut tail_data);
        match (action.id(), reply) {
            (None, Answer::Body(b)) => {
                if let Err(Error::Mpd(_)) = client.on_mpd(Timestamp::from_micros(now_us), &b) {
                    client.retry_mpd(Timestamp::from_micros(now_us));
                }
            }
            (None, Answer::Fail(f)) => {
                client.on_mpd_failure(Timestamp::from_micros(now_us), f);
                client.retry_mpd(Timestamp::from_micros(now_us));
            }
            (Some(id), Answer::Fail(f)) => {
                if let Ok(FailureClass::Retry { .. }) = client.on_failure(Timestamp::from_micros(now_us), id, f) {
                    client.retry(Timestamp::from_micros(now_us), id);
                }
            }
            (Some(id), Answer::Body(b)) => {
                deliver_resource(&mut client, now_us, id, &b);
            }
        }
        if terminal {
            pending.clear();
        }
    }
});
