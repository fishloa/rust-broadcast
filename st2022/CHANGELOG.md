# Changelog

## [Unreleased]

### Fixed

- Doc accuracy (#940): `Cargo.toml` `description`, the crate-root doc
  comment, and the README no longer claim ST 2022-7 seamless protection
  switching / hitless failover support — this crate implements only the
  ST 2022-6 HBRMT payload header. Aspirational scope moved to a README
  "Planned" section.
- Removed two dead validation branches in `PayloadHeader::validate` (#941
  row 9): the `FrameStructure::Reserved`/`FrameRate::Reserved` range checks
  compared a `u8` against an 8-bit mask (`0xFF`), which can never be true.

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
