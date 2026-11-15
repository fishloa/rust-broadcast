# broadcast-smooth 0.2.0

_Released 2026-11-15._

Minor-class release (0.1 -> 0.2). A manifest's `TimeScale` is the shared `Timescale` instead of a bare integer, so a consumer never has to decide what a zero means, and the XML failure is the typed `XmlCharError` the shared helper now reports.

Version: 0.1.0 -> 0.2.0.

## Breaking changes and migration

### `TimeScale` is a `Timescale`

`SmoothManifest::timescale` is a `Timescale` and `StreamIndex::timescale` an `Option<Timescale>` (was `u64`/`Option<u64>`); `StreamIndex::with_timescale` takes a `Timescale`. A present `TimeScale` that is zero, wider than 32 bits or not a number is `SmoothParseError::InvalidAttributeValue`; an absent root `TimeScale` resolves to the new `SMOOTH_DEFAULT_TIMESCALE` (10 000 000) and an absent `StreamIndex@TimeScale` stays `None`.

```rust
// before
let timescale: u64 = manifest.timescale;
let stream: Option<u64> = index.timescale;
// after
let timescale: Timescale = manifest.timescale;
let hz: u32 = timescale.get();
let stream: Option<Timescale> = index.timescale;
```

### The XML failure is typed

`SmoothParseError` gains `From<broadcast_common::xml_chars::XmlCharError>`, the typed character-data and namespace failure the shared XML helper now reports.

## Added

- `SMOOTH_DEFAULT_TIMESCALE`: the Smooth `TimeScale` as a `Timescale`.

## Removed

- The unused `thiserror` dependency and the `thiserror/std` entry in the `std` feature.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `broadcast-smooth-v0.2.0`.
