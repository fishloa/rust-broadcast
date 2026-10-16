# playout-runtime

[![crates.io](https://img.shields.io/crates/v/playout-runtime.svg)](https://crates.io/crates/playout-runtime)
[![docs.rs](https://img.shields.io/docsrs/playout-runtime)](https://docs.rs/playout-runtime)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

Sans-IO **linear channel playout** (issue
[#748](https://github.com/fishloa/rust-broadcast/issues/748)): a schedule model
(programme / ad / slate), transition planning across the join between two
sources, and the SCTE-35 `splice_insert()` emission points a transition implies.

## What it does / what it does not

**Does.** Models a linear channel as an ordered schedule of entries, finds the
next join the schedule implies, and computes it: the channel-timeline instant,
the PTS-rebase offset that lands the incoming source's clock continuously on the
shared timeline, and a discontinuity flag when the codec configuration changes.
It also builds (and can wrap into a `splice_info_section`) the SCTE-35
`splice_insert()` for entering or returning from a break, with its target
conditioned against real candidate boundaries. Used by a channel-playout
orchestrator that owns the clock and the media.

**Does not.**

- No transcoding or conforming: a differing codec config is a discontinuity to
  *signal*, never something re-encoded. Codec bitstreams are never touched.
- No sample-timestamp rewriting: `transmux`'s IR transform would apply
  `pts_rebase_offset`; this crate computes the number, not the rewrite.
  `broadcast_hls::mark_init_discontinuities` would act on `discontinuity` at the
  playlist level; neither is a dependency here.
- No boundary conditioning of its own: it delegates to
  `ssai_runtime::splice::condition_splice_point_wrapping` so two implementations
  cannot disagree about the same boundary.
- No schedule wire format and no SCTE 224 (ESNI): the schedule is an in-memory
  model of our own (there is no `Parse` / `Serialize` pair; SCTE 224 covers
  policy and blackout signalling, not channel assembly).
- No HTTP, no tokio, no real-time clock; a `multimux` adapter driving a channel
  clock is future work.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ANSI/SCTE 35 | 2023r1 (vendored, `private/specs-md/ansi_scte_35_2023r1_dpi_cueing.md`) | `splice_insert()` (section 9.7.3, Table 10) built via `scte35-splice`: event id, `out_of_network_indicator`, program splice, `splice_time()`, `break_duration()` with `auto_return`; clear `splice_info_section` wrapper | Partial | Only `splice_insert`; no `time_signal`, segmentation descriptors, encryption or components. Units must be 90 kHz ticks |
| (none: schedule model) | n/a | Programme / ad / slate schedule | Not a standard | SCTE 224 deliberately not adopted |

## Features

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | Links `std` (and the siblings'). Off gives `#![no_std]` + `alloc`. |
| `serde` | no | `serde` derives on the schedule, transition and decision types. |

## Quick start

```rust
use playout_runtime::{
    BreakEdge, CodecConfigId, EntryKind, Schedule, ScheduleEntry, build_splice_insert,
    next_transition,
};

let mut schedule = Schedule::new();
schedule
    .push(ScheduleEntry {
        id: "news".into(),
        kind: EntryKind::Programme,
        planned_start: 0,
        source_start_pts: 0,
        codec_config: CodecConfigId(1),
    })
    .unwrap();
// A 30 s ad at 10 minutes (90 kHz ticks), cut from an asset whose own clock
// starts at 5_000, with a different codec configuration.
schedule
    .push(ScheduleEntry {
        id: "ad".into(),
        kind: EntryKind::Ad,
        planned_start: 54_000_000,
        source_start_pts: 5_000,
        codec_config: CodecConfigId(2),
    })
    .unwrap();

// The join: when, how to rebase the incoming clock, and whether to signal a
// discontinuity.
let t = next_transition(&schedule, 0).unwrap();
assert_eq!(t.plan.at_pts, 54_000_000);
assert!(t.plan.discontinuity);
assert_eq!(t.plan.rebase(5_000), Some(54_000_000));

// The splice_insert() that entering the break implies, snapped to the nearest
// real boundary (2_000 ticks before the nominal instant beats 3_000 after).
let (point, insert) = build_splice_insert(
    BreakEdge::Enter,
    1,
    t.plan.at_pts,
    &[53_998_000, 54_003_000],
    5_000,
    Some(2_700_000),
)
.unwrap();
assert_eq!(point.snapped_pts, 53_998_000);
assert!(insert.out_of_network_indicator);
```

## What is implemented

- **`schedule`**: `Schedule` (`new`, `push`, `entries`, `len`, `is_empty`,
  `active_at`) of `ScheduleEntry` (`id`, `EntryKind` programme / ad / slate,
  `planned_start`, `source_start_pts`, `CodecConfigId`).
- **`transition`**: `next_transition(&Schedule, now_pts)` returns a
  `PlannedTransition` (`from`, `to`, `plan`); `TransitionPlan::plan(from, to)`
  computes `at_pts`, `pts_rebase_offset` (`i128`, never clamped) and
  `discontinuity`; `TransitionPlan::rebase` maps a source PTS onto the channel
  timeline (`None` rather than wrapping out of range) and `rebase_wrapped`
  unrolls a raw 33-bit PTS first.
- **`scte35`**: `build_splice_insert(BreakEdge, event_id, requested_pts,
  candidates, max_delta_ticks, break_duration_ticks)` returns the
  `ConditionedSplicePoint` and the `SpliceInsert`; `to_section` wraps it in a
  clear `SpliceInfoSection`. Distances are circular modulo 2^33.
- **`error`**: `Error` (including `SpliceConditioning`).

### Timeline continuity: the primary tested property

Issue #748: "a schedule that plays the right thing at the wrong timestamp is
worse than no scheduler." The tests hold `TransitionPlan` to that bar:

- `transition::tests::rebase_lands_the_incoming_source_continuously_on_the_channel_timeline`:
  once rebased, the incoming source's PTS sequence lands exactly on the channel
  timeline with no gap and no overlap.
- `transition::tests::rebase_returns_none_rather_than_wrapping_out_of_range`.
- `tests/real_fixture_transition.rs` plans a programme to ad transition against
  the real, non-IDR-aligned cue in `fixtures/scte35-ssai/` (DASH-IF `livesim2`,
  Apache-2.0): the nearest keyframe is 6000 ticks (67 ms) after the cue's nominal
  instant, measured independently from the fragment's own boxes (see
  `fixtures/scte35-ssai/PROVENANCE.md`), and the emitted cue carries the
  conditioned instant, not the raw request.

## Examples

```sh
cargo run -p playout-runtime --example schedule_and_transition
cargo run -p playout-runtime --example emit_splice_from_real_cue
```

- `schedule_and_transition`: a programme, ad, programme schedule and every
  transition it implies.
- `emit_splice_from_real_cue`: decode the real cue, plan the transition,
  condition the splice point, emit and byte-exact round-trip the `splice_insert()`.

## Design notes

- Sans-IO, `no_std` + `alloc`, `#![forbid(unsafe_code)]`, structured `thiserror`
  errors; spec enums carry the workspace `name()` + `Display` label pair.
- The hard part is the transition, not the schedule: units are 90 kHz ticks and
  the caller must unwrap any source-clock wrap before `rebase` (or use
  `rebase_wrapped`).
- `scte35-splice`'s own `Serialize` builds the cue bytes; nothing is hand-assembled.

## Related crates

Depends on `broadcast-common`, `ssai-runtime` (splice-point conditioning) and
`scte35-splice` (the cue); dev-depends on `mp4-emsg` for the real-fixture tests.
`transmux` applies the rebase; `multimux` would drive it.

## Spec references

- SCTE-35 `splice_insert()`: ANSI/SCTE 35 2023r1 section 9.7.3, transcribed at
  [`scte35-splice/docs/`](../scte35-splice/docs/).
- Splice-point conditioning: [`ssai-runtime`](../ssai-runtime), issue #929.

## MSRV and license

MSRV 1.95.0. Licensed under either of
[Apache License, Version 2.0](../LICENSE-APACHE) or
[MIT license](../LICENSE-MIT) at your option.
