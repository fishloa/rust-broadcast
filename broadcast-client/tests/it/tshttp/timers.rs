use crate::tshttp::TsHttpConfig as Config;
use crate::tshttp::{Action, Event, TsHttpClient};
use broadcast_common::stage::Timestamp;

/// `ms` on the tshttp policy's millisecond clock, as the session's `now`.
fn ts(ms: u64) -> Timestamp {
    Timestamp::from_millis(ms)
}

static D: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| test_fixtures::read("ts/h264-aac.ts"));

fn drain_actions(c: &mut TsHttpClient) -> Vec<Action> {
    let mut v = Vec::new();
    while let Some(a) = c.poll() {
        v.push(a);
    }
    v
}

fn drain_events(c: &mut TsHttpClient) -> Vec<Event> {
    let mut v = Vec::new();
    while let Some(e) = c.poll_event() {
        v.push(e);
    }
    v
}

#[test]
fn start_timer_restarts_on_every_open() {
    let mut c = TsHttpClient::with_seed("u", Config::default(), ts(0), ts(0).as_millis());
    let _ = drain_actions(&mut c);
    c.on_response(200, &[], ts(100));
    c.poll_timeout(ts(15_000));
    assert!(drain_actions(&mut c).contains(&Action::Abort));
    let _ = drain_events(&mut c);
    c.poll_timeout(ts(17_000));
    assert_eq!(
        drain_actions(&mut c),
        vec![Action::Open { url: "u".into() }]
    );
    c.on_response(200, &[], ts(17_050));
    c.poll_timeout(ts(31_999));
    assert!(
        !drain_actions(&mut c).contains(&Action::Abort),
        "second open must get a full start_timeout from its own Open"
    );
    c.poll_timeout(ts(32_000));
    assert!(drain_actions(&mut c).contains(&Action::Abort));
}

#[test]
fn stall_is_measured_from_the_last_received_byte() {
    let mut c = TsHttpClient::with_seed("u", Config::default(), ts(0), ts(0).as_millis());
    let _ = drain_actions(&mut c);
    c.on_response(200, &[], ts(0));
    c.feed(D.as_slice(), ts(0));
    c.poll_timeout(ts(1_000));
    c.feed(&[0u8; 188], ts(10_000));
    let _ = drain_events(&mut c);
    c.poll_timeout(ts(14_999));
    assert!(
        !drain_events(&mut c)
            .iter()
            .any(|e| matches!(e, Event::StallNotice { .. })),
        "5 s after the last byte at 10 s is 15 s, not before"
    );
    c.poll_timeout(ts(15_000));
    assert!(
        drain_events(&mut c)
            .iter()
            .any(|e| matches!(e, Event::StallNotice { .. }))
    );
    c.poll_timeout(ts(39_999));
    assert!(!drain_actions(&mut c).contains(&Action::Abort));
    c.poll_timeout(ts(40_000));
    assert!(drain_actions(&mut c).contains(&Action::Abort));
}

#[test]
fn stall_reconnect_emits_abort_then_zero_delay_reconnect() {
    let mut c = TsHttpClient::with_seed("u", Config::default(), ts(0), ts(0).as_millis());
    let _ = drain_actions(&mut c);
    c.on_response(200, &[], ts(0));
    c.feed(D.as_slice(), ts(0));
    c.poll_timeout(ts(30_000));
    assert_eq!(
        drain_actions(&mut c),
        vec![Action::Abort, Action::Reconnect { after_ms: 0 }]
    );
}
