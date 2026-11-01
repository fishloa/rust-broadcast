# compliance-probe 0.5.0

_Released 2026-11-01._

### Changed (BREAKING: epoch)
- Builds against new caret epochs of workspace siblings (`timed-metadata` 0.7 -> 0.8, `media-plane` 0.7 -> 0.8, `media-doctor` 0.11 -> 0.12). A caret-epoch change is major-class for a 0.x crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` section 2), whatever its own API diff, so this release is a minor bump; consumers must resolve a single copy of each of those crates.

---

Published from tag `compliance-probe-v0.5.0`.
