# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial release: TTML2 / IMSC 1.1 timed-text subtitle parser and validator.
- Full TTML2 element tree (26 element types, 56 style properties) per `ttml2-syntax.md`.
- Exhaustive `<time-expression>` grammar (clock-time, offset-time, wallclock-time) with
  frame/tick/SMPTE constraint enforcement, including negative-case tests.
- IMSC 1.1 feature disposition table (159 rows) and §7.12 "must reject" structural constraints.
- Parse/validate split — callers can inspect non-conformant documents before rejecting.
- Semantic round-trip (parse → serialize → re-parse → equal); no raw-passthrough serializer.
- Real-fixture tests against 11 W3C IMSC conformance suite documents (issue #753).
- `#204` label convention: `name()` + `impl_spec_display!` on all spec/field enums.
- `#[non_exhaustive]` on all public enums.
- Per-crate `label_coverage.rs` and `non_exhaustive_coverage.rs` drift guards.
- Two runnable examples (`parse_document`, `validate_document`).
- Spec citations in all module docs (TTML2 + IMSC 1.1 section references).

### Known limitations

- **IMSC §7.12.1.2** (region overlap) is not enforced. Computing region
  coordinate intersection requires the full TTML2 §10.4 style resolution
  cascade (specified → computed → used values through the `style` chain,
  `initial` elements, and spec defaults), which is deferred to a future
  release. The maximum-region-count constraint (§7.12.1.3) and Image Profile
  text-element prohibition (§9.4.1) are enforced.

- **Byte-identical XML round-trip is not achieved** for parsed documents.
  roxmltree discards XML comments, attribute ordering, namespace prefix
  spelling, and indentation whitespace during DOM construction. These are
  preserved in the parse→serialize→re-parse **semantic equality** test
  (all 11 fixtures pass) and the **mutation test** (mutated fields change
  output), but the re-serialized output is not byte-for-byte identical to
  the input. A raw-passthrough check (`grep raw_source`) confirms the
  serializer does not echo stored input text. Full byte-identity would
  require either replacing roxmltree with a custom position-preserving
  parser or carrying per-element formatting metadata — deferred to a
  future release.

[Unreleased]: https://github.com/fishloa/rust-broadcast/compare/v0.1.0...HEAD
