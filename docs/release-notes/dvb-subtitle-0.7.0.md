---
status: DRAFT
---

# dvb-subtitle 0.7.0

_Released 2026-11-07._

Breaking-class release (0.6 -> 0.7): the error enum wraps the shared kinds, serializers report a too-small output buffer as `OutputBufferTooSmall`, and a misplaced `end_of_PES_data_field_marker` is now rejected.

Version: 0.6.0 -> 0.7.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- error variants `BufferTooShort`, `OutputBufferTooSmall`, `InvalidValue` now wrap `broadcast_common::error` types (tuple variants; `InvalidValue` gains the offending `value`, and the reduced-range CLUT colour check now names the exceeding component); `BadSyncByte(u8)` is replaced by `InvalidSync(broadcast_common::error::InvalidSync)`. Hand-written `From` impls removed.
- `Error::UnknownDataType` and `Error::UnknownObjectCodingMethod`: never constructed. Reserved `data_type` and `object_coding_method` values are carried losslessly as `DataType::Reserved` / `ObjectCodingMethod::Reserved`, and EN 300 743 requires no rejection of them.

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
Error::BadSyncByte(b)
    -> Error::InvalidSync(e)  // e.what, e.expected, e.found
Error::UnknownDataType / Error::UnknownObjectCodingMethod
    -> DataType::Reserved / ObjectCodingMethod::Reserved  // carried losslessly, never an error
```

### Behaviour changes

`PesDataField::parse` returns `Error::BadEndOfPesMarker` when exactly one byte follows the last whole segment and it is not the fixed `0xFF` marker (EN 300 743 section 7.2, Table 3); a field with misframed or truncated segments keeps its remainder verbatim as before. Serializers used to return `Error::BufferTooShort` for a too-small output buffer; they now return `Error::OutputBufferTooSmall { need, have }`.

## Added

- `From<broadcast_common::len::BufferTooShort>` and `From<broadcast_common::len::OutputBufferTooSmall>` for the crate error type.

## Fixed

- `PesDataField::parse` returns `Error::BadEndOfPesMarker` when exactly one byte follows the last whole segment and it is not the fixed `0xFF` `end_of_PES_data_field_marker` (EN 300 743 §7.2, Table 3); a field with misframed or truncated segments keeps its remainder verbatim as before.
- Serializers report a too-small output buffer as the new `Error::OutputBufferTooSmall { need, have }` (with `From<broadcast_common::len::OutputBufferTooSmall>`) instead of the input-side `Error::BufferTooShort`.

## Documentation

- Fixed documentation links/citations to resolve to existing paths (docs/tools hygiene).

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `dvb-subtitle-v0.7.0`.
