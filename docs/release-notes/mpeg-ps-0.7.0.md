---
status: DRAFT
---

# mpeg-ps 0.7.0

_Released 2026-11-07._

Breaking-class release (0.6 -> 0.7): the error enum wraps the shared kinds, `BadCrc` becomes `CrcMismatch`, and the pack header, system header and program stream map serializers report a too-small output buffer as the new `Error::OutputBufferTooSmall`.

Version: 0.6.0 -> 0.7.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- error variants `BufferTooShort`, `OutputBufferTooSmall`, `InvalidField` now wrap `broadcast_common::error` types (tuple variants); `BadCrc { computed, stored }` is replaced by `CrcMismatch(broadcast_common::error::CrcMismatch)` (`what` = "PSM"). The hand-written `From` impls are gone; messages follow the common wording.

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
Error::BadCrc { computed, stored }
    -> Error::CrcMismatch(e)  // e.what == "PSM", e.computed, e.expected
```

## Added

- `Error::OutputBufferTooSmall`.
- `From<broadcast_common::len::BufferTooShort>` and `From<broadcast_common::len::OutputBufferTooSmall>` for the crate error type.

## Fixed

- `PackHeader`, `SystemHeader` and `ProgramStreamMap` `serialize_into` returned `Error::BufferTooShort` for a too-small OUTPUT buffer; they now return the new `Error::OutputBufferTooSmall { need, have }`.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `mpeg-pes` 0.6 -> 0.7

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `mpeg-ps-v0.7.0`.
