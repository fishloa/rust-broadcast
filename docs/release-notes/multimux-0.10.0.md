# multimux 0.10.0

_Released 2026-08-14._

Minor, additive. A multimux route can now take a **media file** as its input and
stream it through every existing output.

## `InputSpec::File`

```json
{ "name": "slate",
  "input": { "type": "file", "path": "/media/slate.ts", "loop": true },
  "outputs": ["llhls"] }
```

The file is served through the same `supervise_driver` + `advance_route` path as
every other source, so LL-HLS, DASH, LL-DASH and TS-HLS all work unchanged, as
do DVR, auth and the metrics surface.

**Container identification** goes through the new
[`container-probe`](container-probe-0.1.0.md) crate, over the whole file. The
verdict selects the demuxer — including the ISOBMFF fragmented/progressive split
via `IsobmffLayout`, so no box-walking is duplicated here. A format `transmux`
cannot demux, and every ambiguous or undetermined verdict, fails with its own
structured error: **a file is never fed to a guessed demuxer.**

**Looping** is on by default. A loop refills from the already-parsed content
with a per-track PTS offset, so the output timeline stays strictly monotonic
across the loop point. That offset derives from the **presentation-max** PTS
rather than the decode-order last sample — a track with B-frames reorders, so a
decode-order basis steps backwards across the loop and produces a stream that
looks fine until a player stalls.

`NewProgram` is emitted on its own drain so `advance_route` creates the
segmenter *before* any sample lands. A segmenter subscribes at the trunk's live
edge, so announcing and publishing in one drain would silently lose the file's
opening samples.

## Known limit

**A long asset holds its parsed samples in memory.** The file is demuxed in one
pass and the sample ring sized to it, so memory scales with asset length. This
suits slates, idents and short filler. A long-file path needs incremental
demux, which this release does not attempt.

Pacing is *not* a limitation. The route path paces to wall clock, across the
loop boundary, in every `pace` x `loop` combination. An earlier draft of this
note listed "the route path does not pace samples" as a permanent limit; that
described the first implementation and was fixed before release. Unpaced, the
route republished the whole file every 10 ms — roughly 300x realtime — which
made the playlist's segment cadence meaningless to a player. Covered by
`file_route_loop_true_paces_near_realtime` and
`file_route_loop_true_paces_past_first_pass`; the latter exists because the
original cadence test measured only inside pass 1 and so could not see the
defect at all.

## Scope note

Issue #748 originally designed a linear-playout **channel**: scheduled switching
between sources, a rebased shared timeline, and SCTE-35 ad-break signalling.
**That was cancelled during implementation** and the code reverted rather than
left half-finished — the complexity was not earning its keep. Only the
file-input capability shipped.

`docs/superpowers/specs/2026-08-11-linear-playout-design.md` is marked
`SUPERSEDED` and carries a per-section table of what shipped and what did not.
Its body is kept unedited, because the timeline-correctness reasoning is worth
reading if scheduled playout is ever revisited.

## Verification

The full workspace suite and all six CI gates are green.

Deliberately without a test count: an audit measured this note's previous
figure (6,829) against a real run (6,922) and found it stale. A number that
changes on every commit is a claim that rots between writing the note and
tagging it, and a reader cannot tell a stale count from a wrong one.

Two mutation proofs, each confirmed applied before being trusted:

- Removing the loop refill offset fails the monotonicity test with a real
  backwards jump — `399600 then 133200`.
- Removing `advance_route` leaves the route ingesting but never serving — no
  `#EXTINF:` appears in the fetched playlist.

The end-to-end test asserts a real HTTP playlist gains segments, **not** merely
that samples reached a `Trunk` — the latter would pass even with `advance_route`
absent, which is exactly the failure that call exists to prevent.

## Upgrading

No breaking changes. `InputSpec` is `#[non_exhaustive]`, so the new variant is
additive; existing configs are unaffected.

`multimux-cli` moves to the `0.10` line with it.

Published from tag `multimux-v0.10.0`.
