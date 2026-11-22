# mpeg-ps 0.10.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 and `mpeg-pes` 0.10, which is the breaking part; the PES packets it yields are `mpeg-pes` 0.10 ones. The work is the audit test pass over the program-stream headers.

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `mpeg-pes` 0.9 -> 0.10). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch.

## Tests

The system-header field/flag bit positions and every marker bit, the PSM flags, pseudo-descriptor marker and extension-entry offsets, whole-`Pack` sizing with a system header + PSM + several PES packets, the descriptor-loop boundary cases, and `Scr::seconds`/`BadScrPrefix` payloads are pinned (#3175, #3176, #3177, #3178).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `mpeg-ps-v0.10.0`.
