# dvb-simulcrypt 0.5.0

**Release date:** 2026-08-10

MSRV bump plus a CI drift guard. No public API or behaviour change.

## What changed

- MSRV raised to **1.95.0** (issue #949), workspace-wide. No functional or API change from this alone.
- Added `tests/non_exhaustive_coverage.rs`, a drift guard (issue #806) that fails CI if a public enum in this crate is missing `#[non_exhaustive]` without being on the documented SKIP list. No public API or behaviour change.

## Migration

No API changes; no action required.
