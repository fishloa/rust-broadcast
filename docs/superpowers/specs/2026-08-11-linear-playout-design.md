# Linear channel playout for multimux — design

**Issue:** [#748](https://github.com/fishloa/rust-broadcast/issues/748)
**Date:** 2026-08-11
**Status:** approved

Assemble a linear channel from a schedule of sources (programmes, ads, slates),
switching between them on a shared channel timeline and emitting SCTE-35
`splice_insert()` cues at ad-break edges. Turns multimux from a just-in-time
repackaging origin into a channel-origination platform.

## What already exists

`playout-runtime` 0.1.0 is the sans-IO core, and is **not changed by this
work**:

- `schedule::Schedule` — ordered entries (`id`, `EntryKind`, `planned_start`,
  `source_start_pts`, `CodecConfigId`), `push` rejecting out-of-order entries,
  `active_at`/`active_index_at` lookup.
- `transition::TransitionPlan::plan(from, to)` — computes
  `pts_rebase_offset = to.planned_start - to.source_start_pts` as `i128`, plus
  `discontinuity = from.codec_config != to.codec_config`. `rebase(source_pts)`
  maps a source PTS onto the channel timeline, returning `None` rather than
  wrapping when the result leaves `u64`.
- `transition::next_transition(schedule, now_pts)` — the next join due at or
  after `now_pts`.
- `scte35::build_splice_insert(edge, id, requested_pts, candidates,
  max_delta_ticks, break_duration)` — delegates splice-point conditioning to
  `ssai_runtime::splice::condition_splice_point`, erroring if no candidate
  boundary is within tolerance. `scte35::to_section(insert)` wraps it into a
  `SpliceInfoSection`.
- `scte35::BreakEdge::{Enter, Return}` — deliberately caller-supplied, because
  a schedule of Programme/Ad/Slate alone cannot disambiguate an intra-break
  `Ad -> Ad` from an `Ad -> Programme` return.

`transmux` already supports a section-carried SCTE-35 track:
`CodecConfig::Data { stream_type: 0x86, descriptors, carriage:
DataCarriage::Sections }`. Its TS muxer handles the PMT declaration and section
packetization for that track shape; `ts_hls.rs`'s own tests exercise it.

`media_plane` provides `Trunk` (bounded sample/segment/event/part rings with
cursor subscribers), `TrunkWriter`, `SampleCursor` (reporting `Lagged` in-band),
and the `Dialer`/`Listener` + `IngestSession` + `IngestDriver` ingest stack.

multimux provides `supervise_driver` (dial → establish → ingest, with capped
exponential backoff and permanent-failure classification) and
`source::advance_route` (publish announced programs + drive per-program
segmenters + drain DVR cursors).

## What is missing

Nothing switches between multiple ingest sources on a schedule, rebases their
timestamps onto a shared channel timeline, or injects SCTE-35 cues at the
resulting transitions. This design adds exactly that, as a new multimux input
scheme.

## Global constraints

- MSRV **1.95.0**; build and test with `--locked`.
- New public enums get `name()` + `broadcast_common::impl_spec_display!` (the
  #204 label convention), or an entry in the crate's `label_coverage` SKIP list.
- No magic numbers outside `#[cfg(test)]` — every hex literal is a named
  constant.
- Module docs cite the spec section for any wire layout they touch.
- `multimux` is `std` + tokio; `playout-runtime` stays `no_std` + `alloc` and is
  **not modified**.
- Version bumps: `multimux` 0.9.0 → 0.10.0 (additive: a new `InputSpec`
  variant on a `#[non_exhaustive]` enum). `playout-runtime` and `media-plane`
  are unchanged.
- Never log a source URL or credentials.

## Architecture

Approach: **playout is a new `InputSpec` variant with its own driver.** Each
source ingests into a private, unserved `Trunk`; a controller reads from the
active source's Trunk, rebases sample timestamps onto the channel timeline, and
writes to the output `Trunk` that egress serves.

```
Source A (RTSP)   ──► supervise_driver ──► Private Trunk A ─┐
Source B (TS-UDP) ──► supervise_driver ──► Private Trunk B ─┼─► Playout ──► Output Trunk ──► egress
Slate (file)      ──► FileReader ────────► Private Trunk C ─┘   Controller    (RouteHandle)   (LL-HLS/DASH/
                                                                                               LL-DASH/TS-HLS)
```

The controller is the only new moving part in the media path. It reuses
`supervise_driver` for every live source, `advance_route` on the output Trunk
for segmentation, and the existing egress unchanged.

### Why not compose `IngestSession`s directly

A composite `IngestSession` that switched between child sessions would avoid
the source-Trunk hop. It does not fit the trait: `IngestSession` is a sans-IO
`Stage` with one byte stream in (`feed(&[u8])`) and one event stream out
(`poll()`). A playout controller needs N independent byte streams from N
independent transports. Making it fit means either managing sockets inside the
session (breaking the workspace's sans-IO discipline) or teaching `IngestDriver`
about multiple dialers (a core `media-plane` change). The avoided cost is also
illusory: Trunk samples hold `Arc<[u8]>`, so moving a sample between Trunks
clones a pointer, not coded video. Inactive pre-connected sources need a buffer
regardless, and that buffer is a `Trunk`.

## Config

New `InputSpec::Playout` variant:

```json
{
  "name": "channel-1",
  "input": {
    "type": "playout",
    "sources": {
      "camera-1": { "type": "rtsp",   "url": "rtsp://10.0.0.1/stream" },
      "camera-2": { "type": "ts_udp", "addr": "239.1.1.1:5000" },
      "slate":    { "type": "file",   "path": "/media/slate.ts", "loop": true }
    },
    "schedule": [
      { "source": "camera-1", "kind": "programme", "offset_secs": 0 },
      { "source": "slate",    "kind": "slate",     "offset_secs": 3600 },
      { "source": "camera-2", "kind": "programme", "offset_secs": 3660 }
    ],
    "fallback_source": "slate",
    "splice_tolerance_secs": 6.0
  },
  "outputs": ["llhls", "dash"]
}
```

- `sources` — a named map of `InputSpec`s. Any existing variant, plus a new
  `InputSpec::File { path, loop }` (`loop` defaults to `true`).
- `schedule` — entries referencing sources by name. `offset_secs` is seconds
  from channel start, converted to 90 kHz ticks
  (`planned_start = round(offset_secs * 90_000)`). `kind` maps to
  `playout_runtime::EntryKind`.
- `fallback_source` — optional; the source the controller switches to when the
  scheduled source has no samples available.
- `splice_tolerance_secs` — optional; the `max_delta_ticks` passed to splice
  conditioning, defaulting to the route's `target_duration_secs`.

`source_start_pts` and `codec_config` are **not** configured — both are
discovered at runtime (see "Timeline correctness").

### Validation, at route-build time

All of these fail before serving starts, never at runtime:

- A `schedule` entry naming a source absent from `sources`.
- A `fallback_source` naming a source absent from `sources`.
- Non-strictly-increasing `offset_secs` (surfaced from `Schedule::push`'s own
  `Error::OutOfOrder`).
- An empty `schedule`, or an empty `sources` map.
- A negative or non-finite `offset_secs` / `splice_tolerance_secs`.

## Components

### `multimux/src/source/playout.rs` — the controller

**Startup.**

1. Build a `playout_runtime::Schedule` from the config entries, converting
   `offset_secs` to 90 kHz ticks. `Schedule::push` enforces ordering.
2. For each unique source: mint a private `Trunk` (minimal ring sizes — the
   controller reads continuously, so it needs buffering for its own read
   cadence, not egress window depth) and spawn its ingest. Live sources go
   through `supervise_driver` exactly as a normal route does; `File` sources go
   through `FileReader` (below).
3. Mint the output `Trunk`, register it in the channel's `RouteHandle`, and
   declare its track set — including the synthetic SCTE-35 track (below),
   declared at channel start so the PMT carries it before any cue exists.

**Steady-state loop.**

Each iteration:

1. Read available samples from the active source's `SampleCursor`.
2. For each sample, apply the active `TransitionPlan::rebase()` to its PTS and
   DTS, check monotonicity, and write it to the output `Trunk` via
   `TrunkWriter`.
3. Call `next_transition(&schedule, channel_now)`; if a transition is due,
   perform it (below).
4. Call `advance_route` on the output Trunk, driving segmentation and DVR
   exactly as every other input does.

**Performing a transition.**

1. Subscribe a fresh `SampleCursor` to the incoming source's private Trunk,
   positioned at the live edge.
2. Read the first available sample to learn the incoming source's actual PTS —
   this becomes `source_start_pts` for the entry.
3. Read the incoming source's announced tracks to learn its `CodecConfigId`
   (see "Codec-config identity").
4. Compute `TransitionPlan::plan(from, to)` with those runtime-discovered
   values.
5. If `plan.discontinuity`, call `TrunkWriter::set_tracks` on the output Trunk,
   bumping the track generation so the segmenter emits a fresh init segment and
   `#EXT-X-DISCONTINUITY`.
6. Derive the `BreakEdge` (below) and, when one is warranted, build and publish
   the cue.
7. Switch the active cursor and plan; continue the loop.

### `multimux/src/source/file_reader.rs` — file ingest

A standalone tokio task, not a `Dialer`/`IngestSession` (there is no connection
to dial and no reconnect semantics to honour).

- **Identify** — probe the file's leading bytes with `container-probe`
  ([sub-project 1](2026-08-11-container-probe-design.md), shipped) to decide
  which `transmux` demuxer to feed. A `Probe::Ambiguous`, `Insufficient`, or
  `Unknown` result fails the source at startup with the candidates named — the
  file is never fed to a guessed demuxer. A format `transmux` cannot demux
  (`Wav`/`Ogg`/`Asf`, or an elementary stream) fails the same way: identified,
  but explicitly unsupported as a playout source.

  Probe the **whole file**, not a prefix: `probe_with_budget(bytes, bytes.len())`.
  The default 64 KiB budget is enough to identify the format but not always to
  settle the ISOBMFF layout below, and a `FileReader` has the whole file in
  hand anyway.
- **Select the demuxer.** `Format` maps to a `transmux` demuxer, except that
  ISOBMFF splits in two — so read `Detail::Isobmff`'s `IsobmffLayout`:

  | Probe result | `transmux` demuxer |
  |---|---|
  | `MpegTs` | `StreamingTsDemux` |
  | `Isobmff` + `IsobmffLayout::Fragmented` | `Fmp4Demux` |
  | `Isobmff` + `IsobmffLayout::Progressive` | `ProgressiveDemux` |
  | `Isobmff` + `IsobmffLayout::Unknown` | fail the source — undetermined, never guessed |
  | `Matroska` / `WebM` | `WebmDemux` |
  | `MpegPs` | `PsDemux` |
  | `Flv` | `StreamingFlvDemux` |
  | `Mxf`, `Wav`, `Ogg`, `Asf`, `AdtsAac`, `Mp3`, `AnnexB` | unsupported as a playout source |

  Do **not** re-derive the layout by walking boxes here — the probe already did
  it, and two implementations of the same rule that can disagree is the bug
  class this workspace keeps finding.
- **Demux** — feed the file bytes to the demuxer selected above, yielding the
  `Media`/`Track`/`Sample` IR every other ingest path produces after its own
  demux.
- **Pace** — write samples to the private Trunk at their natural PTS cadence
  relative to a wall-clock start instant. Without pacing the whole file lands in
  the ring instantly and overflows it.
- **Announce tracks** — on the first sample, and again on any loop where the
  codec configuration differs, using the same `SessionEvent::NewProgram` shape
  every other ingest uses.
- **Loop** — at EOF with `loop: true`, restart from the file's beginning and
  advance the PTS offset so the looped content continues on the same monotonic
  timeline: the first sample of loop N+1 carries the last sample of loop N's PTS
  plus one frame duration. With `loop: false`, the reader stops and the source
  reports no further samples.
- **Supervision** — a read failure restarts the reader from the beginning of the
  file. This is deliberately simpler than `supervise_driver`'s backoff, which
  models a remote server that may be down; a local file that fails to read
  fails immediately and identically on retry, so the restart is bounded by a
  fixed retry interval rather than exponential backoff, and repeated failure
  degrades the source's health rather than retrying forever at increasing
  intervals.

## Timeline correctness

The primary invariant: **the output timeline is strictly monotonic and gapless
across every transition.** A schedule that plays the right thing at the wrong
timestamp is worse than no scheduler.

### Runtime-discovered `source_start_pts`

`TransitionPlan::plan` needs the incoming entry's `source_start_pts`. For a live
RTSP feed that value is whatever the camera happens to be sending, unknowable at
config time. The controller therefore populates it at switch time, from the
first sample it reads from the incoming source after the transition instant. The
plan is computed then, not at parse time.

### Codec-config identity

`CodecConfigId` is an opaque `u64` the caller computes. The controller derives
it as a stable hash of the source's announced track set: for each track, its
codec identity and codec-configuration bytes (the `avcC`/`hvcC`/`esds` payload
the demuxer already parsed), hashed in track order. Two sources whose tracks
carry identical configuration hash equal; any difference — a resolution change,
a profile change, a different track count — produces a different id and so
raises `discontinuity`.

### Monotonicity guard

After rebasing, every output PTS must be strictly greater than the last one
written to that track. A sample failing the check is **not written**: the
controller logs it and drops the sample, and increments a metric. Writing a
backwards timestamp corrupts every downstream segmenter, so dropping is the
lesser harm and the only honest option — silently writing it would produce a
stream that appears fine until a player stalls.

The guard fires on a source whose own clock jumped backwards (a camera
resetting its RTP timestamp base) and on a plan computed against stale state. It
is a safety net, not the mechanism: the rebase itself is what makes the timeline
continuous.

## SCTE-35 emission

### When a cue is warranted

Derived from the `EntryKind` pair either side of the join, and passed explicitly
to `build_splice_insert` as the `BreakEdge` that `playout-runtime` deliberately
does not guess:

| from | to | cue |
|---|---|---|
| Programme | Ad | `BreakEdge::Enter`, with `break_duration` = the ad entry's scheduled length |
| Ad | Programme | `BreakEdge::Return`, no duration |
| Ad | Ad | none — the break is already open |
| Programme | Programme | none |
| any | Slate | none — slate is filler, not a network break |
| Slate | any | none |

An ad entry's scheduled length is the next entry's `planned_start` minus its
own. The final schedule entry has no successor, so an ad in that position
carries no `break_duration` (`auto_return` cannot be honoured without one).

### Splice-point conditioning

`build_splice_insert` delegates to `ssai_runtime::splice::condition_splice_point`,
snapping the requested instant onto the nearest candidate within
`splice_tolerance_secs`. Candidates are the output Trunk's actual segment
boundaries around the transition instant.

On conditioning failure — no boundary within tolerance — the controller logs a
warning and performs the transition **without** a cue. The switch is a
scheduling commitment; the cue is signalling. Losing the cue degrades downstream
ad insertion; refusing the switch breaks the channel.

### Dual carriage from one serialized section

`to_section(insert).to_bytes()` produces the wire bytes **once**. Those same
bytes go to both:

1. **In-band, on a TS PID.** The output Trunk's track set includes a synthetic
   SCTE-35 track, declared at channel start:
   `CodecConfig::Data { stream_type: 0x86, descriptors:
   [registration_descriptor("CUEI")], carriage: DataCarriage::Sections }`. The
   serialized section is published as a sample on that track — no PTS, no DTS,
   `SampleFlags::SYNC`, matching what `TsDemux` emits for a section-carried
   track. `transmux`'s TS muxer already handles the PMT declaration
   (ISO/IEC 13818-1 Table 2-34 stream_type `0x86`, ANSI/SCTE 35) and the section
   packetization. This is what the `TsHls` output carries.
2. **In the manifest.** The same bytes are published as a Trunk event, which
   existing egress renders into `EXT-X-DATERANGE` (HLS, via `timed-metadata`)
   and `emsg` (DASH). This is what the `LlHls`/`Dash`/`LlDash` outputs carry.

Serializing once and fanning out makes divergence structurally impossible. Two
code paths each building "the same" cue is precisely the bug class this
workspace keeps finding.

## Failure modes

**A source has no samples at its scheduled slot** (down, still connecting, or a
non-looping file that ended). The transition still happens on time — the
schedule is a commitment. The controller switches to `fallback_source` if
configured, keeping its schedule position and retrying the intended source each
iteration. With no fallback configured, no samples are written until the source
recovers or the next entry is due. Route health reports degraded, never
healthy-but-frozen.

**Cursor lag.** `SampleCursor` reports `Lagged` in-band when a reader falls
behind the writer. Every inactive source would lag by construction, so the
controller holds **no cursor on an inactive source**: it subscribes a fresh
cursor at switch time, positioned at the Trunk's live edge, discarding whatever
accumulated while the source was inactive. That is the correct semantics for a
live source — joining a live feed means joining live, not replaying its backlog.

**Schedule exhausted.** Past the last entry, `next_transition` returns `None`
and the final entry plays indefinitely. An operator who wants a looping channel
repeats the schedule at later offsets or ends it with a looping slate.

**Config errors** fail at route-build time, before serving starts (see
"Validation").

## Testing

The bar is timeline integrity, not compilation.

**Unit — controller logic, no IO.**
- Transition detection fires at the right instant for a given schedule and clock
  position.
- `BreakEdge` derivation over every `EntryKind` pair, including every no-cue
  case in the table above.
- Ad-break duration derivation, including the final-entry case with no
  successor.
- The monotonicity guard rejects a backwards rebased PTS.
- Config validation rejects each invalid case listed above.
- Codec-config identity: identical track sets hash equal; a differing one does
  not.

**Integration — the timeline invariant (the primary gate).** Two scripted
sources (fake `IngestSession`s producing known PTS sequences, the pattern
`source/mod.rs`'s existing `FakeSession` tests already use) feeding a channel
across a transition. Assert the output Trunk's sample PTS sequence is:
- strictly monotonic across the join;
- gapless — the first post-transition sample lands exactly at the planned
  instant;
- rate-preserving — a source-clock delta of N ticks is an output-clock delta of
  exactly N ticks.

**Mutation proof, recorded in the test's doc comment:** writing `sample.pts`
unchanged instead of `plan.rebase(pts)` must turn this test red.

**Integration — SCTE-35 dual carriage.** A fixture channel with a
Programme → Ad → Programme schedule asserts:
- the TS output's PMT declares a PID with stream_type `0x86` and a CUEI
  registration descriptor;
- the section on that PID parses back through
  `scte35_splice::SpliceInfoSection::parse` to the expected `splice_event_id`,
  `out_of_network_indicator`, and `pts_time`;
- the HLS manifest's `EXT-X-DATERANGE` carries the same event;
- the bytes behind the last two are byte-identical.

**Integration — file source.** A committed, permissively-licensed real TS
fixture (from the existing corpus). Assert it demuxes, paces, and loops with a
seamless monotonic PTS continuation across the loop point.

**Failure modes.** Source-down engages the fallback with health degraded and
schedule position preserved; conditioning failure performs the transition with
no cue and a logged warning; a switch subscribes at the live edge with no
backlog replay.

**Gate suite** — the standard six, run against the real tree, not accepted on a
delegate's report:

```
cargo build   --workspace --all-features --locked
cargo test    --workspace --all-features --locked
cargo build   --workspace --no-default-features --locked
cargo clippy  --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
```

## Explicit non-goals

- **No transcoding or conforming.** A codec-config change across a transition is
  a discontinuity to signal, never something to re-encode. This workspace parses
  containers and never touches the codec bitstream.
- **No upstream cue passthrough.** A DVB/TS source already carrying SCTE-35 cues
  on its own PID has those cues ignored; the schedule drives transitions, not
  upstream signalling. A documented gap, not a silent one.
- **No runtime schedule updates.** The schedule is static config; changing it
  restarts the route. An admin API or file-watch reload is deliberately deferred.
- **No changes to `playout-runtime`.** Its sans-IO core is complete for this
  work; multimux adapts it, the way multimux adapts `rtsp-runtime` and
  `hls-runtime`.
- **No frame-accurate switching.** Transitions land on the nearest sample the
  incoming source has available at or after the transition instant, not on an
  exact frame boundary conditioned against both sources.
