---
status: DRAFT
---

# rtmp-runtime 0.9.0

_Released 2026-11-07._

Breaking-class release (0.8 -> 0.9): `RtmpError::BufferTooShort` wraps the shared kind, two unused `ServerConfig` builders are removed, and the unused `log` dependency is dropped.

Version: 0.8.1 -> 0.9.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- error variant `RtmpError::BufferTooShort` now wraps `broadcast_common::error::BufferTooShort` (same `need`/`have`/`what`; the message text is the shared one).
- Unused `ServerConfig::with_window_ack_size` and `with_peer_bandwidth`.

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
RtmpError::BufferTooShort { need, have, what }
    -> RtmpError::BufferTooShort(e)  // e.need, e.have, e.what
ServerConfig::with_window_ack_size / with_peer_bandwidth
    -> (removed; unused)
```

## Other changes

- Dropped the unused `log` dependency.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `rtmp-runtime-v0.9.0`.
