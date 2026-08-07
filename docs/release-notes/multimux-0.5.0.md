# multimux 0.5.0

Released 2026-07-28.

### Added

- **Media-plane integration**: rebuilt on `media_plane::Trunk` instead of the
  rolling-window `MediaStore`. Single copy of data, cursor-based egress.
- **LL-HLS origin engine** via `hls-runtime` server.
- **Smooth Streaming output** (#742).
- **DVR archive** (#746).
- **SRT ingest** (#739) — Listener and Caller mode.
