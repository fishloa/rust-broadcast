# broadcast-hls 0.3.1

_Released 2026-10-13._

### Fixed

- `MediaPlaylist::to_m3u8` now rejects an `EXT-X-SKIP` `RECENTLY-REMOVED-DATERANGES` id containing `"`, CR, LF or the tab separator (tag-line injection) instead of rendering it.

---

Published from tag `broadcast-hls-v0.3.1`.
