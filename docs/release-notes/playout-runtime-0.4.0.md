# playout-runtime 0.4.0

_Released 2026-10-31._

### Changed (BREAKING: epoch)
- Builds against new caret epochs of workspace siblings (`ssai-runtime` 0.3 -> 0.4). A caret-epoch change is major-class for a 0.x crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` section 2), whatever its own API diff, so this release is a minor bump; consumers must resolve a single copy of each of those crates.

---

Published from tag `playout-runtime-v0.4.0`.
