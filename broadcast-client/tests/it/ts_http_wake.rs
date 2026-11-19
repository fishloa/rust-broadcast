use crate::support;
use broadcast_client::{Action, Config, ConnId, Session, TransportError};
use broadcast_common::stage::Timestamp;
use support::ts_probe::{TS_URL, open};

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

/// The TS policy's default first backoff step (`Config::default().backoff.steps[0]`).
const FIRST_BACKOFF_US: u64 = 2_000_000;
/// The TS policy's default start timeout, the wake a fresh open arms.
const START_TIMEOUT_US: u64 = 15_000_000;

fn drain(session: &mut Session, now: u64) -> Vec<Action> {
    let mut out = Vec::new();
    while let Some(a) = session.poll(ts(now)) {
        out.push(a);
    }
    out
}

fn wakes(actions: &[Action]) -> Vec<Timestamp> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::WakeAt(at) => Some(*at),
            _ => None,
        })
        .collect()
}

fn request_conn(actions: &[Action]) -> ConnId {
    actions
        .iter()
        .find_map(|a| match a {
            Action::HttpRequest { conn, .. } => Some(*conn),
            _ => None,
        })
        .expect("the session opens its URL")
}

#[test]
fn a_failed_connect_arms_exactly_one_wake_at_the_next_backoff() {
    let mut session = open(TS_URL, Config::default());
    let conn = request_conn(&drain(&mut session, 0));
    session.on_closed(conn, Some(TransportError::Refused), ts(0));
    let first = drain(&mut session, 0);
    assert_eq!(wakes(&first), vec![ts(FIRST_BACKOFF_US)]);
    let second = drain(&mut session, 0);
    assert!(
        wakes(&second).is_empty(),
        "an outstanding wake must not be re-armed: {second:?}"
    );
}

#[test]
fn an_earlier_wake_replaces_the_outstanding_one() {
    let mut session = open(TS_URL, Config::default());
    let opening = drain(&mut session, 0);
    assert_eq!(wakes(&opening), vec![ts(START_TIMEOUT_US)]);
    let conn = request_conn(&opening);
    let refused_at = 10_000_000;
    session.on_closed(conn, Some(TransportError::Refused), ts(refused_at));
    let replaced = drain(&mut session, refused_at);
    assert_eq!(
        wakes(&replaced),
        vec![ts(refused_at + FIRST_BACKOFF_US)],
        "the backoff wake (12 s) replaces the opening timeout (15 s)"
    );
    let equal = drain(&mut session, refused_at);
    assert!(
        wakes(&equal).is_empty(),
        "an equal wake must not be re-armed: {equal:?}"
    );
}
