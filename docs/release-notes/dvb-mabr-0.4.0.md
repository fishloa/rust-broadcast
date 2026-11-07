
# dvb-mabr 0.4.0

_Released 2026-11-07._

Breaking-class release (0.3 -> 0.4) driven by `broadcast-common` 11. The pull parser now reads attributes and events through `broadcast_common::xml_chars` instead of a private copy.

Version: 0.3.0 -> 0.4.0.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Bounded parsing (#1716)

- A hostile document could use unbounded memory and time. Input size, element depth and count, and attribute size and count are now bounded, including inside skipped extension subtrees, and a tripped bound is `Error::TooLarge { limit, max }`. Nesting past 32 levels is now an error instead of a clean parse.
- `MulticastServerConfiguration::parse_str_with_limits` and `MulticastGatewayConfiguration::parse_str_with_limits` take a caller-chosen `XmlLimits`; `default_limits()` is what `parse_str` applies.
- The serializer uses the shared `broadcast_common::xml_chars` writer helpers; output is byte-identical.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `dvb-mabr-v0.4.0`.
