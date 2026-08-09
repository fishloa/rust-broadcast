# playout-runtime

[![crates.io](https://img.shields.io/crates/v/playout-runtime.svg)](https://crates.io/crates/playout-runtime)
[![docs.rs](https://img.shields.io/docsrs/playout-runtime)](https://docs.rs/playout-runtime)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

Sans-IO **linear channel playout** (issue
[#748](https://github.com/fishloa/rust-broadcast/issues/748)): a schedule
model (programme / ad / slate), transition planning across the join between
two sources, and the SCTE-35 `splice_insert()` emission points a transition
implies. `no_std` + `alloc`. No HTTP, no tokio.

## What's here

- **[`schedule`]** — [`schedule::Schedule`]: an ordered list of
  [`schedule::ScheduleEntry`] (programme/ad/slate, a source identity, and a
  planned start on the channel timeline). The format is deliberately
  **ours to define** — see the issue's design-decision comment: SCTE 224
  covers policy/blackout signalling, not channel assembly, and adopting it
  would answer a different question. There is therefore no `Parse`/
  `Serialize` pair here; it's an in-memory model, not a wire format.
- **[`transition`]** — [`transition::next_transition`] finds the next join
  the schedule implies and [`transition::TransitionPlan::plan`] computes it:
  the channel-timeline instant it happens at, a PTS-rebase offset so the
  incoming source's own clock lands continuously on the shared timeline, and
  a discontinuity flag when the codec configuration changes across the
  join. **This is the crate's primary tested property** — see
  [Timeline continuity](#timeline-continuity-the-primary-tested-property)
  below.
- **[`scte35`]** — [`scte35::build_splice_insert`] builds the
  `splice_insert()` (ANSI/SCTE 35 2023r1 §9.7.3) a transition implies, using
  `scte35-splice`'s own `Serialize` (not hand-assembled bytes), with its
  target instant conditioned against real candidate boundaries via
  [`ssai_runtime::splice::condition_splice_point`] — reused, not
  re-implemented.

## What this crate is **not**

- **No transcoding or conforming.** A differing codec config across a
  transition is a discontinuity to *signal*
  ([`transition::TransitionPlan::discontinuity`]), never something this
  crate re-encodes. This workspace parses containers and never touches the
  codec bitstream, and that holds here too.
- **No actual sample-timestamp rewriting.** `transmux`'s IR transform would
  apply [`transition::TransitionPlan::pts_rebase_offset`] to real sample
  PTS/DTS values; this crate computes the number, not the rewrite.
  `broadcast_hls::mark_init_discontinuities` would act on
  [`transition::TransitionPlan::discontinuity`] at the playlist/init-segment
  level — neither dependency is pulled in here.
- **No boundary conditioning of its own.** [`scte35::build_splice_insert`]
  calls `ssai-runtime`'s `condition_splice_point` rather than duplicating
  nearest-boundary snapping — two implementations of the same boundary math
  could disagree about the same boundary.
- **No HTTP, no tokio.** A `multimux` adapter driving a real channel clock
  against this crate's planning is deliberately future work, mirroring the
  `rtsp-runtime`/`hls-runtime`/`ssai-runtime` sans-IO split.

## Timeline continuity: the primary tested property

Issue #748's framing: "a schedule that plays the right thing at the wrong
timestamp is worse than no scheduler." This crate's tests hold
`TransitionPlan` to that bar rather than treating schedule bookkeeping as
the interesting part:

- `transition::tests::rebase_lands_the_incoming_source_continuously_on_the_channel_timeline` —
  once rebased, the incoming source's own PTS sequence lands exactly on the
  channel timeline with no gap and no overlap.
- `transition::tests::rebase_returns_none_rather_than_wrapping_out_of_range` —
  a rebase that would fall outside `u64`'s range reports `None`, never a
  wrapped, wrong instant.
- `tests/real_fixture_transition.rs` plans a programme -> ad transition
  against the **real**, non-IDR-aligned cue in `fixtures/scte35-ssai/`
  (DASH-IF `livesim2`, Apache-2.0): the nearest video keyframe is 6000 ticks
  (67 ms) *after* the cue's nominal instant, measured independently
  straight from the fragment's own boxes (`fixtures/scte35-ssai/PROVENANCE.md`).
  The test asserts that measured gap and that the emitted cue carries the
  *conditioned* instant, not the raw request — a regression that made
  `build_splice_insert` a passthrough of `requested_pts` fails this test
  loudly.

## Examples

- `schedule_and_transition` — build a programme -> ad -> programme schedule
  and walk every transition it implies.
- `emit_splice_from_real_cue` — the full pipeline against the real fixture:
  decode the cue, plan the transition, condition the splice point, emit and
  byte-exact-round-trip the `splice_insert()`.

```sh
cargo run -p playout-runtime --example schedule_and_transition
cargo run -p playout-runtime --example emit_splice_from_real_cue
```

## Install

```toml
[dependencies]
playout-runtime = "0.1"
```

## Spec references

- SCTE-35 `splice_insert()`: ANSI/SCTE 35 2023r1 §9.7.3, transcribed at
  [`scte35-splice/docs/`](../scte35-splice/docs/).
- Splice-point conditioning: [`ssai-runtime`](../ssai-runtime), issue #929.

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or
[MIT license](../LICENSE-MIT) at your option.
