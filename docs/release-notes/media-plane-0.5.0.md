# media-plane 0.5.0

_Released 2026-10-05._

Breaking (0.x minor) release of the ingress/egress spine. Its headline is a set of correctness fixes to `Trunk` (#1082, #1056, #1083, #1134) that came out of an audit of what happens when a source reconnects: a dropped `TrunkWriter` used to orphan every subscribed cursor for good, an HLS media sequence number could go backwards, and a stalled DVR pin could deadlock the thread that was supposed to release it. Fixing them changes three signatures. **Breaking for** callers of `SegmentWriter::publish_segment`/`try_publish_segment` (now `Result`) and `ReconnectPolicy::new` / `ReconnectPolicy::max_attempts` (now `NonZeroU32`), and for anything that relied on `ByteMerge` failover never firing from a primary that never spoke. Everyone else gets the fixes without a code change. This release also contains the never-tagged 0.4.1 (`Trunk::time_anchor()`, for `hls-runtime`'s `EXT-X-DATERANGE` rendering), so there is no 0.4.1 on crates.io.

Read together with: [multimux-0.11.0.md](multimux-0.11.0.md) and [hls-runtime-0.7.0.md](hls-runtime-0.7.0.md), the two consumers whose reconnect, DVR and LL-HLS behaviour this release was built for; [transmux-0.25.0.md](transmux-0.25.0.md) and [timed-metadata-0.6.0.md](timed-metadata-0.6.0.md), the epochs it now builds against.

## Breaking changes

### `SegmentWriter::publish_segment` and `try_publish_segment` return `Result` and enforce monotonic numbers

`publish_segment` returns `Result<(), NonMonotonicSequenceNumber>`; `try_publish_segment` returns `Result<(), TryPublishSegmentError>` (variants `WouldStall(SegmentEntry)` and `NonMonotonic`, `#[non_exhaustive]`). A `sequence_number` that is not strictly greater than the last one the `Trunk` accepted is rejected. The rule is global to the `Trunk`, across every `SegmentWriter` it ever issues, including after a re-issue. An HLS media sequence number must never go backwards, and a restarted segmenter that renumbered from 1 used to make every query keyed on `sequence_number` alone (`Trunk::part_bytes`, `Trunk::parts_in_segment`, `Trunk::events_in_segment`, `RetentionDriver::locate`) resolve ambiguously. Seed a fresh or re-issued writer with `SegmentWriter::next_sequence_number()`.

```rust
// 0.4
writer.publish_segment(entry);
// 0.5
let first = writer.next_sequence_number();   // after a re-issue, resume from here
writer.publish_segment(entry)?;              // Err(NonMonotonicSequenceNumber { attempted, last_published })
```

### `ReconnectPolicy` takes `NonZeroU32`

`ReconnectPolicy::new` and the `max_attempts` field use `NonZeroU32` instead of `u32`. The zero policy used to `assert!`-panic in the constructor, a hazard for a value read from a config file; it is now unrepresentable (#1082).

```rust
// 0.4
let p = ReconnectPolicy::new(5);
// 0.5
let p = ReconnectPolicy::new(std::num::NonZeroU32::new(5).expect("non-zero"));
```

### `Trunk::writer` and `Trunk::segment_writer` are re-issuable

The returned `TrunkWriter` or `SegmentWriter` releases its slot on `Drop`, so a later call succeeds again. Before, a dropped writer left `Trunk::writer()` returning `None` for ever and orphaned every already-subscribed `SampleCursor` and `SegmentCursor`, since nothing would publish into their rings again (#1082). If you treated `None` as "a source is already attached" and relied on it staying `None` after a drop, that is no longer true.

For the samples side, a cursor now sees the replacement: `SampleCursorItem::Discontinuity` is reported once, between the old writer's last sample and the new one's first, when the write handle is re-issued after a reconnect (never for a handover the cursor could not have observed any continuity across, and gated per class so neither `Timed` nor `Sparse` data from the new writer overtakes it). A cursor that falls further behind than the fixed handover backlog gets `SampleCursorItem::HandoversLagged { skipped }`, a loss report about bookkeeping (no sample data is lost by it). `SampleCursorItem` is `#[non_exhaustive]`, so a wildcard arm already covers both.

### `ByteMerge` `MergePolicy::Failover` now fails over from a silent primary

Behaviour change, no signature change. The silence clock runs from the first message of either named source (`primary` or `secondary`; a third source does not count) until `primary` has spoken. `next_deadline` is therefore armed, and once `silence_timeout` elapses `on_deadline` switches to `secondary` and its traffic is forwarded. Before, `secondary` was dropped for ever and `next_deadline` stayed `None` (#1082). Separately, `ByteMerge::feed` no longer mutates failover state for a message it rejects with `QueueFull`: a rejected primary message used to reset the silence clock and reclaim the active source, so back-pressure on a discarded primary stream prevented failover.

## Added

- `SegmentWriter::try_publish_segment`: the non-blocking alternative to `publish_segment` for the one case that can stall (`ArchiveOverrun::StallIngest`). It returns `Err(TryPublishSegmentError::WouldStall(entry))` and hands the entry back instead of blocking or losing it. Use it from async code (#1082).
- `SegmentWriter::next_sequence_number`, `SegmentWriter::expire_stalled_pins` (forces every `StallIngest` pin currently blocking eviction to give up, as terminated, for a caller with its own non-blocking retry queue that tracks its own elapsed wait) (#1082).
- `Trunk::waiter_slot_freed() -> Option<SlotFreedFuture>`: resolves the next time a `Trunk::listen` waiter slot is freed, or `None` when a slot is free now, so a caller that got `None` from `listen()` can park on the event (raced with its own cancellation) instead of sleep-polling. `event_listener::EventListener` is re-exported as `SlotFreedFuture`; because that puts a third-party type in the public API, an `event-listener` major bump would be a media-plane major-class change. `WaiterSlot::drop` notifies a dedicated `slot_freed` event and only as many waiters as slots freed, never the shared progress event (which caused a wake-storm).
- `IngestDriver::session_mut`: mutable access to the driver's session, for an out-of-band signal a `Stage` has no input variant for (multimux's `dash_pull` uses it to abandon a live-edge segment whose tolerated-404 retries ran out) (#1083).
- `Trunk::time_anchor() -> Option<TimeAnchor>`: the wall-clock anchor given through `SegmentWriter::set_time_anchor`, `None` until set. Needed by `hls-runtime` to build a `timed_metadata::Timeline` for `EXT-X-DATERANGE` (issue #965; from the unpublished 0.4.1).
- A `trunk_contention` criterion benchmark (1 publisher, 1/4/16/64 cursors).

## Behaviour and fixes

- **`StallIngest` is bounded.** `publish_segment`'s `ArchiveOverrun::StallIngest` block used to be an unconditional `Condvar::wait`, which could deadlock a caller whose own thread also drives the `RetentionDriver` that would release the pin. It is now bounded; when the bound elapses the still-blocking pin is terminated (`ArchiveOverrun::Terminate`'s own signal), not silently downgraded to ordinary `Gap` loss, so the broken DVR guarantee is visible (#1082). `try_publish_segment` no longer marks a `Terminate` pin terminated for an eviction that a different `StallIngest` pin prevented from happening in that call.
- **No lock poisoning.** `Trunk`'s state lock and the back-pressure `Condvar` are `parking_lot` (new optional dependency, enabled by `std`); a panic in one consumer no longer poisons every later writer or reader call (25+ `.expect("... poisoned")` sites removed, one of them in a `Drop` impl). No public API change. A lock-split was measured and reverted for missing the pre-registered 30 % gain bar (16 readers: 574 ns to 1104 ns per publish); numbers are in `benches/RESULTS.md`.
- **LL-HLS part and segment queries are correct and cheaper.** `Trunk::parts_in_segment` no longer scans from the back of the part ring and stops at the first non-matching entry. A live producer may publish part `(N+1, 0)` while segment `N` is open, landing it after `N`'s own parts, and the old scan then reported zero parts for the open segment, dropping its `#EXT-X-PART` tags from a served playlist. `part_bytes` and `parts_in_segment` also resolve a reused `segment_number` to its most recent parts, and use a per-segment position index so a viewer's request rate no longer scales the writer's lock hold time with `part_capacity`.
- **Event lookups resolve reused numbers to the latest boundary.** `EventLog`'s segment-boundary lookups (`events_in_segment`'s start boundary and `try_resolve`'s `Segment` arm) no longer return a stale earlier entry; the end boundary is the first start positionally after the chosen one, so a range can no longer invert. A `splice_schedule`-style `EventAnchor::Utc` earlier than the `TimeAnchor` origin (or beyond `u64` ticks) stays unresolved instead of being clamped to a fabricated media time `0` or `u64::MAX`.
- `RetentionDriver::locate` no longer reports a produced but not yet drained segment as `Evicted`; it compares against the driver's own pin-drain progress (#1056).
- `IngestDriver::feed` and `finish` drain queued `SessionEvent`s before entering `HealthState::Failed` on error, instead of dropping events queued by the call that failed.
- Cursor ring indexing uses a checked offset instead of wrapping `as usize` casts, and two `expect`s on pin lookups in `SegmentCursor::poll` are non-panicking (#1134, #1082). `egress` docs no longer claim `Trunk` has no reader-side notify primitive.

## Dependency and feature changes

From `git diff media-plane-v0.4.0..HEAD -- media-plane/Cargo.toml`:

```toml
broadcast-common = "9.4"    # was 9.3
transmux         = "0.25"   # was 0.24 (optional, via std)
timed-metadata   = "0.6"    # was 0.5 (optional, via std)
parking_lot      = "0.12"   # new, optional, enabled by std
# [dev-dependencies] scte35-splice 2.1 -> 3.0 (tests only), criterion 0.8 (benchmark)
```

The `std` feature now also enables `dep:parking_lot`. A new `[[bench]] trunk_contention` requires `std`. `no_std` + `alloc` byte-layer builds are unaffected. MSRV 1.95.0.

---

Published from tag `media-plane-v0.5.0`.
