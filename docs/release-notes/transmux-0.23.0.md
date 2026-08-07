# transmux 0.23.0

**Release date:** 2026-08-05

Adds MPTS (multi-programme transport stream) support to the IR and segmenter resumption helpers for mid-stream track additions. `TrackSpec::program_number` lets downstream consumers like `multimux` distinguish programmes in an MPTS, and the new `LlHlsSegmenter` builder methods allow resuming sequence numbering when a segmenter is rebuilt mid-stream.

## What's new

- `TrackSpec::program_number: Option<u16>` — the MPEG-2 TS `program_number` from the declaring PMT, populated by `StreamingTsDemux` at ES promotion time. `None` for non-TS sources (issue #906).
- `TrackSpec::with_program(u16)` builder method.
- `LlHlsSegmenter::next_sequence_numbers()` — returns the `(next_seq, current_segment)` pair for segmenter continuity across rebuilds (issue #781).
- `LlHlsSegmenter::with_part_target_at()` — builds a segmenter whose sequence numbers resume from given values.

## Migration

No breaking changes. Requires `broadcast-common` 9.2.
