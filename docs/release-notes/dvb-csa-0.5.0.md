
# dvb-csa 0.5.0

_Released 2026-11-07._

Breaking-class release (0.4 -> 0.5): `mpeg-ts` is a normal dependency and moves to 0.7. The cipher is unchanged; error enums gain `Clone`/`PartialEq`/`Eq`.

Version: 0.4.0 -> 0.5.0.

## Other changes

- Dev-dependency `rand` 0.9 -> 0.10 (bitsliced differential test only; not in public API).

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `mpeg-ts` 0.6 -> 0.7

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `dvb-csa-v0.5.0`.
