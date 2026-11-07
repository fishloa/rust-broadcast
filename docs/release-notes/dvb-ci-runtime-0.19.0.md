---
status: DRAFT
---

# dvb-ci-runtime 0.19.0

_Released 2026-11-07._

Breaking-class release (0.18 -> 0.19) driven by the `dvb-ci`, `dvb-si` and `mpeg-ts` epoch moves. `TS_PACKET_LEN` is now a re-export of `mpeg_ts::ts::TS_PACKET_SIZE` (same value and type).

Version: 0.18.2 -> 0.19.0.

## Other changes

- `TS_PACKET_LEN` is now a re-export of `mpeg_ts::ts::TS_PACKET_SIZE` (same value and type), and the descrambler uses `mpeg_ts::ts::TS_SYNC_BYTE`.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `dvb-ci` 0.10 -> 0.11
- `dvb-si` 12 -> 13
- `mpeg-ts` 0.6 -> 0.7

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `dvb-ci-runtime-v0.19.0`.
