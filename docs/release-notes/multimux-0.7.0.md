# multimux 0.7.0

**Release date:** 2026-08-05

Adds MPTS (multi-program transport stream) input demux, mid-stream track-set changes (#781), Microsoft Smooth Streaming output, and DVR archive with configurable retention and `ArchiveOverrun` policy. Requires `transmux` 0.23 and `media-plane` 0.3.

## What's new

- **MPTS input** — a single TS-UDP/TS-HTTP/SRT source carrying multiple programs is demuxed into separate `Trunk` instances, one per `program_number`. Each program gets its own output routes.
- **Mid-stream track changes** — when a source adds/removes tracks (e.g. ad-break audio-language switch), the `Trunk` track set updates live and downstream segmenters pick up the change via `SessionEvent::TracksChanged`.
- **Smooth Streaming output** (`OutputScheme::Smooth`) — IIS-compatible fragmented-MP4 manifest + fragments served from `Trunk` segments.
- **DVR archive** — tiered hot/cold retention with `SegmentSink` archive backend and `ArchiveOverrun` policy (`Drop` oldest vs. `PauseIngest`). Configured per-route via `RetentionConfig`.

## What changed

- Requires `transmux` 0.23 (`TrackSpec::program_number`), `media-plane` 0.3 (`set_tracks` wakes listeners).

## Migration

Requires `transmux` 0.23 and `media-plane` 0.3. No breaking API changes to existing routes — MPTS, Smooth output, and DVR are additive features.
