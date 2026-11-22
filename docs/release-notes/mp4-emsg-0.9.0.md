# mp4-emsg 0.9.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch), which is the breaking part. The work is the audit test pass over the `emsg` box's size guards and presentation-time token.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Tests

- `EmsgBox::parse`'s declared-size guard is pinned at every boundary: a size of 8..=11 is `InvalidSize`, a size of exactly 12 is a valid empty FullBox whose body parse fails (not a size error), and a version-1 body of 0..=19 bytes is `BufferTooShort { need: 20 }` at each length (#3032).
- `PresentationTime::name()` and its `Display` form are asserted literally against the DASH-IF Part 10 Table 6-2 tokens (#3033).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `mp4-emsg-v0.9.0`.
