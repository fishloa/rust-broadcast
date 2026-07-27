# media-plane

[![Crates.io](https://img.shields.io/crates/v/media-plane.svg)](https://crates.io/crates/media-plane)
[![docs.rs](https://img.shields.io/docsrs/media-plane)](https://docs.rs/media-plane)

The media-plane **integration layer**: the crate that ties ingress, the byte
layer, container demux ([`transmux`](https://crates.io/crates/transmux)), IR
transforms, the sample/segment/event `Trunk`, and the three egress shapes
together into one runnable pipeline, per
[`docs/superpowers/specs/2026-07-26-media-plane-architecture.md`](https://github.com/fishloa/rust-broadcast/blob/main/docs/superpowers/specs/2026-07-26-media-plane-architecture.md)
in the workspace:

```text
Dialer|Listener ──► [ByteStage]* ──► IngestSession ──► [IrTransform]* ──► TrunkWriter
   (N sources)       byte→byte          demux              IR→IR                │
                                                                                 ▼
                                                                  ┌──────── Trunk ────────┐
                                                                  │ sample ring           │
                                                                  │ segment log           │
                                                                  │ EVENT log (90 kHz)    │
                                                                  └───────────────────────┘
                         subscribe() ─► SampleCursor  ─► PushEgress    (WHEP, RTMP-out, SRT-out)
                         subscribe() ─► SegmentCursor ─► SegmentEgress (DVR, MABR, ROUTE, Smooth)
                         resolve()   ─────────────────► ServedEgress   (LL-HLS, DASH, catch-up)
```

## What's implemented (this release)

This crate is functionally complete end to end (plan steps 3a through 3e):

- **Byte layer** — [`ByteStage`] (the pre-demux byte-to-byte drive contract,
  a specialisation of `broadcast_common::Stage`), [`ByteTap`] (a non-blocking
  positional observer for conformance/analysis), [`ByteMerge`] (the one
  bounded multi-input primitive; `no_std` + `alloc`).
- **`Trunk`** — the bounded, dual-retention (`Timed`/`Sparse`) sample ring, the
  segment log (with pinning-cursor retention for lossless DVR/archive, never
  writer back-pressure by default), and the 90 kHz absolute event log
  (`EventCursor`/`EventAnchor`), plus reader wake.
- **`ingress`** — `Dialer`/`Listener`/`IngestSession` and `IngestDriver`
  /`ListenDriver`: the handshake-then-live pump that dispatches each session's
  reported programs/samples into a fresh `Trunk`.
- **`egress`** — `PushEgress` (WHEP/RTMP-out/SRT-out), `SegmentEgress`
  (DVR/MABR/ROUTE/Smooth), `ServedEgress` (LL-HLS/DASH/catch-up), with bounded
  `Await` negotiation.
- **`retention`** — `Retention::HotOnly`/`Tiered`, `RetentionDriver` draining a
  pinning segment cursor into a caller-supplied, sans-IO `SegmentSink`.

## `no_std` note — the byte layer only

**Only the byte layer (`byte_stage`/`byte_tap`/`byte_merge`) is `no_std` +
`alloc`.** `Trunk` and everything built on it (`ingress`, `egress`,
`retention`) require the `std` feature — `Trunk` needs `std::sync::Mutex`/
`Arc`/`Condvar` for cross-thread sharing. "The plane is `no_std`-capable" is
true of the byte layer, not the whole crate; `std` is a default feature, and
`cargo build --no-default-features` builds the byte layer alone.

## Quickstart

```rust
use media_plane::{ByteMerge, MergePolicy, SourceId};
use broadcast_common::stage::Timestamp;
use bytes::Bytes;

// The one bounded multi-input primitive in the byte layer: two UDP sources
// (e.g. a bonded/backup feed) reduced to one output stream.
let mut merge = ByteMerge::new(MergePolicy::FirstArrival, 2, /* max_queued */ 64);
merge.feed(SourceId(0), Bytes::from_static(b"ts-packet"), Timestamp::ZERO)?;
while let Some((msg, _at)) = merge.poll() {
    // hand `msg` to a container demuxer (e.g. `transmux::StreamingTsDemux`)
    let _ = msg;
}
# Ok::<(), media_plane::MergeError>(())
```

See [`examples/ingest_trunk_playback.rs`](examples/ingest_trunk_playback.rs)
for the full ingress → `Trunk` → `SampleCursor` pipeline (`std` feature)
driven against a real broadcast capture, and
[`examples/byte_tap_wire_observer.rs`](examples/byte_tap_wire_observer.rs) for
`ByteTap`'s non-blocking wire-observer contract on the same capture.

## Recorded deviations (not defects)

- [`MergePolicy`] deliberately has **no** `Hitless2022_7` variant yet — SMPTE
  ST 2022-7 seamless switching needs an RTP sequence-number parse this layer
  does not have; see the `byte_merge` module docs. Tracked as #752.
- Pull sources (HLS/DASH/Smooth) are request-driven, not stream-driven, and
  `IngestSession::poll_transmit` has no way to express "issue a GET for this
  URL" yet — a recorded seam, not solved here; see the `ingress` module
  docs' "Known seam" section.

## Features

| Feature | Default | Adds |
|---|---|---|
| `std` | **on** | `Trunk`, `ingress`, `egress`, `retention` (and their re-exports) — everything above the `no_std` byte layer. |

## MSRV

Rust **1.86**.

## License

MIT OR Apache-2.0
