# media-plane 0.13.0

_Released 2026-11-17._

### Changed
- BREAKING: `RetentionDriver::new` returns `Result<Option<Self>, RetentionConfigError>` instead of `Option<Self>`: a `cold_window` longer than the new `MAX_COLD_WINDOW` (30 days) is `RetentionConfigError::ColdWindowTooLong { cold_window, max }` rather than a ledger that can never expire anything (`handed_off_at.checked_add(Duration::MAX)` does not fit the clock, so no entry ages out and the ledger grows without bound). Migration: `.expect("Tiered builds a driver")` on the `Result`, then on the `Option` (#2845).

### Fixed
- `RetentionDriver::locate` binary-searches the cold ledger, which is ascending by sequence number, instead of scanning it linearly on every playback request (#2845).

---

Published from tag `media-plane-v0.13.0`.
