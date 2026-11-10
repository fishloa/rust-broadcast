# dash-runtime 0.4.0

_Released 2026-11-10._

Breaking-class release (0.3 -> 0.4): sibling epochs move; no breaking API change of its own.

Version: 0.3.0 -> 0.4.0.

## Breaking changes and migration

None in this crate's own API. The bump follows the `broadcast-common` epoch move.

## Fixed

- Whole-second `xs:dateTime` centred half a second later with half-second uncertainty.
- A `sidx` reference of size 0 still advances the subsegment timeline.

## Dependencies

- `broadcast-common` 10 -> 11

---

Published from tag `dash-runtime-v0.4.0`.
