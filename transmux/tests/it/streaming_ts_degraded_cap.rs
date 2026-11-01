//! Opt-in cap on queued `InputDegraded` events (issue #1352).
//!
//! `StreamingTsDemux`'s event queue is only emptied by `poll_event()`; on a
//! lossy input a caller that never drains it grows memory linearly. The cap is
//! opt-in (`with_max_degraded_events`), counts what it drops, never touches any
//! other event kind, and leaves the default (uncapped) behaviour unchanged.
//!
//! Real fixture: `fixtures/ts/h264_aac.ts` with every 7th packet's sync byte
//! broken (about 14% loss), so the demuxer emits a real stream of `SyncLoss`
//! (and downstream `ContinuityGap`) degradations.

use transmux::ts_demux::{DemuxEvent, StreamingTsDemux};

const PACKET: usize = 188;
const BREAK_EVERY: usize = 7;

fn lossy_input() -> Vec<u8> {
    let mut data = test_fixtures::read("ts/h264_aac.ts");
    for (i, pkt) in data.chunks_exact_mut(PACKET).enumerate() {
        if i % BREAK_EVERY == 3 {
            pkt[0] = 0x00;
        }
    }
    data
}

fn is_degraded(e: &DemuxEvent) -> bool {
    matches!(e, DemuxEvent::InputDegraded { .. })
}

/// Feed in odd-sized chunks; drain after each feed only when `drain`, else
/// only once at the end. Returns (all events, final dropped counter).
fn run(mut demux: StreamingTsDemux, data: &[u8], drain: bool) -> (Vec<DemuxEvent>, u64) {
    let mut events = Vec::new();
    for chunk in data.chunks(PACKET * 13 + 5) {
        demux.feed(chunk);
        if drain {
            while let Some(e) = demux.poll_event() {
                events.push(e);
            }
        }
    }
    demux.finish();
    while let Some(e) = demux.poll_event() {
        events.push(e);
    }
    (events, demux.degraded_events_dropped())
}

#[test]
fn default_is_unbounded_and_unchanged() {
    let data = lossy_input();
    let (events, dropped) = run(StreamingTsDemux::new(), &data, false);
    let degraded = events.iter().filter(|e| is_degraded(e)).count();
    assert!(
        degraded > 50,
        "fixture must be genuinely lossy, got {degraded}"
    );
    assert_eq!(dropped, 0);
}

#[test]
fn cap_bounds_undrained_queue_and_counts_overflow() {
    const CAP: usize = 10;
    let data = lossy_input();
    let (all, _) = run(StreamingTsDemux::new(), &data, false);
    let total_degraded = all.iter().filter(|e| is_degraded(e)).count();
    assert!(total_degraded > CAP * 3);

    let (capped, dropped) = run(
        StreamingTsDemux::new().with_max_degraded_events(CAP),
        &data,
        false,
    );
    let kept = capped.iter().filter(|e| is_degraded(e)).count();
    assert_eq!(kept, CAP, "queue holds exactly the cap");
    assert_eq!(
        kept as u64 + dropped,
        total_degraded as u64,
        "every degradation is either queued or counted"
    );
    // Nothing but InputDegraded is ever dropped.
    assert_eq!(
        capped.iter().filter(|e| !is_degraded(e)).count(),
        all.iter().filter(|e| !is_degraded(e)).count()
    );
}

#[test]
fn drained_caller_is_unaffected_by_a_generous_cap() {
    let data = lossy_input();
    let (plain, _) = run(StreamingTsDemux::new(), &data, true);
    let (capped, dropped) = run(
        StreamingTsDemux::new().with_max_degraded_events(1000),
        &data,
        true,
    );
    // Draining after each feed keeps the queue under the cap, so nothing is
    // lost: same event count, same degradations.
    assert_eq!(
        dropped, 0,
        "a drained caller under a generous cap loses nothing"
    );
    assert_eq!(plain.len(), capped.len());
    assert_eq!(
        plain.iter().filter(|e| is_degraded(e)).count(),
        capped.iter().filter(|e| is_degraded(e)).count()
    );
}
