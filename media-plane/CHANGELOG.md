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
