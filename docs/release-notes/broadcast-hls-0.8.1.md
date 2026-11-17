# broadcast-hls 0.8.1

_Released 2026-11-17._

Patch release adding parser limits from the audit 20 bundle: a `Limits` value caps playlist bytes, segment count and segment body bytes, and `parse_with` takes it while `parse` keeps the defaults. `Error::InvalidTagLine` also rejects a verbatim `extra_tags`/`pre_tags` line that is empty or does not start with `#EXT`. Both additions leave the existing API in place.

## Added

- `limits` module: `Limits` with `MAX_PLAYLIST_BYTES` (4 MiB), `MAX_SEGMENTS` (1 000 000, a backstop behind the byte cap) and `MAX_SEGMENT_BYTES` (64 MiB) defaults; `MediaPlaylist::parse_with` / `MasterPlaylist::parse_with` take them, `parse` delegates to the defaults, and `Error::TooLarge` reports an over-limit document or segment count (audit 20 finding 41, #2852).
- `Error::InvalidTagLine`: a verbatim `extra_tags`/`pre_tags` line must be non-empty and start with `#EXT`, so it can no longer render as a URI line or a bare terminator (audit 20 finding 51, #2862).

---

Published from tag `broadcast-hls-v0.8.1`.
