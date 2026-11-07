---
status: DRAFT
---

# ttml-subtitle 0.5.0

_Released 2026-11-07._

Breaking-class release (0.4 -> 0.5): four never-constructed `Error` variants, `validate_to_result` and the `serde` feature are removed, and XML handling comes from `broadcast_common::xml_chars`.

Version: 0.4.0 -> 0.5.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- Unused public items, with no use in the workspace beyond their own unit tests: `validate_to_result`.
- `Error::UnexpectedElement`, `Error::ProhibitedElement`, `Error::ProhibitedAttribute` and `Error::Serialize`: never constructed. Foreign elements are preserved and profile violations are reported as validation findings, not errors.
- The `serde` feature and the `serde` dependency: nothing in the crate used them.

### Renamed and removed items

```text
features = ["serde"]
    -> (feature removed; nothing in the crate used it)
Error::UnexpectedElement / ProhibitedElement / ProhibitedAttribute / Serialize
    -> (removed; never constructed. Profile violations are validation findings, not errors)
validate_to_result(..)
    -> (removed; use the validator and inspect its findings)
```

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `ttml-subtitle-v0.5.0`.
