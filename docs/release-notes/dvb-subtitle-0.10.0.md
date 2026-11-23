# dvb-subtitle 0.10.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 and `mpeg-pes` 0.10, which is the breaking part. The work is the audit pass over the EN 300 743 segment layouts and the pixel-data decoders.

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `mpeg-pes` 0.9 -> 0.10). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch.

## Tests

- The 2-, 4- and 8-bit pixel-data run-length decoders are pinned to every code branch of EN 300 743 Tables 22-26 (single pixels, run-of-N in colour 0, runs with a pixel code including the 284/280/127 ceilings), the scanner's exact byte consumption including stuffing bits, the sub-block dispatch with all map-table sizes, and top/bottom field offsets plus stuffing validation (#3223).
- Alternative CLUT 8-/10-bit entries are hand-computed byte vectors with distinct component values, and the disparity signalling segment is byte-anchored with two regions, positioned subregions, page and per-subregion update sequences, negative/positive disparities, the 4-subregion ceiling and the position-mismatch rejection (#3224).
- Region, page, CLUT, display-definition and PES data-field layouts are pinned with distinct field values (compatibility/depth/CLUT ids, 12-bit positions, a 3-region page composition with an exact remainder error, full/reduced CLUT entries, the display window, and the PES header/marker boundaries) (#3225).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `dvb-subtitle-v0.10.0`.
