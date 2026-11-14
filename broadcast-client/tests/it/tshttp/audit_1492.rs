use broadcast_client::tshttp::{Action, Config, TsHttpClient};
use broadcast_common::stage::Timestamp;
use std::time::Duration;

/// `ms` on the tshttp policy's millisecond clock, as the session's `now`.
fn ts(ms: u64) -> Timestamp {
    Timestamp::from_millis(ms)
}

const BACKOFF_WITH_ZERO: [Duration; 3] = [
    Duration::from_millis(2_000),
    Duration::ZERO,
    Duration::from_millis(8_000),
];

#[test]
fn a_zero_backoff_entry_is_rejected_and_clamped_to_one_millisecond() {
    let config = Config {
        backoff: broadcast_common::backoff::BackoffSteps {
            steps: BACKOFF_WITH_ZERO.to_vec(),
            cap: Duration::from_millis(10_000),
        },
        ..Config::default()
    };
    assert!(
        matches!(
            config.validate(),
            Err(broadcast_client::tshttp::config::ConfigError::ZeroBackoffEntry)
        ),
        "a 0 ms backoff spins"
    );
    assert_eq!(
        config.clamped().backoff.steps,
        vec![
            Duration::from_millis(2_000),
            Duration::from_millis(1),
            Duration::from_millis(8_000)
        ]
    );
    assert_eq!(
        Config::default().clamped(),
        Config::default(),
        "a valid configuration is unchanged"
    );
}

#[test]
fn a_client_never_schedules_a_zero_delay_reconnect_from_a_zero_backoff_entry() {
    let config = Config {
        backoff: broadcast_common::backoff::BackoffSteps {
            steps: vec![Duration::ZERO, Duration::ZERO],
            cap: Duration::from_millis(10_000),
        },
        ..Config::default()
    };
    let mut client = TsHttpClient::new("u", config, ts(0));
    while client.poll().is_some() {}
    client.on_error(ts(0));
    let delays: Vec<u64> = std::iter::from_fn(|| client.poll())
        .filter_map(|a| match a {
            Action::Reconnect { after_ms } => Some(after_ms),
            _ => None,
        })
        .collect();
    assert_eq!(delays, vec![1]);
}
