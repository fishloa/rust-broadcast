use tshttp_runtime::{Action, Config, TsHttpClient};

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
    let mut c = TsHttpClient::new(URL, Config::default(), 0);
    c.set_retry_after(REQUESTED_MS);
    c.on_response(503, &[], 0);
    assert_eq!(reconnect_after(&mut c), REQUESTED_MS);
    c.poll_timeout(REQUESTED_MS);
    while c.poll().is_some() {}
    c.on_response(503, &[], REQUESTED_MS);
    assert!(reconnect_after(&mut c) < REQUESTED_MS);
}

#[test]
fn a_requested_delay_below_the_backoff_does_not_shorten_it() {
    let mut c = TsHttpClient::new(URL, Config::default(), 0);
    c.set_retry_after(1);
    c.on_response(503, &[], 0);
    assert_eq!(reconnect_after(&mut c), FIRST_BACKOFF_MS);
}
