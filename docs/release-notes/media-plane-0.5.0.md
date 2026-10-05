# media-plane 0.5.0

_Released 2026-10-05._

### Changed
- `Trunk`'s state lock and the `StallIngest` back-pressure `Condvar` are now `parking_lot` (no poisoning; `lock_state` is now a one-line passthrough, the poison-recovery logic is gone). No public API change.
- Measured the single-lock contention with the new `trunk_contention` criterion benchmark; numbers and the lock-split decision are in `benches/RESULTS.md`: the pre-registered rule said SPLIT, a sample-group / segment-event-part-group split was implemented and measured (16 readers: 574 ns -> 1104 ns per publish), missed the >= 30 % gain bar and was reverted.
- Tests observe a parked `StallIngest` publisher directly (test-only probe) instead of sleeping or waiting a fixed 200 ms.

### Added
- `Trunk::waiter_slot_freed() -> Option<SlotFreedFuture>` — resolves the next
  time a `Trunk::listen` waiter slot is freed (its `ProgressListener` dropped),
  or `None` when a slot is free now. Lets a caller that got `None` from
  `listen()` park on the event (raced with its own cancellation) instead of
  sleep-polling for a slot; `WaiterSlot::drop` now notifies it (W2b-1 B10b).
  `event_listener::EventListener` is re-exported as `SlotFreedFuture` so the
  return type is nameable without a direct dependency; because that puts a
  third-party type in the public API, an `event-listener` MAJOR bump would be a
  `media-plane` MAJOR-class change.
  `WaiterSlot::drop` notifies a DEDICATED `slot_freed` event (never the shared
  `progress` event — doing so woke every waiter and caused a ping-pong
  wake-storm), and only as many waiters as slots freed.
- `SegmentWriter::try_publish_segment` — non-blocking alternative to
  `SegmentWriter::publish_segment` for the one case that can stall
  (`ArchiveOverrun::StallIngest`); returns `Err` instead of blocking or
  losing the entry (issue #1082).
- `SegmentWriter::next_sequence_number` — the `sequence_number` a fresh or
  re-issued `SegmentWriter` must resume numbering from (issue #1082).
- `SegmentWriter::expire_stalled_pins` — non-blocking: forces every
  `ArchiveOverrun::StallIngest` pin currently blocking eviction to give up
  (terminated, the same signal the bounded blocking wait already uses),
  for a caller with its own non-blocking retry queue that tracks its own
  elapsed wait rather than making one blocking call (issue #1082).
- `SampleCursorItem::Discontinuity` — reported once, positioned between a
  dropped `TrunkWriter`'s last sample and its replacement's first, when the
  write handle is re-issued after a source reconnect. Never reported for a
  handover a given cursor could not have observed any continuity across
  (one recorded at or before that cursor's own subscribe position), and
  gated per-class so neither `Timed` nor `Sparse` data from the new writer
  can overtake it (`SampleCursorItem` is `#[non_exhaustive]`, so this is
  additive) (issue #1082).
- `SampleCursorItem::HandoversLagged { skipped }` — a cursor that falls more
  writer hand-offs behind than the (small, fixed) backlog retains now
  reports it distinctly instead of silently catching up and losing the
  report entirely (issue #1082).
- `IngestDriver::session_mut` — mutable access to the driver's session, for
  an out-of-band signal a `Stage` has no input variant for (`dash_pull`
  uses it to abandon a live-edge segment whose tolerated-`404` retries ran
  out) (issue #1083).
- `benches/trunk_contention` (criterion): 1 publisher x N = 1, 4, 16, 64 cursors.
- `tests/no_handroll_guard.rs`.

### Changed (breaking)
- `Trunk::writer`/`Trunk::segment_writer` are now **re-issuable**: the
  returned `TrunkWriter`/`SegmentWriter` releases its slot on `Drop`, so a
  later call succeeds again instead of permanently returning `None` after
  the first — previously a dropped writer orphaned every already-subscribed
  `SampleCursor`/`SegmentCursor` forever (nothing would ever publish into
  their rings again) (issue #1082).
- `SegmentWriter::publish_segment`/`SegmentWriter::try_publish_segment` now
  return `Result` and reject a `sequence_number` that is not strictly
  greater than the last one this `Trunk` accepted
  (`NonMonotonicSequenceNumber`/`TryPublishSegmentError`) — segment numbers
  are monotonic across every `SegmentWriter` a `Trunk` ever issues,
  including after a re-issue: an HLS media sequence number must never go
  backwards, and a restarted segmenter renumbering from 1 again previously
  made every query keyed on `sequence_number` alone
  (`Trunk::part_bytes`/`Trunk::parts_in_segment`/`Trunk::events_in_segment`/
  `RetentionDriver::locate`) resolve ambiguously. Use
  `SegmentWriter::next_sequence_number` to pick a valid number after a
  re-issue (issue #1082).
- `ReconnectPolicy::max_attempts` and `ReconnectPolicy::new` now take
  `NonZeroU32` instead of `u32`: the zero policy used to `assert!`-panic in
  the constructor, a hazard for a value that arrives from a config file; it
  is now unrepresentable (#1082).
- **Behaviour change:** `ByteMerge` `MergePolicy::Failover` now fails over
  from a primary that never spoke: the silence clock runs from the first
  message of either named source (`primary` or `secondary`; a third source
  does not count) until `primary` has spoken, so `next_deadline` is armed
  and, once `silence_timeout` elapses, `on_deadline` switches to `secondary`
  and its traffic is forwarded. Previously secondary was dropped forever
  and `next_deadline` stayed `None` (#1082).

### Fixed
- `RetentionDriver::locate` no longer reports a produced-but-not-yet-drained
  segment as `Evicted` — it compared against `Trunk::last_closed_segment`
  (which reports anything ever produced) instead of this driver's own
  pin-drain progress, so a segment still resident and pin-protected in the
  hot ring, simply not yet looked at by `drive`, was reported gone (issue #1056).
- Replaced 25+ `.expect("... poisoned")` call sites in `trunk.rs` (including
  one in a `Drop` impl) with a poison-recovering lock helper: a panic in one
  consumer while it held the trunk's state lock no longer poisons every
  later writer/reader call on the same `Trunk` (issue #1082).
- `SegmentWriter::publish_segment`'s `ArchiveOverrun::StallIngest` block is
  now bounded instead of unconditional — previously an unconditional
  `Condvar::wait` could deadlock a caller whose own thread also drives the
  `RetentionDriver` that would release the pin. If the bound elapses, the
  still-blocking pin is now **terminated** (`ArchiveOverrun::Terminate`'s
  own signal, reused) rather than silently falling back to ordinary `Gap`
  loss, which would have hidden that a pinned DVR consumer's stronger
  guarantee was broken (issue #1082).
- `SegmentWriter::try_publish_segment` no longer marks an
  `ArchiveOverrun::Terminate` pin terminated for an eviction that, because
  a *different* pin (`ArchiveOverrun::StallIngest`) forced the call to
  return `Err`, never actually happened this call (issue #1082).
- `EventLog`'s segment-boundary lookups (`try_resolve`'s `Segment` arm and
  `Trunk::events_in_segment`'s start boundary) now resolve a reused
  `segment_number` against its most recently recorded boundary instead of a
  stale, earlier one with the same number; `events_in_segment`'s end
  boundary is now the first start positionally after the chosen one, not a
  number lookup that could resolve to a stale, chronologically-earlier
  entry and invert the range (issue #1082).
- `Trunk::part_bytes`/`Trunk::parts_in_segment` now resolve a reused
  `segment_number` to its current, most-recently-published parts instead of
  a stale earlier round mixed in or returned outright (issue #1082).
- `IngestDriver::feed`/`IngestDriver::finish` now drain queued
  `SessionEvent`s before entering `HealthState::Failed` on error, instead of
  dropping events the session had already queued as part of the same call
  that ultimately failed it (issue #1082).
- `Trunk::parts_in_segment` no longer isolates "the current run" by scanning
  from the back of the part ring and stopping at the first non-matching
  entry. That approach assumed a segment's parts are always one contiguous
  run at the tail, which a live LL-HLS producer can legitimately violate —
  it may publish part `(N+1, 0)` while segment `N` is still open, landing
  it after `N`'s own parts in the ring. The from-the-back scan then hit
  `(N+1, 0)` first and reported *zero* parts for the still-open segment
  `N`, silently dropping its `#EXT-X-PART` tags from a served playlist.
  Now a plain filter over the whole ring, matched by `segment_number`
  alone.
- `ByteMerge::feed` no longer mutates failover state for a message it
  rejects with `QueueFull`: a rejected primary message used to reset the
  silence clock and reclaim the active source, so back-pressure on a
  discarded primary stream prevented failover (#1082).
- A `splice_schedule`-style `EventAnchor::Utc` instant earlier than the
  `TimeAnchor`'s timeline origin (or beyond `u64` ticks) is no longer
  clamped to a fabricated media time `0`/`u64::MAX`: the entry stays
  `EventAnchor::Utc` (unresolved), so a stale or replayed schedule can no
  longer appear "at the start of the stream" in `events_between` /
  `events_in_segment` (#1082).
- `egress` module docs no longer claim `Trunk` has no reader-side notify
  primitive or snapshot queries for parts/segments; they now point at
  `Trunk::listen`, `part_bytes`, `parts_in_segment` and `last_closed_segment`
  (#1082).
- `Trunk::part_bytes`/`Trunk::parts_in_segment` no longer scan the whole
  part ring under the trunk's state mutex on every call: a per-segment
  position index makes each touch only the requested segment's own parts,
  so a viewer's request rate no longer scales the ingest writer's lock hold
  time with `part_capacity` (#1082).
- Cursor ring indexing no longer uses wrapping `as usize` casts on
  `consumed - base` (checked `ring_offset`), and two `expect`s on pin lookups
  in `SegmentCursor::poll` became non-panicking `if let`s (#1134, #1082).

---

Published from tag `media-plane-v0.5.0`.
