//! Accounting for a `Trunk` cursor drained while a writer publishes.
//!
//! A `Trunk` never backpressures its writer, so a cursor reading *concurrently*
//! with an unpaced writer may lawfully fall behind the ring; whether it does is
//! the scheduler's decision, never the property a test is checking. A memory or
//! throughput test therefore counts what the ring reports evicted and asserts
//! `drained + missed == generated`. A content test drains nothing until the
//! writer has finished (or sizes its ring above the whole stream), so no
//! eviction is possible by construction and [`Drained::assert_lossless`] is a
//! real invariant rather than a scheduling claim.

use media_plane::{SampleCursor, SampleCursorItem};
use transmux::TrackId;

/// What one cursor saw: the timed samples it read, and every loss the ring
/// reported in place of a sample it could not deliver.
#[derive(Debug, Default)]
pub struct Drained {
    delivered: u64,
    missed: u64,
    sparse: u64,
    degraded: u64,
}

impl Drained {
    /// Record one cursor item, returning the timed sample when the item is one.
    pub fn observe(
        &mut self,
        item: SampleCursorItem,
    ) -> Option<(TrackId, transmux::pipeline::Sample)> {
        match item {
            SampleCursorItem::Timed { track_id, sample } => {
                self.delivered += 1;
                Some((track_id, sample))
            }
            SampleCursorItem::Sparse { .. } => {
                self.sparse += 1;
                None
            }
            SampleCursorItem::Lagged { skipped } => {
                self.missed += skipped;
                None
            }
            SampleCursorItem::Degraded { skipped } => {
                self.degraded += skipped;
                None
            }
            SampleCursorItem::TracksLagged { skipped, .. } => {
                self.missed += skipped;
                None
            }
            SampleCursorItem::TracksChanged { .. }
            | SampleCursorItem::Discontinuity
            | SampleCursorItem::HandoversLagged { .. } => None,
            _ => None,
        }
    }

    /// Timed samples the cursor read.
    pub fn delivered(&self) -> u64 {
        self.delivered
    }

    /// Timed samples the ring evicted before the cursor read them.
    pub fn missed(&self) -> u64 {
        self.missed
    }

    /// Every timed sample the trunk was given: what the cursor read plus what
    /// the ring reports it evicted before the cursor got to it.
    pub fn total(&self) -> u64 {
        self.delivered + self.missed
    }

    /// Assert nothing was lost: only valid when the cursor drains after the
    /// writer finished, or with a ring at least as large as the whole stream.
    pub fn assert_lossless(&self, what: &str) {
        assert_eq!(
            self.missed, 0,
            "{what}: the trunk evicted {} timed sample(s) before the cursor read them",
            self.missed
        );
        assert_eq!(
            self.sparse, 0,
            "{what}: the trunk carried {} sparse sample(s) the file reader never writes",
            self.sparse
        );
        assert_eq!(
            self.degraded, 0,
            "{what}: the trunk evicted {} sparse sample(s) before the cursor read them",
            self.degraded
        );
    }
}

/// Drain every item `cursor` holds right now, handing each timed sample to
/// `on_timed`. Use this after the writer has finished: eviction then depends
/// only on the ring's size against the stream, never on the scheduler.
pub fn drain_now(
    cursor: &mut SampleCursor,
    mut on_timed: impl FnMut(TrackId, transmux::pipeline::Sample),
) -> Drained {
    let mut drained = Drained::default();
    while let Some(item) = cursor.poll() {
        if let Some((track_id, sample)) = drained.observe(item) {
            on_timed(track_id, sample);
        }
    }
    drained
}

/// Drain every item `cursor` has now and, while `finished` is false, keep
/// draining as the writer publishes, handing each timed sample to `on_timed`.
pub async fn drain(
    cursor: &mut SampleCursor,
    finished: impl Fn() -> bool,
    mut on_timed: impl FnMut(TrackId, transmux::pipeline::Sample),
) -> Drained {
    let mut drained = Drained::default();
    loop {
        while let Some(item) = cursor.poll() {
            if let Some((track_id, sample)) = drained.observe(item) {
                on_timed(track_id, sample);
            }
        }
        if finished() {
            return drained;
        }
        tokio::task::yield_now().await;
    }
}
