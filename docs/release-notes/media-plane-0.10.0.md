---
status: DRAFT
---

# media-plane 0.10.0

_Released 2026-11-07._

Breaking-class release (0.9 -> 0.10): `Trunk::sparse_len` and `Trunk::event_len` become test-only, `ids::Generation` is added next to `SegmentSeq`/`PartIndex`, and the `transmux` epoch moves.

Version: 0.9.0 -> 0.10.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- `Trunk::sparse_len` and `Trunk::event_len` are now test-only.

### Renamed and removed items

```text
trunk.sparse_len() / trunk.event_len()
    -> (test-only now; no replacement, unused in the workspace)
```

## Added

- `ids::Generation`, the init-segment generation newtype, next to `SegmentSeq`/`PartIndex`.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `timed-metadata` 0.8 -> 0.9
- `transmux` 0.29 -> 0.30

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `media-plane-v0.10.0`.
