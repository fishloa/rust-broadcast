---
status: DRAFT
---
# playout-runtime 0.6.0

_Released 2026-11-10._

Breaking-class release (0.5 -> 0.6): `build_splice_insert` rejects over-long break duration; sibling epochs move.

Version: 0.5.0 -> 0.6.0.

## Breaking changes and migration

- `build_splice_insert` rejects a `break_duration_ticks` of 2^33 or more with the new `Error::BreakDurationTooLong` (`Error` is `#[non_exhaustive]`).

## Dependencies

- `scte35-splice` 5.0 -> 5.0.1
- `broadcast-common` 10 -> 11

---

Published from tag `playout-runtime-v0.6.0`.
