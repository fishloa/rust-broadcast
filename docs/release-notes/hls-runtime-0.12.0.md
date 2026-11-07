---
status: DRAFT
---

# hls-runtime 0.12.0

_Released 2026-11-07._

Breaking-class release (0.11 -> 0.12): three unused builders and the never-constructed `client::Error::UriResolve` are removed, `server::Generation` is now `media_plane::ids::Generation` re-exported at the same path, and the `transmux`, `media-plane` and `broadcast-common` epochs move.

Version: 0.11.0 -> 0.12.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- `client::Error::UriResolve` (never constructed: URI resolution falls back to the unresolved URI, documented on `resolve`).
- Unused `TokioClientConfig::with_connect_timeout`/`with_max_body_bytes` and `ClosedSegment::with_init_generation`.

### Renamed and removed items

```text
client::Error::UriResolve
    -> (removed; URI resolution falls back to the unresolved URI)
TokioClientConfig::with_connect_timeout / with_max_body_bytes
    -> (removed; unused)
ClosedSegment::with_init_generation
    -> (removed; unused)
```

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `broadcast-hls` 0.5 -> 0.6
- `container-probe` 0.2 -> 0.3
- `media-plane` 0.9 -> 0.10
- `timed-metadata` 0.8 -> 0.9
- `transmux` 0.29 -> 0.30

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `hls-runtime-v0.12.0`.
