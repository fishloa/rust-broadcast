//! `DashConfig` defaults and validation (`docs/dash-runtime/DESIGN.md` 6.4).

use dash_runtime::client::config::{
    DEFAULT_AVAILABILITY_MARGIN_US, DEFAULT_EARLY_GRACE_US, DEFAULT_EARLY_RETRY_US,
    DEFAULT_HOST_CLOCK_UNCERTAINTY_US, DEFAULT_MIN_MPD_REFRESH_US, DEFAULT_UTC_MAX_RTT_US,
    DEFAULT_UTC_RESYNC_US, DEFAULT_UTC_TIMEOUT_US, DEFAULT_UTC_WAIT_US, DashClockPolicy,
    DashConfig,
};

const MICROS_PER_SECOND: u64 = 1_000_000;

fn field(cfg: &DashConfig) -> Option<&'static str> {
    cfg.validate().err().map(|e| e.field)
}

#[test]
fn defaults_are_the_design_table() {
    let cfg = DashConfig::default();
    assert_eq!(cfg.clock_policy, DashClockPolicy::Auto);
    assert_eq!(cfg.utc_timeout_us, 2 * MICROS_PER_SECOND);
    assert_eq!(cfg.utc_wait_us, 5 * MICROS_PER_SECOND);
    assert_eq!(cfg.utc_max_rtt_us, 2 * MICROS_PER_SECOND);
    assert_eq!(cfg.utc_resync_us, 600 * MICROS_PER_SECOND);
    assert_eq!(cfg.host_clock_uncertainty_us, 500_000);
    assert_eq!(cfg.live_delay_us, None);
    assert_eq!(cfg.availability_margin_us, 0);
    assert_eq!(cfg.early_retry_us, 250_000);
    assert_eq!(cfg.early_grace_us, MICROS_PER_SECOND);
    assert_eq!(cfg.min_mpd_refresh_us, MICROS_PER_SECOND);
    assert!(!cfg.use_availability_time_offset);
    assert!(cfg.cross_origin_urls);
    assert_eq!(cfg.validate(), Ok(()));
}

#[test]
fn default_constants_are_the_defaults() {
    let cfg = DashConfig::default();
    assert_eq!(cfg.utc_timeout_us, DEFAULT_UTC_TIMEOUT_US);
    assert_eq!(cfg.utc_wait_us, DEFAULT_UTC_WAIT_US);
    assert_eq!(cfg.utc_max_rtt_us, DEFAULT_UTC_MAX_RTT_US);
    assert_eq!(cfg.utc_resync_us, DEFAULT_UTC_RESYNC_US);
    assert_eq!(
        cfg.host_clock_uncertainty_us,
        DEFAULT_HOST_CLOCK_UNCERTAINTY_US
    );
    assert_eq!(cfg.availability_margin_us, DEFAULT_AVAILABILITY_MARGIN_US);
    assert_eq!(cfg.early_retry_us, DEFAULT_EARLY_RETRY_US);
    assert_eq!(cfg.early_grace_us, DEFAULT_EARLY_GRACE_US);
    assert_eq!(cfg.min_mpd_refresh_us, DEFAULT_MIN_MPD_REFRESH_US);
}

#[test]
fn every_zero_timeout_and_period_is_rejected_by_name() {
    type Set = fn(&mut DashConfig);
    let cases: [(&str, Set); 6] = [
        ("utc_timeout_us", |c| c.utc_timeout_us = 0),
        ("utc_wait_us", |c| c.utc_wait_us = 0),
        ("utc_max_rtt_us", |c| c.utc_max_rtt_us = 0),
        ("utc_resync_us", |c| c.utc_resync_us = 0),
        ("early_retry_us", |c| c.early_retry_us = 0),
        ("min_mpd_refresh_us", |c| c.min_mpd_refresh_us = 0),
    ];
    for (name, set) in cases {
        let mut cfg = DashConfig::default();
        set(&mut cfg);
        assert_eq!(field(&cfg), Some(name), "{name}");
    }
}

#[test]
fn zero_is_a_valid_value_where_it_means_none() {
    let mut cfg = DashConfig::default();
    cfg.availability_margin_us = 0;
    cfg.early_grace_us = 0;
    cfg.host_clock_uncertainty_us = 0;
    cfg.live_delay_us = Some(0);
    assert_eq!(cfg.validate(), Ok(()));
}

#[test]
fn the_largest_value_is_accepted_for_every_numeric_field() {
    let mut cfg = DashConfig::default();
    cfg.utc_timeout_us = u64::MAX;
    cfg.utc_wait_us = u64::MAX;
    cfg.utc_max_rtt_us = u64::MAX;
    cfg.utc_resync_us = u64::MAX;
    cfg.host_clock_uncertainty_us = u64::MAX;
    cfg.live_delay_us = Some(u64::MAX);
    cfg.availability_margin_us = u64::MAX;
    cfg.early_retry_us = u64::MAX;
    cfg.early_grace_us = u64::MAX;
    cfg.min_mpd_refresh_us = u64::MAX;
    assert_eq!(cfg.validate(), Ok(()));
}

#[test]
fn the_ladder_wait_may_not_be_shorter_than_one_request() {
    let mut cfg = DashConfig::default();
    cfg.utc_timeout_us = 3 * MICROS_PER_SECOND;
    cfg.utc_wait_us = 3 * MICROS_PER_SECOND;
    assert_eq!(cfg.validate(), Ok(()), "equal is allowed");
    cfg.utc_wait_us = 3 * MICROS_PER_SECOND - 1;
    assert_eq!(field(&cfg), Some("utc_wait_us"));
}

#[test]
fn a_zero_timeout_is_reported_before_the_combination_rule() {
    let mut cfg = DashConfig::default();
    cfg.utc_timeout_us = 0;
    cfg.utc_wait_us = 0;
    assert_eq!(field(&cfg), Some("utc_timeout_us"));
}

#[test]
fn the_non_boolean_policy_and_flags_never_invalidate() {
    for policy in [DashClockPolicy::Auto, DashClockPolicy::PreferHost] {
        for toa in [false, true] {
            for cross in [false, true] {
                let mut cfg = DashConfig::default();
                cfg.clock_policy = policy;
                cfg.use_availability_time_offset = toa;
                cfg.cross_origin_urls = cross;
                assert_eq!(cfg.validate(), Ok(()));
            }
        }
    }
}

#[test]
fn clock_policy_labels() {
    assert_eq!(DashClockPolicy::Auto.name(), "auto");
    assert_eq!(DashClockPolicy::PreferHost.name(), "prefer_host");
    assert_eq!(DashClockPolicy::Auto.to_string(), "auto");
    assert_eq!(DashClockPolicy::PreferHost.to_string(), "prefer_host");
    assert_eq!(DashClockPolicy::default(), DashClockPolicy::Auto);
}

#[test]
fn the_error_names_the_field_in_its_message() {
    let mut cfg = DashConfig::default();
    cfg.utc_resync_us = 0;
    let err = cfg.validate().unwrap_err();
    assert!(err.to_string().contains("utc_resync_us"), "{err}");
}
