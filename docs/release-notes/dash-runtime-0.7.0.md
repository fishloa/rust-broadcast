# dash-runtime 0.7.0

_Released 2026-11-22._

Minor for a 0.x crate. The wall-clock API moves to `jiff::Timestamp` (the jiff 1.0 epoch), which is the breaking change, and the crate builds against the rest of the wave's epochs. No behavioural change to segment planning or the clock ladder.

## Breaking changes

- `jiff` 0.2 -> 1.0. The wall-clock API (`DashClient::set_wall_clock`, `DashClient::utc_at`, `PlanOptions::with_horizon` and `PlanOptions::horizon_utc`) takes and returns jiff 1.x `Timestamp`s. Migration: read `jiff::Timestamp::now()` where a `SystemTime` was taken, and depend on `jiff` 1.x.
- Builds against `broadcast-common` 14, `broadcast-dash` 0.3, `container-probe` 0.6, `media-plane` 0.14 and `transmux` 0.33. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2); a consumer must resolve a single copy of each.

## Tests

The ingest size budgets and their setters, the `styp`/`lmsg` brand scan, the per-track state machine's clock and the buffered-sample cap have tests that fail if any is removed or off by one (audit 22 finding 52, #3020). The segment-addressing arithmetic (`@endNumber`, the short last segment, a negative `@r` cut at the next `S@t`, `@presentationTimeOffset`, the dynamic availability window, the per-level `@timeShiftBufferDepth`, `sidx` numbering) has tests that fail if an operator or boundary moves (audit 22 finding 51, #3019).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `dash-runtime-v0.7.0`.
