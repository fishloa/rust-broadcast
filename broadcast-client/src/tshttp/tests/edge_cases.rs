//! TS-over-HTTP edge cases the revived suite did not pin: an identity-coded
//! empty `Content-Encoding`, duplicate/conflicting lengths, timers on a body
//! that declares more bytes than it has delivered, and bytes fed to a session
//! that is not accepting a body.

use std::sync::LazyLock;

use super::{TsHttpClient, TsHttpPolicy};
use crate::tshttp::TsHttpConfig as Config;
use crate::tshttp::{Action, Event};
use broadcast_common::stage::Timestamp;

/// `ms` on the tshttp policy's millisecond clock, as the session's `now`.
fn ts(ms: u64) -> Timestamp {
    Timestamp::from_millis(ms)
}

static D: LazyLock<Vec<u8>> = LazyLock::new(|| test_fixtures::read("ts/h264-aac.ts"));

fn hdr(name: &str, value: &str) -> Vec<(String, String)> {
    vec![(name.to_string(), value.to_string())]
}

fn finite_after(status: u16, headers: &[(String, String)]) -> Option<u64> {
    let mut p = TsHttpPolicy::new("u", Config::default(), ts(0));
    let _ = p.poll();
    p.on_response(status, headers, ts(0));
    std::iter::from_fn(|| p.poll_event()).find_map(|e| match e {
        Event::Finite { expected_bytes } => Some(expected_bytes),
        _ => None,
    })
}

fn events(c: &mut TsHttpClient) -> Vec<Event> {
    std::iter::from_fn(|| c.poll_event()).collect()
}

#[test]
fn an_empty_content_encoding_is_identity() {
    let mut headers = hdr("Content-Length", "100");
    headers.push(("Content-Encoding".to_string(), String::new()));
    assert_eq!(finite_after(200, &headers), Some(100));
}

#[test]
fn conflicting_content_lengths_are_not_finite() {
    let headers = vec![
        ("Content-Length".to_string(), "100".to_string()),
        ("Content-Length".to_string(), "200".to_string()),
    ];
    assert_eq!(finite_after(200, &headers), None);
}

#[test]
fn identical_content_lengths_are_finite() {
    let headers = vec![
        ("Content-Length".to_string(), "100".to_string()),
        ("Content-Length".to_string(), "100".to_string()),
    ];
    assert_eq!(finite_after(200, &headers), Some(100));
}

#[test]
fn a_declared_finite_body_keeps_its_start_timeout() {
    let mut c = TsHttpClient::with_seed("u", Config::default(), ts(0), ts(0).as_millis());
    let _ = c.poll();
    c.on_response(200, &hdr("Content-Length", "1000000"), ts(0));
    assert_eq!(c.next_deadline(), Some(ts(15_000)));
    c.poll_timeout(ts(15_000));
    let actions: Vec<Action> = std::iter::from_fn(|| c.poll()).collect();
    assert!(actions.contains(&Action::Abort), "{actions:?}");
}

#[test]
fn a_partly_received_finite_body_still_stalls() {
    let mut c = TsHttpClient::with_seed("u", Config::default(), ts(0), ts(0).as_millis());
    let _ = c.poll();
    c.on_response(200, &hdr("Content-Length", "1000000"), ts(0));
    c.feed(D.as_slice(), ts(0));
    c.poll_timeout(ts(30_000));
    let actions: Vec<Action> = std::iter::from_fn(|| c.poll()).collect();
    assert!(actions.contains(&Action::Abort), "{actions:?}");
}

#[test]
fn feeding_after_ended_emits_nothing() {
    let mut c = TsHttpClient::with_seed("u", Config::default(), ts(0), ts(0).as_millis());
    let _ = c.poll();
    c.on_response(200, &hdr("Content-Length", &D.len().to_string()), ts(0));
    for chunk in D.chunks(1316) {
        c.feed(chunk, ts(0));
    }
    c.on_eof(ts(1));
    let _ = events(&mut c);
    c.feed(D.as_slice(), ts(2));
    c.on_error(ts(3));
    c.on_eof(ts(3));
    assert!(events(&mut c).is_empty());
    assert!(c.poll().is_none());
}

#[test]
fn feeding_while_backing_off_emits_nothing() {
    let mut c = TsHttpClient::with_seed("u", Config::default(), ts(0), ts(0).as_millis());
    let _ = c.poll();
    c.on_error(ts(0));
    let _ = events(&mut c);
    while c.poll().is_some() {}
    let deadline = c.next_deadline();
    c.feed(D.as_slice(), ts(1));
    assert!(events(&mut c).is_empty());
    assert_eq!(c.next_deadline(), deadline);
}

/// The shortest fixture prefix that emits no sample until the demuxer is
/// finished: the state a stall has to flush before it drops the connection.
fn held_prefix() -> &'static [u8] {
    for n in 1..=D.len() {
        let prefix = &D[..n];
        let mut d = transmux::StreamingTsDemux::new();
        let _ = d.feed(prefix);
        let mut before = 0;
        while let Some(e) = d.poll_event() {
            if matches!(e, transmux::DemuxEvent::Sample { .. }) {
                before += 1;
            }
        }
        if before != 0 {
            continue;
        }
        let _ = d.finish();
        let mut after = 0;
        while let Some(e) = d.poll_event() {
            if matches!(e, transmux::DemuxEvent::Sample { .. }) {
                after += 1;
            }
        }
        if after >= 1 {
            return prefix;
        }
    }
    panic!("no fixture prefix holds a sample until finish");
}

#[test]
fn stall_reconnect_flushes_held_samples_before_aborting() {
    let mut c = TsHttpClient::with_seed("u", Config::default(), ts(0), ts(0).as_millis());
    let _ = c.poll();
    c.on_response(200, &[], ts(0));
    c.feed(held_prefix(), ts(0));
    c.poll_timeout(ts(30_000));
    let flushed = events(&mut c)
        .iter()
        .any(|e| matches!(e, Event::Demux(ev) if matches!(ev.as_ref(), transmux::DemuxEvent::Sample { .. })));
    assert!(
        flushed,
        "the held sample is flushed before the stall reconnect"
    );
    let actions: Vec<Action> = std::iter::from_fn(|| c.poll()).collect();
    assert!(actions.contains(&Action::Abort), "{actions:?}");
}
