# media-doctor 0.12.0

_Released 2026-11-01._

### Changed (BREAKING: epoch)
- Builds against new caret epochs of workspace siblings (`broadcast-hls` 0.4 -> 0.5, `transmux` 0.27 -> 0.28, `timed-metadata` 0.7 -> 0.8). A caret-epoch change is major-class for a 0.x crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` section 2), whatever its own API diff, so this release is a minor bump; consumers must resolve a single copy of each of those crates.

---

Published from tag `media-doctor-v0.12.0`.
