# Changelog

## [Unreleased]

## [0.1.0] — 2026-08-08

Initial release.

- `PayloadHeader` parse/serialize for the 4/8/12+-byte HBRMT payload header
  (SMPTE ST 2022-6:2012 §6.4).
- `VideoSourceFormat` with MAP/FRAME/FRATE/SAMPLE field accessors.
- Typed field enums: `ClockFrequency`, `FecUsage`, `FrameRate`,
  `FrameStructure`, `MapStructure`, `SampleStructure`, `Scrambling`,
  `TimestampRef`, `VideoSourceId`.
- Golden-bytes round-trip test with real HBRMT payload.
- `#[non_exhaustive]` + `name()` + `impl_spec_display!` on all spec enums.
- `no_std` + `alloc`, optional `serde`.
