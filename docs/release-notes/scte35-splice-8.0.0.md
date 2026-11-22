# scte35-splice 8.0.0

_Released 2026-11-22._

Major release (7 -> 8). There is no API change of this crate's own: the major exists because it builds against new caret epochs of `broadcast-common` (13 -> 14) and `mpeg-ts` (0.9 -> 0.10), which is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2). The work is the audit test pass over the splice information tables.

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `mpeg-ts` 0.9 -> 0.10). A caret-epoch change is major-class at this crate's version; consumers must resolve a single copy of each at the jiff 1.0 epoch, and the `Timestamp`/PES types this crate takes are those versions' types.

## Tests

Tests decode base64 through `broadcast_common::base64`, and the `base64` dev-dependency is gone (#3957). A segmentation descriptor carrying an all-32-bit `segmentation_event_id`, a 40-bit `segmentation_duration`, a two-entry MID() UPID, two distinct components and the sub-segment appendix is asserted byte-for-byte against hand-assembled Table 20 bytes; each optional section's truncation is a typed error; and the MID() walker's truncated-header and over-long-entry cases are pinned (audit 21 finding 18, #2913). The remaining audit-23 suites pin the splice_insert/command round trips and the reserved-bit policy.

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `scte35-splice-v8.0.0`.
