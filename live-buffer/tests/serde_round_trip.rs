#![cfg(feature = "serde")]
//! The persisted types round-trip through serde; data types serialize.

use live_buffer::{ControllerConfig, Event, Reason, StallMemorySnapshot};

#[test]
fn config_and_snapshot_round_trip() {
    let c = ControllerConfig::default();
    let j = serde_json::to_string(&c).unwrap();
    let back: ControllerConfig = serde_json::from_str(&j).unwrap();
    assert_eq!(c, back);
    let s = StallMemorySnapshot::new(1_447_000, 12_345);
    let back: StallMemorySnapshot =
        serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(s, back);
    assert!(j.contains("\"initial_us\":1800000"));
}

#[test]
fn events_serialize() {
    let e = Event::Pause {
        hold_at_27mhz: 9,
        reason: Reason::NoData,
        depth_us: 1,
        resume_target_us: 2,
        track: None,
    };
    let j = serde_json::to_string(&e).unwrap();
    assert!(j.contains("Pause") && j.contains("NoData"));
}
