---
status: DRAFT
---

# scte35-splice 5.0.0

_Released 2026-11-07._

Breaking release (4 -> 5): `serde` is no longer a default feature, the error enum wraps the shared kinds, and `InvalidValue` is split into typed `InvalidValue`, `FieldOverflow` and `InvalidField` errors that carry the wire value (#1715).

Version: 4.0.0 -> 5.0.0 (major).

## Breaking changes and migration

What the CHANGELOG records as breaking:

- `serde` is no longer a default feature; enable `features = ["serde"]` (owner decision 2026-11-07).
- `Error::InvalidValue { field, reason }` (no value) is replaced. `Error::InvalidValue` now wraps `broadcast_common::error::InvalidValue { field, value, reason }` and is used for unknown/forbidden parsed values (`compact_SCTE_35.message_type`, `compact_splice_insert.descriptor_length`, `private_data_length`); a serializer value above its field's range returns `Error::FieldOverflow { field, value, max }` (33-bit PTS/duration fields, 40-bit duration, 48-bit `tai_seconds`, `tier`, `section_length`, counts and length bytes); structural problems with no single number return `Error::InvalidField { field, reason }` (`splice_command_length` 0xFFF sentinel, empty section body, `e_crc_32` presence, TEMI presence). New variant: `InvalidField`.
- error variants `BufferTooShort`, `OutputBufferTooSmall`, `CrcMismatch` now wrap `broadcast_common::error` types (tuple variants; `CrcMismatch` gains `what` = "splice_info_section", message "<what> CRC-32 mismatch: ..."). Hand-written `From` impls removed. `InvalidValue` stays crate-specific (carries no offending value).
- Unused public items, with no use in the workspace beyond their own unit tests: `duration_value`, `pts_adjustment_duration`, `pts_time_duration`, `set_duration_value`, `set_pts_adjustment_duration`, `set_pts_time_duration`.

### `serde` is now opt-in

```toml
# before: serde was on by default
scte35-splice = "4"
# after: opt in
scte35-splice = { version = "5", features = ["serde"] }
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
Error::InvalidValue { field, reason }
    -> Error::InvalidValue(e)  // unknown/forbidden parsed value: e.field, e.value, e.reason
Error::InvalidValue { .. }  // serializer value too large
    -> Error::FieldOverflow { field, value, max }
Error::InvalidValue { .. }  // structural problem
    -> Error::InvalidField { field, reason }
Error::CrcMismatch { computed, expected }
    -> Error::CrcMismatch(e)  // e.what == "splice_info_section"
```

### Typed errors (#1715)

Unknown or forbidden parsed values (`compact_SCTE_35.message_type`, `compact_splice_insert.descriptor_length`, `private_data_length`) return `Error::InvalidValue`. A serializer value above its field's range returns `Error::FieldOverflow` (33-bit PTS/duration fields, 40-bit duration, 48-bit `tai_seconds`, `tier`, `section_length`, counts and length bytes). Structural problems with no single number return `Error::InvalidField` (`splice_command_length` 0xFFF sentinel, empty section body, `e_crc_32` presence, TEMI presence). The unused `duration_value`, `pts_adjustment_duration`, `pts_time_duration` and their setters are removed.

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

Published from tag `scte35-splice-v5.0.0`.
