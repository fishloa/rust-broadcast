
# dash-runtime 0.3.0

_Released 2026-11-07._

Breaking-class release (0.2 -> 0.3): two unused public `mpd` helpers are removed, XML character handling and the attribute loop now come from `broadcast_common::xml_chars`, and the `transmux` and `broadcast-common` epochs move. One ordering detail of MPD error reporting changes (below).

Version: 0.2.0 -> 0.3.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- Unused public items `mpd::is_unsupported_urlparam` and `mpd::url_query_info`.

### Behaviour change: attribute-limit ordering

The MPD reader's attribute loop is `broadcast_common::xml_chars::read_attributes_checked` with the attribute count and size limits as its hook. An over-limit attribute count is now reported after, not before, a malformed value on the same start tag.

## Other changes

- `timing::ticks_to_us` and the plan's microsecond-to-tick ceiling use `broadcast_common::timescale::{to_micros, from_micros}`.
- The MPD reader's attribute loop is `broadcast_common::xml_chars::read_attributes_checked` with the attribute count and size limits as its hook; an over-limit attribute count is now reported after (not before) a malformed value on the same start tag.
- `timing::ticks_to_us` rescales through `broadcast_common::timescale` (same floor rounding).

## Documentation

- README brought to the common section skeleton (what it does, standards, quick start, what is implemented, design notes, related crates, feature flags, MSRV and licence); the licence heading is spelled consistently.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Shared XML bounds (#1716)

- The MPD tree reader counts through `broadcast_common::xml_chars::XmlBudget`. The public `Limits` and its defaults are unchanged; `Limits::xml_limits()` exposes them as `XmlLimits`, and `From<XmlLimit> for Limit` maps a tripped bound.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11
- `container-probe` 0.2 -> 0.3
- `transmux` 0.29 -> 0.30

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `dash-runtime-v0.3.0`.
