# rtmp-runtime 0.12.0

_Released 2026-11-22._

Minor for a 0.x crate. No functional change: the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch), which is the breaking part, and its error types derive `thiserror::Error` with unchanged messages.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Changed

- Internal: error types derive `thiserror::Error` in place of hand-written `Display`/`Error` impls (messages unchanged) (#3974, #4020).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4020 (`thiserror`).

---

Published from tag `rtmp-runtime-v0.12.0`.
