# broadcast-smooth 0.2.0

_Released 2026-11-15._

### Changed
- `SmoothParseError` gains `From<broadcast_common::xml_chars::XmlCharError>`, the typed character-data and namespace failure the shared XML helper now reports (#2646).
- BREAKING: `SmoothManifest::timescale` is `Timescale` and `StreamIndex::timescale` is `Option<Timescale>` (was `u64` / `Option<u64>`); `StreamIndex::with_timescale` takes a `Timescale`. A present `TimeScale` that is zero, wider than 32 bits or not a number is `SmoothParseError::InvalidAttributeValue`; an absent root `TimeScale` resolves to the new `SMOOTH_DEFAULT_TIMESCALE` (10 000 000) and an absent `StreamIndex@TimeScale` stays `None` (#2657).

### Added
- `SMOOTH_DEFAULT_TIMESCALE`: `SMOOTH_TIMESCALE` as a `Timescale`.

### Removed
- Removed the unused `thiserror` dependency and the `thiserror/std` entry in the `std` feature.

---

Published from tag `broadcast-smooth-v0.2.0`.
