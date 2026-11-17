# media-plane 0.13.0

_Released 2026-11-17._

Minor-class release (0.12 -> 0.13) from the audit 20 bounds bundle. A retention configuration whose `cold_window` exceeds 30 days is rejected at construction instead of building a ledger that can never expire an entry, and `RetentionDriver::locate` binary-searches the cold ledger instead of scanning it per playback request.

## Upgrading

- BREAKING: `RetentionDriver::new` returns `Result<Option<Self>, RetentionConfigError>`; handle the `Result` and then the nested `Option` at each call site. A `cold_window` longer than `MAX_COLD_WINDOW` (30 days) is `RetentionConfigError::ColdWindowTooLong` instead of a ledger that never expires an entry.

## Changed

- BREAKING: `RetentionDriver::new` returns `Result<Option<Self>, RetentionConfigError>` instead of `Option<Self>`: a `cold_window` longer than the new `MAX_COLD_WINDOW` (30 days) is `RetentionConfigError::ColdWindowTooLong { cold_window, max }` rather than a ledger that can never expire anything (`handed_off_at.checked_add(Duration::MAX)` does not fit the clock, so no entry ages out and the ledger grows without bound). Migration: `.expect("Tiered builds a driver")` on the `Result`, then on the `Option` (#2845).

## Fixed

- `RetentionDriver::locate` binary-searches the cold ledger, which is ascending by sequence number, instead of scanning it linearly on every playback request (#2845).

---

Published from tag `media-plane-v0.13.0`.
