# scte35-splice 7.0.0

_Released 2026-11-15._

Major-class release (6 -> 7). A `splice_insert` command can no longer describe two contradictory splice points: the `program_splice_flag`/`splice_immediate_flag`/`splice_time`/`components` quartet is one `mode`, a cancel names the event it cancels, and every 33-bit `pts_time` is a validated clock value. The tracker now handles a legally repeated TS packet and reports a section it cannot judge instead of dropping it silently. The crate also moves to new caret epochs of `broadcast-common` and `mpeg-ts`, which is a major-class change on its own (docs/RELEASE-AUDIT.md section 2).

Version: 6.0.0 -> 7.0.0.

## Breaking changes and migration

### `SpliceInsert` carries its flags and time in one `SpliceMode`

`program_splice_flag`, `splice_immediate_flag`, `splice_time` and `components` are gone from the struct; `mode: SpliceMode` is `SpliceMode::Program(ProgramSplice::Immediate | ProgramSplice::At(SpliceTime))` or `SpliceMode::Components { immediate, components }`, and the wire flags are derived from it. Before, `splice_immediate_flag = true` beside `splice_time: Some(t)` wrote an immediate cue with `t` silently dropped (a 35-byte scheduled command became a 20-byte immediate one), and `splice_immediate_flag = false` beside `splice_time: None` wrote a time-less `splice_time()`.

```rust
// before
let si = SpliceInsert {
    splice_event_id: 1,
    program_splice_flag: true,
    splice_immediate_flag: false,
    splice_time: Some(SpliceTime { pts_time: Some(pts) }),
    ..Default::default()
};
// after
let si = SpliceInsert {
    splice_event_id: 1,
    mode: SpliceMode::Program(ProgramSplice::At(SpliceTime::at(pts))),
    ..Default::default()
};
// an immediate cue:        mode: SpliceMode::Program(ProgramSplice::Immediate)
// component mode:          mode: SpliceMode::Components { immediate, components }
```

### `SpliceTime::pts_time` is a validated `Clock33`

`pts_time` is `Option<broadcast_common::clock33::Clock33>` instead of `Option<u64>`, and `SpliceTime::with_pts(u64)` is replaced by `SpliceTime::at(Clock33)` with `SpliceTime::immediate()` for the `time_specified_flag == 0` form. `with_pts` used to mask its argument while `serialize_into` rejected anything above the 33-bit maximum, so one field had two policies; the field now cannot hold an off-clock value at all. A parsed `pts_time` wider than 33 bits is `Error::InvalidValue { field: "splice_time.pts_time" }` instead of being masked back onto the clock. `SpliceTime` is `#[non_exhaustive]`: build it with `at`, `immediate` or `Default`, and recover the raw tick count with `pts_time.map(|c| c.ticks())`.

```rust
// before
let time = SpliceTime::with_pts(90_000);
let ticks = time.pts_time;                    // Option<u64>
// after
let time = SpliceTime::at(Clock33::try_new(90_000)?);
let ticks = time.pts_time.map(|c| c.ticks()); // Option<u64>
let immediate = SpliceTime::immediate();
```

### A cancel names the event it cancels

`splice_event_cancel_indicator: bool` is `cancel: Option<u32>`, the id the cancel names; parsing fills it with `splice_event_id`. Build it with `SpliceInsert::cancel(id)`, and `serialize_into` rejects a `cancel` that disagrees with `splice_event_id` with `Error::InvalidField` instead of writing a cancel of the other event.

```rust
// before
let si = SpliceInsert { splice_event_id: 7, splice_event_cancel_indicator: true, ..Default::default() };
// after
let si = SpliceInsert::cancel(7);
```

### `SegmentationDescriptor` keeps every byte of its appendix

`sub_segments` is `Option<SubSegments>` and `SegmentationDescriptor` gains `trailing: &[u8]`. Parse used to take any two leftover bytes inside `descriptor_length` as the pair and drop a third, so the pair was guessed for a `segmentation_type_id` that does not carry it and a longer tail lost bytes on re-serialize. The appendix is now read only for the eight Table 23 `Start` types (`SegmentationTypeId::has_sub_segments`), every byte it does not claim is carried verbatim in `trailing`, and building `sub_segments: Some(..)` on a type that does not carry the pair is rejected with `Error::InvalidValue { field: "segmentation_descriptor.sub_segments" }`.

```rust
// before
let descriptor = SegmentationDescriptor { sub_segments: (1, 2), .. };
// after
let descriptor = SegmentationDescriptor {
    sub_segments: Some(SubSegments { sub_segment_num: 1, sub_segments_expected: 2 }),
    trailing: &[],
    ..descriptor
};
```

### `ClearPayload::descriptor_loop` is a `SpliceDescriptorLoop`

`ClearPayload::descriptor_loop` is `SpliceDescriptorLoop` (new module `scte35_splice::descriptor_loop`) instead of `&[u8]`, and `SpliceInfoSection::new_clear` takes one. The loop wraps the wire bytes verbatim (`raw()`), walks the TLV structure with `entries()` (a truncated entry is one `Err` with the loop offset, then the walk fuses), and `typed()` is the dispatch `SpliceInfoSection::descriptors` already performed. Wire bytes are unchanged.

```rust
// before
let clear = ClearPayload { descriptor_loop: &bytes, .. };
let section = SpliceInfoSection::new_clear(cmd, &[]);
// after
let clear = ClearPayload { descriptor_loop: SpliceDescriptorLoop::new(&bytes), .. };
let section = SpliceInfoSection::new_clear(cmd, SpliceDescriptorLoop::new(&[]));
```

### `SpliceTracker::feed` takes the packet's continuity and returns an outcome

`feed` takes the packet's `ContinuityCounter` and adaptation-field `discontinuity_indicator` and returns `FeedOutcome` (`Result<(), NoCue>`) instead of `()`. It fed the reassembler without continuity handling, so a legally repeated TS packet (ISO/IEC 13818-1 §2.4.3.3) doubled the section bytes into a corrupt one, and a section that failed CRC or parse vanished with no count or event. A non-cue section now comes back as `NoCue::Malformed { raw, error }` or `NoCue::NotACue`.

```rust
// before
tracker.feed(payload, pusi, |obs| { .. });
// after
let outcome = tracker.feed(payload, pusi, header.continuity_counter, discontinuity, |obs| { .. });
if let Err(NoCue::Malformed { raw, error }) = outcome { /* report the corrupt cue */ }
```

### `SpliceObservation` is `#[non_exhaustive]`

It is returned by `SpliceTracker::feed`; a field added later is no longer a breaking change.

## Fixes

- The `MAX_OPEN_EVENTS` cap evicts the oldest open event instead of the numerically smallest `splice_event_id`, which deleted a recent, low id whenever ids wrapped or a producer's numbering started low.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `mpeg-ts` 0.8 -> 0.9.

---

Published from tag `scte35-splice-v7.0.0`.
