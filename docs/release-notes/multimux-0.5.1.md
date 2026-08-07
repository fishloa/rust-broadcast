# multimux 0.5.1

**Release date:** 2026-07-30

Patch fixing three defects shipped in 0.5.0: DASH/LL-DASH manifests returning
503 on every driver-backed route (a missing `set_track_specs` call site),
Smooth-pull panicking on future `StreamType` variants, and LL-HLS output
panicking on future `LlHlsBody` variants.

## What's fixed

- **DASH/LL-DASH 503 regression** (issue #831): `report_driver_progress` now
  syncs track specs from each program's `Trunk` into the route on every poll,
  using `track_generation()` to avoid redundant syncs.
- Smooth-pull ingest now skips any `StreamType` other than `Video`/`Audio`
  (surfaced by `transmux`'s `StreamType` gaining `#[non_exhaustive]`, issue
  #806).
- LL-HLS/resource output now handles a future `LlHlsBody` variant defensively
  (same `#[non_exhaustive]` hardening).

## What's new

- `tests/label_coverage.rs` drift guard (issue #806).
