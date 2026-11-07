---
status: DRAFT
---

# mpeg-pes 0.7.0

_Released 2026-11-07._

Breaking-class release (0.6 -> 0.7): the error enum wraps the shared kinds, `BadStartCode` becomes `InvalidSync`, and serializers report a too-small output buffer as `OutputBufferTooSmall`.

Version: 0.6.0 -> 0.7.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- error variants `BufferTooShort`, `OutputBufferTooSmall` now wrap `broadcast_common::error` types (tuple variants); `BadStartCode(u32)` is replaced by `InvalidSync(broadcast_common::error::InvalidSync)`. The hand-written `From` impls are gone; error values are otherwise unchanged, messages follow the common wording.

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
Error::BadStartCode(code)
    -> Error::InvalidSync(e)  // e.what, e.expected, e.found
```

## Added

- `broadcast_common::Parse` / `Serialize` implemented for the crate's wire types, delegating to the existing inherent methods (which stay); each type is round-trip tested through the traits.
- `From<broadcast_common::len::BufferTooShort>` and `From<broadcast_common::len::OutputBufferTooSmall>` for the crate error type.

## Fixed

- Serializers report a too-small output buffer as the new `Error::OutputBufferTooSmall { need, have }` (with `From<broadcast_common::len::OutputBufferTooSmall>`) instead of the input-side `Error::BufferTooShort`.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `mpeg-pes-v0.7.0`.
