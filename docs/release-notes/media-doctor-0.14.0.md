---
status: DRAFT
---

# media-doctor 0.14.0

_Released 2026-11-07._

Breaking-class release (0.13 -> 0.14): `serde` is no longer a default feature, and the `transmux`, `dvb-conformance`, `mpeg-ts` and `broadcast-common` epochs move.

Version: 0.13.0 -> 0.14.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- `serde` is no longer a default feature; enable `features = ["serde"]` (owner decision 2026-11-07).

### `serde` is now opt-in

```toml
# before: serde was on by default
media-doctor = "0.13"
# after: opt in
media-doctor = { version = "0.14", features = ["serde"] }
```

Builds that serialize or deserialize these types must name the feature. Builds that only parse and serialize wire bytes get a smaller dependency tree and need no change beyond the version.

## Other changes

- Dropped the unused `thiserror` dependency.
- TS packet size and sync byte come from `mpeg_ts::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE}` instead of a local copy.

## Documentation

- README brought to the common section skeleton (what it does, standards, quick start, what is implemented, design notes, related crates, feature flags, MSRV and licence); the licence heading is spelled consistently.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `broadcast-hls` 0.5 -> 0.6
- `container-probe` 0.2 -> 0.3
- `dvb-conformance` 12 -> 13
- `dvb-si` 12 -> 13
- `mpeg-pes` 0.6 -> 0.7
- `mpeg-ts` 0.6 -> 0.7
- `scte35-splice` 4 -> 5
- `timed-metadata` 0.8 -> 0.9
- `transmux` 0.29 -> 0.30

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `media-doctor-v0.14.0`.
