# timed-metadata 0.4.2 — 2026-07-30

**Patch.**

See the [lockstep 9.1.0 release note](v9.1.0.md) for the full summary.

## Added

- `tests/non_exhaustive_coverage.rs` drift guard (#806). No public API or behaviour change.

## Removed

- Dev-only: the `ssai_ad_stitch` example + its integration test (#812). The 0.4.1 "move" from `transmux` left a byte-identical 24 KB copy in each crate — a cargo output-filename collision.
