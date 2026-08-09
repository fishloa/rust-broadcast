//! [`Trunk`] — the sample ring, [`TrunkWriter`], and [`SampleCursor`] (plan
//! step 3b-i); the segment log, [`SegmentCursor`], and the
//! lossless-by-retention pinning mechanism (plan step 3b-ii); the 90 kHz
//! event log, [`EventCursor`], and [`EventAnchor`] (plan step 3b-iii); the
//! live-part log and the [`Trunk::listen`] reader-wake primitive (plan step
//! 3b-iv), closing the two gaps step 3d found while reading
//! `hls-runtime/src/server/` before writing the egress traits — see
//! [The live-part log](#the-live-part-log-parts-before-their-segment-closes)
//! and
//! [The reader-wake primitive](#the-reader-wake-primitive-listen-not-one-registration-per-remote-peer)
//! below; and now [`SegmentWriter`], splitting the single write handle 3b-i
//! introduced by **ring group** so a segmenter can exist at all — see
//! [One writer per ring group, not one writer per `Trunk`](#one-writer-per-ring-group-not-one-writer-per-trunk)
//! below — per
//! `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §1.2.
//!
//! This is four bounded rings behind two write handles, split by ring group
//! (see the section linked just above), and the cursors/queries that read
//! them: the sample path, the segment log, the event log, and the live-part
//! log. See
//! [The event log: 90 kHz absolute, and the B1 crux](#the-event-log-90-khz-absolute-and-the-b1-crux)
//! below for why the event log needed a third, genuinely different shape —
//! not just a third copy of `ClassLog`/`SegmentLog` (this module's internal
//! per-class/per-segment logs) — to resolve the architecture audit's
//! blocking finding B1.
//!
//! # One writer per ring group, not one writer per `Trunk`
//!
//! [`Trunk::writer`] used to be the *only* way to publish anything, and its
//! doc said so in terms broader than the actual reason it exists: "a second
//! concurrent writer would silently interleave two unrelated publish
//! sequences into one ring with no way for a reader to tell them apart." That
//! sentence is true, but it over-generalises from "one ring" to "one
//! `Trunk`" — and a real consumer wiring this crate up (a segmenter: it reads
//! samples via a [`SampleCursor`] and, from what it reads, produces segments
//! and parts) hit the gap that over-generalisation opened: with exactly one
//! writer for the entire `Trunk`, whichever component takes it for ingest
//! samples makes it **structurally impossible** for anything else to ever
//! call [`SegmentWriter::publish_segment`] — no segmenter can exist, and a
//! segment log/live-part log that can never be filled is not a real feature,
//! just an unreachable one.
//!
//! **The actual invariant, restated correctly**: *within a given ring, there
//! is exactly one appender* — that is what prevents the interleave the
//! original sentence worried about, because an interleave requires two
//! writers racing to append to the *same* ring. It says nothing about a
//! *different* ring. So the write capability is split by **ring group**,
//! each still taken at most once via the same `compare_exchange` pattern
//! [`Trunk::writer`] always used, just with one flag per group instead of
//! one flag for the whole `Trunk`:
//!
//! - [`TrunkWriter`] (via [`Trunk::writer`]) — the **samples + events**
//!   group: [`TrunkWriter::publish`] (the two [`RetentionClass`] sample
//!   rings) and [`TrunkWriter::publish_event`] (the event ring's only
//!   *appending* operation). Held by the ingest driver — the entity that
//!   actually produces both: a demuxed sample, or an inband SCTE-35/`emsg`
//!   event lifted straight off the incoming stream.
//! - [`SegmentWriter`] (via [`Trunk::segment_writer`]) — the
//!   **segments + parts** group: [`SegmentWriter::publish_segment`],
//!   [`SegmentWriter::publish_part`], plus
//!   [`SegmentWriter::note_segment_start`]/[`SegmentWriter::set_time_anchor`].
//!   Held by whoever owns segmentation.
//!
//! The last two methods are grouped here on purpose, not by accident of
//! naming: neither **appends** an entry to the event ring the way
//! [`TrunkWriter::publish_event`] does — both *resolve an already-stored*
//! [`EventAnchor::Segment`]/[`EventAnchor::Utc`] entry **in place** (see
//! [The event log](#the-event-log-90-khz-absolute-and-the-b1-crux) below).
//! Since neither is an append, giving them to [`SegmentWriter`] does not
//! create a second appender for the event ring — [`TrunkWriter::publish_event`]
//! remains the event ring's only one — while `note_segment_start` in
//! particular *has* to live wherever segmentation lives: only the segmenter
//! knows where a segment boundary actually falls (this is the literal B1 fix
//! — "it cannot be finalised until the segmenter owns a boundary"), so it is
//! the one entity that can honestly report `note_segment_start`. Placing
//! `set_time_anchor` alongside it, rather than on [`TrunkWriter`], keeps this
//! crate's set of "resolve a pending anchor" entry points in one place
//! instead of splitting a single conceptual capability (anchor resolution)
//! across two handles for no test or caller that needs it split further; if
//! a future caller's wall-clock mapping genuinely comes from the ingest side
//! instead, adding it to [`TrunkWriter`] alongside `publish_event` is
//! additive, not a breaking re-split of what is here today.
//!
//! **Still exactly one writer per group, enforced the same way**: both
//! [`Trunk::writer`] and [`Trunk::segment_writer`] return `None` on every
//! call after their first, via their own `AtomicBool` — two concurrent
//! *sample* writers remain exactly as impossible as before this split; what
//! changed is that a *segment* writer and a *sample* writer are no longer
//! forced to be the same handle.
//!
//! **The cross-ring ordering question this split raises, answered rather
//! than left implicit.** A segment is derived from samples the segmenter has
//! already consumed via its own [`SampleCursor`] — causally, those samples
//! exist first. With one shared writer, that causal fact was also a
//! *program-order* fact (one thread called `publish` some number of times,
//! then called `publish_segment`). With the split, ingest and segmentation
//! are ordinarily two different threads — could a consumer ever observe
//! [`SegmentWriter::publish_segment`]'s entry in the segment log *before* the
//! samples that produced it are visible in the sample ring? **No** — and not
//! by luck: every ring here (`timed`, `sparse`, `segments`, `events`,
//! `parts`) still lives inside the *one* `Mutex<TrunkState>` this module has
//! always used (see [the benchmark verdict](#the-benchmark-verdict-this-design-is-built-around)
//! below) — splitting the *write handle* did not split the *lock*. The
//! segmenter can only have samples to build a segment from because its own
//! `SampleCursor::poll` already returned them, which requires those samples
//! to have been committed to `state.timed`/`state.sparse` under an *earlier*
//! acquisition of that same `Mutex`; `SegmentWriter::publish_segment` is then
//! called afterward, in the segmenter's own program order, under a *later*
//! acquisition of the identical `Mutex`. Any third party that subsequently
//! acquires that lock — to poll any ring, from any thread — is guaranteed by
//! the transitivity of the `Mutex`'s release/acquire ordering to observe at
//! least everything the segmenter itself had already observed before it
//! published, samples included. So the specific ordering a consumer must
//! never see reversed (a segment's constituent samples appearing to lag
//! behind the segment itself) cannot happen. What *is* true, and is exactly
//! the existing [`RetentionClass::Timed`]/[`RetentionClass::Sparse`]
//! precedent extended one layer: the *global* order across unrelated
//! ring-group activity is not fixed by any one thread's program order any
//! more — a new, unrelated sample the ingest thread publishes concurrently
//! may land before or after a segment close the segmenter thread publishes,
//! in either order, depending on which wins the lock race. Nothing
//! downstream needs that unrelated cross-ring interleave to be
//! deterministic (see [Two retention classes](#two-retention-classes-and-why-they-are-two-independent-rings)
//! below for why this crate already treats "no global cross-ring order" as
//! an acceptable, load-bearing property, not a defect) — only the *causal*
//! one, which is what the shared `Mutex` structurally guarantees.
//!
//! # Why this module needs `std`, unlike its byte-layer siblings
//!
//! [`crate::byte_stage`], [`crate::byte_tap`], and [`crate::byte_merge`] are
//! `no_std` because each is driven synchronously by a single caller — there is
//! no cross-thread sharing to arrange. `Trunk` is different in kind: one
//! writer thread per ring group (ingest for samples/events; a segmenter for
//! segments/parts, once one exists) and an unbounded set of reader threads
//! (egress, analysis, DVR) must observe the *same* ring concurrently. That
//! needs a
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
//! next [`SegmentWriter::publish_segment`] would evict an entry some pin has
//! not yet consumed, the bound has been hit, and something genuinely has to
//! give — the caller decided what, in advance, via the [`ArchiveOverrun`]
//! passed to [`Trunk::pin_segments`]:
//!
//! - [`ArchiveOverrun::Gap`] (**the default**) — evict the pinned entry
//!   anyway, and tell that cursor it lost data
//!   ([`SegmentCursorItem::Gap`]). The recording gets a hole; the live
//!   stream and every other cursor are unaffected.
//! - [`ArchiveOverrun::StallIngest`] — apply real back-pressure:
//!   [`SegmentWriter::publish_segment`] blocks until this cursor consumes
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
//! # The event log: 90 kHz absolute, and the B1 crux
//!
//! `TrunkState` (the shared state behind a `Trunk`) holds the two per-class
//! sample logs, the segment log, and now the event log — a **sibling ring
//! behind the same one `Mutex`**, exactly the pattern the segment log
//! established (`TrunkState::events: EventLog`, `Trunk::subscribe_events`/
//! `events_between`/`events_in_segment` shaped like `Trunk::subscribe`/
//! `Trunk::subscribe_segments`, `TrunkConfig::event_capacity` alongside
//! `timed_capacity`/`sparse_capacity`/`segment_capacity`). Where it is
//! genuinely a new shape, not a third `ClassLog`/`SegmentLog` copy, is
//! its *clock* and its *addressing* — both forced by architecture audit
//! finding B1
//! (`docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §0/§1.2).
//!
//! **What B1 got wrong.** Revision 1 of the spec claimed one time model for
//! everything the plane carries: an absolute `i64` in the *producing
//! track's* timescale. That is false in two ways this project already
//! parses, and both are events, not samples:
//!
//! - `splice_schedule.utc_splice_time` (SCTE-35 §9.7.4) is **GPS-epoch
//!   UTC** — not a media timestamp in any track's timescale at all.
//! - `emsg` version 0's `presentation_time_delta` (ISO/IEC 23009-1
//!   §5.10.3.3) is **segment-relative** — its value only means something
//!   once you know which segment it lands in, and that segment's earliest
//!   presentation time is not knowable until the segmenter has actually cut
//!   the boundary. `timed_metadata::convert::emsg_convert` already encodes
//!   this exact arithmetic (`T = EPT + presentation_time_delta`) for
//!   *converting* one emsg to another; the event log's job is different —
//!   it has to hold the delta *honestly unresolved* for however long the
//!   boundary is unknown, which a stateless conversion function has no
//!   reason to model.
//!
//! Neither of those is expressible as a single struct field without either
//! (a) losing information (which timescale? relative to what?) or (b)
//! **fabricating** a resolution that has not actually happened yet — an
//! event log that stores a plausible-looking media time for a
//! `splice_schedule` cue before any wall-clock↔media-clock mapping exists,
//! or for an `emsg` v0 before its segment's start is known, has invented
//! data. **The failure mode is not a crash: it is an ad break firing at the
//! wrong wall-clock instant**, because a plausible-but-wrong media time is
//! indistinguishable from a correct one until playout.
//!
//! **Why 90 kHz absolute, not per-track timescale.** A single `Media` can
//! carry several tracks at several timescales (48 kHz audio, a 25 fps
//! video track at 90 000, a subtitle track with none at all) — there is no
//! one track whose timescale the *event* log could borrow without an
//! arbitrary, undocumented choice among them. [`EventAnchor::Media`]
//! therefore carries [`timed_metadata::MediaTime`] — 90 kHz ticks,
//! wrap-unrolled, the same clock SCTE-35's own `pts_time` already uses —
//! rather than any one track's clock. This is also why the event log is a
//! genuinely separate ring from the sample rings, not a third
//! [`RetentionClass`]: a [`transmux::Sample`] is timestamped in its
//! *track's* clock ([`transmux::Sample::pts`]/`dts`, per §4 of the spec);
//! an event lives on the trunk's own, track-independent clock.
//!
//! **Carries [`timed_metadata::TimedEvent`], not a parallel type.** It is
//! owned, lossless, `#[non_exhaustive]`, and already published (0.4.0, live
//! on crates.io) — [`EventEntry::event`] stores it verbatim rather than
//! re-deriving a second event representation this crate would then have to
//! keep in sync by hand. `mp4_emsg::EmsgBox<'a>` is *borrowed* and cannot
//! outlive the buffer it was parsed from, so it cannot sit in a `'static`
//! ring; [`timed_metadata::SourcePayload::Emsg`] is already its owned form
//! (scheme/value/verbatim `message_data`), and is what ends up inside the
//! stored `TimedEvent` for an `emsg`-sourced entry.
//!
//! **The B1 crux: [`EventAnchor`] — an unresolved event stays honestly
//! unresolved.** Every entry's addressability is one of three states, and
//! there is deliberately no path from `Segment`/`Utc` to `Media` other than
//! the specific fact each one is waiting for actually arriving:
//!
//! - [`EventAnchor::Media`] — already on the trunk's 90 kHz clock (a
//!   `splice_time` PTS post-wrap-unroll, or an already-absolute `emsg` v1).
//! - [`EventAnchor::Segment`] — an `emsg` v0's `presentation_time_delta`
//!   plus the `segment_number` it is relative to. Stays exactly this
//!   variant — addressable by segment number, **not** by media time —
//!   until [`SegmentWriter::note_segment_start`] reports that segment's
//!   start, at which point this module's internal event log resolves it
//!   **in place**, computed from *that segment's own* reported start —
//!   never "whichever segment happens to be currently open", which would
//!   silently produce *a* segment instead of *the* segment the emsg
//!   actually named.
//! - [`EventAnchor::Utc`] — a GPS/UTC instant (`splice_schedule`) with no
//!   media-timeline position at all. Stays exactly this variant — not
//!   returned by [`Trunk::events_between`] or [`Trunk::events_in_segment`],
//!   because there is no honest media time to filter on — until
//!   [`SegmentWriter::set_time_anchor`] gives the event log a
//!   [`timed_metadata::TimeAnchor`] to translate through. This is the
//!   literal B1 test: an event with only a wall-clock time and no anchor
//!   must never be handed a fabricated media time.
//!
//! `epoch_ms_to_media` (the UTC→media direction) is the mirror image of
//! [`timed_metadata::TimeAnchor::media_to_epoch_ms`] (which only goes the
//! other way) — plain affine algebra, **not** a reimplementation of
//! [`timed_metadata::Timeline`]'s 33-bit wrap-unroll, which this module
//! reuses rather than hand-rolls: every `MediaTime` this ring ever stores
//! either came out of `Timeline::push_scte35` already unrolled, or is
//! computed from one that did (`Segment`/`Utc` resolution only ever adds a
//! non-negative delta or an anchor-relative offset to an already-unrolled
//! value).
//!
//! **Dual addressing: media time *and* segment, both, not either** — because
//! a manifest renderer needs "the events in segment N" while a playback
//! scheduler needs "the events between T1 and T2", and neither is a special
//! case of the other. [`Trunk::events_between`] answers the first
//! (half-open `[from, to)` over every currently-`Media`-resolved entry);
//! [`Trunk::events_in_segment`] answers the second, by consulting
//! `EventLog::segment_starts` — a small boundary table, populated by
//! [`SegmentWriter::note_segment_start`], bounded by the **same**
//! `TrunkConfig::event_capacity` rather than a second, independent knob
//! (exactly [`TrunkConfig::segment_capacity`]'s "no second capacity knob"
//! precedent for pinning). Both queries only ever return `Media`-resolved
//! entries — an entry still `Segment`/`Utc`-anchored is not fabricated a
//! position just to satisfy either query.
//!
//! Both point-in-time queries read the same log a subscribed
//! [`EventCursor`] does (via [`Trunk::subscribe_events`]) — the same
//! single-`Mutex`, single-digit-reader-by-design, in-band-loss-reporting,
//! writer-never-blocks shape [`Trunk::subscribe`]/[`Trunk::subscribe_segments`]
//! already established, reused verbatim rather than reconsidered: an
//! `EventCursor` sees an entry (and a `Lagged` loss report, if it fell
//! behind [`TrunkConfig::event_capacity`]'s eviction) the moment it is
//! published, whether or not it has resolved yet, while the two query
//! methods are a snapshot of what has resolved *so far*.
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
//!
//! # The live-part log: parts before their segment closes
//!
//! Step 3d built `ServedEgress`/`EgressResponse::Await` and, per its own
//! brief, read `hls-runtime/src/server/` before finishing to report what
//! did **not** fit. It found the segment log alone cannot serve LL-HLS at
//! all: RFC 8216bis's entire low-latency mechanism is **part-level**
//! availability ("does part 3 of the segment currently being written
//! exist"), and before this step there was nowhere in this `Trunk` to ask
//! that — the segment log holds only *finished* segments. This step adds a
//! fourth ring, the live-part log (`TrunkState::parts: PartLog`), storing
//! [`PartEntry`] exactly the way this module's internal `SegmentLog` stores [`SegmentEntry`] —
//! same evict-then-push shape, same zero-copy-fan-out claim (see
//! [Zero-copy fan-out](self#zero-copy-fan-out-honestly)) — bounded by the
//! new [`TrunkConfig::part_capacity`].
//!
//! **Addressed the way a client actually asks**: not a moving cursor
//! position, but a direct `(segment_number, part_index)` key —
//! [`Trunk::part_bytes`] and [`Trunk::parts_in_segment`], the live-part
//! counterparts of [`Trunk::events_between`]/[`Trunk::events_in_segment`].
//! This is deliberate, not an oversight of "should there also be a
//! `PartCursor`": a `ServedEgress` implementing LL-HLS resolves *random*
//! requests ("is part 3.2 ready") against whatever is currently true, not a
//! sequential stream of every part ever produced — exactly the same
//! resolve-a-request-against-shared-state shape
//! [`crate::egress::ServedEgress::resolve`]'s own module doc already argues
//! for the event log's snapshot queries. No `PartCursor` is added because no
//! test in this step (or in `crate::egress`) needs one; streaming every part
//! as it is produced (a hypothetical future low-latency `PushEgress`) is
//! additive later, not a gap today.
//!
//! **What happens when the parent segment closes — decided, not left
//! implicit**: [`SegmentWriter::publish_segment`] does **not** touch the
//! live-part log at all. A part stays addressable via [`Trunk::part_bytes`]
//! for exactly as long as [`TrunkConfig::part_capacity`]'s ordinary
//! evict-oldest bound has not yet reclaimed it — whether its parent segment
//! is still open or has already closed makes no difference to this ring.
//! Three alternatives were considered and rejected:
//!
//! - *Roll a closed segment's parts into its [`SegmentEntry`]* — rejected:
//!   `SegmentEntry::bytes` is already the whole muxed segment; attaching its
//!   parts too would store the same encoded media twice (once whole, once
//!   split), the opposite of this crate's zero-copy-fan-out discipline, for
//!   a property ([`Trunk::part_bytes`] already answers "is this part ready")
//!   nothing needs.
//! - *Evict a segment's parts the instant it closes* — rejected: this is
//!   the exact bug `hls_runtime::server::MediaStore`'s own `recent_parts`
//!   buffer exists to prevent (documented there as "the segmenter emits a
//!   segment's final part and closes the segment in the same pipeline
//!   step... without this the part is evicted microseconds after it
//!   appears — before the blocked part request can wake"). A `ServedEgress`
//!   built on this `Trunk` needs the same guarantee, and immediate eviction
//!   on close would remove it.
//! - *A second, shorter-lived "recently closed" bound, chained after the
//!   live bound* — this is what `MediaStore` actually does
//!   (`live_parts` + a separately-capped `recent_parts`, doubling worst-case
//!   retention) — rejected here as the "second knob" this file's precedent
//!   argues against: one bound, applied uniformly regardless of open/closed
//!   status, gives the same client-visible guarantee (a just-closed part
//!   stays fetchable) without a second, independently-tunable lifetime that
//!   can disagree with the first.
//!
//! **What a client requesting a just-rolled part receives**: the same
//! answer as the instant before the segment closed — `Some(bytes)` from
//! [`Trunk::part_bytes`] — because closing did not touch this ring. It
//! becomes `None` only once ordinary `part_capacity` eviction reclaims it,
//! at which point this ring cannot distinguish "evicted" from "never
//! existed"; a `ServedEgress` wanting RFC 8216bis's sharper "will never
//! exist, stop waiting" signal for a part of an *already-closed* segment
//! (`hls_runtime::server::MediaStore::resolve_resource`'s
//! `ResourceOutcome::NotFound` case) gets that distinction the same way
//! `MediaStore` itself does: by also consulting [`Trunk::last_closed_segment`]
//! — if the requested part's `segment_number` is at or before that value
//! and [`Trunk::part_bytes`] answers `None`, the part will never arrive; if
//! it is beyond it, the segment (and the part) may still be produced.
//!
//! # The reader-wake primitive: `listen`, not one registration per remote peer
//!
//! The second gap step 3d found, recorded rather than solved: every `Trunk`
//! reader was a synchronous, non-blocking `poll()`, so a `ServedEgress`
//! implementing RFC 8216bis §6.2.5.2 blocking reload had nothing to wait
//! *on* — only a poll-with-backoff loop. [`Trunk::listen`] closes that gap
//! by handing back a [`ProgressListener`] wrapping
//! [`event_listener::EventListener`] — the exact runtime-agnostic primitive
//! `hls_runtime::server::MediaStore::listen` already returns (an already
//! std+`event-listener`-feature dependency of this crate's sibling, and now
//! of this one), not a hand-rolled parallel mechanism, so a caller ports
//! mechanically: `.await` it under any executor, or call
//! [`ProgressListener::wait_deadline`] with no executor at all — precisely
//! `MediaStore::listen`'s own two documented ways to wait.
//!
//! **The writer never blocks on this.** [`SegmentWriter::publish_part`]/
//! [`SegmentWriter::publish_segment`] call `Event::notify(usize::MAX)`, which
//! wakes every currently-registered listener without waiting for any of
//! them to actually resume running — the same non-blocking-producer
//! guarantee this module makes everywhere else
//! ([`TrunkWriter::publish`]'s doc), extended to a wake channel instead of a
//! data ring. A registered [`ProgressListener`] that nobody ever polls or
//! waits on again (a vanished HTTP peer, a wedged executor) costs the
//! writer nothing beyond that one `notify` call's O(waiter-count) fan-out —
//! it never becomes a wait.
//!
//! **Bounded, and reusing [`TrunkConfig::part_capacity`] rather than a sixth
//! knob.** [`Trunk::listen`] refuses (`None`) once `part_capacity`
//! concurrent [`ProgressListener`]s are outstanding. This is deliberately
//! **not** sized "one registration per remote viewer": that would repeat
//! exactly the O(N)-in-cursor-count mistake [`Trunk::subscribe`]'s own docs
//! warn against for data cursors, now for wake registrations instead of
//! poll positions. The intended shape mirrors `subscribe`'s "one cursor per
//! distinct consumer, never one per peer" rule: a `ServedEgress` adapter
//! serving a thousand LL-HLS viewers takes **one** (or a small, fixed
//! number of) [`Trunk::listen`] registration(s) for the route and fans the
//! single wake-up out to its own thousand blocked HTTP handlers itself,
//! using its own broadcast mechanism — exactly the same layering
//! [`crate::egress::PushEgress`] already requires for sample fan-out. Under
//! that shape, `part_capacity`-many concurrent *distinct-consumer*
//! registrations is generous headroom, not a production ceiling; if a
//! caller instead wires one HTTP request directly to one `Trunk::listen`
//! call each (mirroring how `MediaStore`'s single, uncapped `Event` is used
//! today), the cap is exactly the backstop this step exists to add — a
//! caller hitting `None` must treat it the same way it treats an already-
//! expired [`crate::egress::AwaitPolicy`]: answer the request as
//! unavailable now rather than waiting with no slot to wait in. Composing
//! with [`crate::egress::AwaitPolicy`]'s deadline is the caller's
//! conversion of `AwaitPolicy::deadline` (a [`Timestamp`]) to the
//! `std::time::Instant` it already anchors that `Timestamp` to, passed to
//! [`ProgressListener::wait_deadline`] (or wrapped in the caller's own
//! executor timeout around the `Future` impl) — see
//! [`ProgressListener::wait_deadline`]'s own doc.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use broadcast_common::stage::Timestamp;
use bytes::Bytes;
use event_listener::{Event, EventListener, Listener};
use timed_metadata::{MediaTime, PTS_HZ, TimeAnchor, TimedEvent};
use transmux::{Sample, SegmentMeta, TrackSpec};

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
/// # Why every capacity is a [`NonZeroUsize`], not a validated `usize`
///
/// A zero capacity is not a value this type rejects — it is a value this type
/// **cannot represent**. Every ring in this module evicts its oldest entry
/// when `entries.len() == capacity`, so a zero capacity would evict every
/// entry the instant it was pushed, and a zero waiter cap would make
/// [`Trunk::listen`] incapable of ever registering anybody: not a
/// configuration, a broken one.
///
/// Two weaker designs were considered and rejected:
///
/// - **Panicking on zero in [`Trunk::new`]** (what this type did before):
///   internally consistent, but a library that panics on a value which
///   arrives *from a file* is a real operational hazard, not a style
///   question — `multimux` takes its routes from a JSON config, so once
///   these capacities become operator-configurable a stray `0` would take
///   down the server process instead of producing a config error. It also
///   contradicted `transmux::ProgressiveDemux::new`'s deliberate
///   panic-to-fallible change, which is exactly the kind of
///   two-crates-apart inconsistency that makes an API feel arbitrary.
/// - **A fallible `TrunkConfig::new -> Result<Self, _>`** (the
///   `ProgressiveDemux` shape): correct, but strictly worse *here*.
///   `ProgressiveDemux` already returns `Result` as part of its `Stage`
///   contract and already has an `Error` type; `TrunkConfig` has neither, so
///   this would mean inventing a construction error type and threading
///   `?`/`unwrap` through every construction site to encode one bit of
///   information the type system can carry for free. `NonZeroUsize` puts
///   the invariant *in the signature*, where a reader learns it without
///   reading this doc — and, for the JSON-config hazard specifically, a
///   `serde` deserialize of `0` into a `NonZeroUsize` field already fails as
///   an ordinary deserialization error at the config boundary, with no
///   hand-written check and no panic.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct TrunkConfig {
    /// Bound, in entry count, on the [`RetentionClass::Timed`] ring.
    pub timed_capacity: NonZeroUsize,
    /// Bound, in entry count, on the [`RetentionClass::Sparse`] ring —
    /// independent of `timed_capacity`; see [`RetentionClass::Sparse`] for
    /// why that independence is the entire point of the retention rule.
    pub sparse_capacity: NonZeroUsize,
    /// Bound, in entry count, on the segment log. **Also** the bound a
    /// pinning [`SegmentCursor`]'s retention is measured against — see
    /// [The DVR contradiction](self#the-dvr-contradiction-losslessness-from-retention-not-back-pressure)
    /// for why there is deliberately no second, independent "pin depth"
    /// knob.
    pub segment_capacity: NonZeroUsize,
    /// Bound, in entry count, on the event log — **and** on its segment
    /// boundary table (`EventLog::segment_starts`). See
    /// [The event log](self#the-event-log-90-khz-absolute-and-the-b1-crux)
    /// for why a segment-relative event's target boundary shares this one
    /// knob rather than getting a second, independently-tuned one —
    /// exactly [`TrunkConfig::segment_capacity`]'s "no second capacity
    /// knob" precedent for pinning.
    pub event_capacity: NonZeroUsize,
    /// Bound, in entry count, on the live-part log (step 3b-iv) — **and**
    /// on how many concurrent [`Trunk::listen`] registrations this trunk
    /// will honor at once. See
    /// [The live-part log](self#the-live-part-log-parts-before-their-segment-closes)
    /// for why a part-of-the-open-segment shares this one knob for both
    /// jobs, rather than getting a second, independently-tuned "how many
    /// waiters" setting — the third instance of this file's "no second
    /// capacity knob" precedent (after [`TrunkConfig::segment_capacity`]'s
    /// pin reuse and [`TrunkConfig::event_capacity`]'s `segment_starts`
    /// reuse).
    pub part_capacity: NonZeroUsize,
}

impl TrunkConfig {
    /// Build a config with all five ring capacities. Nothing is validated
    /// here, and nothing needs to be: [`NonZeroUsize`] makes the only
    /// invalid value unrepresentable rather than merely rejected — see
    /// [this type's own docs](TrunkConfig#why-every-capacity-is-a-nonzerousize-not-a-validated-usize)
    /// for why that beats both the panic this replaced and a fallible
    /// constructor.
    pub fn new(
        timed_capacity: NonZeroUsize,
        sparse_capacity: NonZeroUsize,
        segment_capacity: NonZeroUsize,
        event_capacity: NonZeroUsize,
        part_capacity: NonZeroUsize,
    ) -> Self {
        TrunkConfig {
            timed_capacity,
            sparse_capacity,
            segment_capacity,
            event_capacity,
            part_capacity,
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
/// whoever is feeding [`SegmentWriter::publish_segment`], exactly as
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
    /// Apply real back-pressure: [`SegmentWriter::publish_segment`] blocks
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
/// [`SegmentWriter::publish_segment`], to decide whether evicting the oldest
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
/// evict unconditionally; [`SegmentWriter::publish_segment`] consults that
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
    /// called; see [`SegmentWriter::publish_segment`].
    fn push(&mut self, entry: SegmentEntry) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
            self.base += 1;
        }
        self.entries.push_back(entry);
        self.published += 1;
    }
}

/// How one [`EventEntry`] is currently addressable on the trunk's 90 kHz
/// absolute clock ([`timed_metadata::MediaTime`]) — the distinction
/// architecture-audit finding B1 exists to make honest. See
/// [The event log](self#the-event-log-90-khz-absolute-and-the-b1-crux) for
/// why these three states cannot be collapsed into one `MediaTime` without
/// reintroducing B1's silent-wrong-instant failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventAnchor {
    /// Already expressible on this trunk's 90 kHz absolute clock — a
    /// SCTE-35 `splice_time` PTS after [`timed_metadata::Timeline`]'s
    /// 33-bit wrap-unroll, or an `emsg` v1 (already-absolute)
    /// `presentation_time` on this same clock. The only variant
    /// [`Trunk::events_between`]/[`Trunk::events_in_segment`] can ever
    /// match against.
    Media(MediaTime),
    /// Segment-relative (`emsg` v0's `presentation_time_delta`, ISO/IEC
    /// 23009-1 §5.10.3.3): this event's media time is `delta` ticks after
    /// the *start* of segment `segment_number` — a start this entry does
    /// not know yet. Resolves in place, to that segment's own reported
    /// start, the instant [`SegmentWriter::note_segment_start`] reports it;
    /// until then it stays exactly this variant — addressable by
    /// `segment_number` (once a boundary exists), never by a fabricated
    /// media time.
    Segment {
        /// The target segment's sequence number — matches
        /// [`SegmentEntry::sequence_number`].
        segment_number: u32,
        /// `presentation_time_delta`: ticks after that segment's start.
        delta: u64,
    },
    /// GPS/UTC wall-clock only (SCTE-35 `splice_schedule.utc_splice_time`,
    /// §9.7.4): this event has **no** media-timeline position at all, only
    /// an instant on the wall clock, until
    /// [`SegmentWriter::set_time_anchor`] gives the event log a
    /// [`TimeAnchor`] to translate through. **This is the B1 case** — see
    /// [The event log](self#the-event-log-90-khz-absolute-and-the-b1-crux).
    Utc {
        /// Milliseconds since the Unix epoch — matches
        /// [`TimeAnchor::utc_epoch_ms`]'s unit.
        utc_epoch_ms: i64,
    },
}

/// One entry in the event log: the owned, lossless [`TimedEvent`] this
/// trunk carries verbatim — see
/// [The event log](self#the-event-log-90-khz-absolute-and-the-b1-crux) for
/// why this is *the* published `timed_metadata` type, not a parallel one —
/// plus its current [`EventAnchor`] resolution state.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EventEntry {
    /// The canonical event, carried verbatim.
    pub event: TimedEvent,
    /// This entry's current resolution state.
    pub anchor: EventAnchor,
}

/// The event log: a bounded, append-ordered log of [`EventEntry`] values,
/// plus the two small resolution tables an [`EventAnchor::Segment`]/
/// [`EventAnchor::Utc`] entry resolves against.
///
/// Evict-then-push shape identical to [`ClassLog`]/[`SegmentLog`] —
/// `base`/`published` mean exactly the same thing here as there.
struct EventLog {
    entries: VecDeque<EventEntry>,
    base: u64,
    published: u64,
    capacity: usize,
    /// Recently-reported segment starts, in the order
    /// [`SegmentWriter::note_segment_start`] received them (playlist order in
    /// practice, since segments are announced in sequence). Bounded by the
    /// **same** `capacity` as `entries` — see [`TrunkConfig::event_capacity`]'s
    /// doc for why this deliberately is not a second, independently-tuned
    /// knob.
    segment_starts: VecDeque<(u32, MediaTime)>,
    /// The one wall-clock↔media-clock mapping this trunk's event log
    /// knows, if any. Mirrors [`timed_metadata::Timeline`]'s own
    /// `anchor: Option<TimeAnchor>` field — one mapping per session/trunk,
    /// not one per event.
    time_anchor: Option<TimeAnchor>,
}

impl EventLog {
    fn new(capacity: usize) -> Self {
        EventLog {
            entries: VecDeque::with_capacity(capacity),
            base: 0,
            published: 0,
            capacity,
            segment_starts: VecDeque::with_capacity(capacity),
            time_anchor: None,
        }
    }

    /// Resolve `anchor` against whatever segment starts / time anchor are
    /// already known — **without** fabricating a resolution the log cannot
    /// yet justify. An anchor this call cannot resolve is returned
    /// unchanged: no anchor, no media time, per B1.
    fn try_resolve(&self, anchor: EventAnchor) -> EventAnchor {
        match anchor {
            EventAnchor::Segment {
                segment_number,
                delta,
            } => self
                .segment_starts
                .iter()
                .find(|(n, _)| *n == segment_number)
                .map(|(_, start)| EventAnchor::Media(MediaTime(start.0.saturating_add(delta))))
                .unwrap_or(anchor),
            EventAnchor::Utc { utc_epoch_ms } => self
                .time_anchor
                .as_ref()
                .map(|a| EventAnchor::Media(epoch_ms_to_media(a, utc_epoch_ms)))
                .unwrap_or(anchor),
            EventAnchor::Media(_) => anchor,
        }
    }

    /// Push one event, evicting the oldest if the log is already at
    /// `capacity`. Never rejects, never blocks — exactly [`ClassLog::push`]/
    /// [`SegmentLog::push`]'s contract.
    fn push(&mut self, event: TimedEvent, anchor: EventAnchor) {
        let anchor = self.try_resolve(anchor);
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
            self.base += 1;
        }
        self.entries.push_back(EventEntry { event, anchor });
        self.published += 1;
    }

    /// Record segment `segment_number`'s start on this trunk's 90 kHz
    /// absolute clock, and resolve, **in place**, every still-pending
    /// [`EventAnchor::Segment`] entry that targets exactly this
    /// `segment_number` — not whichever segment happened to be open when
    /// the event was published (that would resolve to *a* segment, not
    /// *the* segment the `emsg` actually named, which is exactly the bug
    /// this design avoids).
    fn note_segment_start(&mut self, segment_number: u32, start: MediaTime) {
        if self.segment_starts.len() == self.capacity {
            self.segment_starts.pop_front();
        }
        self.segment_starts.push_back((segment_number, start));
        for entry in &mut self.entries {
            if let EventAnchor::Segment {
                segment_number: n,
                delta,
            } = entry.anchor
                && n == segment_number
            {
                entry.anchor = EventAnchor::Media(MediaTime(start.0.saturating_add(delta)));
            }
        }
    }

    /// Record this trunk's wall-clock↔media-clock mapping, and resolve, in
    /// place, every still-pending [`EventAnchor::Utc`] entry through it.
    /// Before this call, a `Utc`-anchored entry stays a `Utc` entry — see
    /// [The event log](self#the-event-log-90-khz-absolute-and-the-b1-crux).
    fn set_time_anchor(&mut self, anchor: TimeAnchor) {
        self.time_anchor = Some(anchor);
        for entry in &mut self.entries {
            if let EventAnchor::Utc { utc_epoch_ms } = entry.anchor {
                entry.anchor = EventAnchor::Media(epoch_ms_to_media(&anchor, utc_epoch_ms));
            }
        }
    }
}

/// The inverse of [`TimeAnchor::media_to_epoch_ms`]: the [`MediaTime`]
/// `anchor` implies for a UTC instant (milliseconds since the Unix epoch).
///
/// Plain affine algebra — the mirror image of a function `timed_metadata`
/// already publishes — **not** a reimplementation of
/// [`timed_metadata::Timeline`]'s 33-bit wrap-unroll, a different, modular
/// arithmetic problem this module does not re-solve; see
/// [The event log](self#the-event-log-90-khz-absolute-and-the-b1-crux).
/// Clamps rather than panics on an out-of-range result — a malformed or
/// adversarial `splice_schedule` entry must not crash the writer.
fn epoch_ms_to_media(anchor: &TimeAnchor, utc_epoch_ms: i64) -> MediaTime {
    let delta_ms = i128::from(utc_epoch_ms) - i128::from(anchor.utc_epoch_ms);
    let delta_ticks = delta_ms * i128::from(PTS_HZ) / 1000;
    let media = i128::from(anchor.pts_90k) + delta_ticks;
    MediaTime(media.clamp(0, i128::from(u64::MAX)) as u64)
}

/// One LL-HLS **partial segment** ("part") of the segment currently being
/// written — RFC 8216bis §4.4.4.9's independently-fetchable CMAF chunk,
/// addressable by `(segment_number, part_index)` the way a client actually
/// asks for one (`_HLS_msn`/`_HLS_part`, or a `part-<seq>.<idx>.m4s` URI). See
/// [The live-part log](self#the-live-part-log-parts-before-their-segment-closes).
///
/// Does **not** reuse `transmux::ll_hls::PartInfo` whole, for the same reason
/// [`SegmentEntry`] does not reuse `transmux::ll_hls::SegmentInfo` whole:
/// `PartInfo::bytes` is `Vec<u8>`, and copying it into a `Bytes` here to get
/// zero-copy fan-out ([Zero-copy fan-out](self#zero-copy-fan-out-honestly))
/// would be exactly one copy per part, on the one path this module exists to
/// keep copy-free; a caller publishing a part therefore builds a
/// `bytes::Bytes` directly (e.g. from the encoder's own output buffer)
/// instead of routing through `Vec<u8>` first.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PartEntry {
    /// The part's encoded bytes: a bare `moof`+`mdat` CMAF fragment (no
    /// `styp`). `Bytes`, not `Vec<u8>` — fan-out to every reader of this
    /// entry is a refcount bump, not a copy; see
    /// [Zero-copy fan-out](self#zero-copy-fan-out-honestly).
    pub bytes: Bytes,
    /// The parent segment's sequence number — matches
    /// [`SegmentEntry::sequence_number`] once that segment closes.
    pub segment_number: u32,
    /// 0-based index of this part within its parent segment.
    pub part_index: u32,
    /// This part's duration, wall-clock.
    pub duration: Duration,
    /// `true` when this part's first sample is a sync sample, so it begins
    /// with an independently decodable frame (RFC 8216bis's
    /// `INDEPENDENT=YES`).
    pub independent: bool,
}

impl PartEntry {
    /// Build one part log entry.
    pub fn new(
        bytes: impl Into<Bytes>,
        segment_number: u32,
        part_index: u32,
        duration: Duration,
        independent: bool,
    ) -> Self {
        PartEntry {
            bytes: bytes.into(),
            segment_number,
            part_index,
            duration,
            independent,
        }
    }
}

/// The live-part log: a bounded, append-ordered log of [`PartEntry`] values.
///
/// Evict-then-push shape identical to [`ClassLog`]/[`SegmentLog`]/[`EventLog`]
/// — `base`/`published` mean exactly the same thing here as there. Unlike the
/// segment log, publishing a segment ([`SegmentWriter::publish_segment`]) does
/// **not** touch this ring at all — see
/// [The live-part log](self#the-live-part-log-parts-before-their-segment-closes)
/// for why a part's addressability deliberately does not change the instant
/// its parent segment closes.
struct PartLog {
    entries: VecDeque<PartEntry>,
    base: u64,
    published: u64,
    capacity: usize,
}

impl PartLog {
    fn new(capacity: usize) -> Self {
        PartLog {
            entries: VecDeque::with_capacity(capacity),
            base: 0,
            published: 0,
            capacity,
        }
    }

    /// Push one part, evicting the oldest if the log is already at
    /// `capacity`. Never rejects, never blocks — exactly [`ClassLog::push`]/
    /// [`SegmentLog::push`]/[`EventLog::push`]'s contract.
    fn push(&mut self, entry: PartEntry) {
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
            self.base += 1;
        }
        self.entries.push_back(entry);
        self.published += 1;
    }
}

/// The shared state behind one [`Trunk`]: the two sample [`ClassLog`]s, the
/// [`SegmentLog`], the [`EventLog`], and the [`PartLog`]. See
/// [The event log](self#the-event-log-90-khz-absolute-and-the-b1-crux) for
/// why the event log needed its own shape rather than being a third copy of
/// the other two, and
/// [The live-part log](self#the-live-part-log-parts-before-their-segment-closes)
/// for the fourth.
struct TrunkState {
    timed: ClassLog,
    sparse: ClassLog,
    segments: SegmentLog,
    events: EventLog,
    parts: PartLog,
    /// The program's current complete track set — see
    /// [`TrunkWriter::set_tracks`] for why this is always a full replacement
    /// snapshot, never a delta. `Arc<[TrackSpec]>` rather than a bare `Vec`
    /// so [`Trunk::tracks`] hands back a clone of the *reference*, not the
    /// whole set, to every caller — cheap even for a many-track program.
    tracks: Arc<[TrackSpec]>,
    /// Bumped by exactly one on every [`TrunkWriter::set_tracks`] call — see
    /// [`Trunk::track_generation`] for why a consumer compares this instead
    /// of the track [`Vec`] itself.
    track_generation: u64,
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
    /// Wakes a [`SegmentWriter::publish_segment`] parked on
    /// [`ArchiveOverrun::StallIngest`] once a pin it is waiting on advances
    /// (a [`SegmentCursor::poll`] consuming further) or is released (its
    /// cursor dropped). Paired with `state` in the usual `Condvar` idiom:
    /// `wait` atomically releases the `Mutex` while parked, so a stalled
    /// segment publish does not hold the lock other `Trunk` operations
    /// (sample publish, any cursor's `poll`) need — see
    /// [The DVR contradiction](self#the-dvr-contradiction-losslessness-from-retention-not-back-pressure).
    segment_pin_released: Condvar,
    /// Guards [`Trunk::writer`]'s single-take — the **samples + events** ring
    /// group. See [One writer per ring group](self#one-writer-per-ring-group-not-one-writer-per-trunk).
    writer_taken: AtomicBool,
    /// Guards [`Trunk::segment_writer`]'s single-take — the
    /// **segments + parts** ring group, taken independently of
    /// `writer_taken` so a segmenter and the ingest driver can each hold
    /// their own write handle at once. See
    /// [One writer per ring group](self#one-writer-per-ring-group-not-one-writer-per-trunk).
    segment_writer_taken: AtomicBool,
    /// Broad "a part or a segment close was just published, go re-check
    /// your condition" notification — see
    /// [The reader-wake primitive](self#the-reader-wake-primitive-listen-not-one-registration-per-remote-peer).
    /// Bumped by exactly [`SegmentWriter::publish_part`]/
    /// [`SegmentWriter::publish_segment`] and, since the ingress track-set
    /// plumbing (issue #781), [`TrunkWriter::set_tracks`] — never by a
    /// sample/event publish (nothing today waits on those through this
    /// channel). Track-set changes are rare compared to samples/parts, so
    /// folding them into this same broad wake is additive scope, not a new
    /// channel to reason about.
    progress: Event,
    /// Count of currently-registered, not-yet-dropped [`ProgressListener`]s —
    /// what bounds [`Trunk::listen`] against `part_waiter_cap`. A plain
    /// `AtomicUsize`, not part of `state`'s `Mutex`, so registering/releasing
    /// a listener never contends the same lock `publish`/`poll` do.
    waiter_count: AtomicUsize,
    /// Copy of [`TrunkConfig::part_capacity`], read without locking `state` —
    /// the cap [`Trunk::listen`] enforces against `waiter_count`. See
    /// [The reader-wake primitive](self#the-reader-wake-primitive-listen-not-one-registration-per-remote-peer)
    /// for why this reuses `part_capacity` rather than adding a sixth,
    /// independent knob.
    part_waiter_cap: usize,
}

impl Trunk {
    /// Construct a fresh, empty `Trunk`.
    ///
    /// Cannot fail and cannot panic on its configuration: every
    /// [`TrunkConfig`] capacity is a [`NonZeroUsize`], so the one invalid
    /// value (zero — a ring that evicts every entry the instant it is
    /// pushed) is unrepresentable rather than merely rejected. See
    /// [`TrunkConfig`]'s own docs for why that replaced this method's
    /// former five `assert!`s.
    pub fn new(config: TrunkConfig) -> Arc<Trunk> {
        Arc::new(Trunk {
            state: Mutex::new(TrunkState {
                timed: ClassLog::new(config.timed_capacity.get()),
                sparse: ClassLog::new(config.sparse_capacity.get()),
                segments: SegmentLog::new(config.segment_capacity.get()),
                events: EventLog::new(config.event_capacity.get()),
                parts: PartLog::new(config.part_capacity.get()),
                tracks: Arc::from(Vec::new()),
                track_generation: 0,
            }),
            segment_pin_released: Condvar::new(),
            writer_taken: AtomicBool::new(false),
            segment_writer_taken: AtomicBool::new(false),
            progress: Event::new(),
            waiter_count: AtomicUsize::new(0),
            part_waiter_cap: config.part_capacity.get(),
        })
    }

    /// Take the one [`TrunkWriter`] for this `Trunk` — the write handle for
    /// the **samples + events** ring group ([`TrunkWriter::publish`]/
    /// [`TrunkWriter::publish_event`]). See
    /// [One writer per ring group](self#one-writer-per-ring-group-not-one-writer-per-trunk)
    /// for the invariant this enforces (and why it does not also cover
    /// [`Trunk::segment_writer`]'s group).
    ///
    /// Returns `None` on every call after the first — this ring group has
    /// exactly one writer, enforced here rather than left as a
    /// documented-only convention, because a second concurrent sample/event
    /// writer would silently interleave two unrelated publish sequences into
    /// the same ring with no way for a reader to tell them apart.
    pub fn writer(self: &Arc<Self>) -> Option<TrunkWriter> {
        self.writer_taken
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| TrunkWriter {
                trunk: Arc::clone(self),
            })
    }

    /// Take the one [`SegmentWriter`] for this `Trunk` — the write handle for
    /// the **segments + parts** ring group ([`SegmentWriter::publish_segment`]/
    /// [`SegmentWriter::publish_part`]/[`SegmentWriter::note_segment_start`]/
    /// [`SegmentWriter::set_time_anchor`]), independent of [`Trunk::writer`]'s
    /// group so a segmenter can hold this while the ingest driver
    /// simultaneously holds a [`TrunkWriter`] — see
    /// [One writer per ring group](self#one-writer-per-ring-group-not-one-writer-per-trunk)
    /// for why the split is safe and what it does and does not guarantee
    /// across rings.
    ///
    /// Returns `None` on every call after the first — this ring group has
    /// exactly one writer too, guarded by its own `AtomicBool` rather than
    /// [`Trunk::writer`]'s, for exactly the same reason: a second concurrent
    /// segment/part writer would silently interleave two unrelated publish
    /// sequences into the segment or part ring.
    pub fn segment_writer(self: &Arc<Self>) -> Option<SegmentWriter> {
        self.segment_writer_taken
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| SegmentWriter {
                trunk: Arc::clone(self),
            })
    }

    /// Subscribe a new [`SampleCursor`], starting from *now* — the next
    /// entry [`TrunkWriter::publish`] produces after this call, not any
    /// backlog already in either ring. See [`Trunk::subscribe_from_backlog`]
    /// for the seek-to-past variant this method's own docs used to
    /// anticipate (a consumer built *after* samples it needs already landed
    /// in the ring — e.g. a segmenter reacting to the same batch that
    /// announced its program — wants that one instead).
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
        let state = self.state.lock().expect("Trunk state lock poisoned");
        SampleCursor {
            trunk: Arc::clone(self),
            timed_consumed: state.timed.published,
            sparse_consumed: state.sparse.published,
        }
    }

    /// Subscribe a new [`SampleCursor`], starting from the **oldest entry
    /// each ring currently retains** instead of [`Trunk::subscribe`]'s
    /// "now" — i.e. this cursor's first `poll` replays whatever backlog is
    /// still resident in the `Timed` ring and the `Sparse` ring, each
    /// independently, before catching up to the live tail.
    ///
    /// This is the "seek-to-past variant" [`Trunk::subscribe`]'s own docs
    /// anticipated ("a later step may add a seek-to-past variant... this
    /// step does not need one") — the step turned out to be issue #808's
    /// segment-bridge fix: a [`TrunkWriter::publish`] batch that lands
    /// *before* a consumer subscribes (e.g. the very same `feed` call that
    /// both announces a program and publishes its first samples) is
    /// otherwise invisible to a [`Trunk::subscribe`] cursor forever, even
    /// though the samples are sitting right there in the ring.
    ///
    /// # Replay is bounded by ring capacity, not "everything ever published"
    ///
    /// This does **not** reach further back than what each ring still
    /// holds: an entry already evicted by [`TrunkConfig::timed_capacity`]/
    /// [`TrunkConfig::sparse_capacity`] before this call is gone, exactly as
    /// it would be for any other cursor — there is no unbounded replay log
    /// behind this method, only the same fixed-size rings every other
    /// cursor reads. Concretely: this cursor starts at each ring's current
    /// `base` (the oldest index still resident), not index 0, so its first
    /// `poll` never reports a spurious `Lagged`/`Degraded` for data that was
    /// evicted *before* this call — from this cursor's point of view,
    /// "backlog" means "what the ring can show me right now", not "what was
    /// ever published". Both retention classes replay this way,
    /// independently: a `Timed` backlog and a `Sparse` backlog are each
    /// bounded by their own ring's own capacity.
    ///
    /// [`SampleCursorItem::Lagged`]/[`SampleCursorItem::Degraded`] still
    /// fire exactly as they do for a [`Trunk::subscribe`] cursor for any
    /// loss that happens **after** this call — falling behind the live tail
    /// once subscribed is reported in-band the same way for both kinds of
    /// cursor; only the starting position differs.
    ///
    /// # This call *is* fan-out — read this before calling it per connection
    ///
    /// Exactly [`Trunk::subscribe`]'s own fan-out warning, verbatim: writer
    /// cost is **O(N) in cursor count** (`spikes/trunk-bench` measured 956 ns
    /// → 9.98 µs from 1 → 16 readers; spec §3.1) — every cursor contends the
    /// same shared lock every publish. **A cursor is for a distinct
    /// consumer of the stream**, **never** one per peer of a one-to-many
    /// protocol. Supported reader count is **single-digit by design**; do
    /// not call this once per connection.
    pub fn subscribe_from_backlog(self: &Arc<Self>) -> SampleCursor {
        let state = self.state.lock().expect("Trunk state lock poisoned");
        SampleCursor {
            trunk: Arc::clone(self),
            timed_consumed: state.timed.base,
            sparse_consumed: state.sparse.base,
        }
    }

    /// Diagnostic: entries currently resident in the `Timed` ring. Never
    /// exceeds [`TrunkConfig::timed_capacity`].
    pub fn timed_len(&self) -> usize {
        self.state
            .lock()
            .expect("Trunk state lock poisoned")
            .timed
            .entries
            .len()
    }

    /// Diagnostic: entries currently resident in the `Sparse` ring. Never
    /// exceeds [`TrunkConfig::sparse_capacity`].
    pub fn sparse_len(&self) -> usize {
        self.state
            .lock()
            .expect("Trunk state lock poisoned")
            .sparse
            .entries
            .len()
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
        let state = self.state.lock().expect("Trunk state lock poisoned");
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
        let mut state = self.state.lock().expect("Trunk state lock poisoned");
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
        self.state
            .lock()
            .expect("Trunk state lock poisoned")
            .segments
            .entries
            .len()
    }

    /// Subscribe a new [`EventCursor`] over the event log, starting from
    /// *now* — the same "next entry only, no backlog" rule as
    /// [`Trunk::subscribe`]/[`Trunk::subscribe_segments`], and the same
    /// single-digit-reader, one-cursor-per-distinct-consumer guidance
    /// applies here verbatim (this cursor contends exactly the lock every
    /// other cursor does).
    ///
    /// A streaming consumer — e.g. a playback scheduler that wants every
    /// event as it resolves — wants this. A point-in-time query — "what has
    /// resolved for segment N" (a manifest renderer), or "what resolved
    /// between T1 and T2" (that same scheduler, replaying its window) —
    /// wants [`Trunk::events_in_segment`]/[`Trunk::events_between`] instead;
    /// both read the same log, just as a snapshot rather than a moving
    /// position. See
    /// [The event log](self#the-event-log-90-khz-absolute-and-the-b1-crux).
    pub fn subscribe_events(self: &Arc<Self>) -> EventCursor {
        let state = self.state.lock().expect("Trunk state lock poisoned");
        EventCursor {
            trunk: Arc::clone(self),
            consumed: state.events.published,
        }
    }

    /// Every currently-**resolved** ([`EventAnchor::Media`]) event whose
    /// media time falls in the half-open range `[from, to)` — start
    /// inclusive, end exclusive. An entry still `Segment`/`Utc`-anchored
    /// never appears here: it has no honest media time yet, and
    /// fabricating one to satisfy this query would be exactly B1 — see
    /// [The event log](self#the-event-log-90-khz-absolute-and-the-b1-crux).
    pub fn events_between(&self, from: MediaTime, to: MediaTime) -> Vec<EventEntry> {
        let state = self.state.lock().expect("Trunk state lock poisoned");
        state
            .events
            .entries
            .iter()
            .filter(|e| matches!(e.anchor, EventAnchor::Media(t) if t.0 >= from.0 && t.0 < to.0))
            .cloned()
            .collect()
    }

    /// Every currently-resolved event whose media time falls within segment
    /// `segment_number`'s span: `[start_N, start_{N+1})` once
    /// [`SegmentWriter::note_segment_start`] has reported the *next*
    /// segment's start too, else `[start_N, ∞)` (the segment is still open
    /// — nothing yet says where it ends). Returns nothing for a
    /// `segment_number` this trunk has never reported a start for: there is
    /// no span to contain anything, and an unresolved
    /// [`EventAnchor::Segment`] entry targeting it is not returned either,
    /// for the same B1 reason [`Trunk::events_between`] documents.
    pub fn events_in_segment(&self, segment_number: u32) -> Vec<EventEntry> {
        let state = self.state.lock().expect("Trunk state lock poisoned");
        let log = &state.events;
        let Some(&(_, start)) = log
            .segment_starts
            .iter()
            .find(|(n, _)| *n == segment_number)
        else {
            return Vec::new();
        };
        let end = log
            .segment_starts
            .iter()
            .find(|(n, _)| *n == segment_number + 1)
            .map(|&(_, s)| s.0);
        log.entries
            .iter()
            .filter(|e| match e.anchor {
                EventAnchor::Media(t) => t.0 >= start.0 && end.map(|e2| t.0 < e2).unwrap_or(true),
                _ => false,
            })
            .cloned()
            .collect()
    }

    /// Diagnostic: entries currently resident in the event log. Never
    /// exceeds [`TrunkConfig::event_capacity`].
    pub fn event_len(&self) -> usize {
        self.state
            .lock()
            .expect("Trunk state lock poisoned")
            .events
            .entries
            .len()
    }

    /// A live part's bytes by `(segment_number, part_index)` — the direct,
    /// `&self`-shaped query a [`ServedEgress`](crate::egress::ServedEgress)
    /// implementation needs to answer "does this part exist right now",
    /// exactly the shape [`Trunk::events_between`]/[`Trunk::events_in_segment`]
    /// already give the event log rather than forcing a caller to drain a
    /// cursor into a self-maintained cache. See
    /// [The live-part log](self#the-live-part-log-parts-before-their-segment-closes)
    /// for why a part answers `Some` here for as long as it has not been
    /// evicted by [`TrunkConfig::part_capacity`]'s ordinary bound —
    /// including after its parent segment has closed.
    pub fn part_bytes(&self, segment_number: u32, part_index: u32) -> Option<Bytes> {
        let state = self.state.lock().expect("Trunk state lock poisoned");
        state
            .parts
            .entries
            .iter()
            .find(|p| p.segment_number == segment_number && p.part_index == part_index)
            .map(|p| p.bytes.clone())
    }

    /// Every currently-resident part of segment `segment_number`, in publish
    /// order — the part-log counterpart of [`Trunk::events_in_segment`],
    /// letting a caller derive "how many parts does the open segment have so
    /// far" (RFC 8216bis's `_HLS_part` blocking-reload condition) without a
    /// cursor.
    pub fn parts_in_segment(&self, segment_number: u32) -> Vec<PartEntry> {
        let state = self.state.lock().expect("Trunk state lock poisoned");
        state
            .parts
            .entries
            .iter()
            .filter(|p| p.segment_number == segment_number)
            .cloned()
            .collect()
    }

    /// Diagnostic: entries currently resident in the live-part log. Never
    /// exceeds [`TrunkConfig::part_capacity`].
    pub fn part_len(&self) -> usize {
        self.state
            .lock()
            .expect("Trunk state lock poisoned")
            .parts
            .entries
            .len()
    }

    /// Diagnostic: currently-outstanding [`ProgressListener`] registrations
    /// (from [`Trunk::listen`], not yet dropped). Never exceeds
    /// [`TrunkConfig::part_capacity`] — see
    /// [The reader-wake primitive](self#the-reader-wake-primitive-listen-not-one-registration-per-remote-peer).
    pub fn waiter_count(&self) -> usize {
        self.waiter_count.load(Ordering::Acquire)
    }

    /// The sequence number of the most-recently-**closed** segment (the
    /// newest [`SegmentWriter::publish_segment`] call), or `None` if no
    /// segment has closed yet. Distinguishes "closed" (a whole, fetchable
    /// [`SegmentEntry`]) from merely "has live parts" — RFC 8216bis
    /// §6.2.5.2's bare-`_HLS_msn` blocking-reload condition needs exactly
    /// this distinction (mirrors
    /// `hls_runtime::server::MediaStore::last_closed_segment_seq`, which
    /// this method lets a `ServedEgress` stop duplicating).
    pub fn last_closed_segment(&self) -> Option<u32> {
        self.state
            .lock()
            .expect("Trunk state lock poisoned")
            .segments
            .entries
            .back()
            .map(|e| e.sequence_number)
    }

    /// This program's current complete track set — see
    /// [`TrunkWriter::set_tracks`] for how it is set/replaced. Empty until
    /// the first `set_tracks` call (a freshly-minted `Trunk` announces no
    /// tracks yet). Stored as `Arc<[TrackSpec]>`, so this is a cheap `Arc`
    /// clone (a refcount bump), never a `Vec` copy, however many tracks the
    /// program carries.
    pub fn tracks(&self) -> Arc<[TrackSpec]> {
        Arc::clone(&self.state.lock().expect("Trunk state lock poisoned").tracks)
    }

    /// Bumped by exactly one on every [`TrunkWriter::set_tracks`] call
    /// (including one that happens to set an identical set to what was
    /// already there — this counts *calls*, not distinct sets). Lets a
    /// consumer detect "the track set may have changed" by comparing two
    /// `u64`s rather than diffing two `Vec<TrackSpec>`s — cheap regardless
    /// of how many tracks a program carries. `0` until the first
    /// `set_tracks` call.
    pub fn track_generation(&self) -> u64 {
        self.state
            .lock()
            .expect("Trunk state lock poisoned")
            .track_generation
    }

    /// Register for the next part/segment-close notification — see
    /// [The reader-wake primitive](self#the-reader-wake-primitive-listen-not-one-registration-per-remote-peer).
    ///
    /// Returns `None` once [`TrunkConfig::part_capacity`] concurrent
    /// registrations are already outstanding — the caller must not wait in
    /// that case (there is no slot to wait *in*); it should fall back to an
    /// immediate re-poll or answer its request as unavailable now, exactly
    /// as a caller must once [`crate::egress::AwaitPolicy`] itself has
    /// expired. **Register before re-checking the condition you are waiting
    /// on** — `event_listener`'s standard idiom, and the same ordering
    /// `hls_runtime::server::MediaStore::listen`'s own docs require —
    /// otherwise a `notify` racing your check can be missed.
    pub fn listen(self: &Arc<Self>) -> Option<ProgressListener> {
        loop {
            let current = self.waiter_count.load(Ordering::Acquire);
            if current >= self.part_waiter_cap {
                return None;
            }
            if self
                .waiter_count
                .compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }
        Some(ProgressListener {
            _slot: WaiterSlot(Arc::clone(self)),
            listener: self.progress.listen(),
        })
    }
}

/// RAII release of one [`Trunk`] waiter slot — split out from
/// [`ProgressListener`] itself (rather than a `Drop` impl directly on
/// `ProgressListener`) specifically so [`ProgressListener::wait_deadline`]
/// can destructure `self` and move its `listener` field into
/// [`event_listener::Listener::wait_deadline`] by value: Rust forbids moving
/// a field out of a type that implements `Drop` itself, but does not forbid
/// it for a type that merely *contains* a field whose type implements
/// `Drop` — each field is then dropped independently, in this case when the
/// destructured local bindings go out of scope at the end of that method.
struct WaiterSlot(Arc<Trunk>);

impl Drop for WaiterSlot {
    /// Release this `Trunk`'s bounded waiter slot — see
    /// [The reader-wake primitive](self#the-reader-wake-primitive-listen-not-one-registration-per-remote-peer)
    /// for why this cap exists at all (an unbounded waiter set is a remote
    /// resource-exhaustion vector). Fires whether the owning
    /// [`ProgressListener`] was ever polled/waited on, woken, or simply
    /// dropped un-awaited — a caller that gives up on its own request must
    /// not leak a slot.
    fn drop(&mut self) {
        self.0.waiter_count.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A registered wake-up from [`Trunk::listen`] — the
/// [`event_listener::EventListener`] a [`ServedEgress`](crate::egress::ServedEgress)
/// adapter waits on, so a blocked request need not busy-poll. See
/// [The reader-wake primitive](self#the-reader-wake-primitive-listen-not-one-registration-per-remote-peer).
///
/// Releases this `Trunk`'s bounded waiter slot when dropped (via the
/// `_slot` field's own `Drop`, see this module's internal `WaiterSlot`) — whether this listener
/// was woken, timed out, or is simply discarded — so a caller that gives up
/// does not leak a slot forever.
pub struct ProgressListener {
    _slot: WaiterSlot,
    listener: EventListener,
}

impl ProgressListener {
    /// Block the calling thread until woken or `deadline` passes, whichever
    /// comes first — `true` if woken, `false` on timeout. Composes with
    /// [`crate::egress::AwaitPolicy`]'s deadline: convert
    /// `AwaitPolicy::deadline` (a [`Timestamp`]) to the `std::time::Instant`
    /// your caller already anchors its `Timestamp`s to (see
    /// [`Timestamp::from_instant`]'s inverse — the caller holds the base
    /// `Instant` it built its `Timestamp`s from) and pass that here, so this
    /// call can never park past the caller's own bound.
    ///
    /// Deliberately no unbounded `wait()` is exposed here — only this
    /// deadline-bound form and the `Future` impl below (whose bound is
    /// whatever timeout the caller's own executor wraps it in, exactly the
    /// `hls_runtime::server` "caller-driven wait loop" shape) — matching
    /// this crate's [`crate::egress::AwaitPolicy`] philosophy that a wait on
    /// remote-triggerable input must never be able to park forever.
    pub fn wait_deadline(self, deadline: std::time::Instant) -> bool {
        // Destructuring here (not a method call on `self.listener` while
        // `self` stays intact) is exactly what requires `ProgressListener`
        // itself to carry no `Drop` impl — see `WaiterSlot`'s own doc.
        let ProgressListener { _slot, listener } = self;
        listener.wait_deadline(deadline).is_some()
    }
}

impl Future for ProgressListener {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `EventListener` is `Unpin` (event_listener's own guarantee), and so
        // is `WaiterSlot` (an `Arc` newtype), so `ProgressListener` as a
        // whole is `Unpin` too — safe to reach the inner listener through a
        // plain `&mut` and poll it directly.
        let this = self.get_mut();
        Pin::new(&mut this.listener).poll(cx)
    }
}

/// The write handle for a [`Trunk`]'s **samples + events** ring group.
/// Obtained via [`Trunk::writer`]. See
/// [One writer per ring group](self#one-writer-per-ring-group-not-one-writer-per-trunk)
/// for why this group is exactly these two rings, and
/// [`SegmentWriter`] for the sibling handle covering segments + parts.
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
        let mut state = self.trunk.state.lock().expect("Trunk state lock poisoned");
        match retention {
            RetentionClass::Timed => state.timed.push(track_id, sample),
            RetentionClass::Sparse => state.sparse.push(track_id, sample),
        }
    }

    /// Publish one event. Never blocks and never rejects — a full event log
    /// evicts its oldest entry exactly like the sample/segment logs.
    /// `anchor` is resolved immediately against whatever segment starts /
    /// time anchor this trunk already knows; if it cannot be resolved yet,
    /// the entry is stored exactly as given, and resolves later, in place,
    /// once [`SegmentWriter::note_segment_start`]/[`SegmentWriter::set_time_anchor`]
    /// supplies what was missing. See
    /// [The event log](self#the-event-log-90-khz-absolute-and-the-b1-crux).
    pub fn publish_event(&self, event: TimedEvent, anchor: EventAnchor) {
        let mut state = self.trunk.state.lock().expect("Trunk state lock poisoned");
        state.events.push(event, anchor);
    }

    /// Replace this program's track set wholesale — the write side of
    /// [`Trunk::tracks`]/[`Trunk::track_generation`], and the method
    /// [`crate::ingress::IngestDriver`] calls to seed a freshly-minted
    /// `Trunk` from `SessionEvent::NewProgram`'s `tracks` and to apply a
    /// later `SessionEvent::TracksChanged`.
    ///
    /// `tracks` is taken as the **complete replacement set**, matching
    /// `SessionEvent::TracksChanged`'s own doc: a PMT (or any container's
    /// track-declaration mechanism) carries the whole elementary-stream
    /// list, so this call is idempotent (calling it twice with the same set
    /// leaves the trunk's tracks unchanged in content, only `track_generation`
    /// advances) and immune to delta-ordering bugs — there is no "add
    /// track"/"remove track" pair to apply out of order. A caller that wants
    /// to know *which* track changed diffs the previous [`Trunk::tracks`]
    /// snapshot against this one itself.
    ///
    /// Bumps [`Trunk::track_generation`] by exactly one and wakes any
    /// [`Trunk::listen`] registration, the same
    /// [`event_listener::Event::notify`] fan-out
    /// [`SegmentWriter::publish_segment`]/[`SegmentWriter::publish_part`]
    /// already use — see [`Trunk`]'s `progress` field doc for why a
    /// track-set change is folded into that same broad wake rather than a
    /// new channel.
    pub fn set_tracks(&self, tracks: Vec<TrackSpec>) {
        let mut state = self.trunk.state.lock().expect("Trunk state lock poisoned");
        state.tracks = Arc::from(tracks);
        state.track_generation += 1;
        drop(state);
        self.trunk.progress.notify(usize::MAX);
    }
}

/// The write handle for a [`Trunk`]'s **segments + parts** ring group.
/// Obtained via [`Trunk::segment_writer`], independently of [`TrunkWriter`]
/// (via [`Trunk::writer`]) — see
/// [One writer per ring group](self#one-writer-per-ring-group-not-one-writer-per-trunk)
/// for why this split exists, why `note_segment_start`/`set_time_anchor` are
/// grouped here rather than on [`TrunkWriter`], and what is (and is not)
/// guaranteed about ordering relative to the sample/event rings.
///
/// Like [`TrunkWriter`], every method here either never blocks (ordinary
/// eviction, exactly the sample rings' non-blocking-producer principle) or
/// blocks only in the one documented [`ArchiveOverrun::StallIngest`] case —
/// see [`SegmentWriter::publish_segment`].
pub struct SegmentWriter {
    trunk: Arc<Trunk>,
}

impl SegmentWriter {
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
    ///
    /// Does **not** touch the live-part log — see
    /// [The live-part log](self#the-live-part-log-parts-before-their-segment-closes)
    /// for why a segment closing deliberately leaves that segment's parts
    /// exactly as addressable as they were the instant before. Wakes any
    /// [`Trunk::listen`] registration once this call is about to return
    /// (bare-`_HLS_msn` blocking-reload's condition), even on the
    /// `StallIngest` path — a waiter is woken only after the entry has
    /// actually landed, never merely because a pin released.
    pub fn publish_segment(&self, entry: SegmentEntry) {
        let mut state = self.trunk.state.lock().expect("Trunk state lock poisoned");
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
            state = self
                .trunk
                .segment_pin_released
                .wait(state)
                .expect("Trunk segment_pin_released condvar poisoned");
            // Loop back around: re-check capacity/oldest/pins after waking —
            // the pin that was blocking may have advanced, been dropped, or
            // (if a *different* pin also needed this entry) still be
            // pending.
        }
        state.segments.push(entry);
        drop(state);
        self.trunk.progress.notify(usize::MAX);
    }

    /// Publish one live part of the segment currently being written — see
    /// [The live-part log](self#the-live-part-log-parts-before-their-segment-closes).
    ///
    /// Never blocks and never rejects: a full part log evicts its oldest
    /// entry exactly like every other ring in this module — the same
    /// non-blocking-producer principle as [`TrunkWriter::publish`]/
    /// [`SegmentWriter::publish_segment`]'s ordinary (non-`StallIngest`) path.
    /// Wakes any [`Trunk::listen`] registration once this part has actually
    /// landed (RFC 8216bis blocking-reload's part-availability condition).
    pub fn publish_part(&self, entry: PartEntry) {
        let mut state = self.trunk.state.lock().expect("Trunk state lock poisoned");
        state.parts.push(entry);
        drop(state);
        self.trunk.progress.notify(usize::MAX);
    }

    /// Report that segment `segment_number` starts at `start` on this
    /// trunk's 90 kHz absolute clock — the boundary an
    /// [`EventAnchor::Segment`] (an `emsg` v0's `presentation_time_delta`)
    /// needs before it can resolve. Called by whoever owns segmentation —
    /// the entity the spec's B1 fix names explicitly: "it cannot be
    /// finalised until the segmenter owns a boundary." Lives here, not on
    /// [`TrunkWriter`], for exactly that reason: only the segmenter can
    /// honestly report it. This does **not** append to the event ring — it
    /// resolves an already-published [`EventAnchor::Segment`] entry in
    /// place, so grouping it with [`SegmentWriter::publish_segment`] does not
    /// create a second appender for [`TrunkWriter::publish_event`]'s ring;
    /// see [One writer per ring group](self#one-writer-per-ring-group-not-one-writer-per-trunk).
    pub fn note_segment_start(&self, segment_number: u32, start: MediaTime) {
        let mut state = self.trunk.state.lock().expect("Trunk state lock poisoned");
        state.events.note_segment_start(segment_number, start);
    }

    /// Give the event log a wall-clock↔media-clock mapping. Resolves every
    /// currently-pending [`EventAnchor::Utc`] entry immediately, and every
    /// future one at publish time, until a later call replaces it. Grouped
    /// with [`SegmentWriter::note_segment_start`] rather than split onto
    /// [`TrunkWriter`] — see
    /// [One writer per ring group](self#one-writer-per-ring-group-not-one-writer-per-trunk)
    /// for why, and the same in-place-resolution reasoning: this is not an
    /// append to the event ring either.
    pub fn set_time_anchor(&self, anchor: TimeAnchor) {
        let mut state = self.trunk.state.lock().expect("Trunk state lock poisoned");
        state.events.set_time_anchor(anchor);
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
        let state = self.trunk.state.lock().expect("Trunk state lock poisoned");

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
            let state = self.trunk.state.lock().expect("Trunk state lock poisoned");
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
        // `SegmentWriter::publish_segment` has to consult it before evicting,
        // not merely react to it afterward.
        let mut state = self.trunk.state.lock().expect("Trunk state lock poisoned");
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
            state
                .segments
                .pins
                .get_mut(&pin_id)
                .expect("pin_id was resolved from this same locked state, so its entry exists")
                .consumed = state.segments.base;
            drop(state);
            // A pin advancing can free a `StallIngest` writer waiting on
            // exactly this pin.
            self.trunk.segment_pin_released.notify_all();
            return Some(SegmentCursorItem::Gap { skipped });
        }
        let idx = (consumed - state.segments.base) as usize;
        if let Some(entry) = state.segments.entries.get(idx) {
            let item = entry.clone();
            state
                .segments
                .pins
                .get_mut(&pin_id)
                .expect("pin_id was resolved from this same locked state, so its entry exists")
                .consumed += 1;
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
            let mut state = self.trunk.state.lock().expect("Trunk state lock poisoned");
            state.segments.pins.remove(&pin_id);
            drop(state);
            self.trunk.segment_pin_released.notify_all();
        }
    }
}

/// One item [`EventCursor::poll`] can hand back: one event-log entry (which
/// may itself still be `Segment`/`Utc`-anchored — a cursor sees an entry
/// the instant it is published, not only once it resolves; see
/// [`EventEntry::anchor`]), or a loss report.
///
/// `#[non_exhaustive]`: the growth point for anything a cursor might need
/// to surface beyond "entry" or "loss" later, without a breaking change to
/// every match arm in the workspace.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum EventCursorItem {
    /// One event-log entry, in publish order.
    Event(EventEntry),
    /// This cursor fell behind the event log's ordinary
    /// [`TrunkConfig::event_capacity`] eviction: `skipped` entries were
    /// evicted before it read them. Exactly [`SampleCursorItem::Lagged`]'s
    /// contract.
    Lagged {
        /// Exact count of entries evicted since this cursor's last
        /// successful read.
        skipped: u64,
    },
}

/// A subscribed reader of a [`Trunk`]'s event log. Obtained via
/// [`Trunk::subscribe_events`] — read that method's docs, and
/// [`Trunk::subscribe`]'s fan-out guidance, before creating more than a
/// handful of these.
pub struct EventCursor {
    trunk: Arc<Trunk>,
    consumed: u64,
}

impl EventCursor {
    /// Pull the next item, if any is ready. Loss is always reported before
    /// further data — the same cannot-be-skipped-past precedent as
    /// [`SampleCursor::poll`]/[`SegmentCursor::poll`].
    pub fn poll(&mut self) -> Option<EventCursorItem> {
        let state = self.trunk.state.lock().expect("Trunk state lock poisoned");
        let log = &state.events;
        if self.consumed < log.base {
            let skipped = log.base - self.consumed;
            self.consumed = log.base;
            return Some(EventCursorItem::Lagged { skipped });
        }
        let idx = (self.consumed - log.base) as usize;
        if let Some(entry) = log.entries.get(idx) {
            self.consumed += 1;
            return Some(EventCursorItem::Event(entry.clone()));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use transmux::pipeline::{CodecConfig, DataCarriage};

    /// An opaque `TrackSpec` for track-set tests — mirrors `ingress`'s own
    /// identically-named test helper (same shape, so a track built here and
    /// one built there compare equal field-for-field for any given
    /// `track_id`).
    fn opaque_track(track_id: u32) -> TrackSpec {
        TrackSpec::new(
            track_id,
            90_000,
            CodecConfig::Data {
                stream_type: 0x06,
                descriptors: Vec::new(),
                carriage: DataCarriage::Pes,
            },
        )
    }

    /// `NonZeroUsize` from a literal capacity, for readability at the ~30
    /// `TrunkConfig::new` call sites below. Panicking on `0` here is correct
    /// and is *not* the behaviour the deleted `zero_*_capacity_panics` tests
    /// asserted: this is a test helper rejecting a typo in test source, not
    /// the library accepting then rejecting a zero at run time — the library
    /// can no longer be handed one at all.
    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

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
        let trunk = Trunk::new(TrunkConfig::new(nz(100), nz(10), nz(4), nz(8), nz(8)));
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
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(10), nz(4), nz(8), nz(8)));
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
        let trunk = Trunk::new(TrunkConfig::new(nz(3), nz(10), nz(4), nz(8), nz(8)));
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
        let trunk = Trunk::new(TrunkConfig::new(nz(2), nz(2), nz(4), nz(8), nz(8)));
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
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(3), nz(4), nz(8), nz(8)));
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

    // --- 5b. subscribe_from_backlog: exact replay + Lagged on overwrite ---

    /// [`Trunk::subscribe_from_backlog`]'s core promise: a cursor subscribed
    /// *after* samples have already landed in both rings still sees them,
    /// exactly (in order, no dup), for each retention class independently —
    /// the property issue #808's `ProgramSegmenter` fix depends on.
    ///
    /// MUTATION VERIFIED: changing `subscribe_from_backlog`'s
    /// `timed_consumed: state.timed.base` to
    /// `timed_consumed: state.timed.published` (i.e. accidentally reusing
    /// `subscribe`'s live-tail initialisation) makes this test's
    /// `assert_eq!(timed_bytes, vec![10, 11, 12])` fail: `drain` returns an
    /// empty `Vec` (actual) instead of the expected `[10, 11, 12]`, because
    /// `timed_consumed` now equals `published` — every published entry
    /// already counts as "consumed" the instant the cursor is created, so
    /// `poll` immediately returns `None` instead of replaying the resident
    /// backlog. Recompiled and re-run to confirm this exact failure, then
    /// reverted.
    #[test]
    fn subscribe_from_backlog_replays_exact_resident_entries_both_classes() {
        let trunk = Trunk::new(TrunkConfig::new(nz(100), nz(100), nz(4), nz(8), nz(8)));
        let writer = trunk.writer().unwrap();

        // Published *before* the cursor exists — subscribe() would never see
        // any of this; subscribe_from_backlog() must replay all of it, since
        // ring capacity (100) is nowhere near exhausted.
        for i in 10u8..13 {
            writer.publish(1, RetentionClass::Timed, sample(i, 4));
        }
        for i in 20u8..22 {
            writer.publish(2, RetentionClass::Sparse, sample(i, 4));
        }

        let mut cursor = trunk.subscribe_from_backlog();

        // Merge order (module docs): pending lag reports first (none here),
        // then Sparse data, then Timed data.
        let sparse_items = drain(&mut cursor, 2);
        let sparse_bytes: Vec<u8> = sparse_items
            .iter()
            .map(|item| match item {
                SampleCursorItem::Sparse { sample, .. } => sample.data[0],
                other => panic!("expected Sparse, got {other:?}"),
            })
            .collect();
        assert_eq!(sparse_bytes, vec![20, 21], "exact resident Sparse backlog");

        let timed_items = drain(&mut cursor, 3);
        let timed_bytes: Vec<u8> = timed_items
            .iter()
            .map(|item| timed_data(item).unwrap().1.data[0])
            .collect();
        assert_eq!(
            timed_bytes,
            vec![10, 11, 12],
            "exact resident Timed backlog"
        );

        assert!(
            cursor.poll().is_none(),
            "no extra items beyond the resident backlog"
        );

        // New samples published after subscribing still flow through
        // normally, proving this cursor is a real live cursor afterwards,
        // not a one-shot snapshot.
        writer.publish(1, RetentionClass::Timed, sample(99, 4));
        let live = cursor.poll().unwrap();
        assert_eq!(timed_data(&live).unwrap().1.data[0], 99);
    }

    /// When the backlog a [`Trunk::subscribe_from_backlog`] cursor would
    /// have replayed has *already* been evicted by ring capacity before the
    /// subscribe call, this cursor must behave exactly like an ordinary
    /// [`Trunk::subscribe`] cursor from that point on: report the loss
    /// in-band as `Lagged`/`Degraded`, never silently skip it.
    ///
    /// MUTATION VERIFIED: changing `subscribe_from_backlog`'s
    /// `timed_consumed: state.timed.base` to `timed_consumed: 0` (simulating
    /// "replay from the beginning of time" rather than "replay what the ring
    /// still holds", anchoring at the wrong reference point) makes this
    /// test's `assert!(matches!(first, SampleCursorItem::Lagged { skipped: 6
    /// }))` fail: actual `Lagged { skipped: 12 }` (`12 - 0`, counting the 6
    /// entries already evicted *before* this cursor even subscribed as if
    /// they were its own loss) instead of the expected `Lagged { skipped: 6
    /// }` (`12 - 6`, only the 6 evictions that happened *after* this cursor
    /// subscribed). Recompiled and re-run to confirm this exact failure,
    /// then reverted.
    #[test]
    fn subscribe_from_backlog_reports_lagged_when_backlog_already_overwritten() {
        let trunk = Trunk::new(TrunkConfig::new(nz(3), nz(10), nz(4), nz(8), nz(8)));
        let writer = trunk.writer().unwrap();

        // Capacity 3, publish 9: only bytes 6,7,8 remain resident; 0..6 are
        // already gone by the time subscribe_from_backlog is called — this
        // cursor must NOT report those 6 as its own loss (they were never
        // its backlog to miss), which is exactly why it anchors at `base`
        // (6), not `0`.
        for i in 0u8..9 {
            writer.publish(1, RetentionClass::Timed, sample(i, 4));
        }
        let mut cursor = trunk.subscribe_from_backlog();

        // Publish 6 more before this cursor ever polls — the ring is
        // already full (capacity 3), so each push evicts exactly one
        // resident entry, advancing `base` from 6 to 12. This IS loss this
        // cursor is responsible for (it subscribed to a live cursor at 6,
        // then fell behind by 6 before its first poll).
        for i in 9u8..15 {
            writer.publish(1, RetentionClass::Timed, sample(i, 4));
        }

        let first = cursor.poll().unwrap();
        assert!(
            matches!(first, SampleCursorItem::Lagged { skipped: 6 }),
            "expected Lagged{{skipped: 6}}, got {first:?}"
        );

        // The remaining resident 3 (bytes 12,13,14) must still be readable.
        let items = drain(&mut cursor, 3);
        let bytes: Vec<u8> = items
            .iter()
            .map(|item| timed_data(item).unwrap().1.data[0])
            .collect();
        assert_eq!(bytes, vec![12, 13, 14]);
        assert!(cursor.poll().is_none());
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
        let trunk = Trunk::new(TrunkConfig::new(nz(8), nz(8), nz(4), nz(8), nz(8)));
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
    //
    // The five `zero_*_capacity_panics` tests that used to live here are
    // deliberately GONE, not merely disabled: every `TrunkConfig` capacity is
    // now a `NonZeroUsize`, so a zero capacity is unrepresentable rather than
    // rejected at run time, and `Trunk::new` no longer has (or needs) the
    // `assert!`s they pinned. A test asserting a panic that can no longer
    // occur would not compile against the new signature anyway, and keeping a
    // rewritten version would only be asserting that `NonZeroUsize::new(0)`
    // returns `None` — a property of the standard library, not of this crate.

    #[test]
    fn second_writer_is_refused() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let _first = trunk.writer().unwrap();
        assert!(
            trunk.writer().is_none(),
            "a Trunk has exactly one sample/event writer"
        );
    }

    // --- W1. the SegmentWriter half of the split is single-take too, ------
    // --- independently of TrunkWriter ---------------------------------------

    /// MUTATION VERIFIED: replacing `Trunk::segment_writer`'s
    /// `compare_exchange` call with an unconditional `Some(SegmentWriter {
    /// .. })` (i.e. reintroducing "anyone can take it, any number of times")
    /// makes this test's `assert!(trunk.segment_writer().is_none(), ..)`
    /// fail — the second call succeeds instead of being refused. Recompiled
    /// and re-run to confirm the failure, then reverted.
    #[test]
    fn second_segment_writer_is_refused() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let _first = trunk.segment_writer().unwrap();
        assert!(
            trunk.segment_writer().is_none(),
            "a Trunk has exactly one segment/part writer"
        );
    }

    // --- W2. THE GAP THIS STEP CLOSES: a sample/event writer and a --------
    // --- segment/part writer can be held AT THE SAME TIME -------------------

    /// This is the property that was **structurally impossible** before this
    /// step: `Trunk::writer()` and a hypothetical segment-writing capability
    /// shared one `AtomicBool`, so whichever component (the ingest driver)
    /// took the one writer made it impossible for anything else (a
    /// segmenter) to ever publish a segment or a part.
    ///
    /// MUTATION VERIFIED: changing `Trunk::segment_writer` to gate on
    /// `self.writer_taken` instead of its own `self.segment_writer_taken`
    /// (i.e. reintroducing the single-shared-flag bug this step fixes) makes
    /// this test's `let segments = trunk.segment_writer().unwrap();` line
    /// panic — `segment_writer()` returns `None` because the sample/event
    /// writer taken just above already flipped the shared flag. Recompiled
    /// and re-run to confirm the failure, then reverted.
    #[test]
    fn sample_and_segment_writers_can_be_held_simultaneously() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let samples = trunk.writer().unwrap();
        let segments = trunk.segment_writer().expect(
            "the segment/part writer must still be takeable while the sample/event writer is held",
        );

        // Both are simultaneously live and independently usable — not merely
        // both `Some` a moment apart.
        samples.publish(1, RetentionClass::Timed, sample(1, 4));
        segments.publish_segment(segment_entry(1, 1));
        assert_eq!(trunk.timed_len(), 1);
        assert_eq!(trunk.segment_len(), 1);
    }

    // --- W3. THE SEGMENTER-SHAPED, END-TO-END PROPERTY: a segmenter reads --
    // --- samples through its own cursor and publishes the segment/part it --
    // --- derives from them through the OTHER writer — unreachable before ---
    // --- this step, since there was only one writer for the whole Trunk ----

    /// The load-bearing test for this step: models exactly the component the
    /// gap analysis found could not exist — a segmenter that holds a
    /// [`SampleCursor`] (to read the samples it segments) *and* a
    /// [`SegmentWriter`] (to publish what it produces) at the same time,
    /// distinct from the ingest driver's own [`TrunkWriter`].
    ///
    /// MUTATION VERIFIED: commenting out `state.segments.push(entry);` in
    /// `SegmentWriter::publish_segment` (simulating "the moved method is a
    /// stub that does not actually reach the ring") makes this test's
    /// `let got = seg_cursor.poll().expect(..)` panic — nothing was ever
    /// pushed, so the segment log stays empty and the cursor has nothing to
    /// return. Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn segmenter_holds_sample_cursor_and_segment_writer_at_once() {
        let trunk = Trunk::new(TrunkConfig::new(nz(8), nz(4), nz(4), nz(8), nz(8)));

        // The ingest driver's own handle — a different component, a
        // different ring group.
        let ingest = trunk.writer().unwrap();
        // The segmenter's read side (its own SampleCursor) and write side
        // (the SegmentWriter) — held together, which is exactly what the
        // single-writer-per-Trunk model made impossible.
        let mut samples = trunk.subscribe();
        let segmenter = trunk.segment_writer().expect(
            "a segmenter must be able to take the segment/part writer \
                     while the ingest driver still holds the sample/event writer",
        );
        // `subscribe_segments` only sees entries published *after* this call
        // (exactly `Trunk::subscribe`'s "starts from now" contract) — taken
        // up front so the read-back below has something to see.
        let mut seg_cursor = trunk.subscribe_segments();

        for i in 0u8..3 {
            ingest.publish(1, RetentionClass::Timed, sample(i, 4));
        }

        // The segmenter consumes exactly the samples it is about to derive
        // a segment from.
        let mut muxed = Vec::new();
        for _ in 0..3 {
            match samples.poll() {
                Some(SampleCursorItem::Timed { sample, .. }) => {
                    muxed.push(sample.data[0]);
                }
                other => panic!("expected a Timed sample, got {other:?}"),
            }
        }
        assert_eq!(muxed, vec![0, 1, 2]);

        // ...then publishes the segment (and one live part of it) derived
        // from exactly those samples — through the OTHER writer.
        segmenter.publish_part(part_entry(0xAB, 1, 0));
        segmenter.publish_segment(segment_entry(0xAA, 1));

        // Read both back through the segment/part log's own query surface —
        // proving the publish actually reached the shared Trunk, not just a
        // private buffer inside the segmenter.
        let got = seg_cursor
            .poll()
            .expect("the segmenter's published segment must be visible");
        assert_eq!(segment_data(&got).unwrap().sequence_number, 1);
        assert_eq!(
            trunk.part_bytes(1, 0),
            Some(Bytes::from(vec![0xAB; 8])),
            "the segmenter's published part must be individually addressable too"
        );
    }

    #[test]
    fn subscribe_starts_from_now_not_from_history() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
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
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(100), nz(8), nz(8)));
        let mut c1 = trunk.subscribe_segments();
        let mut c2 = trunk.subscribe_segments();
        let mut c3 = trunk.subscribe_segments();
        let writer = trunk.segment_writer().unwrap();

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
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let mut slow = trunk.subscribe_segments();
        let writer = trunk.segment_writer().unwrap();

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
    /// `SegmentWriter::publish_segment`'s `ArchiveOverrun::StallIngest` arm
    /// from `must_wait = true;` to `{}` (a no-op, i.e. treating
    /// `StallIngest` exactly like `Gap`) makes this test fail: the third
    /// `publish_segment` call no longer blocks, so the background-thread
    /// completion channel's `recv_timeout` at the "still blocked" checkpoint
    /// returns `Ok(())` instead of timing out, and the assertion that it
    /// timed out (`is_err()`) fails. Recompiled and re-run to confirm the
    /// failure, then reverted.
    #[test]
    fn pinning_reader_receives_every_segment_while_non_pinning_reader_lags() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(2), nz(8), nz(8)));
        let mut slow = trunk.subscribe_segments(); // non-pinning: will lag
        let mut archive = trunk.pin_segments(ArchiveOverrun::StallIngest); // pinning: must lose nothing
        let writer = Arc::new(trunk.segment_writer().unwrap());

        // Fill the segment log's capacity (2) without any eviction yet.
        writer.publish_segment(segment_entry(1, 1));
        writer.publish_segment(segment_entry(2, 2));

        // A third publish must evict the oldest (seq 1), which `archive`'s
        // pin has not yet consumed — with `StallIngest`, this call blocks.
        // Run it on a background thread (the same one `SegmentWriter`,
        // shared via `Arc` — this is the segment/part ring group's own
        // single-writer invariant, just called from a different thread) and
        // prove, via a completion channel, that it has NOT returned yet.
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

        // HANG GUARD (issue #807): generous on purpose, same reasoning as
        // `retention.rs`'s `archive_overrun_stall_ingest_blocks_writer_until_driver_advances`
        // (the sibling proof of this same mechanism) -- the claim is "the
        // writer unblocks once the pin is drained", not "within N seconds".
        // The unblock is observed across a thread boundary, so a tight bound
        // measures the machine's scheduler, not this code.
        done_rx
            .recv_timeout(Duration::from_secs(60))
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
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        // Default policy (`Gap`) pinning cursor that never polls at all —
        // the worst case for memory growth: a dead/wedged archive consumer.
        let _archive = trunk.pin_segments(ArchiveOverrun::default());
        let writer = trunk.segment_writer().unwrap();

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
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(2), nz(8), nz(8)));
        let mut archive = trunk.pin_segments(ArchiveOverrun::Gap);
        let writer = trunk.segment_writer().unwrap();

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
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(1), nz(8), nz(8)));
        let mut archive = trunk.pin_segments(ArchiveOverrun::StallIngest);
        let writer = Arc::new(trunk.segment_writer().unwrap());

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

        // HANG GUARD (issue #807): generous on purpose, same reasoning as the
        // sibling test above (`pinning_reader_receives_every_segment_while_non_pinning_reader_lags`)
        // -- claim is "unblocks once drained", not "within N seconds"; the
        // unblock crosses a thread boundary so a tight bound measures the
        // scheduler, not this code.
        done_rx
            .recv_timeout(Duration::from_secs(60))
            .expect("publish_segment must unblock once the pin advances");
        handle.join().unwrap();
    }

    // --- S7. ArchiveOverrun::Terminate drops the cursor --------------------

    /// MUTATION VERIFIED: changing `ArchiveOverrun::Terminate => pin.terminated
    /// = true,` in `SegmentWriter::publish_segment` to `ArchiveOverrun::Terminate
    /// => {}` (a no-op, treating `Terminate` exactly like `Gap`) makes this
    /// test fail: `archive.poll()` returns `Some(Gap { .. })` instead of
    /// `Some(Terminated)`, so the `matches!` assertion on `Terminated` fails.
    /// Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn archive_overrun_terminate_drops_the_cursor() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(2), nz(8), nz(8)));
        let mut archive = trunk.pin_segments(ArchiveOverrun::Terminate);
        let writer = trunk.segment_writer().unwrap();

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
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(8), nz(8), nz(8)));
        let mut c1 = trunk.subscribe_segments();
        let mut c2 = trunk.subscribe_segments();
        let mut c3 = trunk.subscribe_segments();
        let writer = trunk.segment_writer().unwrap();

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

    // ===================== event log =======================================

    use timed_metadata::{EventKind, SourcePayload};

    /// A minimal `TimedEvent` for tests that don't care about the SCTE-35
    /// source payload itself — only about how the event *log* addresses and
    /// resolves it. `at`/`duration` are left `None`: this step's
    /// [`EventAnchor`] carries the resolution state, not `TimedEvent::at`.
    fn basic_event(id: u32) -> TimedEvent {
        TimedEvent {
            id: Some(id),
            kind: EventKind::BreakStart,
            at: None,
            duration: None,
            source: SourcePayload::Scte35 { raw: Vec::new() },
        }
    }

    fn event_id(item: &EventCursorItem) -> Option<u32> {
        match item {
            EventCursorItem::Event(e) => e.event.id,
            _ => None,
        }
    }

    /// Build real, valid (Parse/Serialize round-tripping) `splice_insert()`
    /// bytes carrying `pts_time`, via `scte35-splice`'s own builder +
    /// serializer — not hand-rolled/fabricated wire bytes. Used to drive
    /// `timed_metadata::Timeline::push_scte35`'s 33-bit wrap-unroll across a
    /// genuine wrap boundary (see `a_33_bit_pts_wrap_does_not_corrupt_event_log_ordering`).
    fn splice_insert_bytes(event_id: u32, pts_time: u64) -> Vec<u8> {
        use broadcast_common::Serialize;
        use scte35_splice::SpliceInfoSection;
        use scte35_splice::commands::AnyCommand;
        use scte35_splice::commands::splice_insert::SpliceInsert;
        use scte35_splice::time::SpliceTime;

        let si = SpliceInsert {
            splice_event_id: event_id,
            out_of_network_indicator: true,
            splice_time: Some(SpliceTime::with_pts(pts_time)),
            ..SpliceInsert::default()
        };
        let section = SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(si), &[]);
        section.to_bytes()
    }

    // --- E1. events_between: half-open [from, to), boundaries exact -------

    /// MUTATION VERIFIED: changing the upper-bound comparison in
    /// `Trunk::events_between`'s filter from `t.0 < to.0` to `t.0 <= to.0`
    /// (making the range closed instead of half-open) makes this test fail:
    /// `ids` becomes `[2, 3, 4]` (the boundary event at `to` is wrongly
    /// included) instead of the expected `[2, 3]`. Recompiled and re-run to
    /// confirm the failure, then reverted.
    #[test]
    fn events_between_returns_exactly_the_half_open_range() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let writer = trunk.writer().unwrap();

        for (id, ticks) in [(1u32, 1_000u64), (2, 2_000), (3, 3_000), (4, 4_000)] {
            writer.publish_event(basic_event(id), EventAnchor::Media(MediaTime(ticks)));
        }

        let got = trunk.events_between(MediaTime(2_000), MediaTime(4_000));
        let ids: Vec<u32> = got.iter().map(|e| e.event.id.unwrap()).collect();
        assert_eq!(
            ids,
            vec![2, 3],
            "start (2_000) inclusive, end (4_000) exclusive"
        );
    }

    // --- E2. a Segment-anchored entry resolves at PUBLISH time when the ---
    // --- boundary is already known ------------------------------------------

    /// MUTATION VERIFIED: changing `EventLog::try_resolve`'s `Segment` arm
    /// to always return the anchor unresolved (`_ => anchor` in place of the
    /// `segment_starts` lookup) makes this test fail: `events_in_segment(3)`
    /// comes back empty instead of containing the published event, because
    /// the entry never leaves `EventAnchor::Segment`. Recompiled and re-run
    /// to confirm the failure, then reverted.
    #[test]
    fn segment_relative_event_resolves_at_publish_time_when_boundary_already_known() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        // Two separate writers, held at once: `note_segment_start` lives on
        // the segmenter's `SegmentWriter`, `publish_event` on the ingest
        // driver's `TrunkWriter` — exactly the split this step introduces.
        let writer = trunk.writer().unwrap();
        let segment_writer = trunk.segment_writer().unwrap();

        segment_writer.note_segment_start(3, MediaTime(300_000));
        writer.publish_event(
            basic_event(9),
            EventAnchor::Segment {
                segment_number: 3,
                delta: 1_500,
            },
        );

        let got = trunk.events_in_segment(3);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].event.id, Some(9));
        assert!(matches!(
            got[0].anchor,
            EventAnchor::Media(MediaTime(t)) if t == 301_500
        ));
    }

    // --- E3. THE B1 SEGMENT CASE: a segment-relative event resolves to ----
    // --- the segment it actually named, not whichever segment is open -----

    /// MUTATION VERIFIED: removing the `if n == segment_number` guard in
    /// `EventLog::note_segment_start` (resolving *every* pending `Segment`
    /// entry against whichever boundary arrives, regardless of which
    /// segment it targets) makes this test fail at the first assertion:
    /// after `note_segment_start(1, MediaTime(0))` — segment 1, NOT the
    /// event's actual target segment 2 — the entry is wrongly resolved to
    /// `MediaTime(1_000)` (segment 1's start + delta) instead of staying
    /// `EventAnchor::Segment { segment_number: 2, .. }`, so the
    /// `matches!(entry.anchor, EventAnchor::Segment { segment_number: 2, .. })`
    /// assertion fails. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[test]
    fn segment_relative_event_resolves_to_the_named_segment_not_whichever_is_open() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let writer = trunk.writer().unwrap();
        let segment_writer = trunk.segment_writer().unwrap();
        let mut cursor = trunk.subscribe_events();

        // The event targets segment 2 specifically, delta 1_000 after ITS
        // start — published before ANY segment boundary is known.
        writer.publish_event(
            basic_event(42),
            EventAnchor::Segment {
                segment_number: 2,
                delta: 1_000,
            },
        );

        // Segment 1 — a DIFFERENT, "currently open" segment — reports its
        // start first. This must NOT resolve the segment-2-targeted event.
        segment_writer.note_segment_start(1, MediaTime(0));

        let item = cursor.poll().unwrap();
        let entry = match item {
            EventCursorItem::Event(e) => e,
            other => panic!("expected Event, got {other:?}"),
        };
        assert!(
            matches!(
                entry.anchor,
                EventAnchor::Segment {
                    segment_number: 2,
                    delta: 1_000
                }
            ),
            "must stay pending on segment 2 — segment 1 being open must not \
             resolve it against the wrong boundary: {:?}",
            entry.anchor
        );
        assert!(
            trunk.events_in_segment(2).is_empty(),
            "not resolved yet: must not appear under segment 2 either"
        );
        assert!(trunk.events_in_segment(1).is_empty());

        // Now segment 2's own start arrives: resolves in place, to the
        // RIGHT segment's start + delta.
        segment_writer.note_segment_start(2, MediaTime(90_000));

        let in_seg2 = trunk.events_in_segment(2);
        assert_eq!(in_seg2.len(), 1);
        assert_eq!(in_seg2[0].event.id, Some(42));
        assert!(matches!(
            in_seg2[0].anchor,
            EventAnchor::Media(MediaTime(t)) if t == 91_000
        ));
        assert!(
            trunk.events_in_segment(1).is_empty(),
            "must not ALSO appear under segment 1"
        );
    }

    // --- E4. THE B1 CRUX: a UTC-only event stays honestly unanchored ------
    // --- until a TimeAnchor arrives, then resolves correctly ---------------

    /// MUTATION VERIFIED: changing `EventLog::try_resolve`'s `Utc` arm to
    /// fabricate `EventAnchor::Media(MediaTime(0))` whenever no
    /// `time_anchor` is set yet (in place of returning the anchor
    /// unresolved) — i.e. reintroducing the exact B1 bug this design
    /// exists to prevent — makes this test fail at the first assertion:
    /// `entry.anchor` is `EventAnchor::Media(MediaTime(0))` instead of the
    /// expected `EventAnchor::Utc { utc_epoch_ms: 5_000 }`, so the
    /// `matches!` assertion fails. Recompiled and re-run to confirm the
    /// failure, then reverted.
    #[test]
    fn utc_only_event_stays_unanchored_until_a_time_anchor_arrives() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let writer = trunk.writer().unwrap();
        let segment_writer = trunk.segment_writer().unwrap();
        let mut cursor = trunk.subscribe_events();

        // A GPS/UTC-scheduled event (SCTE-35 splice_schedule.utc_splice_time
        // semantics, §9.7.4) with no media anchor yet.
        writer.publish_event(
            basic_event(7),
            EventAnchor::Utc {
                utc_epoch_ms: 5_000,
            },
        );

        let item = cursor.poll().unwrap();
        let entry = match item {
            EventCursorItem::Event(e) => e,
            other => panic!("expected Event, got {other:?}"),
        };
        assert!(
            matches!(
                entry.anchor,
                EventAnchor::Utc {
                    utc_epoch_ms: 5_000
                }
            ),
            "must stay honestly unanchored — NO fabricated media time: {:?}",
            entry.anchor
        );
        // Nothing to filter a media time against yet: the point-in-time
        // query must not surface it either.
        assert!(
            trunk
                .events_between(MediaTime(0), MediaTime(u64::MAX))
                .is_empty(),
            "an unanchored event must not appear in a media-time query"
        );

        // An anchor arrives: pts 0 == epoch 1_000ms (`TimeAnchor`'s own
        // convention), so epoch 5_000ms is 4_000ms == 360_000 ticks later.
        segment_writer.set_time_anchor(TimeAnchor {
            pts_90k: 0,
            utc_epoch_ms: 1_000,
        });

        let resolved = trunk.events_between(MediaTime(0), MediaTime(u64::MAX));
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].event.id, Some(7));
        assert!(
            matches!(resolved[0].anchor, EventAnchor::Media(MediaTime(t)) if t == 360_000),
            "expected MediaTime(360_000), got {:?}",
            resolved[0].anchor
        );
    }

    // --- E5. a 33-bit PTS wrap does not corrupt event log ordering --------
    // --- (reuses timed_metadata::Timeline's unroll; does not hand-roll it) -

    /// MUTATION VERIFIED: re-introducing a 33-bit mask on an
    /// already-unrolled `MediaTime` in `EventLog::try_resolve`'s `Media`
    /// arm (`EventAnchor::Media(MediaTime(t)) => EventAnchor::Media(MediaTime(t
    /// & ((1u64 << 33) - 1)))` in place of the pass-through `anchor`) makes
    /// this test fail: `ev2`'s post-wrap absolute tick value
    /// (`(1u64 << 33) + 5`) exceeds 33 bits, so it is stored truncated to
    /// `5` instead of the value `Timeline` actually computed, and
    /// `matches!(got[1].anchor, EventAnchor::Media(t) if t.0 == at2.0)`
    /// fails (stored `5` != `at2.0` ≈ `2^33 + 5`). (The earlier
    /// `at2.0 > at1.0` assertion, which only reads `Timeline`'s local return
    /// value, does NOT catch this mutation — a stored-value mutation only
    /// shows up in what the log hands back, which is exactly why this test
    /// asserts against `got[..].anchor`, not just `at1`/`at2`.) Recompiled
    /// and re-run to confirm the failure, then reverted.
    #[test]
    fn a_33_bit_pts_wrap_does_not_corrupt_event_log_ordering() {
        const PTS_WRAP: u64 = 1u64 << 33;

        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let writer = trunk.writer().unwrap();
        let mut timeline = timed_metadata::Timeline::new();

        // Event 1: a PTS 10 ticks before the 33-bit wrap point.
        let before_wrap = splice_insert_bytes(1, PTS_WRAP - 10);
        let ev1 = timeline.push_scte35(&before_wrap).unwrap();
        let at1 = ev1.at.unwrap();
        writer.publish_event(ev1, EventAnchor::Media(at1));

        // Event 2: a small RAW PTS after the wrap. `Timeline` must unroll
        // this into a value larger than `at1`, not a small one.
        let after_wrap = splice_insert_bytes(2, 5);
        let ev2 = timeline.push_scte35(&after_wrap).unwrap();
        let at2 = ev2.at.unwrap();
        writer.publish_event(ev2, EventAnchor::Media(at2));

        assert!(
            at2.0 > at1.0,
            "Timeline itself must unroll monotonically: at1={}, at2={}",
            at1.0,
            at2.0
        );

        // The event log must store EXACTLY the MediaTime `Timeline` already
        // unrolled — no re-derivation, re-masking, or truncation of an
        // already-unrolled value anywhere in this module's storage/
        // resolution path. (Publish-order preservation across a wrap is
        // trivial regardless of the anchor's value — `VecDeque` iteration
        // order does not depend on it — so the real assertion here is
        // value-exactness, not position.)
        let got = trunk.events_between(MediaTime(0), MediaTime(u64::MAX));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].event.id, Some(1));
        assert_eq!(got[1].event.id, Some(2));
        assert!(
            matches!(got[0].anchor, EventAnchor::Media(t) if t.0 == at1.0),
            "event 1's stored anchor must equal Timeline's unrolled value \
             exactly, got {:?}",
            got[0].anchor
        );
        assert!(
            matches!(got[1].anchor, EventAnchor::Media(t) if t.0 == at2.0),
            "event 2's stored (post-wrap) anchor must equal Timeline's \
             unrolled value exactly — not re-masked back into 33 bits, got {:?}",
            got[1].anchor
        );
    }

    // --- E6. the event log is bounded: flooding cannot grow memory --------
    // --- without limit -------------------------------------------------------

    /// MUTATION VERIFIED: removing the eviction check in `EventLog::push`
    /// (replacing `if self.entries.len() == self.capacity { .. }` with a
    /// no-op) makes `trunk.event_len()` grow well past the configured cap
    /// (`3`) instead of staying bounded — the in-loop assertion fails on
    /// the first over-capacity iteration. Recompiled and re-run to confirm
    /// the failure, then reverted.
    #[test]
    fn event_log_is_bounded_under_flood() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(3), nz(8)));
        let writer = trunk.writer().unwrap();

        for i in 0u32..50_000 {
            writer.publish_event(basic_event(i), EventAnchor::Media(MediaTime(u64::from(i))));
            assert!(
                trunk.event_len() <= 3,
                "event log exceeded its cap mid-flood"
            );
        }
        assert_eq!(trunk.event_len(), 3);
    }

    // --- E7. event cursor lag is reported in-band with an accurate --------
    // --- skipped count; the writer never blocks -----------------------------

    /// MUTATION VERIFIED: changing the `skipped` computation in
    /// `EventCursor::poll`'s lag branch from `log.base - self.consumed` to
    /// `log.base - self.consumed + 1` makes this test fail: expected
    /// `skipped: 6`, got `skipped: 7`. Recompiled and re-run to confirm the
    /// failure, then reverted.
    #[test]
    fn event_cursor_lag_is_reported_with_an_accurate_skipped_count_writer_never_blocks() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(3), nz(8)));
        let mut cursor = trunk.subscribe_events();
        let writer = trunk.writer().unwrap();

        // Capacity 3, publish 9: 6 evicted before the cursor ever reads.
        // Never blocks — there is no wait-for-reader path in
        // `EventLog::push`.
        for i in 0u32..9 {
            writer.publish_event(basic_event(i), EventAnchor::Media(MediaTime(u64::from(i))));
        }
        assert_eq!(
            trunk.event_len(),
            3,
            "writer unblocked: event log stayed bounded"
        );

        let first = cursor.poll().unwrap();
        assert!(
            matches!(first, EventCursorItem::Lagged { skipped: 6 }),
            "expected Lagged{{skipped: 6}}, got {first:?}"
        );

        // The remaining 3 (ids 6, 7, 8) must still be readable, in order.
        let mut ids = Vec::new();
        for _ in 0..3 {
            ids.push(event_id(&cursor.poll().unwrap()).unwrap());
        }
        assert_eq!(ids, vec![6, 7, 8]);
        assert!(cursor.poll().is_none());
    }

    // --- E8. `epoch_ms_to_media` really is the inverse of ------------------
    // --- `TimeAnchor::media_to_epoch_ms` -----------------------------------

    /// A wall-clock anchor with room on *both* sides of `pts_90k`, so a sign
    /// error in the (signed) delta shows up rather than being clipped away:
    /// 10 s of media already elapsed, mapped to a realistic epoch instant.
    fn round_trip_anchor() -> TimeAnchor {
        TimeAnchor {
            pts_90k: 900_000,                // 10 s at 90 kHz
            utc_epoch_ms: 1_700_000_000_000, // ~2023-11-14T22:13:20Z
        }
    }

    /// Ticks of the 90 kHz media clock per millisecond of the epoch clock.
    ///
    /// Derived, not asserted: the media clock is [`PTS_HZ`] ticks/second and
    /// `utc_epoch_ms` counts milliseconds, i.e. thousandths of a second, so
    /// one millisecond spans `PTS_HZ / 1000` ticks.
    const TICKS_PER_EPOCH_MS: u64 = PTS_HZ / 1000;

    /// The exact worst-case `media -> epoch_ms -> media` error, in ticks.
    ///
    /// **Derivation (not a tuned constant).**
    /// [`TimeAnchor::media_to_epoch_ms`] computes
    /// `delta_ticks * 1000 / PTS_HZ`, i.e. `delta_ticks / TICKS_PER_EPOCH_MS`,
    /// in integer arithmetic — Rust integer division truncates toward zero,
    /// so it discards a remainder `r` with `|r| <= TICKS_PER_EPOCH_MS - 1`.
    /// `epoch_ms_to_media` then multiplies the surviving whole milliseconds
    /// back up by `TICKS_PER_EPOCH_MS`, reconstructing `delta_ticks - r`
    /// exactly. The round-trip error is therefore *precisely* that discarded
    /// remainder: at most `TICKS_PER_EPOCH_MS - 1` == 89 ticks, i.e. strictly
    /// less than one millisecond. `media_round_trip_is_lossy_by_at_most_one_
    /// millisecond` additionally asserts this bound is **tight** (some input
    /// attains exactly 89), so it cannot silently be loosened into a
    /// tolerance that hides a real error.
    const MEDIA_ROUND_TRIP_MAX_TICKS: u64 = TICKS_PER_EPOCH_MS - 1;

    /// The `epoch_ms -> media -> epoch_ms` direction is **exact** — the media
    /// clock is finer-grained than the millisecond clock (90 ticks per ms),
    /// so no information is lost going to ticks and back. Asserted with
    /// equality, no tolerance.
    ///
    /// MUTATION VERIFIED: flipping the sign of the delta in
    /// `epoch_ms_to_media` (`i128::from(anchor.utc_epoch_ms) -
    /// i128::from(utc_epoch_ms)` in place of the correct
    /// `i128::from(utc_epoch_ms) - i128::from(anchor.utc_epoch_ms)`) makes
    /// this test fail on the first non-zero offset: for `+1` ms the
    /// round-tripped epoch comes back as `1699999999999` instead of
    /// `1700000000001`. **Second mutation, also verified:** changing the
    /// scale conversion from `* PTS_HZ / 1000` to `* PTS_HZ * 1000` fails the
    /// same assertion with `1700001000000` instead of `1700000000001`. So
    /// both the *sign* and the *magnitude* of the inverse are pinned, not
    /// just its shape. Recompiled and re-run to confirm each failure, then
    /// reverted.
    #[test]
    fn epoch_ms_round_trip_through_media_time_is_exact() {
        let anchor = round_trip_anchor();

        // Offsets in ms from the anchor's own epoch instant. Zero, both
        // signs at ±1 ms and ±1 s, a full day forward, a backward offset
        // that lands well clear of the clamp, and one large enough that
        // `delta_ms * PTS_HZ` (2e14 * 9e4 = 1.8e19) exceeds `i64::MAX`
        // (~9.2e18) — the case that exercises the `i128` widening.
        for offset_ms in [
            0i64,
            1,
            -1,
            1_000,
            -1_000,
            86_400_000,
            -9_000,
            200_000_000_000_000,
        ] {
            let epoch_ms = anchor.utc_epoch_ms + offset_ms;
            let media = epoch_ms_to_media(&anchor, epoch_ms);
            let back = anchor.media_to_epoch_ms(media);
            assert_eq!(
                back, epoch_ms,
                "epoch_ms -> media -> epoch_ms must be EXACT at offset {offset_ms} ms \
                 (media = {media:?})"
            );
        }
    }

    /// The `media -> epoch_ms -> media` direction is **lossy**, by a bounded
    /// and derived amount: the media clock is 90× finer than the millisecond
    /// clock, so sub-millisecond tick precision cannot survive the trip. See
    /// [`MEDIA_ROUND_TRIP_MAX_TICKS`] for the derivation. This test also
    /// pins the bound as *tight*, so it is a real property and not a loose
    /// tolerance hiding an error.
    ///
    /// MUTATION VERIFIED: flipping the sign of the delta in
    /// `epoch_ms_to_media` (as in
    /// `epoch_ms_round_trip_through_media_time_is_exact`'s note) makes this
    /// test fail at the first offset that is a whole number of milliseconds
    /// away from the anchor: at media offset `+90` ticks the value comes
    /// back as `899_910` instead of `900_090`, a diff of `180` ticks, so the
    /// `diff <= MEDIA_ROUND_TRIP_MAX_TICKS` (89) assertion fails.
    /// **Second mutation, also verified:** the `* PTS_HZ * 1000` scale error
    /// fails the same assertion with a diff of `89_999_910` ticks. Recompiled
    /// and re-run to confirm each failure, then reverted.
    #[test]
    fn media_round_trip_is_lossy_by_at_most_one_millisecond() {
        let anchor = round_trip_anchor();
        let mut worst = 0u64;

        // Tick offsets from the anchor's own `pts_90k`. Both signs, values
        // that are and are not whole multiples of TICKS_PER_EPOCH_MS (so the
        // truncated remainder is genuinely exercised), the exact worst-case
        // remainder on each side (±89), and a large offset well past the
        // i64/i128 boundary region.
        for offset_ticks in [
            0i64,
            1,
            -1,
            89,
            -89,
            90,
            -90,
            91,
            -91,
            18_000_000_000_000_037,
        ] {
            let media = MediaTime((anchor.pts_90k as i64 + offset_ticks) as u64);
            let epoch_ms = anchor.media_to_epoch_ms(media);
            let back = epoch_ms_to_media(&anchor, epoch_ms);
            let diff = media.0.abs_diff(back.0);
            assert!(
                diff <= MEDIA_ROUND_TRIP_MAX_TICKS,
                "media -> epoch_ms -> media lost {diff} ticks at offset \
                 {offset_ticks} (bound is {MEDIA_ROUND_TRIP_MAX_TICKS}, i.e. \
                 < 1 ms): {media:?} -> {epoch_ms} -> {back:?}"
            );
            worst = worst.max(diff);
        }

        // The bound is TIGHT: the ±89-tick cases attain it exactly. Without
        // this, `MEDIA_ROUND_TRIP_MAX_TICKS` could be quietly raised to
        // paper over a genuine arithmetic error and the test above would
        // still pass.
        assert_eq!(
            worst, MEDIA_ROUND_TRIP_MAX_TICKS,
            "the derived bound must be attained, not merely respected — \
             otherwise it is a loose tolerance, not a property"
        );
    }

    /// `epoch_ms_to_media`'s `clamp(0, u64::MAX)` for an epoch instant far
    /// enough *before* the anchor that the implied media time would be
    /// negative.
    ///
    /// **This documents clamping as SAFE, not CORRECT** — they are different
    /// claims and this test asserts the weaker, true one. A negative media
    /// time is simply not representable in `MediaTime(u64)`, so no return
    /// value here can be right: clamping to `0` reports "at the very start
    /// of this trunk's timeline", which is *not* the instant asked for, and
    /// the round trip provably does not recover the input (asserted below).
    /// What the clamp does buy is that the failure is bounded and obvious
    /// rather than catastrophic: an unchecked `as u64` cast of a negative
    /// value would wrap to something near `u64::MAX` — an event appearing
    /// scheduled ~6.5 million years in the future, which is exactly the
    /// silent wrong-instant class B1 is about. Clamping keeps a
    /// pre-origin event in the past (where a scheduler treats it as already
    /// elapsed) instead of the unreachable future.
    ///
    /// If pre-origin scheduled events turn out to be real rather than
    /// pathological, the *honest* fix is not a different clamp value — it is
    /// to leave the entry `EventAnchor::Utc` (unresolved), exactly as an
    /// event with no anchor at all stays unresolved. That would be an
    /// additive change to `try_resolve`/`set_time_anchor`, not a change to
    /// this helper's contract.
    #[test]
    fn epoch_before_the_timeline_origin_clamps_to_zero_which_is_safe_not_correct() {
        let anchor = round_trip_anchor();

        // 20 s before the anchor's epoch, but only 10 s of media has
        // elapsed at the anchor — so the implied media time is -10 s.
        let epoch_ms = anchor.utc_epoch_ms - 20_000;
        let media = epoch_ms_to_media(&anchor, epoch_ms);

        assert_eq!(
            media,
            MediaTime(0),
            "a pre-origin epoch must clamp to the start of the timeline"
        );

        // Bounded-and-obvious, not catastrophic: emphatically NOT a wrapped
        // near-`u64::MAX` value masquerading as the far future.
        assert!(
            media.0 < u64::from(u32::MAX),
            "must not have wrapped into the far future: {media:?}"
        );

        // And it is genuinely NOT correct: the round trip does not recover
        // the input, because the requested instant is unrepresentable.
        let back = anchor.media_to_epoch_ms(media);
        assert_ne!(
            back, epoch_ms,
            "clamping is lossy by construction — this asserts the honest \
             claim (safe) rather than the false one (correct)"
        );
        assert_eq!(
            back,
            anchor.utc_epoch_ms - 10_000,
            "clamped media time 0 maps back to the timeline origin (10 s \
             before the anchor), not to the requested instant"
        );
    }

    // === Step 3b-iv: live-part log + reader-wake primitive =================

    fn part_entry(byte: u8, segment_number: u32, part_index: u32) -> PartEntry {
        PartEntry::new(
            Bytes::from(vec![byte; 8]),
            segment_number,
            part_index,
            Duration::from_millis(200),
            part_index == 0,
        )
    }

    /// The LL-HLS property this whole step exists for: a part must be
    /// addressable and readable **before** its parent segment closes — if
    /// this does not hold, nothing else in this step matters (blocking
    /// reload has nothing to answer with).
    ///
    /// MUTATION VERIFIED: commenting out `state.parts.push(entry);` in
    /// `SegmentWriter::publish_part` (simulating "the part never actually
    /// lands in the log") makes `trunk.part_bytes(9, 0)` return `None`
    /// instead of `Some(..)`, failing the `expect` below. Recompiled and
    /// re-run to confirm the failure, then reverted.
    #[test]
    fn part_is_addressable_and_readable_before_its_parent_segment_closes() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let writer = trunk.segment_writer().unwrap();

        // Segment 9 has never been closed — no `publish_segment` call for it
        // anywhere in this test.
        assert!(
            trunk.last_closed_segment().is_none(),
            "sanity check: nothing has closed yet"
        );

        writer.publish_part(part_entry(0xAB, 9, 0));

        let bytes = trunk
            .part_bytes(9, 0)
            .expect("a published part of an open segment must be addressable now");
        assert_eq!(bytes, Bytes::from(vec![0xAB; 8]));

        // Still true: segment 9 has still never closed.
        assert!(
            trunk.last_closed_segment().is_none(),
            "the part landed without any segment ever closing"
        );
    }

    /// A waiter blocked on a not-yet-existing part wakes once it is
    /// published, and resolves to exactly that part — not merely "wakes",
    /// which a mutation could satisfy vacuously if `part_bytes` mismatched
    /// the wrong entry (this test publishes a decoy part first to make
    /// "the right one" a real assertion).
    ///
    /// MUTATION VERIFIED (two independent mutations, each reverted after
    /// confirming failure):
    /// 1. Removing `self.trunk.progress.notify(usize::MAX);` from
    ///    `SegmentWriter::publish_part` makes `listener.wait_deadline(..)`
    ///    time out (`false`) instead of waking (`true`) within the 2 s bound
    ///    used below — the first assertion fails.
    /// 2. Changing `Trunk::part_bytes`'s filter from
    ///    `p.segment_number == segment_number && p.part_index == part_index`
    ///    to drop the `part_index` half (matching on `segment_number` alone)
    ///    makes the final `assert_eq!` fail: with the decoy part (index 1)
    ///    published first, `part_bytes(9, 0)` would resolve to the decoy's
    ///    `0xCC` bytes instead of the awaited part's `0xAB` bytes.
    #[test]
    fn waiter_is_woken_when_the_awaited_part_lands_and_resolves_to_it() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let writer = Arc::new(trunk.segment_writer().unwrap());

        // Register BEFORE re-checking/waiting — the documented no-missed-
        // wakeup ordering.
        let listener = trunk.listen().expect("first registration must succeed");
        assert!(trunk.part_bytes(9, 0).is_none(), "not published yet");

        let bg_writer = Arc::clone(&writer);
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            // A decoy: a different part of the same segment, published
            // first. If `part_bytes` ever matched on `segment_number` alone,
            // this would be the value wrongly returned for a request for
            // part 0.
            bg_writer.publish_part(part_entry(0xCC, 9, 1));
            bg_writer.publish_part(part_entry(0xAB, 9, 0));
        });

        // Deliberately generous. The claim under test is "the listener wakes
        // rather than parking forever", NOT "it wakes inside N seconds": the
        // publish happens on another thread, so any tight upper bound is a
        // bound on the *machine's* scheduling, not on this code. A 2s bound
        // here failed once during a loaded full-workspace run and passed on
        // 38 consecutive idle runs -- a false red that trains people to
        // re-run the suite. 60s still fails instantly if the wake channel
        // genuinely never fires, which is the only failure worth reporting.
        let woken = listener.wait_deadline(std::time::Instant::now() + Duration::from_secs(60));
        assert!(
            woken,
            "listener must wake on publish_part, not park forever"
        );
        handle.join().unwrap();

        let bytes = trunk
            .part_bytes(9, 0)
            .expect("the awaited part must now be readable");
        assert_eq!(
            bytes,
            Bytes::from(vec![0xAB; 8]),
            "must resolve to the awaited part (index 0), not the decoy (index 1)"
        );
    }

    /// A waiter whose target never arrives is bounded by its own deadline —
    /// it does not park forever. This composes with
    /// `crate::egress::AwaitPolicy`'s deadline exactly the same way: the
    /// caller converts its own bound to a `std::time::Instant` and passes it
    /// here.
    ///
    /// MUTATION VERIFIED: changing `ProgressListener::wait_deadline`'s body
    /// from `listener.wait_deadline(deadline).is_some()` to unconditionally
    /// `true` makes this test's `assert!(!woken, ..)` fail — `woken` is
    /// `true` even though nothing was ever published. Recompiled and re-run
    /// to confirm the failure, then reverted.
    #[test]
    fn waiter_whose_target_never_arrives_is_bounded_not_parked_forever() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let listener = trunk.listen().unwrap();

        let start = std::time::Instant::now();
        let woken = listener.wait_deadline(start + Duration::from_millis(150));
        let elapsed = start.elapsed();

        assert!(!woken, "must report timeout, not a fabricated wake-up");
        // Same reasoning as the generous bound in
        // `awaited_part_wakes_its_listener`: this asserts "returns rather
        // than parking forever", and any tight bound measures the machine.
        // The real assertion is `!woken` above; this one only catches a hang.
        assert!(
            elapsed < Duration::from_secs(60),
            "must actually return at the deadline, not hang: took {elapsed:?}"
        );
    }

    /// The hard invariant this whole file exists to preserve, extended to
    /// the wake channel: `publish_part`/`publish_segment` must still
    /// complete even with a waiter registered and never serviced (no one
    /// ever calls `wait`/`.await`/drops it) — a slow or vanished reader must
    /// never stall the writer. Proven the same way this file's existing
    /// `StallIngest`-blocks proof works, but for the opposite claim: a
    /// background-thread `publish_*` call, and a bounded `recv_timeout`
    /// proving it completed promptly.
    ///
    /// This specific guarantee is structural, not something a local
    /// mutation of this crate's code can plausibly violate: `publish_part`/
    /// `publish_segment` call `Event::notify(usize::MAX)`, which by
    /// `event_listener`'s own documented contract wakes registered listeners
    /// without waiting for any of them to resume — there is no "wait for
    /// the listener to be serviced" code path in this module to remove.
    /// Reaching a blocking wake-up would require swapping the whole
    /// primitive for a different one (the architectural choice already
    /// argued in this module's docs), not a one-line mutation, so no
    /// mutation transcript is claimed for this test — see this crate's
    /// convention that a structural property is reported as such rather
    /// than backed by an invented mutation.
    #[test]
    fn writer_never_blocks_with_a_registered_never_serviced_waiter() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let writer = Arc::new(trunk.segment_writer().unwrap());

        // Registered, kept alive for the whole test, and never waited on or
        // dropped before the assertions below run.
        let _never_serviced = trunk.listen().unwrap();

        let (done_tx, done_rx) = mpsc::channel();
        let bg_writer = Arc::clone(&writer);
        thread::spawn(move || {
            bg_writer.publish_part(part_entry(1, 1, 0));
            bg_writer.publish_segment(segment_entry(2, 1));
            done_tx.send(()).unwrap();
        });

        // HANG GUARD (issue #807): the property under test is "never blocks",
        // i.e. these calls return almost immediately (`Event::notify`
        // doesn't wait for listeners to resume) -- a stuck/blocked writer
        // here is the only thing this should ever catch, so raised for
        // load-tolerance rather than left as a timing claim.
        done_rx.recv_timeout(Duration::from_secs(60)).expect(
            "publish_part/publish_segment must complete promptly even with \
                 a live, never-serviced waiter registered",
        );
    }

    /// The waiter set itself is bounded — a flood of `listen()` calls cannot
    /// grow memory without limit, and reuses `part_capacity` rather than a
    /// sixth, independent knob.
    ///
    /// MUTATION VERIFIED: replacing `Trunk::listen`'s cap check (the
    /// `if current >= self.part_waiter_cap { return None; }` loop) with a
    /// version that always registers (never returns `None`) makes the
    /// `assert!(trunk.listen().is_none(), ..)` below fail — the call
    /// succeeds instead of being refused at the cap. Recompiled and re-run
    /// to confirm the failure, then reverted.
    #[test]
    fn waiter_set_is_bounded_a_flood_of_listen_calls_cannot_grow_without_limit() {
        let cap = nz(4).get();
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(cap)));

        // Fill exactly to the cap, keeping every registration alive.
        let mut held: Vec<ProgressListener> = Vec::new();
        for _ in 0..cap {
            held.push(trunk.listen().expect("must succeed up to the cap"));
        }
        assert_eq!(trunk.waiter_count(), cap);

        // One more must be refused, not silently over-admitted.
        assert!(
            trunk.listen().is_none(),
            "must refuse a registration beyond part_capacity"
        );

        // A flood of register-then-immediately-drop calls (no one keeping
        // them alive) must never push the live count past the cap, however
        // many times it runs.
        for _ in 0..50_000 {
            let l = trunk.listen();
            assert!(
                trunk.waiter_count() <= cap,
                "waiter count exceeded part_capacity mid-flood"
            );
            drop(l);
        }
        assert_eq!(
            trunk.waiter_count(),
            cap,
            "the held registrations are still exactly at the cap"
        );

        // Releasing one held slot frees exactly one registration.
        held.pop();
        assert_eq!(trunk.waiter_count(), cap - 1);
        assert!(
            trunk.listen().is_some(),
            "a released slot must be re-usable"
        );
    }

    /// The decided close-behaviour, asserted: a part remains addressable via
    /// `part_bytes` after its parent segment closes (this trunk's `Trunk`
    /// does not evict/transform parts on `publish_segment`), right up until
    /// `part_capacity`'s ordinary eviction reclaims it — at which point a
    /// client requesting that same part gets `None`, indistinguishable from
    /// "never existed", exactly like every other ring's eviction in this
    /// module.
    ///
    /// MUTATION VERIFIED: adding an eviction step to `publish_segment` that
    /// removes every `PartLog` entry whose `segment_number` matches the
    /// just-closed segment (simulating the rejected "evict a segment's
    /// parts the instant it closes" alternative documented in this module's
    /// docs) makes the first `part_bytes(1, 0)` assertion below fail
    /// immediately after `publish_segment` — it returns `None` instead of
    /// `Some(..)`. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[test]
    fn parts_remain_addressable_after_segment_close_until_ordinary_eviction_reclaims_them() {
        let part_cap = nz(4).get();
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(part_cap)));
        let writer = trunk.segment_writer().unwrap();

        writer.publish_part(part_entry(0xAB, 1, 0));
        writer.publish_segment(segment_entry(1, 1));

        // The part a client just watched roll into a closed segment is
        // still `Some` — the same answer as before the close.
        assert_eq!(
            trunk.part_bytes(1, 0),
            Some(Bytes::from(vec![0xAB; 8])),
            "a just-closed segment's part must still be individually fetchable"
        );
        assert_eq!(trunk.last_closed_segment(), Some(1));

        // Flood the part ring with `part_cap` more entries for an unrelated
        // segment — enough to evict the original part via ordinary
        // capacity-based eviction, with no further segment closes involved.
        for i in 0..part_cap as u32 {
            writer.publish_part(part_entry(0xFF, 99, i));
        }

        assert!(
            trunk.part_bytes(1, 0).is_none(),
            "the part is gone once ordinary part_capacity eviction reclaims \
             it — NOT because its segment closed, but because the ring's own \
             bound was exceeded, exactly like every other ring in this module"
        );
    }

    // === Issue #781: track-set snapshot + generation counter ==============

    /// A freshly-minted `Trunk` announces no tracks yet — `set_tracks` has
    /// never been called, so there is nothing to seed `tracks()`/
    /// `track_generation()` from other than the empty defaults `Trunk::new`
    /// establishes.
    ///
    /// MUTATION VERIFIED: changing `Trunk::new`'s `track_generation: 0` to
    /// `track_generation: 1` makes the second `assert_eq!` below fail — it
    /// reads back `1` instead of `0`. Recompiled and re-run to confirm the
    /// failure, then reverted.
    #[test]
    fn fresh_trunk_has_no_tracks_and_generation_zero() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        assert_eq!(trunk.tracks().len(), 0, "nothing has ever set a track set");
        assert_eq!(trunk.track_generation(), 0);
    }

    /// `set_tracks` is a **whole-set replacement**, not a merge/append, and
    /// bumps the generation by exactly one per call.
    ///
    /// MUTATION VERIFIED: changing `TrunkWriter::set_tracks`'s body to merge
    /// the old set with the new one (`let mut merged = state.tracks.to_vec();
    /// merged.extend(tracks); state.tracks = Arc::from(merged);`) instead of
    /// replacing outright makes the second `assert_eq!` below fail —
    /// `trunk.tracks()`'s track ids read back `[1, 7, 9]` (the old track 1
    /// still present) instead of `[7, 9]`. Recompiled and re-run to confirm
    /// the failure, then reverted.
    #[test]
    fn set_tracks_replaces_the_whole_set_and_bumps_generation_by_one_per_call() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let writer = trunk.writer().unwrap();

        writer.set_tracks(vec![opaque_track(1)]);
        assert_eq!(
            trunk
                .tracks()
                .iter()
                .map(|t| t.track_id)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(trunk.track_generation(), 1);

        // A completely different, larger set: if this were a merge/append
        // rather than a replacement, the old track_id 1 would still be
        // present alongside the two new ones.
        writer.set_tracks(vec![opaque_track(7), opaque_track(9)]);
        assert_eq!(
            trunk
                .tracks()
                .iter()
                .map(|t| t.track_id)
                .collect::<Vec<_>>(),
            vec![7, 9],
            "set_tracks must replace the set wholesale, not append to it"
        );
        assert_eq!(
            trunk.track_generation(),
            2,
            "generation must advance by exactly one per set_tracks call"
        );
    }

    /// `track_generation` is stable across everything that is *not*
    /// `set_tracks` — publishing samples/events/segments/parts must never
    /// bump it, so a consumer polling the generation as a cheap "did the
    /// track set change" check cannot see false positives.
    ///
    /// MUTATION VERIFIED: adding `state.track_generation += 1;` to
    /// `TrunkWriter::publish` (simulating "generation accidentally bumped by
    /// unrelated activity") makes the final `assert_eq!` below fail — the
    /// generation reads back `11` (bumped once per one of the 10 published
    /// samples, on top of the `1` from `set_tracks`) instead of staying at
    /// `1`. Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn generation_is_stable_across_unrelated_activity() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let writer = trunk.writer().unwrap();

        writer.set_tracks(vec![opaque_track(1)]);
        assert_eq!(trunk.track_generation(), 1);

        for i in 0u8..10 {
            writer.publish(1, RetentionClass::Timed, sample(i, 4));
        }
        writer.publish_event(basic_event(1), EventAnchor::Media(MediaTime(0)));

        assert_eq!(
            trunk.track_generation(),
            1,
            "publishing samples/events must never bump track_generation"
        );
    }

    /// `set_tracks` wakes a registered [`Trunk::listen`] listener, the same
    /// broad `progress` channel [`SegmentWriter::publish_part`]/
    /// [`SegmentWriter::publish_segment`] already wake — see
    /// [`waiter_is_woken_when_the_awaited_part_lands_and_resolves_to_it`] for
    /// the identical pattern this test mirrors.
    ///
    /// MUTATION VERIFIED: removing `self.trunk.progress.notify(usize::MAX);`
    /// from `TrunkWriter::set_tracks` makes `listener.wait_deadline(..)`
    /// time out (`false`) instead of waking (`true`) within the 60s bound
    /// used below. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[test]
    fn set_tracks_wakes_a_registered_listener() {
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(4), nz(8), nz(8)));
        let writer = Arc::new(trunk.writer().unwrap());

        // Register BEFORE the change — the documented no-missed-wakeup
        // ordering every other `listen()` test in this module follows.
        let listener = trunk.listen().expect("first registration must succeed");

        let bg_writer = Arc::clone(&writer);
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            bg_writer.set_tracks(vec![opaque_track(1)]);
        });

        // Same generous, machine-independent bound as this module's other
        // wake tests — see their comments for why 60s asserts only "it woke
        // at all", not "it woke fast".
        let woken = listener.wait_deadline(std::time::Instant::now() + Duration::from_secs(60));
        assert!(woken, "listener must wake on set_tracks, not park forever");
        handle.join().unwrap();

        assert_eq!(trunk.track_generation(), 1);
    }
}
