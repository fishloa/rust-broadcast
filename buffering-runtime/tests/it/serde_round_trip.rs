#![cfg(feature = "serde")]
//! The persisted types round-trip through serde; data types serialize.
use broadcast_common::clock33::PCR_WRAP_27MHZ;

/// A 27 MHz tick count as a validated clock position.
fn clock27(ticks: u64) -> Clock27 {
    Clock27::from_27mhz(ticks % PCR_WRAP_27MHZ).expect("a reduced count is on the clock")
}

const fn dur(us: u64) -> Duration {
    Duration::from_micros(us)
}
use core::time::Duration;

use buffering_runtime::{Clock27, ControllerConfig, Event, Reason, StallMemorySnapshot};

#[test]
fn config_and_snapshot_round_trip() {
    let c = ControllerConfig::default();
    let j = serde_json::to_string(&c).unwrap();
    let back: ControllerConfig = serde_json::from_str(&j).unwrap();
    assert_eq!(c, back);
    let s = StallMemorySnapshot::new(dur(1_447_000), dur(12_345));
    let back: StallMemorySnapshot =
        serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(s, back);
    // a span persists as its seconds and nanoseconds, so a stored config is
    // unambiguous about the unit (the log format keeps whole microseconds)
    assert!(j.contains("\"initial\":{\"secs\":1,\"nanos\":800000000}"));
    let spans: ControllerConfig = serde_json::from_str(
        r#"{"jitter":{"min_window":{"secs":2,"nanos":0},"interval":{"secs":0,"nanos":500000000},"outlier":{"secs":0,"nanos":500000000},"bucket":{"secs":0,"nanos":10000000},"buckets":400,"forget_q15":32113,"quantile_permille":990,"hold":{"secs":60,"nanos":0},"decay":{"secs":60,"nanos":0},"max_samples":4096,"discontinuity":{"secs":1,"nanos":0}},"policy":{"min":{"secs":0,"nanos":300000000},"initial":{"secs":1,"nanos":800000000},"max":{"secs":3,"nanos":0},"k1_q8":512,"margin":{"secs":0,"nanos":100000000},"k2_q8":307,"after_rebuffer_q8":320,"max_step_down_per_s":{"secs":0,"nanos":100000000},"reprime_floor":{"secs":0,"nanos":0},"reprime_hold":{"secs":2,"nanos":0}},"starve_threshold":{"secs":0,"nanos":20000000},"nodata_after":{"secs":0,"nanos":200000000}}"#,
    )
    .unwrap();
    assert_eq!(spans, ControllerConfig::default());
}

#[test]
fn events_serialize() {
    let e = Event::Pause {
        hold_at_27mhz: clock27(9),
        reason: Reason::NoData,
        depth: dur(1),
        resume_target: dur(2),
        track: None,
    };
    let j = serde_json::to_string(&e).unwrap();
    assert!(j.contains("Pause") && j.contains("NoData"));
}
