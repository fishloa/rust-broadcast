---
status: DRAFT
---

# container-probe 0.3.0

_Released 2026-11-07._

Breaking-class release (0.2 -> 0.3) only because `broadcast-common` moves to 11. Six probers share one signature-gate prologue; probing results are unchanged.

Version: 0.2.1 -> 0.3.0.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `container-probe-v0.3.0`.
