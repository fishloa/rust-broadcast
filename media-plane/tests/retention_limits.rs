//! #2944: the cold-window cap's numeric value is what bounds the ledger's
//! lifetime, so it is pinned literally, with both edges of the accept/refuse
//! boundary and the error's own labels.

#![cfg(feature = "std")]

use std::time::Duration;

use media_plane::Retention;
use media_plane::retention::{
    MAX_COLD_WINDOW, RetentionConfigError, RetentionDriver, SegmentSink, SinkOutcome,
};
use media_plane::trunk::{ArchiveOverrun, SegmentEntry, Trunk, TrunkConfig};

struct DiscardSink;

impl SegmentSink for DiscardSink {
    fn offer(&mut self, _entry: &SegmentEntry) -> SinkOutcome {
        SinkOutcome::Taken
    }
}

fn tiered(cold_window: Duration) -> Retention {
    Retention::Tiered {
        on_overrun: ArchiveOverrun::Gap,
        cold_window,
    }
}

#[test]
fn the_cold_window_cap_is_thirty_days() {
    assert_eq!(MAX_COLD_WINDOW, Duration::from_secs(2_592_000));
    assert_eq!(MAX_COLD_WINDOW, Duration::from_secs(30 * 24 * 60 * 60));
}

#[test]
fn a_cold_window_at_the_cap_is_accepted() {
    let trunk = Trunk::new(TrunkConfig::default());
    let driver = RetentionDriver::new(&trunk, tiered(MAX_COLD_WINDOW), DiscardSink)
        .expect("the cap's own window is accepted");
    assert!(driver.is_some(), "Tiered builds a driver");
}

#[test]
fn one_second_past_the_cap_is_refused_with_its_labels() {
    let trunk = Trunk::new(TrunkConfig::default());
    let over = MAX_COLD_WINDOW + Duration::from_secs(1);
    let err = RetentionDriver::new(&trunk, tiered(over), DiscardSink)
        .err()
        .expect("a window past the cap is refused");
    assert_eq!(
        err,
        RetentionConfigError::ColdWindowTooLong {
            cold_window: over,
            max: Duration::from_secs(2_592_000),
        }
    );
    assert_eq!(err.name(), "cold window too long");
    assert_eq!(
        err.to_string(),
        "retention cold_window 2592001s is past the 2592000s cap"
    );
}

/// #2945: `pending_len` is 0 until a hand-off is stuck, 1 while it is, and 0
/// again once the sink takes the segment.
#[test]
fn pending_len_tracks_a_stuck_hand_off() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use broadcast_common::Timestamp;
    use bytes::Bytes;
    use media_plane::ids::SegmentSeq;

    struct ToggleSink(Arc<AtomicBool>);

    impl SegmentSink for ToggleSink {
        fn offer(&mut self, _entry: &SegmentEntry) -> SinkOutcome {
            if self.0.load(Ordering::Relaxed) {
                SinkOutcome::Busy
            } else {
                SinkOutcome::Taken
            }
        }
    }

    let busy = Arc::new(AtomicBool::new(true));
    let trunk = Trunk::new(TrunkConfig::default());
    let mut driver = RetentionDriver::new(
        &trunk,
        tiered(Duration::from_secs(10)),
        ToggleSink(busy.clone()),
    )
    .expect("an in-range window is accepted")
    .expect("Tiered builds a driver");
    assert_eq!(driver.pending_len(), 0);

    let writer = trunk.segment_writer().expect("a segment log");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from_static(b"seg"),
            SegmentSeq::new(1),
            Duration::from_secs(2),
            Timestamp::from_nanos(0),
            transmux::SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    driver.drive(Timestamp::from_nanos(0));
    assert_eq!(
        driver.pending_len(),
        1,
        "the busy sink leaves one in flight"
    );

    busy.store(false, Ordering::Relaxed);
    driver.drive(Timestamp::from_nanos(0));
    assert_eq!(driver.pending_len(), 0);
}
