# multimux 0.7.0

Released 2026-08-05.

### Added

- **MPTS (multi-programme transport stream) ingest** (#906): `ProgramTracker`
  groups tracks by `program_number`, producing one `ProgramId` per distinct TS
  programme.
- **Mid-stream track additions** (#781): PMT version changes adding an
  elementary stream now reach the running segmenter.
- **Smooth Streaming output** (#742).
- **DVR archive** (#746).
