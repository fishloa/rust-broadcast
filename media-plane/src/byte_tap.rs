//! [`ByteTap`] — a positional, non-blocking observer of bytes in flight.
//!
//! # What a tap is for
//!
//! `dvb-conformance`'s `ConformanceMonitor` implements TR 101 290's 19
//! indicators, but a demuxed IR only ever preserves 2 of them
//! (`transmux::StreamingTsDemux` reads `pid`/`pusi` off `TsHeader` and
//! discards `tei`, `continuity_counter`, `scrambling` — see
//! `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §1.1).
//! That is correct layering for a demuxer, whose job is framing — but it
//! means the *only* place TR 101 290 conformance (and #737's T-STD buffer
//! model, which needs packet arrival timing) can be measured is on the bytes
//! themselves, before anything interprets or discards them. `ByteTap` is
//! that place: a positional observer that yields bytes **exactly as
//! received, with their arrival time, including bytes a demuxer will reject**
//! (bad sync byte, TEI set, bad CRC, unaligned framing). `ByteTap` performs
//! no validation and no filtering — it does not know what a "valid" packet
//! looks like, deliberately, because deciding that is exactly the analysis
//! its consumers are for.
//!
//! `dvb_conformance::ConformanceMonitor::feed(&mut self, ts_packet: &[u8], t:
//! Duration) -> &[ConformanceEvent]` (`dvb-conformance/src/lib.rs:649`) is
//! the primary intended consumer: already a per-packet streaming API over
//! exactly `(bytes, arrival time)`, which is what [`ByteTap::poll`] hands
//! back (`Timestamp` in place of `Duration` — see
//! [`broadcast_common::stage::Timestamp`]).
//!
//! # The non-blocking trade, stated honestly
//!
//! **A tap must never block or back-pressure the producer.** Stalling live
//! ingest so that analysis can keep up is a worse failure than gapped
//! analysis — a broadcast head-end does not get to pause the incoming
//! transport stream because a conformance monitor is slow. So [`ByteTap`]'s
//! ring is bounded, and when a consumer falls behind, **the consumer loses
//! data, not the stream**: [`ByteTap::record`] (the producer side) evicts the
//! oldest buffered item and keeps going; it never waits, never grows without
//! bound, and never fails.
//!
//! That trade has a consequence most people miss, so it is stated plainly
//! here rather than left to be rediscovered from a wrong conformance report:
//! **a lagged tap silently invalidates counter-based analysis over the gap.**
//! TR 101 290's continuity-count indicator (1.4) and anything else that
//! depends on having seen every packet in sequence (CC state, PCR
//! interval/discontinuity tracking, #737's T-STD buffer occupancy) cannot be
//! trusted across a period where packets were dropped before the consumer
//! ever saw them — a gap looks exactly like a real continuity error unless
//! the consumer knows a gap happened. That is why loss is surfaced as data
//! ([`TapItem::Lagged`]) rather than a side channel a consumer could ignore:
//! a consumer **must** treat `Lagged` as "reset or flag counter-based state
//! for this stretch", not as "carry on as if nothing happened".
//!
//! # Not a `Stage`
//!
//! [`ByteTap`] is deliberately **not** a [`crate::ByteStage`]. It consumes
//! nothing (it is fed, not driven with `feed`/`poll` as one contract — the
//! producer and consumer sides are different callers entirely) and
//! transforms nothing; it is a pure observer. Forcing it into the `Stage`
//! shape would invent a `finish()`/`demand()` story neither side needs.
//!
//! # `TapPoint` is metadata, not behaviour
//!
//! [`TapPoint`] records *where* a tap sits — raw wire bytes, or bytes after
//! some [`crate::ByteStage`] transform (CAM descramble, `ts-fix`, T2-MI inner
//! recovery) — purely so a consumer can label what it is looking at (e.g. "is
//! this TEI-set packet a real transmission error, or did the CAM produce it
//! while descrambling?"). It does not change [`ByteTap`]'s ring, eviction, or
//! `Lagged` accounting at all; both variants behave identically.

use alloc::collections::VecDeque;
use broadcast_common::stage::Timestamp;
use bytes::Bytes;

/// Where in the byte pipeline a [`ByteTap`] is observing.
///
/// Purely descriptive — see the [module docs](self) for why this carries no
/// behaviour of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TapPoint {
    /// Bytes exactly as they arrived off the wire, before any
    /// [`crate::ByteStage`] transform — including bytes a downstream demuxer
    /// will reject. This is the only tap point that can see wire-level
    /// TR 101 290 indicators (sync loss, TEI, bad CRC) at all, because a
    /// demuxed IR has already discarded them.
    Wire,
    /// Bytes after some byte-layer transform (CAM descramble, `ts-fix`
    /// continuity/PCR repair, T2-MI/BBFrame inner-TS recovery).
    PostTransform,
}

/// What [`ByteTap::poll`] hands back to the consumer.
///
/// A consumer cannot poll past a [`TapItem::Lagged`] without seeing it: it is
/// returned in-band, in the same `Option<TapItem>` as real data, ordered
/// ahead of the data that follows the gap it reports — see the
/// [module docs](self) on why loss must never be a side channel a consumer
/// could fail to check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TapItem {
    /// One observed byte unit with its arrival time, verbatim — never
    /// validated, never filtered.
    Data(Bytes, Timestamp),
    /// The consumer fell behind the producer: `skipped` items were evicted
    /// from the ring (oldest first) since the last successful poll, to keep
    /// [`ByteTap::record`] non-blocking. See the [module docs](self) for the
    /// consequence for counter-based analysis (TR 101 290 continuity
    /// counting, T-STD buffer state, …) across this gap.
    Lagged {
        /// Count of items dropped since the last poll returned data (or
        /// since construction, for the first poll).
        skipped: u64,
    },
}

/// A positional, non-blocking observer of bytes passing a point in the byte
/// layer. See the [module docs](self).
pub struct ByteTap {
    point: TapPoint,
    capacity: usize,
    ring: VecDeque<(Bytes, Timestamp)>,
    skipped_since_last_poll: u64,
}

impl ByteTap {
    /// Create a tap at `point` with a ring bounded to `capacity` items.
    ///
    /// `capacity` is a hard cap independent of how fast [`ByteTap::record`]
    /// is called or how slowly [`ByteTap::poll`] drains it — see the
    /// [module docs](self)'s non-blocking trade. Panics if `capacity == 0`,
    /// which would make every recorded item lost immediately and is almost
    /// certainly a construction mistake rather than an intended tap.
    pub fn new(point: TapPoint, capacity: usize) -> Self {
        assert!(capacity > 0, "ByteTap capacity must be > 0");
        ByteTap {
            point,
            capacity,
            ring: VecDeque::with_capacity(capacity),
            skipped_since_last_poll: 0,
        }
    }

    /// Where this tap sits in the byte pipeline.
    pub fn point(&self) -> TapPoint {
        self.point
    }

    /// The fixed ring bound this tap was constructed with.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Items currently buffered, awaiting [`ByteTap::poll`]. Never exceeds
    /// [`ByteTap::capacity`] — useful for a caller that wants to watch a
    /// tap's backlog before it starts lagging.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// `true` if no items are currently buffered.
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// Producer side: record one observed byte unit with its arrival time.
    ///
    /// **Never blocks, never errors, never grows the ring past
    /// [`ByteTap::capacity`].** When the ring is already full, the oldest
    /// buffered item is evicted to make room and a skip is recorded for the
    /// next [`ByteTap::poll`] to report — the producer always completes in
    /// O(1) regardless of whether anything has ever called `poll`. See the
    /// [module docs](self) for why this is the correct trade and what it
    /// costs a slow consumer.
    pub fn record(&mut self, bytes: Bytes, at: Timestamp) {
        if self.ring.len() >= self.capacity {
            self.ring.pop_front();
            self.skipped_since_last_poll = self.skipped_since_last_poll.saturating_add(1);
        }
        self.ring.push_back((bytes, at));
    }

    /// Consumer side: pull the next observed item.
    ///
    /// Returns [`TapItem::Lagged`] first if any items were evicted since the
    /// last poll — a consumer cannot skip past it to reach the data that
    /// follows a gap. Returns `None` when the ring is empty and no loss is
    /// pending.
    pub fn poll(&mut self) -> Option<TapItem> {
        if self.skipped_since_last_poll > 0 {
            let skipped = self.skipped_since_last_poll;
            self.skipped_since_last_poll = 0;
            return Some(TapItem::Lagged { skipped });
        }
        self.ring.pop_front().map(|(b, t)| TapItem::Data(b, t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// A tap must yield bytes a demuxer would reject — bad sync byte, TEI
    /// set, unaligned length — unaltered, with the timestamp it was recorded
    /// with. `ByteTap` performs no validation, so this is really testing
    /// that `record`/`poll` are a plain pass-through, not a filter.
    #[test]
    fn tap_yields_bytes_a_demuxer_would_reject() {
        let mut tap = ByteTap::new(TapPoint::Wire, 4);
        // Sync byte 0x00 (not 0x47), TEI bit set (bit 7 of byte 1), and only
        // 5 bytes long (not the expected 188) — three separate reasons a
        // real TS demuxer / dvb-conformance would flag or drop this packet.
        let malformed = Bytes::from_static(&[0x00, 0x80, 0xFF, 0xFF, 0xFF]);
        tap.record(malformed.clone(), Timestamp::from_nanos(42));

        assert_eq!(
            tap.poll(),
            Some(TapItem::Data(malformed, Timestamp::from_nanos(42)))
        );
        assert_eq!(tap.poll(), None);
    }

    /// A slow consumer that never polls must not stop the producer, and must
    /// receive an accurate `Lagged { skipped }` the first time it does poll.
    #[test]
    fn slow_consumer_gets_accurate_lagged_and_producer_is_never_blocked() {
        let capacity = 4;
        let mut tap = ByteTap::new(TapPoint::Wire, capacity);

        // The producer records far more than capacity while the consumer
        // never once calls poll(). This must simply complete — there is no
        // blocking call in `record` for a test to hang on, but the
        // assertions below prove the *state* is what a non-blocking producer
        // would leave behind, not merely that the loop returned.
        let total_records: u64 = 1_000;
        for i in 0..total_records {
            tap.record(
                Bytes::copy_from_slice(&i.to_be_bytes()),
                Timestamp::from_nanos(i),
            );
        }
        // The producer completed regardless of consumer progress: exactly
        // `capacity` items are resident, never more.
        assert_eq!(tap.len(), capacity);

        // First poll after falling behind must report the loss, with an
        // exact count: total records minus the `capacity` that fit.
        let expected_skipped = total_records - capacity as u64;
        assert_eq!(
            tap.poll(),
            Some(TapItem::Lagged {
                skipped: expected_skipped
            })
        );

        // After the Lagged report, the remaining ring drains as the last
        // `capacity` items recorded, oldest-first, with no further loss
        // reports mixed in.
        let mut drained = Vec::new();
        while let Some(item) = tap.poll() {
            drained.push(item);
        }
        assert_eq!(drained.len(), capacity);
        for (offset, item) in drained.into_iter().enumerate() {
            let expected_index = total_records - capacity as u64 + offset as u64;
            assert_eq!(
                item,
                TapItem::Data(
                    Bytes::copy_from_slice(&expected_index.to_be_bytes()),
                    Timestamp::from_nanos(expected_index)
                )
            );
        }
    }

    /// Flooding the ring at scale cannot grow memory without limit: `len()`
    /// must never exceed the configured `capacity`, no matter how many
    /// `record` calls are made.
    #[test]
    fn tap_ring_is_bounded_under_flood() {
        let capacity = 8;
        let mut tap = ByteTap::new(TapPoint::PostTransform, capacity);
        for i in 0..200_000u64 {
            tap.record(Bytes::from_static(b"x"), Timestamp::from_nanos(i));
            assert!(tap.len() <= capacity, "ring exceeded capacity mid-flood");
        }
        assert_eq!(tap.len(), capacity);
        assert_eq!(tap.capacity(), capacity);
    }

    #[test]
    fn empty_tap_polls_none() {
        let mut tap = ByteTap::new(TapPoint::Wire, 2);
        assert_eq!(tap.poll(), None);
        assert!(tap.is_empty());
    }

    #[test]
    #[should_panic(expected = "capacity must be > 0")]
    fn zero_capacity_panics() {
        let _ = ByteTap::new(TapPoint::Wire, 0);
    }
}
