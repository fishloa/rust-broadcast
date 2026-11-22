# srt-runtime 0.12.0

_Released 2026-11-22._

Minor for a 0.x crate, breaking on the epoch: it builds against `broadcast-common` 14 (the jiff 1.0 epoch). Additive: `SocketStats::too_late_dropped` counts the sequence numbers the TSBPD scheduler gave up on, so a starved receiver no longer shows its losses only as holes in the delivered stream.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Added

- `SocketStats::too_late_dropped` counts the sequence numbers the TSBPD scheduler gave up on (too-late arrivals and gaps skipped as unrecoverable, §4.6). A live receiver under a starved runtime can drop payload this way, and until now the only outward sign was a hole in the delivered stream (#3552). `SocketStats` is `#[non_exhaustive]`, so the field is additive.

## Changed

- Internal: error types derive `thiserror::Error` (messages unchanged) (#3974, #4020).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4020 (`thiserror`).

---

Published from tag `srt-runtime-v0.12.0`.
