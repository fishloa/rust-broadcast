//! [`Trunk`] — the sample ring, [`TrunkWriter`], and [`SampleCursor`] (plan
//! step 3b-i), and now the segment log, [`SegmentCursor`], and the
//! lossless-by-retention pinning mechanism (plan step 3b-ii).
//!
//! This is the sample path plus the segment log: two bounded rings, the one
//! writer that publishes into both, and the cursors that read them. The
//! 90 kHz event log
//! (`docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §1.2) is
//! the one remaining piece (3b-iii) of the same plan — see
//! [Where the rest of `Trunk` attaches](#where-the-rest-of-trunk-attaches)
//! below for why nothing here needs to change shape to make room for it.
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
//! The segment log added in this step is a **sibling ring behind the same
//! one `Mutex`** (the internal `TrunkState`), not a second lock — a
//! [`SegmentCursor`] contends exactly the lock a [`SampleCursor`] does, so
//! the same O(N)-in-cursor-count rule and the same single-digit-reader
//! guidance apply to it verbatim; see [`Trunk::subscribe_segments`] and
//! [`Trunk::pin_segments`].
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
//! The segment log added in this step makes the **same** claim, honestly, on
//! the **same** terms: [`SegmentEntry::bytes`] is [`bytes::Bytes`],
//! [`SegmentCursor::poll`] hands it back via `Clone` on the whole
//! [`SegmentEntry`] (which clones `Bytes` through `Bytes`'s own `Clone`), and
//! there is no `.slice()`/`copy_from_slice`/re-allocation anywhere on that
//! path either. `segment_bytes_are_shared_not_copied_across_cursors` asserts
//! the same pointer-identity property for segments that
//! `payload_is_shared_not_copied_across_cursors` asserts for samples — this
//! is the **production** path achieving zero-copy fan-out, not a test
//! manufacturing its own evidence.
//!
//! # The DVR contradiction: losslessness from retention, not back-pressure
//!
//! A DVR/archive consumer must not miss a segment — a hole in a recording is
//! a defect, not a degradation, unlike a dropped video frame. But the writer
//! must **never** block, for exactly the reason stated everywhere else in
//! this module: a stalled archive writer must not stall live ingest. Those
//! two requirements contradict each other directly if "losslessness" is
//! implemented the obvious way — by making the writer wait for a slow
//! archive reader.
//!
//! **The resolution: losslessness comes from retention, not from
//! back-pressure.** A [`SegmentCursor`] obtained via [`Trunk::pin_segments`]
//! *pins* every segment it has not yet consumed — the log will not evict a
//! pinned entry as a matter of course, the way it freely evicts for an
//! ordinary [`Trunk::subscribe_segments`] cursor. "Consumed" here means
//! "returned by [`SegmentCursor::poll`]" — the same progress counter that
//! already governs in-order delivery does double duty as the pin floor,
//! rather than adding a second, explicit acknowledge-after-durable-write API
//! call. That two-call shape (poll to receive, then a separate `ack` once
//! the archive write actually lands on disk) was considered — it is the more
//! conservative choice, since a consumer that has polled a segment but not
//! yet finished writing it to disk is not truly safe from loss if the trunk
//! evicts under it — and rejected for *this* step: it doubles the API
//! surface and the bookkeeping (two offsets per pin instead of one) for a
//! distinction (poll's delivery vs. a durable write landing) this step has
//! no test that needs, since nothing downstream is implemented yet
//! (`docs/superpowers/plans/2026-07-26-media-plane-implementation.md` step
//! 3d's `SegmentEgress`/DVR writer is what would consume it). If that step
//! needs the finer-grained split, it is additive — a second, later
//! acknowledgement point on the same pin — not a breaking change to this
//! one.
//!
//! **Pinning is bounded, and by design there is no second capacity knob for
//! it**: a pin is measured against exactly [`TrunkConfig::segment_capacity`],
//! the same bound that governs ordinary eviction for every cursor. There is
//! no independent "how far behind may a pin fall" setting to tune
//! separately and get wrong. When the segment log is at capacity and the
//! next [`TrunkWriter::publish_segment`] would evict an entry some pin has
//! not yet consumed, the bound has been hit, and something genuinely has to
//! give — the caller decided what, in advance, via the [`ArchiveOverrun`]
//! passed to [`Trunk::pin_segments`]:
//!
//! - [`ArchiveOverrun::Gap`] (**the default**) — evict the pinned entry
//!   anyway, and tell that cursor it lost data
//!   ([`SegmentCursorItem::Gap`]). The recording gets a hole; the live
//!   stream and every other cursor are unaffected.
//! - [`ArchiveOverrun::StallIngest`] — apply real back-pressure:
//!   [`TrunkWriter::publish_segment`] blocks until this cursor consumes
//!   enough to release its pin (or is dropped). **The only place in this
//!   entire design where a reader may block the writer** — opt-in,
//!   documented loudly here and on the variant itself, and never the
//!   default.
//! - [`ArchiveOverrun::Terminate`] — drop the cursor's pin outright instead
//!   of gapping the recording or stalling ingest; the cursor is done
//!   ([`SegmentCursorItem::Terminated`]) and the log continues without it.
//!
//! This is a genuine three-way trade between the recording, the live
//! stream, and the archive consumer — **no option is free**, and there is
//! deliberately no fourth "just make it work" variant: any such variant
//! would have to secretly pick one of the three trade-offs above anyway
//! (drop bytes, block the writer, or drop the consumer), just without
//! naming which — which is worse, not better.
//!
//! # Where the rest of `Trunk` attaches
//!
//! `TrunkState` (the shared state behind a `Trunk`) now holds the two
//! per-class sample logs *and* the segment log this step adds. The 90 kHz
//! event log (3b-iii) is the one remaining **sibling field on the same
//! `TrunkState`, behind the same one `Mutex`** — not a redesign:
//! `TrunkState` gains an `events: EventLog` field, `Trunk` gains an
//! `events()` method shaped like [`Trunk::subscribe`]/
//! [`Trunk::subscribe_segments`], and `TrunkConfig` gains its window field
//! alongside `timed_capacity`/`sparse_capacity`/`segment_capacity`. The
//! one-writer-never-blocks-by-default and per-cursor-lag-accounting shape
//! established across both logs is intended to be reused verbatim, not
//! reconsidered, when the event log lands.
//!
//! `SegmentEgress` and tiered `Retention` (plan steps 3d/3e — an egress
//! trait that owns one [`SegmentCursor`] and pushes to DVR/MABR/ROUTE/Smooth,
//! and a hot/cold archive store behind it) are **not** built here, and their
//! attachment point is exactly [`Trunk::pin_segments`]: a `SegmentEgress`
//! implementation is the caller this step's [`ArchiveOverrun`] was written
//! for — it takes a pinning cursor with whichever policy its durability
//! contract requires (`StallIngest` for "this archive must never have a
//! hole", `Gap` for "best-effort is fine"), drains [`SegmentCursor::poll`],
//! and writes [`SegmentEntry::bytes`] to its store. Nothing in this step's
//! shape needs to change to make room for that; it is exactly the sample
//! path's `PushEgress`-owns-one-`SampleCursor` story repeated one layer up.
//! (`SegmentEgress`/`Retention` are named here only to document the
//! attachment point per this step's brief — neither type exists in this
//! crate yet.)

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use broadcast_common::stage::Timestamp;
use bytes::Bytes;
use transmux::{Sample, SegmentMeta};

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
/// `#[non_exhaustive]`: plan step 3b-iii adds an event-log window field here
/// (see the [module docs](self#where-the-rest-of-trunk-attaches)).
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct TrunkConfig {
    /// Bound, in entry count, on the [`RetentionClass::Timed`] ring.
    pub timed_capacity: usize,
    /// Bound, in entry count, on the [`RetentionClass::Sparse`] ring —
    /// independent of `timed_capacity`; see [`RetentionClass::Sparse`] for
    /// why that independence is the entire point of the retention rule.
    pub sparse_capacity: usize,
    /// Bound, in entry count, on the segment log. **Also** the bound a
    /// pinning [`SegmentCursor`]'s retention is measured against — see
    /// [The DVR contradiction](self#the-dvr-contradiction-losslessness-from-retention-not-back-pressure)
    /// for why there is deliberately no second, independent "pin depth"
    /// knob.
    pub segment_capacity: usize,
}

impl TrunkConfig {
    /// Build a config with all three ring capacities. None is validated
    /// here — [`Trunk::new`] panics on a zero capacity, matching this
    /// crate's [`crate::byte_tap::ByteTap::new`]/[`crate::byte_merge::ByteMerge::new`]
    /// precedent of panicking at the point a ring is actually allocated.
    pub fn new(timed_capacity: usize, sparse_capacity: usize, segment_capacity: usize) -> Self {
        TrunkConfig {
            timed_capacity,
            sparse_capacity,
            segment_capacity,
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

/// One finished media segment recorded by the segment log, in playlist
/// order.
///
/// Reuses [`transmux::SegmentMeta`] for exactly what it already models — the
/// per-segment discontinuity bit [`transmux::Segmenter::take_ready_with_meta`]
/// returns — by holding the whole type rather than copying its one field out
/// into a `discontinuous: bool` of this struct's own; a field `SegmentMeta`
/// gains later is picked up here for free. It does **not** fit whole,
/// though, and this struct says so rather than pretending it does: nothing
/// in `transmux` computes a segment's wall-clock duration, its `moof`/`mfhd`
/// sequence number, or its position on *this trunk's* absolute timeline —
/// those are properties of the log a segment lands in, not of the segmenter
/// that produced its bytes, so they are new fields here, supplied by
/// whoever is feeding [`TrunkWriter::publish_segment`], exactly as
/// `track_id`/[`RetentionClass`] are supplied by whoever feeds
/// [`TrunkWriter::publish`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SegmentEntry {
    /// The segment's encoded bytes. `Bytes`, not `Vec<u8>`, for the same
    /// reason as [`transmux::Sample::data`]: fan-out to every
    /// [`SegmentCursor`] reading this entry is a refcount bump, not a copy —
    /// see [Zero-copy fan-out](self#zero-copy-fan-out-honestly).
    pub bytes: Bytes,
    /// This segment's `moof`/`mfhd` sequence number (1-based, matching
    /// [`transmux::Segmenter`]'s own numbering) — what a consumer needs to
    /// name the segment in a playlist or manifest.
    pub sequence_number: u32,
    /// This segment's duration, wall-clock — what a consumer needs for
    /// `#EXTINF`/`<S d="...">`.
    pub duration: Duration,
    /// This segment's start position on the trunk's absolute timeline.
    pub timeline_position: Timestamp,
    /// The discontinuity bit from the segmenter itself; see this struct's
    /// own doc for why it is reused by embedding the whole type, not
    /// re-derived as a field of this struct.
    pub meta: SegmentMeta,
}

impl SegmentEntry {
    /// Build one segment log entry.
    pub fn new(
        bytes: impl Into<Bytes>,
        sequence_number: u32,
        duration: Duration,
        timeline_position: Timestamp,
        meta: SegmentMeta,
    ) -> Self {
        SegmentEntry {
            bytes: bytes.into(),
            sequence_number,
            duration,
            timeline_position,
            meta,
        }
    }
}

/// The caller-chosen policy for what happens when a **pinning**
/// [`SegmentCursor`] (from [`Trunk::pin_segments`]) has not yet consumed an
/// entry the segment log needs to evict because it is at
/// [`TrunkConfig::segment_capacity`].
///
/// See [The DVR contradiction](self#the-dvr-contradiction-losslessness-from-retention-not-back-pressure)
/// for why this is a three-way, caller-chosen trade with no free option and
/// deliberately no fourth "just make it work" variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ArchiveOverrun {
    /// Evict the pinned entry anyway, and report the loss to this cursor as
    /// [`SegmentCursorItem::Gap`] on its next [`SegmentCursor::poll`]. The
    /// recording gets a hole; live ingest and every other cursor are
    /// unaffected. **The default** — a pinning cursor that does not choose
    /// otherwise gets availability over completeness, the same trade
    /// [`RetentionClass::Timed`]'s ordinary `Lagged` already makes for the
    /// sample ring.
    Gap,
    /// Apply real back-pressure: [`TrunkWriter::publish_segment`] blocks
    /// until this cursor consumes far enough to release its pin (or the
    /// cursor is dropped). **The only place in this entire design where a
    /// reader may block the writer** — opt-in only, never the default;
    /// choosing it means a wedged or malicious archive consumer can stall
    /// segment publication indefinitely.
    StallIngest,
    /// Drop this cursor's pin outright instead of gapping the recording or
    /// stalling ingest: the cursor is terminated (its next `poll` returns
    /// [`SegmentCursorItem::Terminated`], and every `poll` after that
    /// returns `None`) and the log continues without it.
    Terminate,
}

impl Default for ArchiveOverrun {
    /// [`ArchiveOverrun::Gap`] — see that variant's doc for why gapping the
    /// recording, rather than stalling ingest, is the safe default.
    fn default() -> Self {
        ArchiveOverrun::Gap
    }
}

/// Per-pinning-cursor bookkeeping the segment log consults, at each
/// [`TrunkWriter::publish_segment`], to decide whether evicting the oldest
/// entry is safe.
struct PinState {
    /// This pin's own read progress: the same role [`SampleCursor`]'s local
    /// `*_consumed` fields play, made visible to the *writer* instead of
    /// staying purely cursor-local, because eviction has to consult it
    /// *before* evicting, not merely report loss after the fact.
    /// "Acknowledged" (module docs) means "returned by
    /// [`SegmentCursor::poll`]" — see the module docs' DVR section for why a
    /// separate ack-after-durable-write step was considered and rejected
    /// for this step.
    consumed: u64,
    /// The policy chosen at [`Trunk::pin_segments`] time.
    policy: ArchiveOverrun,
    /// Set once [`ArchiveOverrun::Terminate`] has fired for this pin; the
    /// next `poll` on the owning cursor reports
    /// [`SegmentCursorItem::Terminated`] and removes this entry.
    terminated: bool,
}

/// The segment log: a bounded, append-ordered log of [`SegmentEntry`]
/// values, plus the pin bookkeeping [`ArchiveOverrun`] needs.
///
/// Evict-then-push shape identical to [`ClassLog`] — `base`/`published`
/// mean exactly the same thing here as there — with one addition: a publish
/// that would evict an entry a pinning cursor has not yet consumed does not
/// evict unconditionally; [`TrunkWriter::publish_segment`] consults that
/// pin's [`ArchiveOverrun`] first.
struct SegmentLog {
    entries: VecDeque<SegmentEntry>,
    base: u64,
    published: u64,
    capacity: usize,
    pins: HashMap<u64, PinState>,
    next_pin_id: u64,
}

impl SegmentLog {
    fn new(capacity: usize) -> Self {
        SegmentLog {
            entries: VecDeque::with_capacity(capacity),
            base: 0,
            published: 0,
            capacity,
            pins: HashMap::new(),
            next_pin_id: 0,
        }
    }

    /// Unconditional evict-then-push — exactly [`ClassLog::push`]'s shape.
    /// [`ArchiveOverrun`] handling against `pins` happens *before* this is
    /// called; see [`TrunkWriter::publish_segment`].
    fn push(&mut self, entry: SegmentEntry) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
            self.base += 1;
        }
        self.entries.push_back(entry);
        self.published += 1;
    }
}

/// The shared state behind one [`Trunk`]: the two sample [`ClassLog`]s and
/// the [`SegmentLog`]. See [module docs](self#where-the-rest-of-trunk-attaches)
/// for the sibling field 3b-iii adds here.
struct TrunkState {
    timed: ClassLog,
    sparse: ClassLog,
    segments: SegmentLog,
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
    /// Wakes a [`TrunkWriter::publish_segment`] parked on
    /// [`ArchiveOverrun::StallIngest`] once a pin it is waiting on advances
    /// (a [`SegmentCursor::poll`] consuming further) or is released (its
    /// cursor dropped). Paired with `state` in the usual `Condvar` idiom:
    /// `wait` atomically releases the `Mutex` while parked, so a stalled
    /// segment publish does not hold the lock other `Trunk` operations
    /// (sample publish, any cursor's `poll`) need — see
    /// [The DVR contradiction](self#the-dvr-contradiction-losslessness-from-retention-not-back-pressure).
    segment_pin_released: Condvar,
    writer_taken: AtomicBool,
}

impl Trunk {
    /// Construct a fresh, empty `Trunk`.
    ///
    /// Panics if `config.timed_capacity`, `config.sparse_capacity`, or
    /// `config.segment_capacity` is zero — a construction mistake (every
    /// entry would be evicted the instant it was pushed), not remote input,
    /// so it panics rather than returning a `Result` a caller could ignore
    /// (matching
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
        assert!(
            config.segment_capacity > 0,
            "Trunk segment_capacity must be > 0"
        );
        Arc::new(Trunk {
            state: Mutex::new(TrunkState {
                timed: ClassLog::new(config.timed_capacity),
                sparse: ClassLog::new(config.sparse_capacity),
                segments: SegmentLog::new(config.segment_capacity),
            }),
            segment_pin_released: Condvar::new(),
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

    /// Subscribe a new **non-pinning** [`SegmentCursor`], starting from
    /// *now* — the same "next entry only, no backlog" rule as
    /// [`Trunk::subscribe`], and the same single-digit-reader,
    /// one-cursor-per-distinct-consumer guidance from that method's docs
    /// applies here verbatim (this cursor contends exactly the lock
    /// `subscribe`'s cursors do).
    ///
    /// This cursor is **not** protected by [`ArchiveOverrun`]: if it falls
    /// behind the segment log's ordinary [`TrunkConfig::segment_capacity`]
    /// eviction, it simply sees [`SegmentCursorItem::Lagged`], exactly like
    /// an ordinary [`RetentionClass::Timed`] sample reader. Use this for a
    /// consumer that tolerates ordinary loss (LL-HLS window rendering,
    /// catch-up within the live window) — use [`Trunk::pin_segments`]
    /// instead for a consumer that must not miss a segment (DVR/archive).
    pub fn subscribe_segments(self: &Arc<Self>) -> SegmentCursor {
        let state = self.state.lock().unwrap();
        SegmentCursor {
            trunk: Arc::clone(self),
            consumed: state.segments.published,
            pin_id: None,
            done: false,
        }
    }

    /// Subscribe a new **pinning** [`SegmentCursor`] for a DVR/archive
    /// consumer that must not miss a segment — see
    /// [The DVR contradiction](self#the-dvr-contradiction-losslessness-from-retention-not-back-pressure)
    /// for the full design story this method is the entry point for.
    ///
    /// `on_overrun` is this cursor's chosen [`ArchiveOverrun`] for the one
    /// moment its guarantee runs out: the segment log at
    /// [`TrunkConfig::segment_capacity`], about to evict an entry this
    /// cursor has not yet consumed. There is no default parameter here on
    /// purpose — pinning is an explicit request for a stronger guarantee
    /// than [`Trunk::subscribe_segments`] gives, so the trade made when that
    /// guarantee cannot be kept is an explicit choice too, not a silent
    /// fallback (though [`ArchiveOverrun::default`] exists for a caller that
    /// affirmatively wants the same default the rest of this module uses).
    ///
    /// Also starts from *now*, and also single-digit-by-design — the same
    /// fan-out rule as [`Trunk::subscribe`] and [`Trunk::subscribe_segments`]
    /// applies; a pinning cursor is exactly as expensive per publish as any
    /// other.
    pub fn pin_segments(self: &Arc<Self>, on_overrun: ArchiveOverrun) -> SegmentCursor {
        let mut state = self.state.lock().unwrap();
        let pin_id = state.segments.next_pin_id;
        state.segments.next_pin_id += 1;
        let consumed = state.segments.published;
        state.segments.pins.insert(
            pin_id,
            PinState {
                consumed,
                policy: on_overrun,
                terminated: false,
            },
        );
        SegmentCursor {
            trunk: Arc::clone(self),
            consumed: 0,
            pin_id: Some(pin_id),
            done: false,
        }
    }

    /// Diagnostic: entries currently resident in the segment log. Never
    /// exceeds [`TrunkConfig::segment_capacity`] — true even with an
    /// un-acking pinning cursor attached, which is exactly the property
    /// [The DVR contradiction](self#the-dvr-contradiction-losslessness-from-retention-not-back-pressure)'s
    /// "pinning is bounded" claim means.
    pub fn segment_len(&self) -> usize {
        self.state.lock().unwrap().segments.entries.len()
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

    /// Publish one finished segment, in playlist order.
    ///
    /// Never blocks and never rejects for **every non-pinning**
    /// [`SegmentCursor`] and for every pinning cursor using
    /// [`ArchiveOverrun::Gap`] (the default) or [`ArchiveOverrun::Terminate`]
    /// — a full segment log evicts its oldest entry exactly like
    /// [`TrunkWriter::publish`]'s sample rings. The **one** exception, by
    /// design, is a pinning cursor using [`ArchiveOverrun::StallIngest`]
    /// that has not yet consumed the entry about to be evicted: this call
    /// blocks until that cursor consumes further (or is dropped) — see
    /// [The DVR contradiction](self#the-dvr-contradiction-losslessness-from-retention-not-back-pressure).
    /// The block is a [`std::sync::Condvar::wait`], which releases the
    /// shared `Mutex` while parked, so [`TrunkWriter::publish`] and every
    /// cursor's `poll` on *other* data remain free to proceed even while
    /// this call is stalled.
    pub fn publish_segment(&self, entry: SegmentEntry) {
        let mut state = self.trunk.state.lock().unwrap();
        loop {
            if state.segments.entries.len() < state.segments.capacity {
                // Room to push without evicting anything: no pin can be at
                // risk this round.
                break;
            }
            let oldest = state.segments.base;
            let mut must_wait = false;
            for pin in state.segments.pins.values_mut() {
                if pin.terminated || pin.consumed > oldest {
                    // Either already given up on (Terminate already fired),
                    // or this pin has already consumed the entry about to be
                    // evicted — not at risk.
                    continue;
                }
                match pin.policy {
                    // Nothing to do here: eviction proceeds, and the owning
                    // cursor's own `poll` reports the loss as `Gap` the same
                    // way a non-pinning cursor's `poll` reports it as
                    // ordinary `Lagged` — both read `base` vs. their own
                    // progress, after the fact.
                    ArchiveOverrun::Gap => {}
                    ArchiveOverrun::Terminate => pin.terminated = true,
                    ArchiveOverrun::StallIngest => must_wait = true,
                }
            }
            if !must_wait {
                break;
            }
            state = self.trunk.segment_pin_released.wait(state).unwrap();
            // Loop back around: re-check capacity/oldest/pins after waking —
            // the pin that was blocking may have advanced, been dropped, or
            // (if a *different* pin also needed this entry) still be
            // pending.
        }
        state.segments.push(entry);
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

/// One item [`SegmentCursor::poll`] can hand back: a finished segment, or a
/// loss report.
///
/// `#[non_exhaustive]`: this is the growth point for anything a segment
/// cursor might need to surface beyond "segment" or "loss" later, without a
/// breaking change to every match arm in the workspace.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SegmentCursorItem {
    /// One finished segment, in playlist order.
    Segment(SegmentEntry),
    /// A **non-pinning** cursor (from [`Trunk::subscribe_segments`]) fell
    /// behind the segment log's ordinary [`TrunkConfig::segment_capacity`]
    /// eviction: `skipped` segments were evicted before it read them.
    /// Ordinary loss, exactly [`SampleCursorItem::Lagged`]'s contract —
    /// resume from the next segment.
    Lagged {
        /// Exact count of segments evicted since this cursor's last
        /// successful read.
        skipped: u64,
    },
    /// A **pinning** cursor's (from [`Trunk::pin_segments`])
    /// [`ArchiveOverrun::Gap`] policy fired: the log evicted `skipped`
    /// segments this cursor had not yet consumed rather than let its pin
    /// grow retention without bound. Unlike `Lagged`, this is the defect a
    /// DVR consumer must record as a hole in the archive — see
    /// [The DVR contradiction](self#the-dvr-contradiction-losslessness-from-retention-not-back-pressure).
    Gap {
        /// Exact count of segments evicted out from under this cursor's
        /// pin.
        skipped: u64,
    },
    /// This **pinning** cursor's [`ArchiveOverrun::Terminate`] policy fired:
    /// the log dropped its pin instead of gapping the recording or
    /// stalling ingest. This is the last item this cursor will ever yield —
    /// every `poll` after this one returns `None`.
    Terminated,
}

/// A subscribed reader of a [`Trunk`]'s segment log. Obtained via
/// [`Trunk::subscribe_segments`] (ordinary, lossy-on-overflow) or
/// [`Trunk::pin_segments`] (pinning, [`ArchiveOverrun`]-governed) — **read
/// those methods' docs, and [The DVR contradiction](self#the-dvr-contradiction-losslessness-from-retention-not-back-pressure),
/// before creating more than a handful of these.**
pub struct SegmentCursor {
    trunk: Arc<Trunk>,
    /// Read progress for a **non-pinning** cursor (`pin_id.is_none()`) —
    /// exactly [`SampleCursor`]'s local `*_consumed` fields. Unused (and left
    /// at `0`) for a pinning cursor, whose progress instead lives in the
    /// shared `PinState::consumed` the writer must be able to see; see
    /// [`SegmentLog`].
    consumed: u64,
    /// `Some(id)` for a pinning cursor — the key into
    /// `TrunkState::segments.pins` this cursor's progress and policy are
    /// recorded under. `None` for an ordinary [`Trunk::subscribe_segments`]
    /// cursor.
    pin_id: Option<u64>,
    /// Set once this cursor has reported [`SegmentCursorItem::Terminated`] —
    /// every `poll` after that returns `None` rather than re-reporting it or
    /// resuming as if nothing happened.
    done: bool,
}

impl SegmentCursor {
    /// Pull the next item, if any is ready.
    ///
    /// Loss is always reported before further data, in the same
    /// `Option<SegmentCursorItem>` as real segments — the same
    /// cannot-be-skipped-past precedent as [`SampleCursor::poll`]/
    /// [`crate::byte_tap::TapItem`].
    pub fn poll(&mut self) -> Option<SegmentCursorItem> {
        if self.done {
            return None;
        }

        let Some(pin_id) = self.pin_id else {
            // Non-pinning: local `consumed`, exactly `SampleCursor::poll`'s
            // shape, against the one segment log instead of two class rings.
            let state = self.trunk.state.lock().unwrap();
            if self.consumed < state.segments.base {
                let skipped = state.segments.base - self.consumed;
                self.consumed = state.segments.base;
                return Some(SegmentCursorItem::Lagged { skipped });
            }
            let idx = (self.consumed - state.segments.base) as usize;
            return if let Some(entry) = state.segments.entries.get(idx) {
                self.consumed += 1;
                Some(SegmentCursorItem::Segment(entry.clone()))
            } else {
                None
            };
        };

        // Pinning: progress lives in the shared `PinState`, because
        // `TrunkWriter::publish_segment` has to consult it before evicting,
        // not merely react to it afterward.
        let mut state = self.trunk.state.lock().unwrap();
        let Some(pin) = state.segments.pins.get(&pin_id) else {
            // Already removed (defensive: `Drop`/prior `Terminated` report
            // should make this unreachable in practice) — treat as done.
            self.done = true;
            return None;
        };
        if pin.terminated {
            state.segments.pins.remove(&pin_id);
            self.pin_id = None;
            self.done = true;
            return Some(SegmentCursorItem::Terminated);
        }
        let consumed = pin.consumed;
        if consumed < state.segments.base {
            let skipped = state.segments.base - consumed;
            state.segments.pins.get_mut(&pin_id).unwrap().consumed = state.segments.base;
            drop(state);
            // A pin advancing can free a `StallIngest` writer waiting on
            // exactly this pin.
            self.trunk.segment_pin_released.notify_all();
            return Some(SegmentCursorItem::Gap { skipped });
        }
        let idx = (consumed - state.segments.base) as usize;
        if let Some(entry) = state.segments.entries.get(idx) {
            let item = entry.clone();
            state.segments.pins.get_mut(&pin_id).unwrap().consumed += 1;
            drop(state);
            self.trunk.segment_pin_released.notify_all();
            return Some(SegmentCursorItem::Segment(item));
        }
        None
    }
}

impl Drop for SegmentCursor {
    /// Release this cursor's pin, if it has one, so a dropped/abandoned
    /// pinning cursor cannot hold retention open (or a `StallIngest` writer
    /// blocked) forever — the same "a dead consumer must not grow memory
    /// without limit" guarantee as an actively-`Gap`-ping cursor, for the
    /// case where the consumer disappeared instead of choosing a policy.
    fn drop(&mut self) {
        if let Some(pin_id) = self.pin_id.take() {
            let mut state = self.trunk.state.lock().unwrap();
            state.segments.pins.remove(&pin_id);
            drop(state);
            self.trunk.segment_pin_released.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    fn sample(byte: u8, len: usize) -> Sample {
        Sample::new(Bytes::from(vec![byte; len]), Some(0), Some(0), None, true)
    }

    fn timed_data(item: &SampleCursorItem) -> Option<(u32, &Sample)> {
        match item {
            SampleCursorItem::Timed { track_id, sample } => Some((*track_id, sample)),
            _ => None,
        }
    }

    fn segment_entry(byte: u8, seq: u32) -> SegmentEntry {
        SegmentEntry::new(
            Bytes::from(vec![byte; 16]),
            seq,
            Duration::from_secs(2),
            Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
            SegmentMeta {
                discontinuous: false,
            },
        )
    }

    fn segment_data(item: &SegmentCursorItem) -> Option<&SegmentEntry> {
        match item {
            SegmentCursorItem::Segment(entry) => Some(entry),
            _ => None,
        }
    }

    /// Drains up to `n` items from `cursor`, stopping early if `poll`
    /// returns `None` — see [`drain`]'s doc for why this is bounded rather
    /// than looping until `None`.
    fn drain_segments(cursor: &mut SegmentCursor, n: usize) -> Vec<SegmentCursorItem> {
        let mut out = Vec::new();
        for _ in 0..n {
            match cursor.poll() {
                Some(item) => out.push(item),
                None => break,
            }
        }
        out
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
        let trunk = Trunk::new(TrunkConfig::new(100, 10, 4));
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
        let trunk = Trunk::new(TrunkConfig::new(4, 10, 4));
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
        let trunk = Trunk::new(TrunkConfig::new(3, 10, 4));
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
        let trunk = Trunk::new(TrunkConfig::new(2, 2, 4));
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
        let trunk = Trunk::new(TrunkConfig::new(4, 3, 4));
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
        let trunk = Trunk::new(TrunkConfig::new(8, 8, 4));
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
        let _ = Trunk::new(TrunkConfig::new(0, 4, 4));
    }

    #[test]
    #[should_panic(expected = "sparse_capacity must be > 0")]
    fn zero_sparse_capacity_panics() {
        let _ = Trunk::new(TrunkConfig::new(4, 0, 4));
    }

    #[test]
    #[should_panic(expected = "segment_capacity must be > 0")]
    fn zero_segment_capacity_panics() {
        let _ = Trunk::new(TrunkConfig::new(4, 4, 0));
    }

    #[test]
    fn second_writer_is_refused() {
        let trunk = Trunk::new(TrunkConfig::new(4, 4, 4));
        let _first = trunk.writer().unwrap();
        assert!(trunk.writer().is_none(), "a Trunk has exactly one writer");
    }

    #[test]
    fn subscribe_starts_from_now_not_from_history() {
        let trunk = Trunk::new(TrunkConfig::new(4, 4, 4));
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

    // ===================== segment log ====================================

    // --- S1. multiple cursors, every segment, in order, no dup/no loss ----

    /// MUTATION VERIFIED: removing the `self.consumed += 1;` from the
    /// non-pinning data-return arm of `SegmentCursor::poll` (so the same
    /// ring index is re-read every call) makes this test fail exactly like
    /// `SampleCursor::poll`'s equivalent mutation: `drain_segments` still
    /// returns 5 items, but all 5 are the first published segment
    /// (`sequence_number == 1`) instead of the distinct sequence `1..=5`, so
    /// the `assert_eq!` on the reconstructed sequence-number list fails at
    /// index 1. Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn multiple_segment_cursors_see_every_segment_in_order_with_no_dup_or_loss() {
        let trunk = Trunk::new(TrunkConfig::new(4, 4, 100));
        let mut c1 = trunk.subscribe_segments();
        let mut c2 = trunk.subscribe_segments();
        let mut c3 = trunk.subscribe_segments();
        let writer = trunk.writer().unwrap();

        for i in 0u32..5 {
            writer.publish_segment(segment_entry(i as u8, i + 1));
        }

        for cursor in [&mut c1, &mut c2, &mut c3] {
            let items = drain_segments(cursor, 5);
            assert_eq!(items.len(), 5, "each cursor must see exactly 5 segments");
            let seqs: Vec<u32> = items
                .iter()
                .map(|item| segment_data(item).unwrap().sequence_number)
                .collect();
            assert_eq!(seqs, vec![1, 2, 3, 4, 5], "must be in playlist order");
            assert!(cursor.poll().is_none(), "no extra/duplicated items");
        }
    }

    // --- S2. a non-pinning slow reader lags; writer completes regardless --

    /// MUTATION VERIFIED: changing `SegmentLog::push`'s eviction condition
    /// from `self.entries.len() == self.capacity` to `false` (disabling
    /// eviction) makes `trunk.segment_len()` grow to 1024 instead of staying
    /// at the configured cap of 4, and the subsequent `Lagged` assertion
    /// fails because `base` never advanced (`skipped` reads back as `0`, not
    /// `1020`). Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn non_pinning_slow_segment_reader_lags_but_writer_completes_regardless() {
        let trunk = Trunk::new(TrunkConfig::new(4, 4, 4));
        let mut slow = trunk.subscribe_segments();
        let writer = trunk.writer().unwrap();

        // The slow (non-pinning) reader never polls while 1024 segments are
        // published — there is no wait-for-reader path for a non-pinning
        // cursor, so this simply completes (same reasoning as
        // `slow_reader_lags_but_writer_completes_regardless`).
        for i in 0u32..1024 {
            writer.publish_segment(segment_entry((i % 256) as u8, i + 1));
        }
        assert_eq!(
            trunk.segment_len(),
            4,
            "writer unblocked: segment log stayed bounded"
        );

        let first = slow.poll().unwrap();
        assert!(
            matches!(first, SegmentCursorItem::Lagged { skipped: 1020 }),
            "expected Lagged{{skipped: 1020}}, got {first:?}"
        );
    }

    // --- S3. THE DVR PROPERTY: a pinning reader loses nothing while a ------
    // --- non-pinning sibling lags, and StallIngest is what makes it true --

    /// MUTATION VERIFIED: changing the `must_wait` computation in
    /// `TrunkWriter::publish_segment`'s `ArchiveOverrun::StallIngest` arm
    /// from `must_wait = true;` to `{}` (a no-op, i.e. treating
    /// `StallIngest` exactly like `Gap`) makes this test fail: the third
    /// `publish_segment` call no longer blocks, so the background-thread
    /// completion channel's `recv_timeout` at the "still blocked" checkpoint
    /// returns `Ok(())` instead of timing out, and the assertion that it
    /// timed out (`is_err()`) fails. Recompiled and re-run to confirm the
    /// failure, then reverted.
    #[test]
    fn pinning_reader_receives_every_segment_while_non_pinning_reader_lags() {
        let trunk = Trunk::new(TrunkConfig::new(4, 4, 2));
        let mut slow = trunk.subscribe_segments(); // non-pinning: will lag
        let mut archive = trunk.pin_segments(ArchiveOverrun::StallIngest); // pinning: must lose nothing
        let writer = Arc::new(trunk.writer().unwrap());

        // Fill the segment log's capacity (2) without any eviction yet.
        writer.publish_segment(segment_entry(1, 1));
        writer.publish_segment(segment_entry(2, 2));

        // A third publish must evict the oldest (seq 1), which `archive`'s
        // pin has not yet consumed — with `StallIngest`, this call blocks.
        // Run it on a background thread (the same one `TrunkWriter`, shared
        // via `Arc` — this is the one-writer invariant, just called from a
        // different thread) and prove, via a completion channel, that it
        // has NOT returned yet.
        let (done_tx, done_rx) = mpsc::channel();
        let blocked_writer = Arc::clone(&writer);
        let handle = thread::spawn(move || {
            blocked_writer.publish_segment(segment_entry(3, 3));
            done_tx.send(()).unwrap();
        });

        assert!(
            done_rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "publish_segment must still be blocked: archive has not consumed seq 1 yet"
        );

        // `archive` catches up: consuming seq 1 releases its pin on it,
        // which must wake and unblock the writer thread.
        let first = archive.poll().unwrap();
        assert_eq!(segment_data(&first).unwrap().sequence_number, 1);

        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("publish_segment must unblock once the pin advances");
        handle.join().unwrap();

        // `archive` receives every remaining segment with ZERO loss — no
        // `Gap`, no `Lagged` — proving the DVR property: pinning protected
        // it from the eviction that just happened.
        let second = archive.poll().unwrap();
        assert_eq!(segment_data(&second).unwrap().sequence_number, 2);
        let third = archive.poll().unwrap();
        assert_eq!(segment_data(&third).unwrap().sequence_number, 3);
        assert!(archive.poll().is_none());

        // Meanwhile `slow` (non-pinning, never polled) DID lag: exactly one
        // segment (seq 1) was evicted out from under it.
        let lag = slow.poll().unwrap();
        assert!(
            matches!(lag, SegmentCursorItem::Lagged { skipped: 1 }),
            "expected Lagged{{skipped: 1}}, got {lag:?}"
        );
        let remaining: Vec<u32> = drain_segments(&mut slow, 2)
            .iter()
            .map(|item| segment_data(item).unwrap().sequence_number)
            .collect();
        assert_eq!(remaining, vec![2, 3]);
    }

    // --- S4. pinning is bounded: an un-acking consumer cannot grow --------
    // --- memory without limit ----------------------------------------------

    /// MUTATION VERIFIED: removing the eviction check in `SegmentLog::push`
    /// (replacing `if self.entries.len() == self.capacity { .. }` with a
    /// no-op, exactly like the sample-ring equivalent mutation) makes
    /// `trunk.segment_len()` grow past the configured cap of `4` instead of
    /// staying bounded — the in-loop assertion below fails on the first
    /// over-capacity iteration. Recompiled and re-run to confirm the
    /// failure, then reverted.
    #[test]
    fn pinning_is_bounded_an_unacking_consumer_cannot_grow_memory_without_limit() {
        let trunk = Trunk::new(TrunkConfig::new(4, 4, 4));
        // Default policy (`Gap`) pinning cursor that never polls at all —
        // the worst case for memory growth: a dead/wedged archive consumer.
        let _archive = trunk.pin_segments(ArchiveOverrun::default());
        let writer = trunk.writer().unwrap();

        for i in 0u32..50_000 {
            writer.publish_segment(segment_entry((i % 256) as u8, i + 1));
            assert!(
                trunk.segment_len() <= 4,
                "segment log exceeded its cap mid-flood despite an un-acking pinning cursor"
            );
        }
        assert_eq!(trunk.segment_len(), 4);
    }

    // --- S5. ArchiveOverrun::Gap gaps and reports --------------------------

    /// MUTATION VERIFIED: changing the pinning branch of `SegmentCursor::poll`
    /// to report `SegmentCursorItem::Lagged` instead of `SegmentCursorItem::Gap`
    /// (collapsing the two, mirroring the sample path's Timed/Sparse
    /// mutation) makes the `matches!(item, SegmentCursorItem::Gap { .. })`
    /// assertion below fail — the item is a `Lagged` instead. Recompiled and
    /// re-run to confirm the failure, then reverted.
    #[test]
    fn archive_overrun_gap_evicts_and_reports_gap() {
        let trunk = Trunk::new(TrunkConfig::new(4, 4, 2));
        let mut archive = trunk.pin_segments(ArchiveOverrun::Gap);
        let writer = trunk.writer().unwrap();

        // Publish 5 segments into a capacity-2 log without archive ever
        // polling: with `Gap`, eviction proceeds unconditionally, so this
        // never blocks.
        for i in 0u32..5 {
            writer.publish_segment(segment_entry(i as u8, i + 1));
        }
        assert_eq!(trunk.segment_len(), 2);

        let gap = archive.poll().unwrap();
        assert!(
            matches!(gap, SegmentCursorItem::Gap { skipped: 3 }),
            "expected Gap{{skipped: 3}}, got {gap:?}"
        );
        // The recording has a hole, but the stream survives: archive keeps
        // reading the segments that remain.
        let remaining: Vec<u32> = drain_segments(&mut archive, 2)
            .iter()
            .map(|item| segment_data(item).unwrap().sequence_number)
            .collect();
        assert_eq!(remaining, vec![4, 5]);
        assert!(archive.poll().is_none());
    }

    // --- S6. ArchiveOverrun::StallIngest actually applies back-pressure ---

    /// MUTATION VERIFIED: same mutation and same observed failure as
    /// `pinning_reader_receives_every_segment_while_non_pinning_reader_lags`'s
    /// doc comment (removing `must_wait = true;` from the `StallIngest`
    /// arm) — this test is the narrower, single-purpose proof that
    /// `publish_segment` genuinely blocks, isolated from the sibling-lag
    /// scenario. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[test]
    fn archive_overrun_stall_ingest_actually_blocks_the_writer() {
        let trunk = Trunk::new(TrunkConfig::new(4, 4, 1));
        let mut archive = trunk.pin_segments(ArchiveOverrun::StallIngest);
        let writer = Arc::new(trunk.writer().unwrap());

        writer.publish_segment(segment_entry(1, 1)); // fills capacity-1 log

        let (done_tx, done_rx) = mpsc::channel();
        let blocked_writer = Arc::clone(&writer);
        let handle = thread::spawn(move || {
            blocked_writer.publish_segment(segment_entry(2, 2));
            done_tx.send(()).unwrap();
        });

        assert!(
            done_rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "publish_segment must block: the pin has not consumed seq 1 yet"
        );

        let first = archive.poll().unwrap();
        assert_eq!(segment_data(&first).unwrap().sequence_number, 1);

        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("publish_segment must unblock once the pin advances");
        handle.join().unwrap();
    }

    // --- S7. ArchiveOverrun::Terminate drops the cursor --------------------

    /// MUTATION VERIFIED: changing `ArchiveOverrun::Terminate => pin.terminated
    /// = true,` in `TrunkWriter::publish_segment` to `ArchiveOverrun::Terminate
    /// => {}` (a no-op, treating `Terminate` exactly like `Gap`) makes this
    /// test fail: `archive.poll()` returns `Some(Gap { .. })` instead of
    /// `Some(Terminated)`, so the `matches!` assertion on `Terminated` fails.
    /// Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn archive_overrun_terminate_drops_the_cursor() {
        let trunk = Trunk::new(TrunkConfig::new(4, 4, 2));
        let mut archive = trunk.pin_segments(ArchiveOverrun::Terminate);
        let writer = trunk.writer().unwrap();

        // Publish past capacity without archive ever polling: `Terminate`
        // never blocks (like `Gap`), so this completes.
        for i in 0u32..5 {
            writer.publish_segment(segment_entry(i as u8, i + 1));
        }
        assert_eq!(trunk.segment_len(), 2, "writer unblocked despite Terminate");

        let item = archive.poll().unwrap();
        assert!(
            matches!(item, SegmentCursorItem::Terminated),
            "expected Terminated, got {item:?}"
        );
        // The cursor is done: every poll after `Terminated` returns `None`,
        // never resuming as if nothing happened.
        assert!(archive.poll().is_none());
        assert!(archive.poll().is_none());

        // The log itself is unaffected: publishing continues to work, and a
        // fresh cursor still sees ordinary segment log behaviour.
        writer.publish_segment(segment_entry(9, 6));
        assert_eq!(trunk.segment_len(), 2);
    }

    // --- S8. segment bytes are shared, not copied, across cursors ---------

    /// MUTATION VERIFIED: replacing `entry.clone()` in
    /// `SegmentCursor::poll`'s non-pinning data-return arm with a hand-rolled
    /// copy (`SegmentEntry { bytes: Bytes::copy_from_slice(entry.bytes.as_ref()),
    /// ..entry.clone() }`, preserving every field's *value*) makes this
    /// test's pointer-identity assertion fail — `p1 == p2` becomes `false`
    /// (two distinct heap allocations with equal contents) instead of the
    /// unmutated `clone()` path's `p1 == p2 == p3`. This is exactly the
    /// distinction a content-equality assertion would have missed.
    /// Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn segment_bytes_are_shared_not_copied_across_cursors() {
        let trunk = Trunk::new(TrunkConfig::new(4, 4, 8));
        let mut c1 = trunk.subscribe_segments();
        let mut c2 = trunk.subscribe_segments();
        let mut c3 = trunk.subscribe_segments();
        let writer = trunk.writer().unwrap();

        writer.publish_segment(SegmentEntry::new(
            Bytes::from(vec![0xCDu8; 65536]),
            1,
            Duration::from_secs(2),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ));

        let i1 = c1.poll().unwrap();
        let i2 = c2.poll().unwrap();
        let i3 = c3.poll().unwrap();
        let p1 = segment_data(&i1).unwrap().bytes.as_ptr();
        let p2 = segment_data(&i2).unwrap().bytes.as_ptr();
        let p3 = segment_data(&i3).unwrap().bytes.as_ptr();

        assert_eq!(
            p1, p2,
            "cursor 2's segment payload must be the SAME allocation as cursor 1's"
        );
        assert_eq!(
            p2, p3,
            "cursor 3's segment payload must be the SAME allocation as cursor 1's"
        );
        assert_eq!(segment_data(&i1).unwrap().bytes.len(), 65536);
    }
}
