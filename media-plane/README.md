# media-plane

[![Crates.io](https://img.shields.io/crates/v/media-plane.svg)](https://crates.io/crates/media-plane)
[![docs.rs](https://img.shields.io/docsrs/media-plane)](https://docs.rs/media-plane)

The ingress and egress spine of a live media origin: sources are dialled or listened for, bytes pass through stages into a container demuxer, and the resulting samples, segments and timed events land in a per-program `Trunk` that any number of egress shapes read from.

## What it does / what it does not

**Does:** define and implement the four-layer shape that every protocol adapter plugs into, instead of each protocol gluing its own pipeline.

```text
Dialer|Listener ──► [ByteStage]* ──► IngestSession ──► [IrTransform]* ──► TrunkWriter
   (N sources)       byte→byte          demux              IR→IR                │
                                                                                 ▼
                                                                  ┌──────── Trunk ────────┐
                                                                  │ sample ring           │
                                                                  │ segment + part log    │
                                                                  │ EVENT log (90 kHz)    │
                                                                  └───────────────────────┘
                         subscribe() ─► SampleCursor  ─► PushEgress    (defined shape, no in-tree impl)
                         subscribe() ─► SegmentCursor ─► SegmentEgress (DVR, MABR, ROUTE, Smooth)
                         resolve()   ─────────────────► ServedEgress   (LL-HLS, DASH, catch-up)
```

A `Trunk` is bounded everywhere: sample, segment, part and event rings evict their oldest entry rather than block the writer, and a slow reader is told so in-band (`Lagged`), never silently skipped. Writer cost is linear in the number of *cursors*, so a cursor is per distinct consumer, not per peer (single-digit reader counts by design).

**Does not:** open sockets or speak any wire protocol (`rtsp-runtime`, `rtmp-runtime`, `srt-runtime`, `multimux` and others supply `IngestSession` implementors), mux or demux containers (that is `transmux`), implement ST 2022-7 hitless merge (`MergePolicy` has no such variant, tracked as #752), provide a disk or object-store archive (the caller supplies a `SegmentSink`), or support pull-style sources (HLS/DASH/Smooth) in the ingest session yet (a recorded seam: `IngestSession::poll_transmit` cannot express "issue a GET").

## Standards supported

media-plane implements an architecture, not a wire standard, so there is no spec edition to cite. The standards it touches:

| Standard | Edition | Scope here | Status | Notes |
|---|---|---|---|---|
| ISO/IEC 13818-1 (MPEG-2 Systems) | n/a | 90 kHz timestamp clock for the event log; 33-bit PTS wrap-unroll via `timed-metadata` | Time base only | Container parsing lives in `transmux` |
| ANSI/SCTE 35, DASH `emsg`, HLS `EXT-X-DATERANGE` | n/a | Timed events stored as `timed_metadata::TimedEvent`, anchored by media time or segment, never given a fabricated time | Typed (via `timed-metadata`) | |
| SMPTE ST 2022-7 | 2019 (vendored) | Not implemented | Not implemented | The `byte_merge` module documents why it is absent rather than stubbed |

## Features

| Feature | Default | Description |
|---|---|---|
| `std` | yes | `Trunk`, `ingress`, `egress`, `retention` and their re-exports, plus the `transmux`, `timed-metadata`, `event-listener` and `parking_lot` dependencies. Without it only the byte layer exists. |
| `test-support` | no | Implies `std` and compiles `ts_session::test_support`: real `TsMux`-muxed TS bytes, `TrunkConfig` and `HandshakePolicy` builders for a host crate's tests of a TS transport. Meant for `[dev-dependencies]`, not for production builds. |

**`no_std` note:** only the byte layer (`byte_stage`, `byte_tap`, `byte_merge`, `ids`) is `no_std` + `alloc`. `Trunk` and everything above it need `std`. `cargo build --no-default-features` builds the byte layer alone.

## Quick start

Demux a real broadcast capture with `transmux`, publish every sample into a `Trunk`, and read them back through a `SampleCursor`:

```rust
use std::num::NonZeroUsize;

use media_plane::{RetentionClass, SampleCursorItem, Trunk, TrunkConfig};
use transmux::{DemuxEvent, StreamingTsDemux};

let nz = |n| NonZeroUsize::new(n).unwrap();
let trunk = Trunk::new(TrunkConfig::default()
    .with_timed_capacity(nz(4096))
    .with_sparse_capacity(nz(64))
    .with_segment_capacity(nz(32))
    .with_event_capacity(nz(32))
    .with_part_capacity(nz(16)));
let writer = trunk.writer().expect("the first writer is always granted");
let mut cursor = trunk.subscribe(); // live tail: reads what is published from now on

let ts = std::fs::read("../fixtures/ts/h264-aac.ts").unwrap();
let mut demux = StreamingTsDemux::new();
for chunk in ts.chunks(188 * 64) {
    demux.feed(chunk);
}
demux.finish();
while let Some(event) = demux.poll_event() {
    if let DemuxEvent::Sample { track_id, sample, .. } = event {
        writer.publish(track_id, RetentionClass::Timed, sample);
    }
}

let mut timed = 0;
while let Some(item) = cursor.poll() {
    match item {
        SampleCursorItem::Timed { .. } => timed += 1,
        SampleCursorItem::Lagged { skipped } => panic!("ring too small, lost {skipped}"),
        _ => {}
    }
}
assert!(timed > 0, "real H.264 and AAC samples came out of the trunk");
```

The byte layer's multi-input primitive, with two UDP-style sources reduced to one stream:

```rust
use broadcast_common::stage::Timestamp;
use bytes::Bytes;
use media_plane::{ByteMerge, MergePolicy, SourceId};

let mut merge = ByteMerge::new(MergePolicy::FirstArrival, core::num::NonZeroUsize::new(2).unwrap(), /* max_queued */ core::num::NonZeroUsize::new(64).unwrap()).unwrap();
merge.feed(SourceId(0), Bytes::from_static(b"ts-packet"), Timestamp::ZERO).unwrap();
let (msg, _at) = merge.poll().expect("one message queued");
assert_eq!(&msg[..], b"ts-packet");
```

The full ingress to `Trunk` to cursor pipeline, driven by `IngestDriver` against the same capture, is `examples/ingest_trunk_playback.rs`; `examples/byte_tap_wire_observer.rs` shows `ByteTap`'s non-blocking observer contract.

## What is implemented

- **Byte layer:** `ByteStage` (pre-demux byte-to-byte work, a specialisation of `broadcast_common::Stage`), `ByteTap` (non-blocking positional observer for conformance and analysis, with `Lagged`), `ByteMerge` (the single bounded N-to-1 primitive, with `MergePolicy::FirstArrival` and `Failover`; messages, not a byte soup).
- **`Trunk`:** dual-retention sample ring (`Timed`, `Sparse`), segment log with pinning cursors for lossless DVR (`pin_segments`, `ArchiveOverrun` policy instead of writer back-pressure), live part log, 90 kHz event log (`EventCursor`, `EventAnchor`, `events_between`, `events_in_segment`), reader wake through `listen()`, single-writer enforcement with handover across reconnects, `time_anchor()`, `open_segment` (the lowest segment above the last closed one that has resident parts).
- **Typed ids (`ids`):** `SegmentSeq` and `PartIndex` newtypes with checked arithmetic (`checked_next`, `checked_add`, `checked_sub`, `checked_distance_from`), explicit `wrapping_next` and `saturating_next`, and no overflowing operators. Tracks are `transmux::TrackId`. A swapped segment/part argument is a compile error.
- **`ingress`:** `Dialer`, `Listener`, `IngestSession`, `IngestDriver`, `ListenDriver`, `DialSupervisor`, `ReconnectPolicy`, `HandshakePolicy`, `HealthState`, with a `max_programs` bound on how many trunks one session may mint.
- **`ingress` drivers:** `IngestDriver::new(session, IngestConfig)` / `ListenDriver::new(listener, IngestConfig)`, and `AdmittedDriver` for concurrently admitting push ingest (one plain session per admitted peer, per-session clocks, a live admitted-session counter). `TrunkConfig::default()` plus `with_*` setters sizes every ring.
- **`egress`:** `ServedEgress` (request/response with bounded `Await`; production implementors in `hls-runtime` and `multimux`), `SegmentEgress` (production implementors in `multimux`), `PushEgress` (a defined shape; its only impls are test doubles, `multimux`'s push path drives its own transport off a sample cursor).
- **`retention`:** `Retention::HotOnly` and `Tiered`, `RetentionDriver` draining a pinning segment cursor into a caller-supplied sans-IO `SegmentSink`, and `locate` for catch-up against an evicted segment.
- **`ts_session` (feature `std`):** `TsIngestSession`, the shared MPEG-2 TS ingest session every TS-carrying source of a host feeds (program tracking, held-sample and silent-PID bounds).
- **`file_source` (feature `std`):** the sans-IO file playout core: `select_demuxer`/`probe_and_demux`, `ParsedFile`, `PublishItem`, `FileIngestSession` (paced, looping replay with monotonic offsets).
- **`segmenter` (feature `std`):** `program::ProgramSegmenter` (cuts a `Trunk`'s samples into parts and segments for fMP4 or TS, `SegmenterConfig`, `SegmenterEvent`, `TsHlsIdrFallback`), `audio::AudioFollower` (split-mux audio renditions) and `subtitle::SubtitleFollower` (WebVTT renditions from DVB teletext, `teletext_pages`).
- **`publish_queue` (feature `std`):** `PendingPublish`, `drain_publish_queue`, `part_entry`: the non-blocking queue a segmenter publishes to a `Trunk` through.

## Design notes

Sans-IO and bounded: no thread or socket is created here, every queue has a cap, and over-capacity is an explicit event (`Lagged`, `Degraded`, `Discontinuity`), never silent loss. The event log never fabricates a media time for a segment-relative (`emsg` v0) or wall-clock-only (SCTE 35 `splice_schedule`) event until the anchor it needs exists. `#[non_exhaustive]` on policies and errors; a lexical no-hand-roll guard and label-coverage tests apply.

## Related crates

Depends on `broadcast-common` (`Stage`, `Timestamp`, `Demand`), `transmux` (the neutral IR and `TrackId`) and `timed-metadata` (event model). Consumed by `hls-runtime` (`HlsOrigin` as a `ServedEgress`) and `multimux` (origins and the DVR recorder), and by `compliance-probe` (a `Trunk`/`ByteTap` consumer). The design document is `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` in the workspace.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
