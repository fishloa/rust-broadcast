# Changelog

## [Unreleased]

### Added

- Initial contents, moved out of `broadcast-common` so the wire-format
  contract can reach `1.0` and stay frozen while the format crates keep
  growing. The moved modules are `traits` (`Parse`, `Serialize`), `len`,
  `bits`, `error`, `reserved`, `timescale` (with `timescale!`) and `cenc`
  (`CencScheme`), plus the `impl_spec_display!` macro. Their behaviour and
  error messages are unchanged; the only implementation change is that the
  error kinds hand-write `Display` and `core::error::Error` instead of
  deriving `thiserror::Error`, so the crate has no non-optional dependency.
