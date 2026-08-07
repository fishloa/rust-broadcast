# hls-runtime 0.2.0

Released 2026-07-28.

### Added

- Sans-IO **LL-HLS origin engine** (`server` module, feature `std`): `HlsOrigin`
  resolving init/segment/part bytes and rendering playlists over a
  `media_plane::Trunk`'s rings, with blocking-reload/part-availability logic.
- **DVR archive** support via pinned segments.
- Renamed from `ll-hls-client` to `hls-runtime` (client + server).
