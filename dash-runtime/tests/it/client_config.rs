//! `DashConfig` defaults and validation (`docs/dash-runtime/DESIGN.md` 6.4).

use core::time::Duration;

use dash_runtime::client::config::{
    DEFAULT_AVAILABILITY_MARGIN, DEFAULT_EARLY_GRACE, DEFAULT_EARLY_RETRY,
    DEFAULT_HOST_CLOCK_UNCERTAINTY, DEFAULT_MIN_MPD_REFRESH, DEFAULT_UTC_MAX_RTT,
    DEFAULT_UTC_RESYNC, DEFAULT_UTC_TIMEOUT, DEFAULT_UTC_WAIT, DashClockPolicy, DashConfig,
};

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn field(cfg: &DashConfig) -> Option<&'static str> {
    cfg.validate().err().map(|e| e.field)
}

#[test]
fn defaults_are_the_design_table() {
    let cfg = DashConfig::default();
    assert_eq!(cfg.clock_policy, DashClockPolicy::Auto);
    assert_eq!(cfg.utc_timeout, secs(2));
    assert_eq!(cfg.utc_wait, secs(5));
    assert_eq!(cfg.utc_max_rtt, secs(2));
    assert_eq!(cfg.utc_resync, Some(secs(600)));
    assert_eq!(cfg.host_clock_uncertainty, Duration::from_millis(500));
    assert_eq!(cfg.live_delay, None);
    assert_eq!(cfg.availability_margin, Duration::ZERO);
    assert_eq!(cfg.early_retry, Duration::from_millis(250));
    assert_eq!(cfg.early_grace, secs(1));
    assert_eq!(cfg.min_mpd_refresh, secs(1));
    assert!(!cfg.use_availability_time_offset);
    assert!(cfg.cross_origin_urls);
    assert_eq!(cfg.validate(), Ok(()));
}

#[test]
fn default_constants_are_the_defaults() {
    let cfg = DashConfig::default();
    assert_eq!(cfg.utc_timeout, DEFAULT_UTC_TIMEOUT);
    assert_eq!(cfg.utc_wait, DEFAULT_UTC_WAIT);
    assert_eq!(cfg.utc_max_rtt, DEFAULT_UTC_MAX_RTT);
    assert_eq!(cfg.utc_resync, Some(DEFAULT_UTC_RESYNC));
    assert_eq!(cfg.host_clock_uncertainty, DEFAULT_HOST_CLOCK_UNCERTAINTY);
    assert_eq!(cfg.availability_margin, DEFAULT_AVAILABILITY_MARGIN);
    assert_eq!(cfg.early_retry, DEFAULT_EARLY_RETRY);
    assert_eq!(cfg.early_grace, DEFAULT_EARLY_GRACE);
    assert_eq!(cfg.min_mpd_refresh, DEFAULT_MIN_MPD_REFRESH);
}

#[test]
fn every_zero_timeout_and_period_is_rejected_by_name() {
    type Set = fn(&mut DashConfig);
    let cases: [(&str, Set); 6] = [
        ("utc_timeout", |c| c.utc_timeout = Duration::ZERO),
        ("utc_wait", |c| c.utc_wait = Duration::ZERO),
        ("utc_max_rtt", |c| c.utc_max_rtt = Duration::ZERO),
        ("utc_resync", |c| c.utc_resync = Some(Duration::ZERO)),
        ("early_retry", |c| c.early_retry = Duration::ZERO),
        ("min_mpd_refresh", |c| c.min_mpd_refresh = Duration::ZERO),
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
    cfg.availability_margin = Duration::ZERO;
    cfg.early_grace = Duration::ZERO;
    cfg.host_clock_uncertainty = Duration::ZERO;
    cfg.live_delay = Some(Duration::ZERO);
    assert_eq!(cfg.validate(), Ok(()));
    cfg.utc_resync = None;
    assert_eq!(cfg.validate(), Ok(()), "None never re-syncs on age");
}

#[test]
fn the_largest_value_is_accepted_for_every_numeric_field() {
    let mut cfg = DashConfig::default();
    cfg.utc_timeout = Duration::MAX;
    cfg.utc_wait = Duration::MAX;
    cfg.utc_max_rtt = Duration::MAX;
    cfg.utc_resync = Some(Duration::MAX);
    cfg.host_clock_uncertainty = Duration::MAX;
    cfg.live_delay = Some(Duration::MAX);
    cfg.availability_margin = Duration::MAX;
    cfg.early_retry = Duration::MAX;
    cfg.early_grace = Duration::MAX;
    cfg.min_mpd_refresh = Duration::MAX;
    assert_eq!(cfg.validate(), Ok(()));
}

#[test]
fn the_ladder_wait_may_not_be_shorter_than_one_request() {
    let mut cfg = DashConfig::default();
    cfg.utc_timeout = secs(3);
    cfg.utc_wait = secs(3);
    assert_eq!(cfg.validate(), Ok(()), "equal is allowed");
    cfg.utc_wait = secs(3) - Duration::from_micros(1);
    assert_eq!(field(&cfg), Some("utc_wait"));
}

#[test]
fn a_zero_timeout_is_reported_before_the_combination_rule() {
    let mut cfg = DashConfig::default();
    cfg.utc_timeout = Duration::ZERO;
    cfg.utc_wait = Duration::ZERO;
    assert_eq!(field(&cfg), Some("utc_timeout"));
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
fn the_error_names_the_offending_field() {
    let mut cfg = DashConfig::default();
    cfg.utc_resync = Some(Duration::ZERO);
    let err = cfg.validate().unwrap_err();
    assert_eq!(err.field, "utc_resync");
}
