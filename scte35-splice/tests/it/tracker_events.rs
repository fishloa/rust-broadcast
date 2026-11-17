//! `SpliceTracker` open-event bookkeeping: closing one event must not evict
//! the others from the eviction order, retransmission detection, and the
//! `NoCue`/`SpliceSanity` labels (ANSI/SCTE 35 §9.7.3, §9.8.2).

use broadcast_common::{Parse, Serialize};
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::{AnyCommand, ProgramSplice, SpliceInsert, SpliceMode};
use scte35_splice::descriptor_loop::SpliceDescriptorLoop;
use scte35_splice::tracker::{MAX_OPEN_EVENTS, NoCue, SpliceSanity, SpliceTracker};

fn insert_section(event_id: u32, avail_num: u8) -> Vec<u8> {
    let insert = SpliceInsert {
        splice_event_id: event_id,
        mode: SpliceMode::Program(ProgramSplice::Immediate),
        out_of_network_indicator: true,
        avail_num,
        ..Default::default()
    };
    let section =
        SpliceInfoSection::new_clear(AnyCommand::from(insert), SpliceDescriptorLoop::new(&[]));
    section.try_to_bytes().unwrap()
}

#[test]
fn closing_one_open_event_keeps_the_others_in_the_eviction_order() {
    let mut tracker = SpliceTracker::default();
    for id in 1..=3 {
        assert!(!tracker.observe(id, true, false));
    }
    assert_eq!(tracker.open_count(), 3);
    assert!(!tracker.observe(2, false, false));
    assert_eq!(tracker.open_count(), 2);
    assert_eq!(tracker.open_events().collect::<Vec<_>>(), vec![1, 3]);

    // Fill to the cap, close a middle event, then keep opening: the cap must
    // still hold, which requires every surviving id to stay in the order.
    let mut tracker = SpliceTracker::default();
    for id in 0..MAX_OPEN_EVENTS as u32 {
        tracker.observe(id, true, false);
    }
    assert_eq!(tracker.open_count(), MAX_OPEN_EVENTS);
    tracker.observe(500, false, false);
    assert_eq!(tracker.open_count(), MAX_OPEN_EVENTS - 1);
    for id in 0..8u32 {
        tracker.observe(MAX_OPEN_EVENTS as u32 + id, true, false);
    }
    assert_eq!(tracker.open_count(), MAX_OPEN_EVENTS);
}

#[test]
fn a_different_section_with_the_same_event_is_a_duplicate_open() {
    let mut tracker = SpliceTracker::default();
    let first_raw = insert_section(1, 1);
    let first = SpliceInfoSection::parse(&first_raw).unwrap();
    let observation = tracker.observe_section(&first, &first_raw).unwrap();
    assert_eq!(observation.event_id, 1);
    assert!(!observation.duplicate_open);
    assert!(observation.event.out_of_network_indicator);
    assert!(observation.event.times.is_empty());

    // The identical bytes are a retransmission, not a second open.
    let observation = tracker.observe_section(&first, &first_raw).unwrap();
    assert!(!observation.duplicate_open);
    assert_eq!(tracker.open_count(), 1);

    // Different bytes for the same still-open event are a duplicate open.
    let second_raw = insert_section(1, 2);
    assert_ne!(second_raw, first_raw);
    let second = SpliceInfoSection::parse(&second_raw).unwrap();
    let observation = tracker.observe_section(&second, &second_raw).unwrap();
    assert!(observation.duplicate_open);
    assert_eq!(tracker.open_count(), 1);
}

#[test]
fn no_cue_and_sanity_labels_are_the_monitor_tokens() {
    assert_eq!(NoCue::NotACue.name(), "not_a_cue");
    let malformed = NoCue::Malformed {
        raw: Vec::new(),
        error: scte35_splice::Error::UnexpectedTableId { table_id: 0 },
    };
    assert_eq!(malformed.name(), "malformed");
    assert_eq!(std::format!("{}", NoCue::NotACue), "not_a_cue");

    assert_eq!(SpliceSanity::Encrypted.name(), "encrypted");
    assert_eq!(SpliceSanity::Cancelled.name(), "cancelled");
    assert_eq!(SpliceSanity::NotSpliceInsert.name(), "not_splice_insert");
    assert_eq!(std::format!("{}", SpliceSanity::Cancelled), "cancelled");
}
