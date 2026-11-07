---
status: DRAFT
---

# timed-metadata 0.9.0

_Released 2026-11-07._

Breaking-class release (0.8 -> 0.9): `serde` is no longer a default feature, the unused `chrono` feature and dependency are removed, two one-line parity wrappers are removed, and the `scte35-splice`, `mp4-emsg`, `cc-data`, `dvb-vbi`, `broadcast-hls` and `broadcast-common` epochs move. The crate's own error enum is unchanged; it wraps `scte35_splice::Error` and `mp4_emsg::Error`, which now use the shared kinds.

Version: 0.8.0 -> 0.9.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- `serde` is no longer a default feature; enable `features = ["serde"]` (owner decision 2026-11-07).
- The `chrono` feature (on by default) and the `chrono` dependency: nothing in the crate used them.
- `webvtt::teletext::decode_odd_parity` and `encode_odd_parity` (one-line forwards). Use `broadcast_common::parity::strip_parity` and `broadcast_common::parity::set_odd_parity`.

### `serde` is now opt-in

```toml
# before: serde was on by default
timed-metadata = "0.8"
# after: opt in
timed-metadata = { version = "0.9", features = ["serde"] }
```

Builds that serialize or deserialize these types must name the feature. Builds that only parse and serialize wire bytes get a smaller dependency tree and need no change beyond the version.

### Renamed and removed items

```text
webvtt::teletext::decode_odd_parity(b)
    -> broadcast_common::parity::strip_parity(b)
webvtt::teletext::encode_odd_parity(b)
    -> broadcast_common::parity::set_odd_parity(b)
```

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `broadcast-hls` 0.5 -> 0.6
- `cc-data` 0.7 -> 0.8
- `dvb-vbi` 0.5 -> 0.6
- `mp4-emsg` 0.5 -> 0.6
- `scte35-splice` 4 -> 5

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `timed-metadata-v0.9.0`.
