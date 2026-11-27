# Changelog

## [Unreleased]

### Added

- `damage`: the workspace's one report of content a reader skipped
  (`Damage`, `DamageKind`, `Severity`, `DamageLocation`, `ByteRange`,
  `MediaTimeRange`, `DamageCause`), the caller's `DamagePolicy` and
  `Disposition` (report or fail, per severity and optionally per kind), the
  `DamageError` a failing policy stops with, and `DamagePolicy::admit`, the one
  helper a reader calls. Best-effort reading and the rule that a silent skip is
  a defect are stated in `docs/API-CONVENTIONS.md`.
- Initial contents, moved out of `broadcast-common` so the wire-format
  contract can reach `1.0` and stay frozen while the format crates keep
  growing. The moved modules are `traits` (`Parse`, `Serialize`), `len`,
  `bits`, `error`, `reserved`, `timescale` (with `timescale!`) and `cenc`
  (`CencScheme`), plus the `impl_spec_display!` macro. Their behaviour and
  error messages are unchanged; the only implementation change is that the
  error kinds hand-write `Display` and `core::error::Error` instead of
  deriving `thiserror::Error`, so the crate has no non-optional dependency.
- `wire`: the big-endian field readers (`take`, `read_u8`/`read_u16`/
  `read_u32`/`read_u48`, `be_u16`/`be_u32`/`be_u64`, the unchecked `u16_at` …
  `fourcc_at`, and `U24_LEN`), moved from `broadcast-common` unchanged, so
  `isobmff` reads its fields from this crate alone. The `avcC`-specific
  16-bit-length-prefixed run reader did not come along: `isobmff` is its only
  user and keeps it private.
