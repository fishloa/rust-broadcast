//! [`Trunk`] — the sample ring, [`TrunkWriter`], and [`SampleCursor`] (plan
//! step 3b-i).
//!
//! This is the **sample path only**: the ring, the single writer, the two
//! retention classes, and the cursor that reads them. The segment log and the
//! 90 kHz event log
//! (`docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §1.2) are
//! later steps (3b-ii, 3b-iii) of the same plan — see
//! [Where the rest of `Trunk` attaches](#where-the-rest-of-trunk-attaches)
//! below for why nothing here needs to change shape to make room for them.
//!
//! # Why this module needs `std`, unlike its byte-layer siblings
//!
//! [`crate::byte_stage`], [`crate::byte_tap`], and [`crate::byte_merge`] are
//! `no_std` because each is driven synchronously by a single caller — there is
//! no cross-thread sharing to arrange. `Trunk` is different in kind: one
//! writer thread (ingest) and an unbounded set of reader threads (egress,
//! analysis, DVR) must observe the *same* ring concurrently. That needs a
//! shared, lockable interior — `std::sync::{Arc, Mutex}` here, matching
//! exactly the shape validated by `spikes/trunk-bench` (§3.1 of the spec).
//! Pulling in a `no_std` spinlock crate just to keep this one module
//! `no_std`-capable was considered and rejected: every real `Trunk` consumer
//! (`IngestSession`, `PushEgress`/`SegmentEgress`/`ServedEgress` impls) is
//! already `std`+`tokio` per the architecture, so there is no `no_std` caller
//! this would ever serve. This module is therefore `#[cfg(feature = "std")]`
//! — `media-plane --no-default-features` builds clean without it, exactly
//! like every other `std`-only corner of the workspace.
//!
//! # The benchmark verdict this design is built around
//!
//! `spikes/trunk-bench` (commit `acdbf3d0`) measured the naive
//! one-`Mutex`-guarded-log shape this module implements: **PASS** at the
//! specced scale (200-track MPTS × 6 readers, 999.97/1000 Mbit/s sustained,
//! publish mean 5.6 µs / p99 44.3 µs against a ~111 µs budget), but it
//! **refuted the original O(1)-fan-out premise** — writer cost is **O(N) in
//! cursor count** (956 ns → 9.98 µs from 1 → 16 readers), because writer and
//! readers all contend one shared `Mutex`.
//!
//! **Consequence, stated where it will actually be read:** see
//! [`Trunk::subscribe`]. A cursor is for a distinct *consumer of the stream*
//! — never one per peer of a one-to-many protocol. There is no tee, no
//! broadcast channel, and no per-consumer queue here, and there will not be
//! one added to chase higher fan-out: fan-out *is* `subscribe()`, and a
//! sample's payload is already [`bytes::Bytes`], so handing a clone to a
//! second, third, or sixteenth reader is a refcount bump, not a copy (see
//! [Zero-copy fan-out](#zero-copy-fan-out-honestly) below). If a route needs
//! to serve hundreds or thousands of peers (LL-HLS, WHEP), it takes **one**
//! cursor here and fans out to its peers itself, at the layer that already
//! has to hold per-peer state (congestion window, pacing epoch, SRTP
//! context) anyway.
//!
//! # Two retention classes, and why they are two independent rings
//!
//! [`RetentionClass::Timed`] (regular-cadence media) and
//! [`RetentionClass::Sparse`] (irregular, semantically-critical entries — an
//! SCTE-35 splice cue, a subtitle sample) are **not** stored in one merged,
//! globally-ordered log. An earlier design considered exactly that: one
//! `VecDeque` in strict publish order, with `Sparse` entries migrated to a
//! small overflow buffer instead of being dropped when the main ring evicted
//! them. It was rejected as needless complexity for a property nothing
//! actually needs: nothing downstream reads a `Trunk` expecting a strict
//! chronological interleave of, say, video samples and SCTE-35 sections —
//! consumers correlate by PTS/DTS themselves, and the *real* requirement (see
//! [`RetentionClass::Sparse`]) is only that `Sparse` retention must never be
//! collateral damage from unrelated `Timed` churn. Two independently
//! capacity-bounded rings give that guarantee *by construction* — a flood of
//! video frames cannot evict a still-live splice cue, because there is
//! nowhere for it to reach it — while a single merged ring would have to
//! re-implement the same isolation by hand (the rejected overflow-buffer
//! design above), for no observable benefit. [`SampleCursor::poll`] merges
//! the two rings only at read time, and documents the (best-effort, not
//! globally-ordered) precedence it uses.
//!
//! # Zero-copy fan-out, honestly
//!
//! **This claim was made falsely on this project before**: an earlier
//! zero-copy fan-out claim was proven only by a test that sliced `Bytes`
//! itself, while the crate under test contained zero `.slice()` calls of its
//! own — i.e. the test manufactured the evidence it was supposed to be
//! checking for. So, stated plainly: **the production path in this module
//! achieves zero-copy fan-out, not only the test.** the internal per-class
//! log stores the [`transmux::Sample`] handed to it; [`SampleCursor::poll`] returns it to
//! a reader via [`Clone::clone`] on the whole `Sample`, which clones
//! `Sample.data: Bytes` through `Bytes`'s own `Clone` impl — an `Arc`-style
//! refcount bump, not a byte copy. There is no `.slice()`, no
//! `Bytes::copy_from_slice`, and no re-allocation anywhere on this path. The
//! test in this module (`payload_is_shared_not_copied_across_cursors`)
//! asserts `Bytes::as_ptr()` *identity* across multiple cursors reading the
//! same published entry, precisely so it cannot be satisfied by two payloads
//! that merely have equal contents — and a mutation swapping the `clone()`
//! for a real copy is recorded as run against it (see that test's doc
//! comment).
//!
//! # Where the rest of `Trunk` attaches
//!
//! `TrunkState` (the shared state behind a `Trunk`) holds exactly the two
//! per-class logs this step needs. The
//! segment log (3b-ii) and the 90 kHz event log (3b-iii) are **sibling
//! fields on the same `TrunkState`, behind the same one `Mutex`** — not a
//! redesign: `TrunkState` gains `segments: SegmentLog` and `events: EventLog`
//! fields, `Trunk` gains `subscribe_segments()` and `events()` methods
//! shaped exactly like [`Trunk::subscribe`], and `TrunkConfig` gains their
//! capacity/window fields alongside `timed_capacity`/`sparse_capacity`. The
//! one-writer-never-blocks and per-cursor-lag-accounting shape established
//! here is intended to be reused verbatim, not reconsidered, when those two
//! logs land. `Trunk::writer()`'s one-writer enforcement
//! ([`Trunk::writer`]'s `AtomicBool`) also already covers the full `Trunk`,
//! not just the sample path — a single [`TrunkWriter`] will publish samples,
//! segments, and events alike once those exist.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use transmux::Sample;

/// Which retention discipline a published entry follows once inside the
/// [`Trunk`]'s sample ring.
///
/// Named `RetentionClass`, not `Retention` — plan step 3e's tiered hot/cold
/// archive policy (`docs/superpowers/plans/2026-07-26-media-plane-implementation.md`)
/// owns the name `Retention` for an unrelated, later concept. This is the
/// orthogonal, in-ring question of "how eagerly can this entry be evicted",
/// decided per [`TrunkWriter::publish`] call by whoever is feeding the
/// writer — it reflects a *track's* nature (video/audio vs. an SCTE-35
/// section PID), not something intrinsic to a [`transmux::Sample`] itself,
/// so it is not a field the spec's `Sample` type carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RetentionClass {
    /// Regular-cadence media samples (audio, video, ...): count-bounded, and
    /// ordinary eviction is reported to a lagging [`SampleCursor`] as
    /// [`SampleCursorItem::Lagged`] — a consumer that misses a video frame is
    /// gapped, not wrong; it resumes from the next sample.
    Timed,
    /// Irregular, semantically-critical entries — an SCTE-35 splice cue, a
    /// subtitle sample — where losing one leaves a consumer's *derived
    /// state* wrong, not merely gapped: a missed splice cue means splicing
    /// in the wrong place, or not at all.
    ///
    /// # The retention rule
    ///
    /// A `Sparse` entry lives in a ring bounded **independently** of the
    /// `Timed` ring ([`TrunkConfig::sparse_capacity`], separate from
    /// [`TrunkConfig::timed_capacity`]). It is therefore never evicted
    /// "merely because a time window rolled" on the unrelated `Timed`
    /// class: no volume of video/audio publishes can push a still-live
    /// splice cue out of the trunk, because `Timed` publishes never touch
    /// the `Sparse` ring at all. A `Sparse` entry is only evicted once
    /// `Sparse` publish volume *itself* exceeds the `Sparse` ring's own
    /// bound — and when that happens, [`SampleCursor::poll`] reports it as
    /// [`SampleCursorItem::Degraded`], not ordinary `Lagged`: a distinct,
    /// stronger signal, because the consumer's semantic state (e.g. "where
    /// the next ad break splices") is now wrong. A consumer that sees
    /// `Lagged` should simply resume from the next sample; a consumer that
    /// sees `Degraded` should treat its derived state as unsynchronised
    /// until the next authoritative signal (a fresh cue, a manifest
    /// reload) re-establishes it — resuming silently would splice on stale
    /// information.
    Sparse,
}

/// Construction parameters for a [`Trunk`].
///
/// `#[non_exhaustive]`: plan steps 3b-ii/3b-iii add segment-log and
/// event-log capacity/window fields here (see the
/// [module docs](self#where-the-rest-of-trunk-attaches)).
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct TrunkConfig {
    /// Bound, in entry count, on the [`RetentionClass::Timed`] ring.
    pub timed_capacity: usize,
    /// Bound, in entry count, on the [`RetentionClass::Sparse`] ring —
    /// independent of `timed_capacity`; see [`RetentionClass::Sparse`] for
    /// why that independence is the entire point of the retention rule.
    pub sparse_capacity: usize,
}

impl TrunkConfig {
    /// Build a config with both ring capacities. Neither is validated here —
    /// [`Trunk::new`] panics on a zero capacity, matching this crate's
    /// [`crate::byte_tap::ByteTap::new`]/[`crate::byte_merge::ByteMerge::new`]
    /// precedent of panicking at the point a ring is actually allocated.
    pub fn new(timed_capacity: usize, sparse_capacity: usize) -> Self {
        TrunkConfig {
            timed_capacity,
            sparse_capacity,
        }
    }
}

/// One retention class's bounded, append-ordered log of `(track_id, Sample)`
/// entries.
///
/// Bench-identical bounding: when full, the oldest entry is evicted and
/// `base` (the count of entries ever evicted from *this* log) advances by
/// one; `published` is the count of entries ever pushed. A cursor's lag for
/// this class is computed purely from `base` vs. how much of it the cursor
/// has consumed — see [`SampleCursor::poll`].
struct ClassLog {
    entries: VecDeque<(u32, Sample)>,
    base: u64,
    published: u64,
    capacity: usize,
}

impl ClassLog {
    fn new(capacity: usize) -> Self {
        ClassLog {
            entries: VecDeque::with_capacity(capacity),
            base: 0,
            published: 0,
            capacity,
        }
    }

    /// Push one entry, evicting the oldest if the log is already at
    /// `capacity`. Never rejects, never blocks — this is what lets
    /// [`TrunkWriter::publish`] complete unconditionally regardless of how
    /// far behind any reader has fallen.
    fn push(&mut self, track_id: u32, sample: Sample) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
            self.base += 1;
        }
        self.entries.push_back((track_id, sample));
        self.published += 1;
    }
}

/// The shared state behind one [`Trunk`]: today, the two [`ClassLog`]s. See
/// [module docs](self#where-the-rest-of-trunk-attaches) for the sibling
/// fields 3b-ii/3b-iii add here.
struct TrunkState {
    timed: ClassLog,
    sparse: ClassLog,
}

/// The sample ring: bounded, dual-retention-class, single-writer,
/// multi-cursor. See the [module docs](self) for the design this
/// implements and the benchmark that shaped it.
///
/// Always held as `Arc<Trunk>` — [`Trunk::writer`] and [`Trunk::subscribe`]
/// take `self: &Arc<Self>` because a [`TrunkWriter`]/[`SampleCursor`] each
/// need to keep the shared state alive independently of the `Trunk` handle
/// that created them, exactly as `spikes/trunk-bench`'s validated shape
/// does.
pub struct Trunk {
    state: Mutex<TrunkState>,
    writer_taken: AtomicBool,
}

impl Trunk {
    /// Construct a fresh, empty `Trunk`.
    ///
    /// Panics if either `config.timed_capacity` or `config.sparse_capacity`
    /// is zero — a construction mistake (every entry would be evicted the
    /// instant it was pushed), not remote input, so it panics rather than
    /// returning a `Result` a caller could ignore (matching
    /// [`crate::byte_tap::ByteTap::new`]/[`crate::byte_merge::ByteMerge::new`]).
    pub fn new(config: TrunkConfig) -> Arc<Trunk> {
        assert!(
            config.timed_capacity > 0,
            "Trunk timed_capacity must be > 0"
        );
        assert!(
            config.sparse_capacity > 0,
            "Trunk sparse_capacity must be > 0"
        );
        Arc::new(Trunk {
            state: Mutex::new(TrunkState {
                timed: ClassLog::new(config.timed_capacity),
                sparse: ClassLog::new(config.sparse_capacity),
            }),
            writer_taken: AtomicBool::new(false),
        })
    }

    /// Take the one [`TrunkWriter`] for this `Trunk`.
    ///
    /// Returns `None` on every call after the first — a `Trunk` has exactly
    /// one writer, enforced here rather than left as a documented-only
    /// convention, because a second concurrent writer would silently
    /// interleave two unrelated publish sequences into one ring with no way
    /// for a reader to tell them apart.
    pub fn writer(self: &Arc<Self>) -> Option<TrunkWriter> {
        self.writer_taken
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| TrunkWriter {
                trunk: Arc::clone(self),
            })
    }

    /// Subscribe a new [`SampleCursor`], starting from *now* — the next
    /// entry [`TrunkWriter::publish`] produces after this call, not any
    /// backlog already in either ring. (A later step may add a seek-to-past
    /// variant once the segment log gives "a past point" a real meaning;
    /// this step does not need one.)
    ///
    /// # This call *is* fan-out — read this before calling it per connection
    ///
    /// `spikes/trunk-bench` measured writer cost as **O(N) in cursor
    /// count** (956 ns → 9.98 µs from 1 → 16 readers; spec §3.1) — every
    /// cursor contends the same shared lock every publish. **A cursor is
    /// for a distinct consumer of the stream** (a segmenter, a DVR writer,
    /// an analysis tap, one push relay) — **never** one per peer of a
    /// one-to-many protocol. Supported reader count is **single-digit by
    /// design**: LL-HLS serving a thousand viewers takes **one** cursor
    /// here and fans out to its viewers itself, at the layer that already
    /// holds per-viewer state anyway. Do not call this once per connection;
    /// there is no tee, broadcast channel, or per-consumer queue to reach
    /// for instead — a sample's payload is already [`bytes::Bytes`], so
    /// fan-out beyond this one cursor is a refcount bump the relay performs
    /// itself, not something this type needs to do for you.
    pub fn subscribe(self: &Arc<Self>) -> SampleCursor {
        let state = self.state.lock().unwrap();
        SampleCursor {
            trunk: Arc::clone(self),
            timed_consumed: state.timed.published,
            sparse_consumed: state.sparse.published,
        }
    }

    /// Diagnostic: entries currently resident in the `Timed` ring. Never
    /// exceeds [`TrunkConfig::timed_capacity`].
    pub fn timed_len(&self) -> usize {
        self.state.lock().unwrap().timed.entries.len()
    }

    /// Diagnostic: entries currently resident in the `Sparse` ring. Never
    /// exceeds [`TrunkConfig::sparse_capacity`].
    pub fn sparse_len(&self) -> usize {
        self.state.lock().unwrap().sparse.entries.len()
    }
}

/// The one writer for a [`Trunk`]. Obtained via [`Trunk::writer`].
///
/// `publish` never blocks and never rejects: a full class ring evicts its
/// oldest entry (see the internal per-class log's push logic) rather than waiting for a reader or
/// erroring, so ingest never stalls because some [`SampleCursor`] is slow —
/// the same non-blocking-producer principle as [`crate::byte_tap::ByteTap::record`],
/// for the same reason (a broadcast head-end does not pause live ingest for
/// a lagging analysis tap or a stalled egress peer).
///
/// "Never blocks" describes the absence of any wait-for-a-reader code path,
/// not a claim that the underlying `Mutex` critical section is instant —
/// `publish` briefly contends the same lock [`SampleCursor::poll`] does, a
/// bounded amount of work independent of how far behind any reader is (this
/// is exactly what `spikes/trunk-bench` measured as the O(N)-in-cursor-count
/// cost, not an unbounded wait).
pub struct TrunkWriter {
    trunk: Arc<Trunk>,
}

impl TrunkWriter {
    /// Publish one sample for `track_id` under `retention`.
    pub fn publish(&self, track_id: u32, retention: RetentionClass, sample: Sample) {
        let mut state = self.trunk.state.lock().unwrap();
        match retention {
            RetentionClass::Timed => state.timed.push(track_id, sample),
            RetentionClass::Sparse => state.sparse.push(track_id, sample),
        }
    }
}

/// One item [`SampleCursor::poll`] can hand back: data from either retention
/// class, or a loss report.
///
/// `#[non_exhaustive]`: this is the growth point for anything a cursor might
/// need to surface beyond "sample" or "loss" later, without a breaking
/// change to every match arm in the workspace.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SampleCursorItem {
    /// A [`RetentionClass::Timed`] sample for `track_id`.
    Timed {
        /// The publishing track.
        track_id: u32,
        /// The sample itself. Cloned from the ring's stored copy —
        /// `Sample.data: Bytes` is shared, not copied; see
        /// [Zero-copy fan-out](self#zero-copy-fan-out-honestly).
        sample: Sample,
    },
    /// A [`RetentionClass::Sparse`] sample for `track_id`.
    Sparse {
        /// The publishing track.
        track_id: u32,
        /// The sample itself; see the `Timed` variant's doc for the
        /// zero-copy note.
        sample: Sample,
    },
    /// This cursor fell behind the `Timed` ring: `skipped` entries were
    /// evicted before it read them. Ordinary loss — resume from the next
    /// sample; see [`RetentionClass::Timed`].
    Lagged {
        /// Exact count of `Timed` entries evicted since this cursor's last
        /// successful read of that class.
        skipped: u64,
    },
    /// This cursor fell behind the `Sparse` ring: `skipped` entries were
    /// evicted before it read them. **Not** ordinary loss — the consumer's
    /// derived state (e.g. splice-point tracking) is now wrong, not merely
    /// gapped; see [`RetentionClass::Sparse`] for what a consumer is
    /// expected to do about it.
    Degraded {
        /// Exact count of `Sparse` entries evicted since this cursor's last
        /// successful read of that class.
        skipped: u64,
    },
}

/// A subscribed reader of a [`Trunk`]'s sample ring. Obtained via
/// [`Trunk::subscribe`] — **read that method's docs before creating more
/// than a handful of these.**
pub struct SampleCursor {
    trunk: Arc<Trunk>,
    /// How many `Timed` entries this cursor has consumed (returned via
    /// `poll`, or accounted for via a reported `Lagged`) since it
    /// subscribed. Compared against the shared `ClassLog::base` to detect
    /// loss — the same technique as `spikes/trunk-bench`'s `Cursor::read_seq`
    /// vs. `TrunkInner::base_seq`.
    timed_consumed: u64,
    /// The `Sparse`-class equivalent of `timed_consumed`.
    sparse_consumed: u64,
}

impl SampleCursor {
    /// Pull the next item, if any is ready.
    ///
    /// Loss is always reported before further data, in the same
    /// `Option<SampleCursorItem>` as real samples — following
    /// [`crate::byte_tap::TapItem`]'s precedent: a consumer cannot poll past
    /// a `Lagged`/`Degraded` report to reach the data that follows a gap,
    /// because there is no side channel it could forget to check instead.
    ///
    /// # Merge order across the two retention classes
    ///
    /// A pending `Timed` lag report is checked first, then a pending
    /// `Sparse` lag report, then a ready `Sparse` sample, then a ready
    /// `Timed` sample. This gives **no cross-class chronological interleave
    /// guarantee** (see the [module docs](self) for why that is not
    /// something anything downstream needs) — only that, within each
    /// class, entries are returned in the exact order
    /// [`TrunkWriter::publish`] produced them, with no duplication and no
    /// unreported loss.
    pub fn poll(&mut self) -> Option<SampleCursorItem> {
        let state = self.trunk.state.lock().unwrap();

        if self.timed_consumed < state.timed.base {
            let skipped = state.timed.base - self.timed_consumed;
            self.timed_consumed = state.timed.base;
            return Some(SampleCursorItem::Lagged { skipped });
        }
        if self.sparse_consumed < state.sparse.base {
            let skipped = state.sparse.base - self.sparse_consumed;
            self.sparse_consumed = state.sparse.base;
            return Some(SampleCursorItem::Degraded { skipped });
        }

        let sparse_idx = (self.sparse_consumed - state.sparse.base) as usize;
        if let Some((track_id, sample)) = state.sparse.entries.get(sparse_idx) {
            self.sparse_consumed += 1;
            return Some(SampleCursorItem::Sparse {
                track_id: *track_id,
                sample: sample.clone(),
            });
        }

        let timed_idx = (self.timed_consumed - state.timed.base) as usize;
        if let Some((track_id, sample)) = state.timed.entries.get(timed_idx) {
            self.timed_consumed += 1;
            return Some(SampleCursorItem::Timed {
                track_id: *track_id,
                sample: sample.clone(),
            });
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn sample(byte: u8, len: usize) -> Sample {
        Sample::new(Bytes::from(vec![byte; len]), Some(0), Some(0), None, true)
    }

    fn timed_data(item: &SampleCursorItem) -> Option<(u32, &Sample)> {
        match item {
            SampleCursorItem::Timed { track_id, sample } => Some((*track_id, sample)),
            _ => None,
        }
    }

    /// Drains up to `n` items from `cursor`, stopping early if `poll`
    /// returns `None` — a bounded collection loop so a mutation that never
    /// advances `*_consumed` (and would otherwise re-yield the same item
    /// forever) fails the test's length/content assertions instead of
    /// hanging it.
    fn drain(cursor: &mut SampleCursor, n: usize) -> Vec<SampleCursorItem> {
        let mut out = Vec::new();
        for _ in 0..n {
            match cursor.poll() {
                Some(item) => out.push(item),
                None => break,
            }
        }
        out
    }

    // --- 1. multiple cursors, every sample, in order, no dup/no loss -----

    /// MUTATION VERIFIED: removing `self.timed_consumed += 1;` from the
    /// `Timed`-data return arm of `SampleCursor::poll` (so the same ring
    /// index is re-read every call) makes this test fail — `drain` still
    /// returns exactly 5 items (poll never runs out), but they are five
    /// copies of the first published sample (`byte = 0`) instead of the
    /// distinct sequence `0..5`, so the `assert_eq!` on the reconstructed
    /// byte sequence fails with a mismatch at index 1. Recompiled and
    /// re-run to confirm the failure, then reverted.
    #[test]
    fn multiple_cursors_see_every_sample_in_order_with_no_dup_or_loss() {
        let trunk = Trunk::new(TrunkConfig::new(100, 10));
        let mut c1 = trunk.subscribe();
        let mut c2 = trunk.subscribe();
        let mut c3 = trunk.subscribe();
        let writer = trunk.writer().unwrap();

        for i in 0u8..5 {
            writer.publish(7, RetentionClass::Timed, sample(i, 16));
        }

        for cursor in [&mut c1, &mut c2, &mut c3] {
            let items = drain(cursor, 5);
            assert_eq!(items.len(), 5, "each cursor must see exactly 5 samples");
            let bytes: Vec<u8> = items
                .iter()
                .map(|item| timed_data(item).unwrap().1.data[0])
                .collect();
            assert_eq!(bytes, vec![0, 1, 2, 3, 4], "must be in publish order");
            assert!(cursor.poll().is_none(), "no extra/duplicated items");
        }
    }

    // --- 2. slow reader lags, writer completes regardless -----------------

    /// MUTATION VERIFIED: changing `ClassLog::push`'s eviction condition from
    /// `self.entries.len() == self.capacity` to `false` (i.e. disabling
    /// eviction, simulating a writer that would instead have to wait/reject
    /// once "full") makes `trunk.timed_len()` grow to 1024 instead of
    /// staying at the configured cap of 4, and the lag report's `skipped`
    /// reads back as `0` (base never advances), not `1020`. Recompiled and
    /// re-run to confirm the failure, then reverted.
    #[test]
    fn slow_reader_lags_but_writer_completes_regardless() {
        let trunk = Trunk::new(TrunkConfig::new(4, 10));
        let mut slow = trunk.subscribe();
        let writer = trunk.writer().unwrap();

        // The slow reader never polls while 1024 samples are published —
        // there is no wait-for-reader code path in `publish` for this loop
        // to block on (see `TrunkWriter`'s docs), so this simply completes.
        // A single thread is sufficient to demonstrate this: the absence of
        // a blocking path is a structural property of `publish`, not a race
        // that needs real concurrency to expose (`crate::byte_tap`'s
        // equivalent test uses the same reasoning).
        for i in 0u8..=255u8 {
            for _ in 0..4 {
                writer.publish(1, RetentionClass::Timed, sample(i, 8));
            }
        }
        // 256 * 4 = 1024 published; ring capacity is 4.
        assert_eq!(
            trunk.timed_len(),
            4,
            "writer unblocked: ring stayed bounded"
        );

        let first = slow.poll().unwrap();
        assert!(
            matches!(first, SampleCursorItem::Lagged { skipped: 1020 }),
            "expected Lagged{{skipped: 1020}}, got {first:?}"
        );
    }

    // --- 3. lag reports an accurate skipped count -------------------------

    /// MUTATION VERIFIED: changing the `skipped` computation in
    /// `SampleCursor::poll`'s `Timed`-lag branch from
    /// `state.timed.base - self.timed_consumed` to
    /// `state.timed.base - self.timed_consumed + 1` makes this test fail:
    /// expected `skipped: 6`, got `skipped: 7`. Recompiled and re-run to
    /// confirm the failure, then reverted.
    #[test]
    fn lag_is_reported_with_an_accurate_skipped_count() {
        let trunk = Trunk::new(TrunkConfig::new(3, 10));
        let mut cursor = trunk.subscribe();
        let writer = trunk.writer().unwrap();

        // Capacity 3, publish 9: 6 evicted before the cursor ever reads.
        for i in 0u8..9 {
            writer.publish(2, RetentionClass::Timed, sample(i, 4));
        }

        let first = cursor.poll().unwrap();
        assert!(
            matches!(first, SampleCursorItem::Lagged { skipped: 6 }),
            "expected Lagged{{skipped: 6}}, got {first:?}"
        );

        // The remaining 3 (bytes 6,7,8) must still be readable, in order.
        let items = drain(&mut cursor, 3);
        let bytes: Vec<u8> = items
            .iter()
            .map(|item| timed_data(item).unwrap().1.data[0])
            .collect();
        assert_eq!(bytes, vec![6, 7, 8]);
        assert!(cursor.poll().is_none());
    }

    // --- 4. Sparse loss reports Degraded, distinct from Timed's Lagged ----

    /// MUTATION VERIFIED: changing the `Sparse`-lag branch of
    /// `SampleCursor::poll` to also return `SampleCursorItem::Lagged` (i.e.
    /// collapsing the two variants) makes the
    /// `matches!(item, SampleCursorItem::Degraded { .. })` assertion below
    /// fail — the item is a `Lagged` instead. Recompiled and re-run to
    /// confirm the failure, then reverted.
    #[test]
    fn sparse_reader_loses_data_reports_degraded_distinguishable_from_timed_lagged() {
        let trunk = Trunk::new(TrunkConfig::new(2, 2));
        let mut cursor = trunk.subscribe();
        let writer = trunk.writer().unwrap();

        // Overflow the Timed ring (cap 2) with 5 publishes: ordinary loss.
        for i in 0u8..5 {
            writer.publish(3, RetentionClass::Timed, sample(i, 4));
        }
        // Overflow the Sparse ring (cap 2) with 4 publishes: escalated loss.
        for i in 0u8..4 {
            writer.publish(9, RetentionClass::Sparse, sample(100 + i, 4));
        }

        let timed_loss = cursor.poll().unwrap();
        assert!(
            matches!(timed_loss, SampleCursorItem::Lagged { skipped: 3 }),
            "expected ordinary Lagged{{skipped: 3}} for the Timed ring, got {timed_loss:?}"
        );

        let sparse_loss = cursor.poll().unwrap();
        assert!(
            matches!(sparse_loss, SampleCursorItem::Degraded { skipped: 2 }),
            "expected escalated Degraded{{skipped: 2}} for the Sparse ring, got {sparse_loss:?}"
        );
        assert_ne!(
            core::mem::discriminant(&timed_loss),
            core::mem::discriminant(&sparse_loss),
            "Lagged and Degraded must be distinct variants, not merely different field values"
        );
    }

    // --- 5. the ring is bounded: flooding cannot grow memory unboundedly --

    /// MUTATION VERIFIED: removing the eviction check in `ClassLog::push`
    /// (replacing `if self.entries.len() == self.capacity { .. }` with a
    /// no-op) makes `trunk.timed_len()`/`trunk.sparse_len()` grow well past
    /// the configured caps (`4`/`3`) instead of staying bounded — the
    /// assertions inside the flood loop below fail on the first
    /// over-capacity iteration. Recompiled and re-run to confirm the
    /// failure, then reverted.
    #[test]
    fn ring_is_bounded_under_flood_on_both_classes() {
        let trunk = Trunk::new(TrunkConfig::new(4, 3));
        let writer = trunk.writer().unwrap();

        for i in 0u32..50_000 {
            writer.publish(5, RetentionClass::Timed, sample((i % 256) as u8, 2));
            assert!(
                trunk.timed_len() <= 4,
                "Timed ring exceeded its cap mid-flood"
            );
            if i % 7 == 0 {
                writer.publish(6, RetentionClass::Sparse, sample((i % 256) as u8, 2));
                assert!(
                    trunk.sparse_len() <= 3,
                    "Sparse ring exceeded its cap mid-flood"
                );
            }
        }
        assert_eq!(trunk.timed_len(), 4);
        assert_eq!(trunk.sparse_len(), 3);
    }

    // --- 6. payload sharing: Bytes::as_ptr() identity, not equality -------

    /// MUTATION VERIFIED: replacing `sample.clone()` in both of
    /// `SampleCursor::poll`'s data-return arms with a hand-rolled copy
    /// (`Sample::new(Bytes::copy_from_slice(sample.data.as_ref()), ...)`,
    /// preserving every field's *value* so content-equality would still
    /// hold) makes this test's pointer-identity assertion fail — with the
    /// mutation, `p1 == p2` is `false` (two distinct heap allocations with
    /// equal contents), whereas the unmutated `clone()` path yields
    /// `p1 == p2 == p3`. This is precisely the distinction a
    /// content-equality assertion would have missed. Recompiled and re-run
    /// to confirm the failure, then reverted.
    #[test]
    fn payload_is_shared_not_copied_across_cursors() {
        let trunk = Trunk::new(TrunkConfig::new(8, 8));
        let mut c1 = trunk.subscribe();
        let mut c2 = trunk.subscribe();
        let mut c3 = trunk.subscribe();
        let writer = trunk.writer().unwrap();

        writer.publish(4, RetentionClass::Timed, sample(0xAB, 65536));

        let i1 = c1.poll().unwrap();
        let i2 = c2.poll().unwrap();
        let i3 = c3.poll().unwrap();
        let p1 = timed_data(&i1).unwrap().1.data.as_ptr();
        let p2 = timed_data(&i2).unwrap().1.data.as_ptr();
        let p3 = timed_data(&i3).unwrap().1.data.as_ptr();

        assert_eq!(
            p1, p2,
            "cursor 2's payload must be the SAME allocation as cursor 1's"
        );
        assert_eq!(
            p2, p3,
            "cursor 3's payload must be the SAME allocation as cursor 1's"
        );
        // Not just equal contents (that would also pass for two independent
        // 64KiB copies) — the ptr comparison above is the real assertion;
        // this just confirms the payload wasn't corrupted in the process.
        assert_eq!(timed_data(&i1).unwrap().1.data.len(), 65536);
    }

    // --- Construction invariants -------------------------------------------

    #[test]
    #[should_panic(expected = "timed_capacity must be > 0")]
    fn zero_timed_capacity_panics() {
        let _ = Trunk::new(TrunkConfig::new(0, 4));
    }

    #[test]
    #[should_panic(expected = "sparse_capacity must be > 0")]
    fn zero_sparse_capacity_panics() {
        let _ = Trunk::new(TrunkConfig::new(4, 0));
    }

    #[test]
    fn second_writer_is_refused() {
        let trunk = Trunk::new(TrunkConfig::new(4, 4));
        let _first = trunk.writer().unwrap();
        assert!(trunk.writer().is_none(), "a Trunk has exactly one writer");
    }

    #[test]
    fn subscribe_starts_from_now_not_from_history() {
        let trunk = Trunk::new(TrunkConfig::new(4, 4));
        let writer = trunk.writer().unwrap();
        writer.publish(1, RetentionClass::Timed, sample(1, 4));
        writer.publish(1, RetentionClass::Timed, sample(2, 4));

        // Subscribing after two publishes must not see either of them.
        let mut cursor = trunk.subscribe();
        assert!(cursor.poll().is_none());

        writer.publish(1, RetentionClass::Timed, sample(3, 4));
        let item = cursor.poll().unwrap();
        assert_eq!(timed_data(&item).unwrap().1.data[0], 3);
    }
}
