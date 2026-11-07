
# ssai-runtime 0.5.0

_Released 2026-11-07._

Breaking-class release (0.4 -> 0.5): `AdBreakDecision::asset_list` and `SessionPlaylistBase::render_plain` become test-only, and the `scte35-splice` and `broadcast-common` epochs move.

Version: 0.4.0 -> 0.5.0.

## Breaking changes and migration

### Renamed and removed items

```text
decision.asset_list()
    -> (removed from the public API; unused)
base.render_plain()
    -> (removed from the public API; unused)
```

## Other changes

- `AdBreakDecision::asset_list` and `SessionPlaylistBase::render_plain` are now test-only: nothing outside the unit tests used them.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `broadcast-hls` 0.5 -> 0.6

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `ssai-runtime-v0.5.0`.
