# dvb-ci-runtime 0.16.0

**Release date:** 2026-08-10

MSRV bump plus one breaking API tightening: `DeviceOp`, `LinkEvent`, and `TcState` are now `#[non_exhaustive]`.

## What changed

- MSRV raised to **1.95.0** (issue #949), workspace-wide. No functional or API change from this alone.
- Added `tests/label_coverage.rs` + `tests/non_exhaustive_coverage.rs` drift guards (issue #806). No public API or behaviour change beyond the breaking item below.

## Breaking changes

- `DeviceOp` and `LinkEvent` (`device`), and `TcState` (`transport`), now carry `#[non_exhaustive]` (issue #806's non-exhaustive drift-guard audit).

## Migration

A downstream `match` on any of `DeviceOp`, `LinkEvent`, or `TcState` needs a wildcard (`_ =>`) arm added; no other change is required.
