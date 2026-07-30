# rtmp-runtime 0.3.0 — 2026-07-30

**Minor (breaking: `#[non_exhaustive]` on `LimitType`/`Fmt`/`MessageHeader`).**

See the [lockstep 9.1.0 release note](v9.1.0.md) for the full summary.

## Breaking

- `LimitType`, `Fmt`, `MessageHeader` (`chunk`, `message`) now carry `#[non_exhaustive]` (#806). A downstream `match` on any of these now needs a wildcard arm.

## Added

- `tests/non_exhaustive_coverage.rs` drift guard (#806).
