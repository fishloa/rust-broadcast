# container-probe 0.6.0

_Released 2026-11-22._

Minor for a 0.x crate. One behaviour fix (an MP3 probe at an exact region boundary reported `Unknown` instead of "read more"), the rest is the audit-23 test pass. The breaking part is the epoch move: `broadcast-common` 14, `mpeg-ps` 0.10, `mpeg-ts` 0.10, `st377-1` 0.9.

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `mpeg-ps` 0.9 -> 0.10, `mpeg-ts` 0.9 -> 0.10, `st377-1` 0.8 -> 0.9). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch.

## Fixed

- MP3 probing with a leading ID3v2 tag whose declared length ends exactly at the region boundary now reports `Insufficient` (the first frame has not been examined) instead of `Unknown`, so a streaming caller is told to read more (#3069).

## Tests

The scoring lattice, the per-prober evidence tiers and the near-tie `Ambiguous` rule are exercised against the committed fixtures, with each prober's structural requirement (lane coverage, frame-length chain) pinned so a magic-byte-only regression fails.

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `container-probe-v0.6.0`.
