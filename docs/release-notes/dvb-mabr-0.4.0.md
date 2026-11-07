---
status: DRAFT
---

# dvb-mabr 0.4.0

_Released 2026-11-07._

Breaking-class release (0.3 -> 0.4) driven by `broadcast-common` 11. The pull parser now reads attributes and events through `broadcast_common::xml_chars` instead of a private copy.

Version: 0.3.0 -> 0.4.0.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `dvb-mabr-v0.4.0`.
