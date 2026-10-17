# transmux 0.27.1

_Released 2026-10-17._

**Patch release.** A downstream user (zenith-tv) found that on `transmux` 0.27.0 `TsDemux::unpackage` silently dropped TS packets with a broken sync byte and left `Media::degraded` empty, so a damaged capture passed the `degraded.is_empty()` validity gate. The loss of samples itself was always legitimate (the packets carried payload; 0.26.0 behaves identically and had no `degraded` field at all) but it was not reported. 0.27.1 reports it.

## Why this version

- **Proposed version:** 0.27.0 -> **0.27.1** (patch: the gap is a reporting bug; the new `InputDegradation` variants are additive on a `#[non_exhaustive]` enum).
- **Requires `mpeg-ts` 0.6.1** (the resynchroniser now reports losses); the manifest requirement is `mpeg-ts = "0.6.1"`.

## Behaviour change to know about

`Media::degraded` is now filled for every kind of TS damage the streaming core detects, not only the two new sync-loss variants: continuity gaps and transport-error packets that used to be dropped on the floor now appear. Real lossy captures (satellite recordings with dropped packets) therefore report entries where 0.27.0 returned an empty list. If you gate on `degraded.is_empty()`, clean captures stay empty and damaged ones are now rejected, which is the purpose of the field. The continuity-gap counts on some captures are high (hundreds to thousands on multi-minute lossy captures); that is the existing continuity check, not changed here.

Callers that match `InputDegradation` exhaustively must already have a wildcard arm (the enum is `#[non_exhaustive]`).

## Migration

```rust
let media = TsDemux::new().unpackage(&bytes)?;
if !media.degraded.is_empty() {
    for d in &media.degraded {
        match d {
            InputDegradation::SyncLoss { offset, dropped_bytes } => { /* bytes discarded at offset */ }
            InputDegradation::TruncatedPacket { offset, dropped_bytes } => { /* input ended mid-packet */ }
            _ => {}
        }
    }
}
```

`multimux`, `hls-runtime`, `ts_hls`/`ll_hls` and the media-plane ingest receive the new `InputDegraded` events but do not yet surface them anywhere.

## What changed

### Fixed (a damaged TS no longer passes the `Media::degraded.is_empty()` gate)
- **`TsDemux` reported nothing for a damaged transport stream.** Zeroing the sync byte of every 7th packet of a real 720p50 H.264 + AC-3 + 3x MP2 capture (`orf1-dvbs2-raw-zenith-tv.ts`, packets 56392..56792, 58 packets) lost samples (h264 1471 -> 1468, ac3 916 -> 912, mp2 1224 -> 1224 / 1216 / 1220) while `Media::degraded` stayed empty. Two causes, both present in 0.26.0 and 0.27.0 (0.26.0 had no `degraded` field at all): (1) `mpeg_ts::resync::TsResync` silently skipped the 188 bytes of a packet with a broken sync byte (only an internal counter moved), and (2) the batch `TsDemux::demux` threw away *every* `DemuxEvent::InputDegraded` the streaming core had raised (transport_error_indicator, continuity gaps, rejected parameter sets), so `Media::degraded` could never be non-empty for a TS.
- `Media::degraded` now carries every `InputDegraded` the streaming core raises, in input order, plus the two new variants below. Behaviour change: captures with real continuity gaps (`m6-*.ts`, the `france-tnt-*` and `hotbird-*` excerpts) now report them; they used to report an empty list only because the events were dropped.
- `StreamingTsDemux` emits the same `InputDegraded` events for sync loss at any `feed` chunking, with `provenance.packet_index` = the index of the next intact packet and `pid: None`.

### Added
- `InputDegradation::SyncLoss { offset, dropped_bytes }`: bytes discarded between the loss of sync (bad sync byte, garbage between packets, or a tail that never re-locks) and the resynchroniser re-locking; one entry per span. Junk before the first lock (a capture that starts mid-packet) is not degradation.
- `InputDegradation::TruncatedPacket { offset, dropped_bytes }`: the input ended inside a packet.
- Not breaking: `InputDegradation` is `#[non_exhaustive]`; both variants carry `name()` and `Display`.
- Where there is no reporting channel: `multimux`, `hls-runtime`, `ts_hls`/`ll_hls` and the media-plane ingest ignore `DemuxEvent::InputDegraded` (they have no counter for it); the event is now emitted for them to consume, none of them surfaces it yet.

---

Published from tag `transmux-v0.27.1`.
