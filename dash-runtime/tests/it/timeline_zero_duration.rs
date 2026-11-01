use dash_runtime::address::AddressError;
use dash_runtime::address::timeline::TimelineIndex;
use dash_runtime::mpd::model::{SegmentTimeline, TimelineEntry};

fn timeline(entries: Vec<TimelineEntry>) -> SegmentTimeline {
    let mut t = SegmentTimeline::default();
    t.entries = entries;
    t
}

#[test]
fn zero_duration_entry_is_a_typed_error_not_a_divide_by_zero() {
    let mut open_ended = TimelineEntry::new(0);
    open_ended.r = -1;
    for entry in [TimelineEntry::new(0), open_ended] {
        let r = TimelineIndex::build(&timeline(vec![entry]), 1, Some(1000));
        assert!(matches!(r, Err(AddressError::ZeroSegmentDuration)));
    }
}

#[test]
fn zero_duration_after_a_valid_entry_is_rejected() {
    let r = TimelineIndex::build(
        &timeline(vec![TimelineEntry::new(10), TimelineEntry::new(0)]),
        1,
        None,
    );
    assert!(matches!(r, Err(AddressError::ZeroSegmentDuration)));
}
