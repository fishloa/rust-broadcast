# dvb-csa 0.6.0

_Released 2026-11-08._

Breaking-class release (0.5 -> 0.6): the never-constructed `Error::BufferTooShort` is removed.

Version: 0.5.0 -> 0.6.0.

## Breaking changes and migration

`Error::BufferTooShort { need, have }` is removed (#1726). The packet APIs take a fixed 188-byte array, so no input can be too short and the variant was never returned. A `match` on `Error` that named it must drop that arm; no behaviour changes.

```text
Error::BufferTooShort { need, have }  ->  (removed; never constructed)
```

```rust
// before
match err {
    Error::BufferTooShort { need, have } => short(need, have),
    other => fail(other),
}

// after
fail(err)
```

The README dependency snippet now reads `dvb-csa = "0.6"`.

## Dependencies

No workspace sibling requirement changes epoch.

---

Published from tag `dvb-csa-v0.6.0`.
