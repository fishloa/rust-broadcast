//! A session's retry jitter is a function of the instant it was opened: two
//! sessions opened a microsecond apart do not retry in lockstep, and two opened
//! at the same instant replay identically.

use broadcast_client::{Action, Config, ConnId, Session, TransportError};
use broadcast_common::stage::Timestamp;
use url::Url;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const URL: &str = "http://origin.test/live.ts";

fn link_lost() -> TransportError {
    TransportError::Other {
        reason: "reset".to_string(),
    }
}

fn next_request(session: &mut Session, now: u64) -> ConnId {
    while let Some(action) = session.poll(ts(now)) {
        if let Action::HttpRequest { conn, .. } = action {
            return conn;
        }
    }
    panic!("an HttpRequest at {now}");
}

fn next_wake(session: &mut Session, now: u64) -> u64 {
    let mut wake = None;
    while let Some(action) = session.poll(ts(now)) {
        if let Action::WakeAt(at) = action {
            wake = Some(at.as_micros());
        }
    }
    wake.expect("a reconnect wake")
}

/// The wait before the second reconnect: the first retry is its unjittered
/// step, so only this one carries the seed's jitter term.
fn second_reconnect_wait(open_us: u64) -> u64 {
    let mut session =
        Session::open(Url::parse(URL).unwrap(), Config::default(), ts(open_us)).expect("open");
    let first = next_request(&mut session, open_us);
    session.on_closed(first, Some(link_lost()), ts(open_us));
    let first_wake = next_wake(&mut session, open_us);
    let second = next_request(&mut session, first_wake);
    session.on_closed(second, Some(link_lost()), ts(first_wake));
    next_wake(&mut session, first_wake) - first_wake
}

#[test]
fn sessions_opened_a_microsecond_apart_jitter_apart() {
    assert_ne!(
        second_reconnect_wait(0),
        second_reconnect_wait(1),
        "the jitter term follows the opening instant"
    );
}

#[test]
fn sessions_opened_at_the_same_instant_replay_identically() {
    assert_eq!(second_reconnect_wait(0), second_reconnect_wait(0));
}
