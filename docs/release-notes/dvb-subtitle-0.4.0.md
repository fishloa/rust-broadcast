# dvb-subtitle 0.4.0

**Release date:** 2026-08-10

MSRV-only release. No functional or API change.

## What changed

- MSRV raised to **1.95.0** (issue #949) — the workspace-wide move that
  removes the separate MSRV lane `webrtc-runtime`'s optional `media` feature
  previously required. Only let-chain / `is_multiple_of` style adoption
  where the 1.95 lints require it.
- Added `tests/label_coverage.rs` and `tests/non_exhaustive_coverage.rs`
  drift guards (issue #806). Test-only; no public API or behaviour change.

## Migration

No API changes; no action required beyond building with rustc >= 1.95.0.
