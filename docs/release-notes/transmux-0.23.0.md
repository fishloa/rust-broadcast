# transmux 0.23.0

**Release date:** 2026-08-05

Adds `TrackSpec::program_number` for MPTS demux (a single TS source carrying multiple programs can now route tracks to the correct `Trunk`), plus segmenter sequence helpers (`SegmentSequence::next`, `PartSequence::next`) that `hls-runtime` and `multimux` use to derive `#EXT-X-MEDIA-SEQUENCE` and `#EXT-X-PART` indices without manual bookkeeping.

## What's new

- `TrackSpec::program_number` — associates each track with its MPEG-2 program_number (for MPTS fan-out).
- `SegmentSequence::next` / `PartSequence::next` — monotonic sequence generators for HLS segmenters.

## What changed

- Requires `broadcast-hls` 0.1.

## Migration

`TrackSpec` has a new field (`program_number: Option<u16>`). Existing constructors that use struct-literal syntax need to add it (default `None` for single-program sources).
