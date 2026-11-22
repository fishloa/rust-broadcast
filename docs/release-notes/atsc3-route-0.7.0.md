# atsc3-route 0.7.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 and `rmt-flute` 0.11, which is the breaking part. The work is a test pass over the ROUTE FEC Payload IDs.

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `rmt-flute` 0.10 -> 0.11). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch.

## Tests

The three FEC Payload ID `serialized_len` values and their short-buffer rejection are pinned (#3232).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `atsc3-route-v0.7.0`.
