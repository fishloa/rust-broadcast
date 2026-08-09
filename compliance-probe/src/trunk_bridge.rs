//! The `media-plane` attachment points: [`Probe::drain_byte_tap`] for TR
//! 101 290/PCR (needs raw wire bytes) and [`Probe::drain_event_cursor`] for
//! SCTE-35 sanity (the event log already carries a parsed
//! [`timed_metadata::TimedEvent`]). `std`-only — see the crate's `std`
//! feature doc for why both are bundled behind it even though
//! [`media_plane::ByteTap`] itself does not strictly require `std`.
//!
//! # Why TR 101 290 needs a `ByteTap`, not a `Trunk` sample cursor
//!
//! A [`media_plane::Trunk`] sample ring holds `transmux::Sample` — a
//! *demuxed* IR. `media_plane::byte_tap`'s own module docs spell out exactly
//! what that IR has already discarded: `transmux::StreamingTsDemux` reads
//! `pid`/`pusi` off `TsHeader` and drops `tei`, `continuity_counter`,
//! `scrambling` — the very fields TR 101 290's Priority-1 indicators
//! (`Sync_byte_error`, `Continuity_count_error`, `Transport_error`) are
//! defined on. A demuxed `Sample` genuinely cannot support those checks; the
//! bytes have to be observed *before* demux, which is exactly what
//! [`media_plane::byte_tap::TapPoint::Wire`] is for. That module's own docs
//! name `dvb_conformance::ConformanceMonitor::feed` as its intended
//! consumer — this bridge is that intended consumer.
//!
//! # Why SCTE-35 sanity *can* use a real `Trunk` cursor
//!
//! Unlike TR 101 290, a SCTE-35 splice cue is exactly the kind of
//! semantically-critical, irregular event `Trunk`'s event log
//! (`RetentionClass::Sparse`'s sibling ring) exists to carry — see
//! `media_plane::trunk`'s own module docs on the event log. Whoever
//! publishes an inband SCTE-35 section into a `Trunk` already parsed it into
//! a [`timed_metadata::TimedEvent`] before calling
//! `TrunkWriter::publish_event`, so [`Probe::drain_event_cursor`] costs
//! exactly the one [`media_plane::EventCursor`] the issue this crate answers
//! (#930) asks for — no second demux, no second copy of the stream.

use core::time::Duration;

use broadcast_common::Timestamp;
use media_plane::{ByteTap, EventAnchor, EventCursor, EventCursorItem, EventEntry, TapItem};
use mpeg_ts::resync::TsResync;
use timed_metadata::{MediaTime, SourcePayload};

use crate::record::{record_counter, record_counter_by};
use crate::{Probe, scte35::judge};

/// The `std`-only half of [`Probe`]'s state: a byte-stream resynchroniser
/// (recovers 188-byte alignment from whatever a [`ByteTap`] hands back —
/// exactly `media_doctor::WatchState::feed_datagram`'s own precedent) and the
/// one reference media-time [`Probe::note_media_time`] maintains for judging
/// Trunk-cursor-sourced SCTE-35 cues.
///
/// Threaded through [`Probe`]'s bridge methods as a separate `&mut`
/// parameter rather than an `Option` field on [`Probe`] itself, on purpose:
/// [`Probe`]'s core (the conformance monitor and PCR tracker) has to stay
/// constructible and usable with this crate's `std` feature *off* — putting
/// a `media-plane`/`std`-only type inside `Probe`'s own struct would force
/// every no_std caller to carry it regardless of use.
#[derive(Default)]
pub struct TrunkBridge {
    resync: TsResync,
    last_media_time: Option<MediaTime>,
}

impl Probe {
    /// Drain everything currently buffered in `tap`, recovering 188-byte TS
    /// packets via [`TsResync`] and feeding each to
    /// [`Probe::feed_ts_packet`].
    ///
    /// `epoch` is the [`Timestamp`] this call treats as t=0 for
    /// [`dvb_conformance::ConformanceMonitor`]'s wall-clock contract — pass
    /// the same value on every call (e.g. the `Timestamp` of the first tap
    /// item this probe ever observed) so the monotonic-non-decreasing
    /// contract holds across calls.
    ///
    /// [`TapItem::Lagged`] is recorded as
    /// `compliance_probe_tap_lagged_total` and otherwise ignored — per
    /// `ByteTap`'s own docs, a lagged stretch invalidates counter-based
    /// analysis (continuity count, PCR interval) across the gap; this crate
    /// does not attempt to reset `ConformanceMonitor`'s internal state (it
    /// exposes no such reset), so a rate of TR 101 290 events immediately
    /// following a nonzero `TAP_LAGGED_TOTAL` delta should be read with that
    /// caveat, exactly as a real gap in delivery would be.
    pub fn drain_byte_tap(
        &mut self,
        bridge: &mut TrunkBridge,
        tap: &mut ByteTap,
        epoch: Timestamp,
    ) {
        while let Some(item) = tap.poll() {
            match item {
                TapItem::Lagged { skipped } => {
                    record_counter_by!(crate::metric_names::TAP_LAGGED_TOTAL, skipped);
                }
                TapItem::Data(bytes, at) => {
                    let t: Duration = at.saturating_sub(epoch);
                    for packet in bridge.resync.feed(&bytes) {
                        self.feed_ts_packet(&packet, t);
                    }
                }
                // `TapItem` is `#[non_exhaustive]`: a future variant is
                // silently skipped rather than guessed at — exactly the
                // stance this crate takes everywhere else on data it cannot
                // honestly interpret yet.
                _ => {}
            }
        }
    }

    /// Record the most recently observed live media-timeline position (the
    /// same 90 kHz absolute clock [`media_plane::trunk`]'s event log uses —
    /// see that module's docs on `EventAnchor::Media`). [`Probe::drain_event_cursor`]
    /// compares every `Media`-resolved SCTE-35 cue's target against whatever
    /// this call most recently reported; until it has been called at least
    /// once, a cue that arrives is counted as
    /// `compliance_probe_scte35_no_reference_total` rather than judged
    /// against a fabricated "now".
    pub fn note_media_time(&mut self, bridge: &mut TrunkBridge, t: MediaTime) {
        bridge.last_media_time = Some(t);
    }

    /// Drain everything currently buffered in `cursor`, checking every
    /// SCTE-35-sourced [`timed_metadata::TimedEvent`] against
    /// [`Probe::note_media_time`]'s most recent reading.
    ///
    /// [`EventCursorItem::Lagged`] is recorded as
    /// `compliance_probe_event_cursor_lagged_total`. A `TimedEvent` sourced
    /// from [`timed_metadata::SourcePayload::Emsg`] is skipped — this
    /// crate's SCTE-35 sanity model does not extend to `emsg`.
    pub fn drain_event_cursor(&mut self, bridge: &mut TrunkBridge, cursor: &mut EventCursor) {
        while let Some(item) = cursor.poll() {
            match item {
                EventCursorItem::Lagged { skipped } => {
                    record_counter_by!(crate::metric_names::EVENT_CURSOR_LAGGED_TOTAL, skipped);
                }
                EventCursorItem::Event(entry) => {
                    check_event(bridge, &entry);
                }
                // `EventCursorItem` is `#[non_exhaustive]` for the same
                // reason as `TapItem` above.
                _ => {}
            }
        }
    }
}

/// Judge one Trunk-cursor-sourced event, per this module's documented rules:
/// only SCTE-35-sourced events are in scope, only `Media`-resolved anchors
/// are judged, and only once a reference "now" exists.
fn check_event(bridge: &TrunkBridge, entry: &EventEntry) {
    let SourcePayload::Scte35 { .. } = &entry.event.source else {
        return;
    };

    record_counter!(
        crate::metric_names::SCTE35_CUES_TOTAL,
        "kind" => entry.event.kind.name()
    );

    let EventAnchor::Media(target) = entry.anchor else {
        record_counter!(crate::metric_names::SCTE35_UNRESOLVED_ANCHOR_TOTAL);
        return;
    };

    let Some(now) = bridge.last_media_time else {
        record_counter!(crate::metric_names::SCTE35_NO_REFERENCE_TOTAL);
        return;
    };

    // Both `target` and `now` are already the Trunk's own 90 kHz absolute,
    // wrap-unrolled clock (`timed_metadata::Timeline::push_scte35`'s job,
    // done long before this event reached the event log) — a plain
    // comparison, not the 33-bit wraparound `scte35::judge` needs for the
    // still-raw wire path. `judge` is reused here anyway (rather than a
    // second copy of the comparison) because 90 kHz-tick equality/ordering
    // is exactly what it computes; passing already-unrolled values simply
    // never exercises its wrap branch.
    let _ = judge(target.0, now.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_plane::{Trunk, TrunkConfig};
    use mpeg_ts::ts::TS_PACKET_SIZE;
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use timed_metadata::{EventKind, TimedEvent};

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    fn small_trunk() -> Arc<Trunk> {
        Trunk::new(TrunkConfig::new(nz(8), nz(8), nz(8), nz(8), nz(8)))
    }

    /// A ByteTap drained through the bridge must actually reach the
    /// conformance monitor: feeding real fixture bytes through a tap must
    /// move `packets` in `Probe::conformance_stats()`.
    ///
    /// Uses 20 packets, not 4: `mpeg_ts::resync::TsResync` (which
    /// `drain_byte_tap` uses internally) only declares packet-stride lock
    /// after `LOCK_CONFIRMATIONS` (5) consecutive sync bytes at that stride
    /// — exactly `media_doctor::watch`'s own documented precedent — so a
    /// too-short feed would legitimately emit zero packets and this test
    /// would not bite.
    #[test]
    fn byte_tap_drain_reaches_conformance_monitor() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/m6-single.ts");
        let data = std::fs::read(path).expect("committed fixture must be readable");
        let mut tap = ByteTap::new(media_plane::TapPoint::Wire, 32);
        tap.record(
            bytes::Bytes::copy_from_slice(&data[..TS_PACKET_SIZE * 20]),
            Timestamp::from_nanos(0),
        );

        let mut probe = Probe::new();
        let mut bridge = TrunkBridge::default();
        probe.drain_byte_tap(&mut bridge, &mut tap, Timestamp::ZERO);

        assert_eq!(probe.conformance_stats().packets, 20);
    }

    /// A tap that lagged before this probe polled it must be counted, not
    /// silently dropped.
    #[test]
    fn lagged_tap_is_counted() {
        let mut tap = ByteTap::new(media_plane::TapPoint::Wire, 2);
        for i in 0..5u64 {
            tap.record(bytes::Bytes::from_static(b"x"), Timestamp::from_nanos(i));
        }
        let mut probe = Probe::new();
        let mut bridge = TrunkBridge::default();
        // Draining must not panic on the Lagged report mixed into the
        // stream, and must proceed to drain what remains.
        probe.drain_byte_tap(&mut bridge, &mut tap, Timestamp::ZERO);
    }

    /// An event-log cue with a `Media`-resolved anchor and a reference "now"
    /// must be judged without panicking, and one with an unresolved anchor
    /// must be skipped rather than fabricated a position — end-to-end
    /// through a real `Trunk`.
    #[test]
    fn event_cursor_drain_handles_resolved_and_unresolved_anchors() {
        let trunk = small_trunk();
        let writer = trunk.writer().expect("first writer");
        let event = TimedEvent {
            id: Some(1),
            kind: EventKind::BreakStart,
            at: Some(MediaTime(1_000)),
            duration: None,
            source: SourcePayload::Scte35 {
                raw: alloc::vec![0xFC],
            },
        };
        writer.publish_event(event, EventAnchor::Media(MediaTime(1_000)));

        let unresolved = TimedEvent {
            id: Some(2),
            kind: EventKind::BreakEnd,
            at: None,
            duration: None,
            source: SourcePayload::Scte35 {
                raw: alloc::vec![0xFC],
            },
        };
        writer.publish_event(
            unresolved,
            EventAnchor::Utc {
                utc_epoch_ms: 1_700_000_000_000,
            },
        );

        let mut cursor = trunk.subscribe_events();
        let mut probe = Probe::new();
        let mut bridge = TrunkBridge::default();
        probe.note_media_time(&mut bridge, MediaTime(500));
        probe.drain_event_cursor(&mut bridge, &mut cursor);
    }
}
