use std::collections::VecDeque;
use std::time::Duration;

use broadcast_common::stage::Timestamp;

use dash_runtime::client::{
    Action, DashClient, DashClockPolicy, DashConfig, Error, Failure, FailureClass, Output,
    ResponseMeta, UtcResponse,
};
use test_fixtures::fuzz::{
    DEFAULT_PUBLISH_US, DashWorld, GRACE_US, HTTP_DATES, MAX_INPUT, PIN_ORIGIN, PREFER_HOST,
    RESYNC_US, STEPS_US, USE_ATO, flip_byte, pick_mpd, take_arbitrary, truncate_tail, utc_body,
};

#[path = "fuzz_pull.rs"]
mod pull;
use pull::{BASE_URL, MPD_NAME, PENDING_CAP, deliver_resource, pull_limits};

const HEADER: usize = 6;
const CHUNK: usize = 3;
const STEP_BUDGET: usize = 6_000;
const MODES: u8 = 20;
const MAX_DONE: usize = 4_096;

enum Answer {
    Body(Vec<u8>),
    NotModified,
    Fail(Failure),
}

fn answer(
    real: Option<Vec<u8>>,
    mode: u8,
    aux: u8,
    rest: &mut &[u8],
    alt: Option<&[u8]>,
) -> Answer {
    match mode % MODES {
        0..=7 => real.map_or(Answer::Fail(Failure::Status(404)), Answer::Body),
        8 => match real {
            Some(mut b) => {
                truncate_tail(&mut b, aux);
                Answer::Body(b)
            }
            None => Answer::Fail(Failure::Status(404)),
        },
        9 => match real {
            Some(mut b) if !b.is_empty() => {
                flip_byte(&mut b, aux);
                Answer::Body(b)
            }
            _ => Answer::Fail(Failure::Status(404)),
        },
        10 => Answer::Body(take_arbitrary(rest, aux)),
        11 => Answer::Fail(Failure::Status(404)),
        12 => Answer::Fail(Failure::Status(410)),
        13 => Answer::Fail(Failure::Status(425)),
        14 => Answer::Fail(Failure::Status(503)),
        15 => Answer::Fail(Failure::Transport),
        16 => Answer::Fail(if aux.is_multiple_of(2) {
            Failure::Timeout
        } else {
            Failure::TooLarge
        }),
        17 => Answer::NotModified,
        18 => alt.map_or(Answer::Fail(Failure::Status(500)), |b| {
            Answer::Body(b.to_vec())
        }),
        _ => Answer::Fail(Failure::Status(405)),
    }
}

/// Fuzz the live client: a dynamic MPD (a real fixture, a damaged one, or the
/// fuzz bytes), MPD refreshes that swap in other fixtures, `UTCTiming` answers,
/// host clock calls, time jumps and a steered mix of real, damaged and failing
/// segment answers completed in arbitrary order. Must not panic, never exceed
/// its request window, never run time backwards, stop after a terminal event
/// and keep its bookkeeping bounded.
/// Returns `false` when `data` is outside the accepted size range.
pub fn run(data: &[u8], load: &dyn Fn(&str) -> Vec<u8>) -> bool {
    let world = DashWorld::get_or_load(load, &[]);
    if data.len() < HEADER || data.len() > MAX_INPUT {
        return false;
    }
    let (head, mut rest) = data.split_at(HEADER);
    let limits = pull_limits(head[0]);
    let mut dash = DashConfig::default();
    if head[0] & PREFER_HOST != 0 {
        dash.clock_policy = DashClockPolicy::PreferHost;
    }
    dash.cross_origin_urls = head[0] & PIN_ORIGIN == 0;
    dash.use_availability_time_offset = head[0] & USE_ATO != 0;
    dash.utc_resync = (RESYNC_US[usize::from(head[4] & 3)] != u64::MAX)
        .then(|| Duration::from_micros(RESYNC_US[usize::from(head[4] & 3)]));
    dash.early_grace = Duration::from_micros(GRACE_US[usize::from((head[4] >> 2) % 3)]);
    let window = limits.max_inflight_segments;

    let mpd = pick_mpd(&world.mpds, head[1], head[2], head[3], &mut rest);
    let publish = dash_runtime::mpd::Mpd::parse(&mpd)
        .ok()
        .and_then(|m| m.publish_time_us)
        .unwrap_or(DEFAULT_PUBLISH_US);
    let host_mode = head[5] % 4;

    let url = format!("{BASE_URL}{MPD_NAME}");
    let mut client = DashClient::new(
        dash_runtime::client::DashClientConfig::new(url::Url::parse(&url).expect("URL"))
            .with_dash(dash)
            .with_limits(limits),
    );
    let mut pending: VecDeque<(Action, u64)> = VecDeque::new();
    let mut terminal = false;
    let mut chunks = rest
        .chunks_exact(CHUNK)
        .map(|c| (c[0], c[1], c[2]))
        .collect::<Vec<_>>();
    chunks.reverse();
    let mut tail_data: &[u8] = rest;
    let mut now_us = 0u64;

    let give_clock = |client: &mut DashClient, now_us: u64, skew: i64| {
        let unix = publish
            .saturating_add(i64::try_from(now_us).unwrap_or(0))
            .saturating_add(skew);
        let read = unix.max(0);
        client.set_wall_clock(
            Timestamp::from_micros(now_us),
            jiff::Timestamp::from_microsecond(read).unwrap_or(jiff::Timestamp::UNIX_EPOCH),
        );
    };
    match host_mode {
        1 => give_clock(&mut client, now_us, 0),
        2 => give_clock(&mut client, now_us, 7 * 1_000_000),
        3 => give_clock(&mut client, now_us, -90 * 1_000_000),
        _ => {}
    }

    for _ in 0..STEP_BUDGET {
        while let Some(a) = client.poll(broadcast_common::stage::Timestamp::from_micros(now_us)) {
            assert!(!terminal, "an action after a terminal output");
            pending.push_back((a, now_us));
            assert!(pending.len() < PENDING_CAP, "unbounded requests");
        }
        pending.retain(|(a, _)| a.id().is_none_or(|id| client.awaits(id)));
        let segments = pending
            .iter()
            .filter(|(a, _)| matches!(a, Action::FetchSegment { .. }))
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
        let st = client.state_size();
        assert!(st.done <= MAX_DONE, "{st:?}");
        let Some((mode, aux, jump)) = chunks.pop() else {
            break;
        };
        let wake = client.next_wake();
        let step = STEPS_US[usize::from(jump) % STEPS_US.len()];
        let previous = now_us;
        now_us = if jump.is_multiple_of(7) {
            wake.map_or(now_us, |w| w.as_micros().max(now_us))
        } else {
            now_us.saturating_add(step)
        };
        assert!(now_us >= previous, "time ran backwards");
        if mode % MODES == 19 && aux.is_multiple_of(4) {
            give_clock(
                &mut client,
                now_us,
                i64::from(aux) * 1_000_000 - 100_000_000,
            );
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
        let received = now_us.max(sent);
        let sent = Timestamp::from_micros(sent);
        let received = Timestamp::from_micros(received);
        let alt = world.mpds[usize::from(aux) % world.mpds.len()].as_slice();
        match action {
            Action::FetchMpd { .. } => {
                let reply = answer(Some(mpd.clone()), mode, aux, &mut tail_data, Some(alt));
                let date = HTTP_DATES[usize::from(aux) % HTTP_DATES.len()];
                let mut meta = ResponseMeta::new(sent, received);
                if aux.is_multiple_of(3) {
                    meta = meta.with_date(date).with_etag("\"x\"");
                }
                if aux.is_multiple_of(11) {
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
                        let body =
                            utc_body(mode, aux, received.as_micros(), publish, &mut tail_data);
                        let mut resp = UtcResponse::new(sent, received, &body);
                        if aux.is_multiple_of(2) {
                            resp =
                                resp.with_date(HTTP_DATES[usize::from(aux / 2) % HTTP_DATES.len()]);
                        }
                        let _ = client.on_utc_response(received, id, &resp);
                    }
                }
            }
            other => {
                let Some(id) = other.id() else { continue };
                let real = world.real_body(
                    other.url().as_str(),
                    other.byte_range().map(|s| (s.offset, s.len.get())),
                );
                match answer(real, mode, aux, &mut tail_data, None) {
                    Answer::Fail(f) => {
                        if let Ok(FailureClass::Retry { .. }) = client.on_failure(received, id, f) {
                            client.retry(received, id);
                        }
                    }
                    Answer::Body(b) => deliver_resource(&mut client, received, id, &b),
                    Answer::NotModified => {}
                }
            }
        }
        if terminal {
            pending.clear();
        }
    }
    true
}
