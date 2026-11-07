
# dvb-tools 13.0.0

_Released 2026-11-07._

Lockstep release with `dvb-si` 13.0.0; this crate carries the version and the sibling dependency moves only.

## Changed
- Lockstep release 13.0.0: no functional change; the CLI builds against `dvb-si` 13, `dvb-t2mi` 13 and `mpeg-ts` 0.7.

## Documentation
- README brought to the common section skeleton (what it does, standards, quick start, what is implemented, design notes, related crates, feature flags, MSRV and licence); the licence heading is spelled consistently (c6 C6-10).
## Dependencies
- Sibling crates move to a new epoch in this release wave: `broadcast-common` 10 -> 11, `dvb-si` 12 -> 13, `dvb-t2mi` 12 -> 13, `mpeg-ts` 0.6 -> 0.7. The `Cargo.toml` requirements name the new versions; no other manifest change.

---

Published from tag `v13.0.0`.
