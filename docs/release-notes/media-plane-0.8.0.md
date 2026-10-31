# media-plane 0.8.0

_Released 2026-10-31._

### Changed (BREAKING: epoch)
- Builds against new caret epochs of workspace siblings (`transmux` 0.27 -> 0.28, `timed-metadata` 0.7 -> 0.8). A caret-epoch change is major-class for a 0.x crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` section 2), whatever its own API diff, so this release is a minor bump; consumers must resolve a single copy of each of those crates.
### Changed
- Dependency `parking_lot` 0.12 -> 0.13.

### Added
- `Trunk::segment_start_time(&self, seq) -> Option<MediaTime>` — reported start of a segment on the 90 kHz source clock (issue #1423 B1); `None` when never reported or evicted from the bounded table.

---

Published from tag `media-plane-v0.8.0`.
