
# broadcast-hls 0.6.0

_Released 2026-11-07._

Breaking-class release (0.5 -> 0.6) only because it depends on `broadcast-common` 11. There is no public API change; the README and crate-root documentation were corrected and the unit tests now assert exact error variants.

Version: 0.5.0 -> 0.6.0.

## Other changes

- The `serde_json` dev-dependency is dropped and the unit tests assert the exact error variant instead of `is_err()`; no public API change.

## Documentation

- README and crate-root documentation corrections; no behaviour change.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `broadcast-hls-v0.6.0`.
