# multimux 0.5.1 — 2026-07-30

**Patch.**

See the [lockstep 9.1.0 release note](v9.1.0.md) for the full summary.

## Fixed

- **DASH and LL-DASH manifests returned 503 forever on every driver-backed route** (shipped in v0.5.0). `RouteHandle::set_track_specs` had no production call site — `report_driver_progress` now syncs track specs from each published program's `Trunk` into the route on every poll (#831).
- Smooth-pull ingest now skips any `StreamType` other than `Video`/`Audio` at manifest-parse time, surfaced by `transmux`'s `StreamType` gaining `#[non_exhaustive]` (#806).
- `output::llhls`/`origin::resource` handle a future `LlHlsBody` variant defensively.
