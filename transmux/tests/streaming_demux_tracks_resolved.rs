//! `DemuxEvent::TracksResolved` gate (issue #624).
//!
//! The bug this event fixes: a live `StreamingTsDemux` resolves
//! `DemuxEvent::TrackAdded` incrementally, one PID at a time, as each track's
//! in-band config becomes recoverable — video's config commonly resolves
//! before audio's, and on a real live feed the two resolutions land in
//! *different* `feed()` calls. A consumer that builds a multi-track
//! segmenter needs a "every currently-known track is ready" signal instead of
//! guessing from the first `TrackAdded` alone.
//!
//! Real fixture: `fixtures/ts/h264_aac.ts` (2-track H.264 + AAC MPEG-2 TS),
//! fed in chunks well under one 188-byte TS packet so video's and audio's
//! `TrackAdded` are essentially guaranteed to land in separate `feed()`
//! calls, mirroring the real timing race. Asserts `TracksResolved` fires
//! strictly after both `TrackAdded` events, and fires exactly once — proving
//! the de-dup logic doesn't spam the event once per repeated PMT section or
//! per packet on an already-stable track set.

use transmux::ts_demux::{DemuxEvent, StreamingTsDemux};

fn fixture_bytes() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264_aac.ts");
    std::fs::read(path).expect("h264_aac.ts fixture must exist")
}

/// Feed `data` through `demux` in `chunk_size`-byte pieces, draining and
/// recording every event after each `feed()` call so the returned order
/// exactly reflects when each event became available — not just the final
/// batch order.
fn feed_recording_events(
    demux: &mut StreamingTsDemux,
    data: &[u8],
    chunk_size: usize,
) -> Vec<DemuxEvent> {
    let mut events = Vec::new();
    for chunk in data.chunks(chunk_size.max(1)) {
        demux.feed(chunk);
        while let Some(ev) = demux.poll_event() {
            events.push(ev);
        }
    }
    demux.finish();
    while let Some(ev) = demux.poll_event() {
        events.push(ev);
    }
    events
}

#[test]
fn tracks_resolved_fires_once_after_both_tracks_added_no_spam() {
    let data = fixture_bytes();
    let mut demux = StreamingTsDemux::new();

    // 7 bytes is far under one 188-byte TS packet, and not aligned to it —
    // this forces many small `feed()` calls per packet and, in particular,
    // splits video's and audio's independent config-resolution moments
    // across different calls (the real live-ingest race issue #624 reports).
    let events = feed_recording_events(&mut demux, &data, 7);

    let track_added_positions: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, DemuxEvent::TrackAdded(_)))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        track_added_positions.len(),
        2,
        "h264_aac.ts must yield exactly 2 TrackAdded events (video + audio)"
    );

    let resolved_positions: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, DemuxEvent::TracksResolved))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        resolved_positions.len(),
        1,
        "TracksResolved must fire exactly once for a stable 2-track stream \
         (no per-packet / per-repeated-PMT spam); got positions {resolved_positions:?} \
         out of {} total events",
        events.len()
    );

    let last_track_added = *track_added_positions
        .iter()
        .max()
        .expect("2 TrackAdded positions recorded above");
    assert!(
        resolved_positions[0] > last_track_added,
        "TracksResolved (event #{}) must fire strictly after the last TrackAdded \
         (event #{}) — never before every currently-known track is ready",
        resolved_positions[0],
        last_track_added
    );
}

/// A single-packet, single-`feed()` call still yields the same event
/// (sanity: the chunking in the primary test isn't what causes the signal to
/// fire — the underlying resolved-state transition is).
#[test]
fn tracks_resolved_fires_with_whole_buffer_fed_at_once() {
    let data = fixture_bytes();
    let mut demux = StreamingTsDemux::new();
    demux.feed(&data);
    demux.finish();

    let mut resolved_count = 0usize;
    let mut track_added_count = 0usize;
    while let Some(ev) = demux.poll_event() {
        match ev {
            DemuxEvent::TracksResolved => resolved_count += 1,
            DemuxEvent::TrackAdded(_) => track_added_count += 1,
            _ => {}
        }
    }
    assert_eq!(track_added_count, 2);
    assert_eq!(
        resolved_count, 1,
        "TracksResolved must still fire exactly once when the whole buffer arrives \
         as a single feed() call"
    );
}
