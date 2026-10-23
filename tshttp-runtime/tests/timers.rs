use tshttp_runtime::{Action, Config, Event, TsHttpClient};

const D: &[u8] = include_bytes!("../../fixtures/ts/h264_aac.ts");

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
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = drain_actions(&mut c);
    c.on_response(200, 100);
    c.poll_timeout(15_000);
    assert!(drain_actions(&mut c).contains(&Action::Abort));
    let _ = drain_events(&mut c);
    c.poll_timeout(17_000);
    assert_eq!(drain_actions(&mut c), vec![Action::Open { url: "u".into() }]);
    c.on_response(200, 17_050);
    c.poll_timeout(31_999);
    assert!(
        !drain_actions(&mut c).contains(&Action::Abort),
        "second open must get a full start_timeout from its own Open"
    );
    c.poll_timeout(32_000);
    assert!(drain_actions(&mut c).contains(&Action::Abort));
}

#[test]
fn stall_is_measured_from_the_last_received_byte() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = drain_actions(&mut c);
    c.on_response(200, 0);
    c.feed(D, 0);
    c.poll_timeout(1_000);
    c.feed(&[0u8; 188], 10_000);
    let _ = drain_events(&mut c);
    c.poll_timeout(14_999);
    assert!(
        !drain_events(&mut c)
            .iter()
            .any(|e| matches!(e, Event::StallNotice { .. })),
        "5 s after the last byte at 10 s is 15 s, not before"
    );
    c.poll_timeout(15_000);
    assert!(
        drain_events(&mut c)
            .iter()
            .any(|e| matches!(e, Event::StallNotice { .. }))
    );
    c.poll_timeout(39_999);
    assert!(!drain_actions(&mut c).contains(&Action::Abort));
    c.poll_timeout(40_000);
    assert!(drain_actions(&mut c).contains(&Action::Abort));
}

#[test]
fn stall_reconnect_emits_abort_then_zero_delay_reconnect() {
    let mut c = TsHttpClient::new("u", Config::default(), 0);
    let _ = drain_actions(&mut c);
    c.on_response(200, 0);
    c.feed(D, 0);
    c.poll_timeout(30_000);
    assert_eq!(
        drain_actions(&mut c),
        vec![Action::Abort, Action::Reconnect { after_ms: 0 }]
    );
}
