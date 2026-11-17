//! The PUSI unit-size cap and the accumulator's read-back accessors (audit 21,
//! #2924). A unit of exactly the cap is a real unit (a PES packet that lands
//! exactly on the configured bound must not vanish), and one byte past it is
//! what the cap exists to drop.

use broadcast_common::pusi::PusiAccumulator;

fn unit(len: usize, byte: u8) -> Vec<u8> {
    vec![byte; len]
}

#[test]
fn a_unit_of_exactly_the_cap_is_kept() {
    let mut accumulator = PusiAccumulator::with_max_unit_size(10);
    assert_eq!(accumulator.max_unit_size(), 10);
    assert!(!accumulator.is_started());
    assert!(accumulator.is_empty());
    assert_eq!(accumulator.len(), 0);
    assert_eq!(accumulator.feed(true, &unit(10, 7)), None);
    assert!(accumulator.is_started());
    assert!(!accumulator.is_empty());
    assert_eq!(accumulator.len(), 10);
    assert_eq!(accumulator.flush(), Some(unit(10, 7)));
    assert!(!accumulator.is_started());
    assert_eq!(accumulator.len(), 0);
}

#[test]
fn a_pusi_payload_one_past_the_cap_is_dropped() {
    let mut accumulator = PusiAccumulator::with_max_unit_size(10);
    assert_eq!(accumulator.feed(true, &unit(11, 7)), None);
    assert!(!accumulator.is_started());
    assert!(accumulator.is_empty());
    assert_eq!(accumulator.len(), 0);
    assert_eq!(accumulator.feed(false, &unit(1, 7)), None);
    assert_eq!(accumulator.flush(), None);
}

#[test]
fn the_next_pusi_hands_back_the_completed_capped_unit() {
    let mut accumulator = PusiAccumulator::with_max_unit_size(10);
    assert_eq!(accumulator.feed(true, &unit(10, 3)), None);
    assert_eq!(accumulator.feed(true, b"next"), Some(unit(10, 3)));
    assert_eq!(accumulator.len(), 4);
    assert!(accumulator.is_started());
    assert_eq!(accumulator.flush(), Some(b"next".to_vec()));
}

#[test]
fn a_continuation_that_would_pass_the_cap_drops_the_unit() {
    let mut accumulator = PusiAccumulator::with_max_unit_size(10);
    assert_eq!(accumulator.feed(true, &unit(6, 1)), None);
    assert_eq!(accumulator.feed(false, &unit(4, 2)), None);
    assert_eq!(accumulator.len(), 10);
    assert_eq!(accumulator.feed(false, &unit(1, 3)), None);
    assert!(!accumulator.is_started());
    assert_eq!(accumulator.len(), 0);
    assert_eq!(accumulator.flush(), None);
}
