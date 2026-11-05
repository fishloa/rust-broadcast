# playout-runtime 0.1.0

**Release date:** 2026-08-10

First publish. A sans-IO linear channel playout core (issue #748): a schedule
model for an ordered programme/ad/slate list, transition planning across a
join, and the SCTE-35 `splice_insert()` a transition implies. No transcoding,
no HTTP, no tokio — this is a planning/decision library that a live playout
process drives, not a playout process itself.

## What it is

- `schedule::Schedule` / `schedule::ScheduleEntry` / `schedule::EntryKind` — an
  ordered, strictly-increasing list of programme/ad/slate entries. There is no
  applicable open standard for channel-assembly playlists to adopt (SCTE 224
  covers policy/blackout signalling, not assembly), so this is a plain
  in-memory model with no `Parse`/`Serialize` pair — there is no wire format
  to be symmetric about.
- `transition::TransitionPlan` / `transition::next_transition` — transition
  planning across a join: a PTS-rebase offset for timeline continuity across
  the source change, and a discontinuity flag when the codec configuration
  changes. Verified against a real, non-IDR-aligned SCTE-35 cue
  (`fixtures/scte35-ssai/`, DASH-IF `livesim2`, Apache-2.0).
- `scte35::build_splice_insert` / `scte35::to_section` / `scte35::BreakEdge` —
  build and serialize the `splice_insert()` a transition implies, with its
  target instant conditioned via `ssai_runtime::splice::condition_splice_point`
  rather than a second, duplicate nearest-boundary implementation.

## What this crate does not do

- **No transcoding or conforming.** A differing codec config across a
  transition is a discontinuity to *signal*, never something this crate
  re-encodes.
- **No sample-timestamp rewriting.** It computes the PTS-rebase offset and the
  discontinuity flag; applying either to real sample data is `transmux`'s
  (IR transform) and `broadcast-hls`'s (`mark_init_discontinuities`) job, and
  neither dependency is pulled in here.
- **No HTTP, no tokio.** There is no network client and no async runtime
  dependency anywhere in this crate. A `multimux` adapter driving a real
  channel clock against this crate's planning is deliberately future work.

`no_std` + `alloc`.

## Migration

New crate; no migration.
