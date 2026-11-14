use crate::tshttp::TsHttpConfig as Config;
use crate::tshttp::{Action, TsHttpClient};
use broadcast_common::stage::Timestamp;

/// `ms` on the tshttp policy's millisecond clock, as the session's `now`.
fn ts(ms: u64) -> Timestamp {
    Timestamp::from_millis(ms)
}

const URL: &str = "http://example/live.ts";
const FIRST_BACKOFF_MS: u64 = 2_000;
const REQUESTED_MS: u64 = 30_000;

fn reconnect_after(c: &mut TsHttpClient) -> u64 {
    let mut after = None;
    while let Some(a) = c.poll() {
        if let Action::Reconnect { after_ms } = a {
            after = Some(after_ms);
        }
    }
    after.expect("a reconnect was scheduled")
}

#[test]
fn a_requested_delay_raises_the_next_reconnect_and_is_consumed_by_it() {
    let mut c = TsHttpClient::with_seed(URL, Config::default(), ts(0), ts(0).as_millis());
    c.set_retry_after(REQUESTED_MS);
    c.on_response(503, &[], ts(0));
    assert_eq!(reconnect_after(&mut c), REQUESTED_MS);
    c.poll_timeout(ts(REQUESTED_MS));
    while c.poll().is_some() {}
    c.on_response(503, &[], ts(REQUESTED_MS));
    assert!(reconnect_after(&mut c) < REQUESTED_MS);
}

#[test]
fn a_requested_delay_below_the_backoff_does_not_shorten_it() {
    let mut c = TsHttpClient::with_seed(URL, Config::default(), ts(0), ts(0).as_millis());
    c.set_retry_after(1);
    c.on_response(503, &[], ts(0));
    assert_eq!(reconnect_after(&mut c), FIRST_BACKOFF_MS);
}
