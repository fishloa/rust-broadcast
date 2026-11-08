# ttml-subtitle 0.6.0

_Released 2026-11-08._

Breaking-class release (0.5 -> 0.6): four `Error` variants that no code ever constructed are removed.

Version: 0.5.0 -> 0.6.0.

## Breaking changes and migration

`Error::MissingElement`, `Error::MissingAttribute`, `Error::InvalidAttribute` and `Error::Validation` are removed (#1726). The parser never produced them: TTML2 `tt` content is `head?, body?` with both optional, the transcribed syntax marks no element attribute required, and profile validation reports through its own report type rather than `Error`.

```text
Error::MissingElement(&'static str)                    -> (removed; never constructed)
Error::MissingAttribute { element, attr }              -> (removed; never constructed)
Error::InvalidAttribute { element, attr, reason }      -> (removed; never constructed)
Error::Validation(String)                              -> (removed; never constructed)
```

A `match` on `Error` that named any of these arms must drop them. Because they were never returned, no behaviour changes: take profile validation findings from the validation report, not from `Error`.

```rust
// before
match err {
    Error::MissingElement(name) => report(name),
    Error::Validation(msg) => report(&msg),
    other => report(&other.to_string()),
}

// after
report(&err.to_string());
```

## Dependencies

No workspace sibling requirement changes epoch.

---

Published from tag `ttml-subtitle-v0.6.0`.
