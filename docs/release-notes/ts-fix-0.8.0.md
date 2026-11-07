
# ts-fix 0.8.0

_Released 2026-11-07._

Breaking-class release (0.7 -> 0.8): `Error::NoSyncByte` becomes `InvalidSync` and the `dvb-si`, `dvb-conformance`, `mpeg-ts` and `broadcast-common` epochs move. A regression test now pins that a regenerated PAT version wraps modulo 32 and holds when the table is unchanged (#1704).

Version: 0.7.0 -> 0.8.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- `Error::NoSyncByte { found }` is replaced by `InvalidSync(broadcast_common::error::InvalidSync)`.

### Renamed and removed items

```text
Error::NoSyncByte { found }
    -> Error::InvalidSync(e)  // e.what, e.expected, e.found
```

## Other changes

- TS packet size and sync byte come from `mpeg_ts::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE}` instead of a local copy.

## Documentation

- README brought to the common section skeleton (what it does, standards, quick start, what is implemented, design notes, related crates, feature flags, MSRV and licence); the licence heading is spelled consistently.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `dvb-conformance` 12 -> 13
- `dvb-si` 12 -> 13
- `mpeg-pes` 0.6 -> 0.7
- `mpeg-ts` 0.6 -> 0.7

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `ts-fix-v0.8.0`.
