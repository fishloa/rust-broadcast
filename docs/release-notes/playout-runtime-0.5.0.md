---
status: DRAFT
---

# playout-runtime 0.5.0

_Released 2026-11-07._

Breaking-class release (0.4 -> 0.5) driven by the `scte35-splice` 5 and `ssai-runtime` 0.5 epoch moves. The crate's own change is documentation and `Eq` derives.

Version: 0.4.0 -> 0.5.0.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `scte35-splice` 4 -> 5
- `ssai-runtime` 0.4 -> 0.5

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `playout-runtime-v0.5.0`.
