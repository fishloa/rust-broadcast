#![cfg(feature = "std")]
//! The Trunk event-cursor bridge must record what it drains. The only test
//! that drained a cursor asserted nothing, so dropping the whole drain, the
//! lagged report, the per-cue counters or the media-time reference would
//! leave every `compliance_probe_scte35_*` and
//! `compliance_probe_event_cursor_lagged_total` series flat (audit a23cp
//! #3431).

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use compliance_probe::Probe;
use compliance_probe::metric_names::{
    EVENT_CURSOR_LAGGED_TOTAL, SCTE35_CUES_TOTAL, SCTE35_NO_REFERENCE_TOTAL,
    SCTE35_PTS_IN_PAST_TOTAL, SCTE35_UNRESOLVED_ANCHOR_TOTAL,
};
use compliance_probe::trunk_bridge::TrunkBridge;
use media_plane::{EventAnchor, Trunk, TrunkConfig};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use timed_metadata::{EventKind, MediaTime, SourcePayload, TimedEvent};

fn install() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| {
            let recorder = PrometheusBuilder::new().build_recorder();
            let handle = recorder.handle();
            metrics::set_global_recorder(recorder)
                .expect("the process-wide recorder is installed exactly once");
            handle
        })
        .clone()
}

/// The recorder is process-wide and counters accumulate over the whole test
/// binary, so each test must own its before/after window.
fn test_guard() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn metric_sum(exposition: &str, name: &str) -> f64 {
    exposition
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let (series, value) = line.rsplit_once(' ')?;
            if series.split('{').next().unwrap_or(series) != name {
                return None;
            }
            value.parse::<f64>().ok()
        })
        .sum()
}

fn delta(before: &str, after: &str, name: &str) -> f64 {
    metric_sum(after, name) - metric_sum(before, name)
}

fn small_trunk(event_capacity: usize) -> Arc<Trunk> {
    let nz = |n: usize| NonZeroUsize::new(n).expect("a non-zero capacity");
    Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(8))
            .with_sparse_capacity(nz(8))
            .with_segment_capacity(nz(8))
            .with_event_capacity(nz(event_capacity))
            .with_part_capacity(nz(8)),
    )
}

fn scte35_event(id: u32) -> TimedEvent {
    TimedEvent::new(
        EventKind::BreakStart,
        SourcePayload::Scte35 {
            raw: vec![0xFC],
        },
    )
    .with_id(id)
}

#[test]
fn drain_records_resolved_and_unresolved_cue_counters() {
    let _guard = test_guard();
    let handle = install();
    let trunk = small_trunk(8);
    let writer = trunk.writer().expect("first writer");
    let mut cursor = trunk.subscribe_events();

    writer.publish_event(
        scte35_event(1).with_at(MediaTime(1_000)),
        EventAnchor::Media(MediaTime(1_000)),
    );
    writer.publish_event(
        scte35_event(2),
        EventAnchor::Utc {
            utc_epoch_ms: 1_700_000_000_000,
        },
    );

    let before = handle.render();
    let mut probe = Probe::new();
    let mut bridge = TrunkBridge::default();
    probe.note_media_time(&mut bridge, MediaTime(5_000));
    probe.drain_event_cursor(&mut bridge, &mut cursor);
    let after = handle.render();

    assert_eq!(
        delta(&before, &after, SCTE35_CUES_TOTAL),
        2.0,
        "both SCTE-35-sourced events are counted"
    );
    assert_eq!(
        delta(&before, &after, SCTE35_PTS_IN_PAST_TOTAL),
        1.0,
        "the Media-anchored cue targets 1000, behind the 5000 reference"
    );
    assert_eq!(
        delta(&before, &after, SCTE35_UNRESOLVED_ANCHOR_TOTAL),
        1.0,
        "the Utc-anchored cue is not judged"
    );
    assert_eq!(
        delta(&before, &after, SCTE35_NO_REFERENCE_TOTAL),
        0.0,
        "a reference time was supplied"
    );
}

#[test]
fn drain_without_a_media_time_counts_no_reference() {
    let _guard = test_guard();
    let handle = install();
    let trunk = small_trunk(8);
    let writer = trunk.writer().expect("first writer");
    let mut cursor = trunk.subscribe_events();

    writer.publish_event(
        scte35_event(1).with_at(MediaTime(1_000)),
        EventAnchor::Media(MediaTime(1_000)),
    );

    let before = handle.render();
    let mut probe = Probe::new();
    let mut bridge = TrunkBridge::default();
    probe.drain_event_cursor(&mut bridge, &mut cursor);
    let after = handle.render();

    assert_eq!(
        delta(&before, &after, SCTE35_CUES_TOTAL),
        1.0,
        "the cue itself is counted"
    );
    assert_eq!(
        delta(&before, &after, SCTE35_NO_REFERENCE_TOTAL),
        1.0,
        "without note_media_time there is no honest reference to judge against"
    );
    assert_eq!(
        delta(&before, &after, SCTE35_PTS_IN_PAST_TOTAL),
        0.0,
        "no reference means no past verdict"
    );
}

#[test]
fn an_overrun_event_ring_records_the_lagged_count() {
    let _guard = test_guard();
    let handle = install();
    const CAPACITY: usize = 4;
    const PUBLISHED: usize = 10;
    let trunk = small_trunk(CAPACITY);
    let writer = trunk.writer().expect("first writer");
    let mut cursor = trunk.subscribe_events();

    for id in 0..PUBLISHED {
        writer.publish_event(
            scte35_event(u32::try_from(id).expect("fits u32")).with_at(MediaTime(id as u64)),
            EventAnchor::Media(MediaTime(id as u64)),
        );
    }

    let before = handle.render();
    let mut probe = Probe::new();
    let mut bridge = TrunkBridge::default();
    probe.drain_event_cursor(&mut bridge, &mut cursor);
    let after = handle.render();

    let skipped = (PUBLISHED - CAPACITY) as f64;
    assert_eq!(
        delta(&before, &after, EVENT_CURSOR_LAGGED_TOTAL),
        skipped,
        "every entry evicted before the cursor read it is reported"
    );
}
