use tshttp_runtime::{Action, Config, TsHttpClient};

const BACKOFF_WITH_ZERO: [u64; 3] = [2_000, 0, 8_000];

#[test]
fn a_zero_backoff_entry_is_rejected_and_clamped_to_one_millisecond() {
    let config = Config {
        backoff_ms: BACKOFF_WITH_ZERO.to_vec(),
        ..Config::default()
    };
    assert!(config.validate().is_err(), "a 0 ms backoff spins");
    assert_eq!(config.clamped().backoff_ms, vec![2_000, 1, 8_000]);
    assert_eq!(
        Config::default().clamped(),
        Config::default(),
        "a valid configuration is unchanged"
    );
}

#[test]
fn a_client_never_schedules_a_zero_delay_reconnect_from_a_zero_backoff_entry() {
    let config = Config {
        backoff_ms: vec![0, 0],
        ..Config::default()
    };
    let mut client = TsHttpClient::new("u", config, 0);
    while client.poll().is_some() {}
    client.on_error(0);
    let delays: Vec<u64> = std::iter::from_fn(|| client.poll())
        .filter_map(|a| match a {
            Action::Reconnect { after_ms } => Some(after_ms),
            _ => None,
        })
        .collect();
    assert_eq!(delays, vec![1]);
}
