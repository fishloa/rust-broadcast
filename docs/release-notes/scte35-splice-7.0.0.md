# scte35-splice 7.0.0

_Released 2026-11-15._

**BREAKING (MSRV 1.75 → 1.81).**

### Added
- **no_std + alloc support** (#63).

### Changed (breaking)
- Every public enum is `#[non_exhaustive]` (#208) — incl. `DeviceRestrictions`,
  `SegmentationUpidType`, `SegmentationTypeId`.
- MSRV **1.81**.

---

Published from tag `scte35-splice-v7.0.0`.
