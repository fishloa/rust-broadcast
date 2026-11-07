---
status: DRAFT
---

# mpeg-ts 0.7.0

_Released 2026-11-07._

Breaking-class release (0.6 -> 0.7): `serde` is no longer a default feature, the error enum wraps the shared kinds, `InvalidSyncByte` becomes `InvalidSync`, and the unused `into_raw` is removed.

Version: 0.6.2 -> 0.7.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- `serde` is no longer a default feature; enable `features = ["serde"]` (owner decision 2026-11-07).
- error variants `BufferTooShort`, `OutputBufferTooSmall`, `CrcMismatch` now wrap `broadcast_common::error` types (tuple variants; `CrcMismatch` gains `what` and its message becomes "<what> CRC-32 mismatch: ..."); `InvalidSyncByte { found }` is replaced by `InvalidSync(broadcast_common::error::InvalidSync)`. The hand-written `From` impls are gone (`#[from]` replaces them); error values are otherwise unchanged.
- Unused public items, with no use in the workspace beyond their own unit tests: `into_raw`.

### `serde` is now opt-in

```toml
# before: serde was on by default
mpeg-ts = "0.6"
# after: opt in
mpeg-ts = { version = "0.7", features = ["serde"] }
```

Builds that serialize or deserialize these types must name the feature. Builds that only parse and serialize wire bytes get a smaller dependency tree and need no change beyond the version.

### Shared error kinds from `broadcast_common::error`

`broadcast-common` 11 defines the common failure kinds once, as `Copy` structs. This crate's error enum wraps them with `#[error(transparent)] Kind(#[from] broadcast_common::error::Kind)`, so the variants `BufferTooShort` and `OutputBufferTooSmall` (and any other kind named in the list above) changed from struct variants to tuple variants holding the shared struct. The fields are the same, so only the pattern changes, and `From` impls exist, so `?` and `.into()` still build them.

```rust
// before
match err {
    Error::BufferTooShort { need, have, what } => eprintln!("{what}: need {need}, have {have}"),
    _ => {}
}
// after
match err {
    Error::BufferTooShort(e) => eprintln!("{}: need {}, have {}", e.what, e.need, e.have),
    _ => {}
}
```

The display text of these variants now follows the common wording shared by every crate in the wave.

### Renamed and removed items

```text
Error::InvalidSyncByte { found }
    -> Error::InvalidSync(e)  // e.what, e.expected, e.found
Error::CrcMismatch { computed, expected }
    -> Error::CrcMismatch(e)  // e.what, e.computed, e.expected
```

## Added

- `From<broadcast_common::len::BufferTooShort>` and `From<broadcast_common::len::OutputBufferTooSmall>` for the crate error type.

## Documentation

- Fixed documentation links/citations to resolve to existing paths (docs/tools hygiene).

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `mpeg-ts-v0.7.0`.
