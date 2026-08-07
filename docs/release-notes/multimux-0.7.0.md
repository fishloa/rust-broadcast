# multimux 0.7.0

**Release date:** 2026-08-05

Unlocks real DVB multiplex ingest with MPTS support, adds mid-stream track handling, Smooth Streaming output, and durable DVR segment archiving.

## What's new

- **MPTS ingest** (#906): `ProgramTracker` groups tracks by `TrackSpec::program_number`, producing one `ProgramId` per distinct TS programme. Every real DVB-T/S/C multiplex is MPTS — this was the sole gap preventing real DVB ingest.
- **Mid-stream track additions** (#781): a broadcaster adding an audio language or subtitle track mid-programme now reaches the running segmenter. Previously these were logged and silently dropped.
- **Smooth Streaming output** (`OutputKind::Smooth`, config `"smooth"`, #742): serves an MS-SSTR client Manifest and fragment responses, sharing the same fMP4 segment bytes from the `Trunk`.
- **DVR durable segment archive** (#746): per-route `dvr` config with period-container files, byte-range index sidecars, retention policies, and configurable overrun behaviour. Recording is a `SegmentEgress` that never holds a lock the live-serving path needs.

## Migration

Requires `multimux` 0.7 (pre-1.0 caret boundary `^0.6` -> `^0.7`). MPTS addressing for HTTP requests is documented but not yet implemented — all routes currently serve `SPTS_PROGRAM_ID` by default.
