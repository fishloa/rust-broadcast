---
status: DRAFT
---

# caption-convert 0.5.0

_Released 2026-11-07._

Breaking-class release (0.4 -> 0.5) driven by the `cc-data`, `dvb-vbi`, `timed-metadata` and `broadcast-common` epoch moves. The conversion matrix is unchanged; the crate-root docs are now the README.

Version: 0.4.0 -> 0.5.0.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `cc-data` 0.7 -> 0.8
- `dvb-vbi` 0.5 -> 0.6
- `timed-metadata` 0.8 -> 0.9

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `caption-convert-v0.5.0`.
