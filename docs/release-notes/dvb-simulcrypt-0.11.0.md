# dvb-simulcrypt 0.11.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch), which is the breaking part. The work is a test pass over the TS 103 197 registries and generic-message framing.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Tests

- Every registry `name()` label and its `Display` rendering (including each `reserved(0x…)` arm) is pinned to its ETSI TS 103 197 token (#3124).
- Generic-message framing is pinned at its boundaries: a header-only frame, a zero-length parameter value, a truncated parameter value and a trailing partial TLV header (#3125).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `dvb-simulcrypt-v0.11.0`.
