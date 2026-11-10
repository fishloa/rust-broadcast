# broadcast-hls 0.7.0

_Released 2026-11-10._

Breaking-class release (0.6 -> 0.7): `MasterPlaylist::to_m3u8` no longer fails on dangling group references.

Version: 0.6.0 -> 0.7.0.

## Breaking changes and migration

- Behaviour: `MasterPlaylist::to_m3u8` no longer fails with "group not found" for an `#EXT-X-STREAM-INF` whose group has no `#EXT-X-MEDIA`; parse keeps the reference and render emits it unchanged. Callers relying on `to_m3u8` as a validator must now call `dangling_group_references()`.

## Added

- `MasterPlaylist::dangling_group_references()` lists groups a variant names that no `#EXT-X-MEDIA` declares.
- `OpenSegment::discontinuous`: an `#EXT-X-DISCONTINUITY` ahead of live-edge parts kept on parse/render.

## Fixed

- `MediaPlaylist::parse` no longer drops `#EXT-X-DISCONTINUITY` preceding open segment's parts.

---

Published from tag `broadcast-hls-v0.7.0`.
