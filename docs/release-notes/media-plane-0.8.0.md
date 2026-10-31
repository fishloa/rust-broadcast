# media-plane 0.8.0

_Released 2026-10-31._

### Changed
- Dependency `parking_lot` 0.12 -> 0.13.

### Added
- `Trunk::segment_start_time(&self, seq) -> Option<MediaTime>` — reported start of a segment on the 90 kHz source clock (issue #1423 B1); `None` when never reported or evicted from the bounded table.

---

Published from tag `media-plane-v0.8.0`.
