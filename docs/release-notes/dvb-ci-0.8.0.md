# dvb-ci 0.8.0

**Release date:** 2026-08-10

MSRV bump plus one breaking API tightening: `SamplePayload` is now `#[non_exhaustive]`.

## What changed

- MSRV raised to **1.95.0** (issue #949), workspace-wide. No functional or API change from this alone.
- Added `tests/label_coverage.rs` + `tests/non_exhaustive_coverage.rs` drift guards (issue #806). No public API or behaviour change beyond the breaking item below.

## Breaking changes

- `SamplePayload` (`ci_plus::sample_decryption`) now carries `#[non_exhaustive]` (issue #806's non-exhaustive drift-guard audit).

## Migration

A downstream `match` on `SamplePayload` needs a wildcard (`_ =>`) arm added; no other change is required.
