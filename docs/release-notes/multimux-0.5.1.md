# multimux 0.5.1

**Release date:** 2026-07-30

Fixes three issues surfaced by the 0.5.0 media-plane port: DASH manifests returning 503 on every driver-backed route, a latent panic in Smooth-pull on future `StreamType` variants, and defensive handling of future `LlHlsBody` variants.

## What's fixed

- **DASH and LL-DASH manifests returned 503 forever** on driver-backed routes. `report_driver_progress` now syncs track specs from each program's `Trunk` into the route on every poll, using `track_generation()` to avoid redundant syncs.
- Smooth-pull ingest now skips unknown `StreamType` variants instead of panicking (surfaced by `transmux`'s `StreamType` gaining `#[non_exhaustive]`).
- LL-HLS output handles future `LlHlsBody` variants defensively.

## What's new

- `tests/label_coverage.rs` drift guard (#806).

## Migration

No breaking changes.
