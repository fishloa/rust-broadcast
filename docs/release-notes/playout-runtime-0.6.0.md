# playout-runtime 0.6.0

_Released 2026-11-14._

Major-class release (0.5 -> 0.6) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.5.0 -> 0.6.0.

## Breaking changes and migration

- `build_splice_insert` rejects a `break_duration_ticks` of 2^33 or more with the new `Error::BreakDurationTooLong` (`Error` is `#[non_exhaustive]`).

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `scte35-splice` 5 -> 6, `ssai-runtime` 0.5 -> 0.6.

---

Published from tag `playout-runtime-v0.6.0`.
