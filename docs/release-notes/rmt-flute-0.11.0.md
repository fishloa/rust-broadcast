# rmt-flute 0.11.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch), which is the breaking part. The work is the audit test pass over the LCT/ALC/NORM wire formats.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Tests

LCT CCI/TSI/TOI and `HDR_LEN` validation plus the close flags, EXT_TIME lengths with several values, NORM `serialize_into` return values and truncation guards, and the ALC SPI, partition accessors, emptiness and ack labels are pinned (#3182, #3183, #3184, #3185).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `rmt-flute-v0.11.0`.
