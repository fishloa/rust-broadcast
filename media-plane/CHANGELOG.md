# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- New crate: `media-plane`, the media-plane integration layer (plan step 3a-i).
- `ByteStage`: the pre-demux byte-to-byte stage contract, defined as
  `Stage<In<'a> = &'a [u8], Out = Bytes>` rather than a second drive trait
  (per the 2026-07-27 revision of `docs/superpowers/specs/2026-07-26-media-plane-architecture.md`
  §1.1). Validated to compile at MSRV 1.86.
- `no_std` + `alloc` crate skeleton with a `std` feature, mirroring `transmux`.
- `ByteTap` (plan step 3a-ii): a positional, non-blocking observer of bytes
  in the byte layer, yielding `(Bytes, Timestamp)` exactly as received —
  including bytes a demuxer will reject (bad sync byte, TEI set, bad CRC,
  unaligned framing) — via a bounded ring. The producer (`record`) never
  blocks and never grows the ring past its configured capacity; a slow
  consumer instead observes an in-band `TapItem::Lagged { skipped }` from
  `poll`, which cannot be missed the way a side-channel counter could be.
  `TapPoint::{Wire, PostTransform}` is descriptive metadata only. Not a
  `Stage` — it is fed and polled by different callers, not driven as one
  contract.
- `ByteMerge` (plan step 3a-ii): the one bounded multi-input primitive in the
  byte layer — `N` byte sources reduced to one output stream of discrete
  messages (never an undelimited byte soup). `MergePolicy::FirstArrival`
  interleaves every source in arrival order; `MergePolicy::Failover` prefers
  a primary source, switches to a secondary after a configurable silence
  timeout (reset by any primary message, so a single late arrival does not
  cause a spurious switch), and switches back to primary the instant it is
  heard from again. `MergePolicy` is `#[non_exhaustive]`; ST 2022-7 hitless
  switching (`Hitless2022_7`) is deliberately **absent, not stubbed** — it
  needs RTP sequence-number semantics this layer does not have and lands
  with #752. Per-source state and the output queue are both bounded
  independently of call volume (`MergeError::QueueFull` rejects outright
  once the queue is at its cap, rather than growing or evicting silently).
- `Trunk`, `TrunkWriter`, `SampleCursor` (plan step 3b-i, `std`-only): the
  sample path of the `Trunk` — the bounded sample ring, its single writer,
  and the cursor(s) that read it. Two independent retention classes,
  `RetentionClass::Timed` (regular-cadence media, ordinary count-bounded
  eviction) and `RetentionClass::Sparse` (irregular, semantically-critical
  entries such as SCTE-35 cues), each with its **own** capacity so a flood of
  `Timed` publishes can never evict a still-live `Sparse` entry. A `Trunk`
  has exactly one `TrunkWriter` (`Trunk::writer()` returns `None` on every
  call after the first); `TrunkWriter::publish` never blocks or rejects — a
  full ring evicts its oldest entry rather than waiting on a slow reader.
  `Trunk::subscribe()` returns a `SampleCursor` starting from "now"; loss is
  reported in-band via `SampleCursorItem::Lagged` (ordinary, `Timed`) or
  `SampleCursorItem::Degraded` (escalated, `Sparse` — the consumer's derived
  state is now wrong, not merely gapped), following `ByteTap`'s `TapItem`
  precedent so loss can never be skipped past via a side channel. Adds a
  `transmux` (`Sample`) dependency, `std`-gated: `spikes/trunk-bench`
  (spec §3.1) showed writer cost is O(N) in cursor count, so `subscribe()`
  documents — at the call site, not only in the module docs — that a cursor
  is for one distinct consumer of the stream, never one per peer of a
  one-to-many protocol; supported reader count is single-digit by design.
  Payload fan-out is a `Bytes` refcount bump on the production path (no
  `.slice()`/copy anywhere in this module), verified by pointer-identity
  assertion, not content equality.
- `SegmentEntry`, `SegmentCursor`, `ArchiveOverrun` (plan step 3b-ii,
  `std`-only): the segment log — a bounded, append-ordered log of finished
  media segments in playlist order, recorded alongside the sample ring on
  the same `Trunk`/`TrunkWriter`/one-`Mutex` shape. `SegmentEntry` reuses
  `transmux::SegmentMeta` for the discontinuity bit (embedding the whole
  type rather than copying the field out, so a future `SegmentMeta` field is
  picked up automatically) and adds the three fields nothing in `transmux`
  computes: `sequence_number`, wall-clock `duration`, and the segment's
  `timeline_position` on the trunk's absolute clock.
  `Trunk::subscribe_segments()` gives an ordinary, lossy-on-overflow cursor
  (`SegmentCursorItem::Lagged`, exactly `RetentionClass::Timed`'s contract);
  `Trunk::pin_segments(on_overrun)` gives a **pinning** cursor for a
  DVR/archive consumer that must not miss a segment. Resolves the DVR
  contradiction (a recording must not have a hole, but the writer must never
  block) as **losslessness from retention, not back-pressure**: a pinning
  cursor holds every not-yet-consumed segment retained up to
  `TrunkConfig::segment_capacity` — the same bound ordinary eviction uses,
  not a second independent knob — and when that bound is finally hit, the
  caller's chosen `ArchiveOverrun` decides what gives: `ArchiveOverrun::Gap`
  (default) evicts and reports `SegmentCursorItem::Gap` (a hole in the
  recording, stream survives); `ArchiveOverrun::StallIngest` applies real
  back-pressure — the *one* place in this design a reader may block the
  writer, opt-in and never the default; `ArchiveOverrun::Terminate` drops
  the cursor (`SegmentCursorItem::Terminated`) instead of gapping or
  stalling. Segment bytes are shared, not copied, across cursors on the
  production path, on the same terms and with the same pointer-identity
  test discipline as the sample ring's payload sharing. `SegmentEgress`/
  tiered `Retention` (plan steps 3d/3e) are not built here;
  `Trunk::pin_segments` is their documented attachment point.
- `EventAnchor`, `EventEntry`, `EventCursor` (plan step 3b-iii, `std`-only):
  the 90 kHz event log, completing the `Trunk` per
  `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §1.2 and
  resolving the architecture audit's **blocking finding B1** — rev 1's false
  claim that every event fits one absolute `i64` in track timescale, which
  is untrue for SCTE-35 `splice_schedule.utc_splice_time` (GPS-epoch UTC,
  not a media timestamp) and `emsg` v0's `presentation_time_delta`
  (segment-relative, unresolvable until the segmenter owns a boundary). The
  event log's reference clock is **90 kHz absolute**
  (`timed_metadata::MediaTime`), not any one track's timescale — a `Media`
  with several tracks at several timescales has no single track clock an
  event log could borrow. It carries `timed_metadata::TimedEvent` (owned,
  lossless, `#[non_exhaustive]`, already published at 0.4.0) verbatim rather
  than a parallel type; `mp4_emsg::EmsgBox<'a>` is borrowed and cannot be
  stored, `timed_metadata::SourcePayload::Emsg` already is its owned form.
  **The B1 fix itself is `EventAnchor`**: every entry is `Media` (already on
  the trunk's clock), `Segment { segment_number, delta }` (an `emsg` v0,
  stays exactly this variant — addressable by segment number only — until
  `TrunkWriter::note_segment_start` reports *that* segment's own start, and
  resolves against it specifically, never against whichever segment happens
  to be open), or `Utc { utc_epoch_ms }` (a GPS/UTC-only `splice_schedule`
  cue, stays exactly this variant, with **no fabricated media time**, until
  `TrunkWriter::set_time_anchor` gives the log a `timed_metadata::TimeAnchor`
  to translate through). Addressable **both** by media time
  (`Trunk::events_between`, half-open `[from, to)`) and by segment
  (`Trunk::events_in_segment`, via a small boundary table bounded by the
  same `TrunkConfig::event_capacity` rather than a second knob — the same
  "no second capacity knob" precedent `TrunkConfig::segment_capacity`
  already set for pinning) — both queries only ever return `Media`-resolved
  entries. `Trunk::subscribe_events()` gives a streaming `EventCursor` on the
  same one-`Mutex`, single-digit-reader-by-design, in-band-`Lagged`,
  writer-never-blocks shape the sample/segment cursors already established.
  Reuses `timed_metadata::Timeline`'s 33-bit PTS wrap-unroll rather than
  hand-rolling it; `timed-metadata` becomes a real (non-dev) dependency of
  `media-plane`.
- `Dialer`, `Listener`, `IngestSession`, `run_dial`/`run_listen` (plan step
  3c, `std`-only, new `ingress` module): the ingress traits and the generic
  drivers that pump them into a `Trunk`, so no protocol reimplements its own
  feed/poll/dispatch loop (every one of `multimux`'s nine sources hand-rolls
  this today). `IngestSession` is a `Stage<In<'a> = &'a [u8], Out =
  SessionEvent>` specialisation plus a `poll_transmit` hook, implemented
  explicitly (**not** blanket like `ByteStage`, which adds nothing — a blanket
  impl would make `poll_transmit`'s default impossible to override, breaking
  the handshake mechanism it exists for; documented on the trait).
  - **Establishment is sans-IO, through the ordinary pump.** `Dialer::dial`
    performs **no I/O and completes no handshake** — it constructs a session
    in a not-yet-established state along with its first outbound request. The
    handshake then completes through the same feed/poll pump as everything
    else: `poll_transmit` out, `Stage::feed` in, until the session emits
    `SessionEvent::Established`. This mirrors `rtsp-runtime`'s existing
    driveable `ClientSession` (request builders return bytes to send,
    `handle_data` consumes replies and returns events, phase readable via
    `state()`) rather than inventing a second pattern, and keeps tokio out of
    the layer: an earlier revision had `dial()` "perform the whole
    connect/handshake", which only ever fit sources whose connect is purely
    local and would have forced an executor bridge for RTSP/SRT-caller/TS-UDP.
    A separate `PendingSession` type was considered and rejected (it would
    duplicate `feed`/`poll_transmit`/`next_deadline`/`on_deadline` for one bit
    of state; `rtsp-runtime` does not do it either) — the phase is visible as
    `HealthState::Establishing` vs `Live`.
  - **The handshake is bounded** by `HandshakePolicy::establish_by`, an
    absolute caller-supplied `Timestamp`; past it a still-establishing session
    terminates as `HealthState::HandshakeTimedOut { deadline }` and, in a
    `ListenDriver`, is **reaped**, freeing its `max_sessions` slot — so a flood
    of half-open connections cannot squat the bound. A deadline rather than an
    attempt/pump-iteration cap because the failure mode is wall-clock (matching
    `multimux`'s already-proven `IngestTimeouts::connect`) and because a real
    RTSP handshake is DESCRIBE + SETUP × N + PLAY with N unknowable up front,
    so any fixed iteration cap is either too small for a real multi-track SDP
    or too large to bound anything. `IngestDriver::next_deadline` surfaces it
    while establishing; per this crate's sans-IO rule there is no internal
    timer.
  - **B5 (the program dimension)**: `SessionEvent::NewProgram` may be
    announced at any point in a session's lifetime, not only at the start.
    `IngestDriver`/`ListenDriver` mint a fresh `Trunk` per `ProgramId` the
    instant it is announced — including a second program on an already-live
    connection — closing the gap `multimux::source::ts_udp::TsUdpSession`
    has today (a post-connect `DemuxEvent::TrackAdded` is only logged and
    dropped).
  - **EOF vs failure**: `HealthState<E>`
    (`Establishing`/`Live`/`Ended`/`Failed(E)`/`HandshakeTimedOut`) fixes the
    bug that made `ll_hls_runtime::server::store::HealthState::Failed`
    unproducible — `multimux::origin::supervisor::supervise` today folds a
    clean `run_pipeline` EOF and a real error into the same `Reconnecting`
    transition. `Stage::finish` returning `Ok(())` drives `Ended`;
    `feed`/`finish` returning `Err` drives `Failed`, carrying the concrete
    error rather than a formatted string. `HandshakeTimedOut` is deliberately
    *not* folded into `Failed`: a handshake that never progressed produced no
    session error to carry, so inventing an `E` would mean fabricating one.
  - **`max_sessions` is a hard bound, enforced by `ListenDriver`**, not by
    each `Listener` implementation: once `max_sessions` sessions are
    admitted, every further accepted connection is dropped immediately,
    before being fed a single byte — never queued, never buffered.
  - **Reconnect is bounded and caller-chosen**: `ReconnectPolicy` bounds how
    many times `DialSupervisor` retries `Dialer::dial`; it never sleeps or
    decides backoff duration itself (`DialAttempt::Exhausted` is an `O(1)`
    no-op once `max_attempts` is spent — a permanently-failing dialer cannot
    spin or grow).
  - **Known seam, recorded not solved**: `multimux`'s HLS/DASH/Smooth *pull*
    sources are request-driven (playlist reload timer + whole-object GETs), not
    stream-driven. `feed(&[u8])` is fine for them (`Stage` says nothing about
    chunk size), and `next_deadline`/`on_deadline` already *is* a reload timer;
    the real gap is that `poll_transmit() -> Option<Bytes>` cannot express
    "GET this URL". That wants a request-*addressing* type, not a second drive
    model or a pull-shaped sibling trait — deferred to Step 5, when the first
    real caller exists.
- `ServedEgress`, `PushEgress`, `SegmentEgress` (plan step 3d, `std`-only, new
  `egress` module): the three egress traits, kept separate rather than unified
  because they are structurally different operations — resolve a request
  (`ServedEgress`), stream existing samples (`PushEgress`), consume finished
  segments (`SegmentEgress`) — not three configurations of one interface (see
  the module docs for why a merge attempt forces an unreachable branch either
  way).
  - **`ServedEgress::resolve` takes no `&Trunk`**, correcting the architecture
    spec's pseudocode after reading `ll-hls-runtime/src/server/`: the segment
    log has only a moving `SegmentCursor`, no snapshot/random-access query the
    way the event log has `events_between`, so every real implementation would
    ignore a passed-in `&Trunk` and consult its own synced cache instead
    (exactly what `MediaStore` already does, fed by `add_segment`/`add_part`
    separately from its `resolve_*` read side). `resolve` is bounded instead by
    `AwaitPolicy`/`EgressResponse::Await`, mirroring
    `HandshakePolicy::establish_by`'s absolute-deadline shape; `Trunk` has no
    reader-side notify primitive today (only a writer-side `Condvar` for
    `ArchiveOverrun::StallIngest`), so an adapter must poll-with-backoff bounded
    by the same policy — recorded as a gap for Step 4 to decide on, not solved
    here.
  - **`EgressResponse::Await` is bounded by construction**: `AwaitPolicy::expired`
    and `EgressResponse::pending` are real, mutation-tested code (not a
    documented convention) that turns "not ready" into `Await` before a
    caller-chosen deadline and `NotFound` after it — a hostile client asking
    for a part that never arrives cannot park a request forever.
  - **`PushEgress`** owns exactly one `SampleCursor` (never one per peer,
    matching `Trunk::subscribe`'s O(N)-in-cursor-count finding) and `send`
    takes one `&SampleCursorItem` per call (including `Lagged`/`Degraded`),
    reusing the cursor's own item type rather than inventing the architecture
    spec's unimplemented `TrackBatch`. `poll_transmit` mirrors
    `IngestSession::poll_transmit`'s `None`-default exactly.
  - **`PushEgress::renegotiate`** is the mechanism issue #781 needs (a
    mid-stream track addition currently cannot reach a running segmenter):
    given the current `&[TrackSpec]`, it returns `NegotiationOutcome::Accepted`
    with the new selection, or `NegotiationOutcome::Refused { reason }` when
    the output cannot honour the change without breaking an already-established
    peer contract (an answered WHEP SDP offer, an already-published DASH
    `Period`) — a truthful, in-band refusal rather than a silent drop or a
    fabricated `Accepted`.
  - **`SegmentEgress::on_segment`** takes `&SegmentCursorItem` verbatim,
    reusing 3b-ii's pinning/`ArchiveOverrun` rather than reimplementing it; the
    architecture spec's sketched `on_manifest(&ManifestSnapshot)` is
    deliberately not included — no concrete DVR/MABR/ROUTE/Smooth producer
    exists yet to shape a single `ManifestSnapshot` type honestly.
- `Trunk` live-part log and reader-wake primitive (plan step 3b-iv,
  `std`-only): the two gaps step 3d found while reading
  `ll-hls-runtime/src/server/` before finishing the egress traits, closed so
  Step 4 can actually render blocking-reload decisions from `&Trunk` instead
  of keeping `MediaStore` as a parallel store.
  - **`PartEntry`/`TrunkWriter::publish_part`**: a fourth bounded ring
    (`TrunkConfig::part_capacity`) holding parts of the segment currently
    being written — RFC 8216bis §4.4.4.9's independently-fetchable CMAF
    chunks — addressed directly by `(segment_number, part_index)` via
    `Trunk::part_bytes`/`Trunk::parts_in_segment` (the live-part
    counterparts of `Trunk::events_between`/`events_in_segment`), not a
    moving cursor: a `ServedEgress` resolves *random* part requests, it does
    not stream every part ever produced. `Trunk::last_closed_segment` closes
    the companion gap (distinguishing "segment N is closed" from "segment N
    merely has live parts", RFC 8216bis's bare-`_HLS_msn` condition).
  - **Segment close does not touch the part log** — a deliberate choice,
    not an oversight: `TrunkWriter::publish_segment` never evicts or
    transforms that segment's parts, so a client requesting a just-rolled
    part gets exactly the same `Some(bytes)` answer as before the close,
    closing the exact preload-hint race `ll_hls_runtime::server::MediaStore`'s
    `recent_parts` buffer exists to patch — without `MediaStore`'s
    second, independently-tuned "recently closed" bound chained after the
    first. A part is reclaimed only by `part_capacity`'s ordinary
    evict-oldest bound, whether its segment is open or closed.
  - **`Trunk::listen`/`ProgressListener`**: a bounded reader-wake primitive
    wrapping `event_listener::EventListener` — the same runtime-agnostic
    primitive `ll_hls_runtime::server::MediaStore::listen` already returns,
    reused rather than hand-rolled, so a `ServedEgress` adapter ports
    mechanically (`.await` under any executor, or
    `ProgressListener::wait_deadline` with none). `publish_part`/
    `publish_segment` wake registered listeners via `Event::notify`, which
    never waits for a listener to actually resume — the writer-never-blocks
    invariant holds for the wake channel exactly as it does for every data
    ring in this module. `Trunk::listen` refuses (`None`) once
    `part_capacity` concurrent registrations are outstanding — reusing that
    bound rather than adding a sixth, independent one — deliberately sized
    for "one registration per distinct consumer" (mirroring
    `Trunk::subscribe`'s fan-out rule), not one per remote peer; an adapter
    serving many viewers takes one registration and fans the wake-up out to
    its own peers itself.
  - `TrunkConfig::new` now takes a fifth `part_capacity` argument
    (**breaking**, pre-1.0).
- **`TrunkConfig`'s five capacities are now `NonZeroUsize`, not `usize`**
  (**breaking**, pre-1.0 — both the `TrunkConfig::new` signature and the five
  public struct fields, so a zero cannot be reintroduced by field assignment
  after construction). `Trunk::new`'s five `assert!(… > 0)` panics are
  **removed**: a zero capacity is now unrepresentable rather than rejected at
  run time.
  - Fixes an inherited inconsistency with `transmux::ProgressiveDemux::new`,
    which was deliberately changed from panicking-on-zero to fallible
    construction — the opposite rule two crates over made the API feel
    arbitrary.
  - Fixes a real operational hazard, not just a style point: `multimux` takes
    its routes from a JSON config file, so once trunk capacities become
    operator-configurable (Step 5) a stray `0` would have panicked the server
    process. A `serde` deserialize of `0` into a `NonZeroUsize` field instead
    fails as an ordinary config/deserialization error at the parse boundary,
    with no hand-written check anywhere.
  - `NonZeroUsize` was chosen over a fallible `TrunkConfig::new -> Result`
    because `TrunkConfig` (unlike `ProgressiveDemux`) has no error type and
    does not otherwise return `Result`, so `Result` would mean inventing a
    construction error and threading `?`/`unwrap` through every call site to
    encode one bit the type system carries for free — and the invariant lives
    in the signature, where a reader sees it without reading the docs.
  - The five `zero_*_capacity_panics` tests are **deleted** rather than
    rewritten: they asserted a panic that can no longer occur, and a rewritten
    version would only be asserting that `NonZeroUsize::new(0)` returns `None`
    (a `core` property, not this crate's).
- `Retention`, `SegmentSink`, `RetentionDriver`, `SegmentLocation`, `SinkOutcome`
  (plan step 3e, `std`-only, new `retention` module): the hot/cold archive
  policy layered on top of the segment log — the last functional piece of the
  plane (step 3f is acceptance furniture only).
  - **`SegmentSink` is sans-IO**: `offer(&SegmentEntry) -> SinkOutcome` touches
    no filesystem, socket, or executor; a real disk/object-store adapter (Step
    5) does its actual I/O entirely behind that synchronous boundary.
  - **`Retention::Tiered { on_overrun, cold_window }` reuses `ArchiveOverrun`
    verbatim**, not a parallel enum: `RetentionDriver` is built directly on
    `Trunk::pin_segments(on_overrun)`, so the same three-way Gap/StallIngest/
    Terminate trade 3b-ii already proved governs this layer too. Only
    `cold_window` (a `Duration`, not a `NonZeroUsize` — it bounds an eviction
    deadline, not a ring's structural capacity) is genuinely new state; the
    plan sketch's `hot`/`cold` fields are `ArchiveOverrun` (reused) and
    `SegmentSink` (supplied separately, at `RetentionDriver::new`, since a
    sink is a caller-supplied handle, not `Copy` policy data) respectively.
  - **A failing or slow sink cannot stall ingest**: `TrunkWriter::publish_segment`
    has no reference to `SegmentSink` at all — a structural guarantee, not a
    race defended against — and `RetentionDriver::drive` holds at most one
    `SegmentEntry` awaiting hand-off (`RetentionDriver::pending_len` is always
    `0` or `1`), never polling its pinning cursor for the next segment while
    one is stuck on `SinkOutcome::Busy`. No second bounded queue was added for
    this: everything still queued behind a stuck hand-off stays inside the
    trunk's segment log, already bounded by `segment_capacity` and already
    governed by `on_overrun`.
  - **Cold segments stay addressable — "cold, ask the sink"** (issue #746,
    DVR/catch-up): `RetentionDriver::locate(sequence_number, now)` resolves to
    `SegmentLocation::Hot` (still, or not yet, on the trunk's ordinary hot-ring
    path), `SegmentLocation::Cold` (handed off and still inside `cold_window`
    — this crate does not hold the bytes; the caller resolves them through its
    own sink), or `SegmentLocation::Evicted` (gapped before hand-off, or aged
    out of the cold window). Reuses `Trunk::last_closed_segment` for "has this
    sequence number even been produced yet" rather than tracking a second,
    parallel high-water mark.
  - The cold-tier ledger is bounded by `cold_window` itself (purged on every
    `drive`/`locate` call), the temporal analogue of `segment_capacity`'s
    count-based bound on the hot ring — not a second, independently-tuned
    count knob.
- Step 3f: `docs/CRATE-ACCEPTANCE.md` furniture — no behaviour change.
  - Three fuzz targets in the shared `fuzz/` crate: `media_plane_byte_merge`
    (arbitrary `feed`/`poll`/`on_deadline` sequences under both
    `MergePolicy` variants, asserting the output queue bound holds),
    `media_plane_byte_tap` (arbitrary `record`/`poll` interleavings,
    asserting the ring bound holds and every recorded item is accounted for),
    and `media_plane_ingest_driver` (a fuzzer-scripted `IngestSession`
    driving `IngestDriver`'s program/track dispatch). Each ran >=1M
    executions with zero crashes.
  - `tests/label_coverage.rs` (issue #204 drift-guard): every current public
    enum in this crate is either a `thiserror` error or a data-carrying ADT,
    not a decoded external-spec token, so the skip-list is comprehensive and
    documented per-enum; `CachePolicy` (already labelled) is the one
    exception.
  - Two real-fixture examples (`examples/ingest_trunk_playback.rs`,
    `examples/byte_tap_wire_observer.rs`), both reading the shared
    `fixtures/ts/h264_aac.ts` real broadcast capture via `std::fs`, not
    `include_bytes!` or synthetic bytes.
  - `.github/workflows/release-media-plane.yml`: this crate's own release
    lane, modelled on `release-transmux.yml`.
  - `[package.metadata.docs.rs]` now sets `rustdoc-args = ["--cfg",
    "docsrs"]`, and `src/lib.rs`/`#[cfg(feature = "std")]` items gained
    `doc(cfg(...))` pills, so docs.rs shows which API needs `std` instead of
    silently building only the byte layer's docs.
  - Crate-root docs and README corrected to reflect that this crate is now
    functionally complete through step 3e (previously stale text still said
    ingress/egress/retention were "later steps... deliberately absent"), and
    to state plainly that only the byte layer is `no_std` + `alloc` — `Trunk`
    and everything built on it require `std`.
### Fixed
- **`max_programs`: bound the number of `Trunk`s `IngestDriver`/`ListenDriver`
  will mint per session** — the fifth unbounded-allocation vector shipped
  from this codebase. `SessionEvent::NewProgram` previously minted a fresh
  `Trunk` (five bounded rings) for every distinct `ProgramId` a session
  reported, with no cap on how many distinct ids one session could report;
  every ring was bounded, but the *count of `Trunk`s* was not, so a malformed
  or hostile multiplex announcing thousands of programs allocated thousands
  of trunks. `IngestDriver::new`/`ListenDriver::new`/`run_dial`/`run_listen`/
  `DialSupervisor::try_dial` now take a `max_programs: NonZeroUsize`,
  enforced in exactly one place (`IngestDriver`'s internal `drain()`),
  mirroring where `max_sessions` is already enforced — structural, not a
  per-`IngestSession`-implementor discipline. The `(max_programs + 1)`th
  program is **refused, not fatal**: it gets no `Trunk` (no `Trunk::new` call
  happens for it at all) and its later `Sample`s are dropped via the
  already-existing "sample for an unannounced program" path, while every
  already-admitted program keeps flowing — refusing was chosen over failing
  the whole session so a 200-program hostile/malformed multiplex cannot take
  down ingest for the programs a real caller asked for. The refusal is
  **reported**, via a new monotonic `IngestDriver::refused_program_count()` /
  `ListenDriver::refused_program_count(id)` counter (never a stored list of
  refused `ProgramId`s, which would just move the same unbounded-growth shape
  from `Trunk`s to a `Vec`) — never a silent drop. `DEFAULT_MAX_PROGRAMS`
  (`64`) is provided as a documented, justified default (real DVB MPTS run
  single-digit to low-tens of programs; ATSC/cable can run higher) rather
  than requiring every caller to invent their own bare literal.
